use crate::api::{ApiClient, CallStats, ProxyCreds};
use crate::config::{self, ApiMode, ColumnSpec, Config, PeppolField};
use crate::csv_io;
use crate::modes::{compute_todo, RunMode};
use crate::output;
use crate::pid::{canonical_line_counts, unique_canonical};
use crate::report;
use crate::resolver::{calibrate, CalibrationReport, Engine, EngineEvent, EngineParams, RunHandle};
use crate::store::Store;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, State};

pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    /// Fichier des réglages auto-persistés (superpopaul.yaml, dossier données).
    pub settings_path: PathBuf,
    pub config: Mutex<Option<Config>>,
    pub proxy_creds: Mutex<Option<ProxyCreds>>,
    pub run: Mutex<Option<Arc<RunHandle>>>,
    /// Photographie du dernier run terminé (snapshot final + nom du fichier
    /// d'entrée), capturée par clear_run au moment où le slot est libéré —
    /// c'est la matière du rapport HTML (export_report).
    pub last_run: Mutex<Option<LastRun>>,
    /// Annulation du calibrage en cours — armée par cancel_calibration,
    /// réarmée à false au début de chaque calibrate_api.
    pub calibrate_cancel: Arc<AtomicBool>,
}

pub struct LastRun {
    pub snapshot: crate::telemetry::Snapshot,
    pub file_name: String,
}

impl AppState {
    pub fn new(store: Store, settings_path: PathBuf) -> Self {
        AppState {
            store: Arc::new(Mutex::new(store)),
            settings_path,
            config: Mutex::new(None),
            proxy_creds: Mutex::new(None),
            run: Mutex::new(None),
            last_run: Mutex::new(None),
            calibrate_cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn current_config(&self) -> Result<Config, String> {
        self.config
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| String::from("Aucune configuration active."))
    }

    fn input_path(&self) -> Result<PathBuf, String> {
        Ok(PathBuf::from(&self.current_config()?.input.path))
    }

    fn client(&self) -> Result<ApiClient, String> {
        let cfg = self.current_config()?;
        let creds = self.proxy_creds.lock().unwrap().clone();
        let proxy = cfg.api.proxy.as_ref().map(|p| p.url.as_str());
        match cfg.api.mode {
            ApiMode::Direct => {
                ApiClient::new_direct(
                    cfg.api.resolver.as_deref(),
                    Some(cfg.api.resolver_fallback.as_str()),
                    cfg.api.dns_concurrency,
                    proxy,
                    creds.as_ref(),
                )
            }
            ApiMode::Api => ApiClient::new(&cfg.api.url, &cfg.api.key, proxy, creds.as_ref()),
        }
    }
}

/// Scan complet du fichier d'entrée : sniff + lecture de colonne + dédup
/// canonique + lignes par PID canonique. BLOQUANT (le fichier peut faire
/// 500k lignes) : à appeler uniquement depuis `tokio::task::spawn_blocking`.
fn scan_unique_pids(
    path: &std::path::Path,
    pid_column: &str,
) -> Result<(csv_io::CsvMeta, Vec<String>, HashMap<String, u64>), String> {
    let meta = csv_io::sniff(path)?;
    let vals = csv_io::read_column(path, &meta, pid_column)?;
    let line_counts = canonical_line_counts(&vals);
    Ok((meta, unique_canonical(vals), line_counts))
}

/// Couverture annuaire déclarative à partir d'un scan déjà effectué. Gate
/// INDÉPENDANT par annuaire (chargé ou non) — miroir des gates de
/// `generate_output`, mais SANS condition « colonne demandée » : le panneau de
/// couverture est indépendant de la config des colonnes de sortie. Comptage
/// par ligne : chaque PID unique est pondéré par son nombre de lignes.
fn coverage_from_scan(
    store: &Store,
    pids: &[String],
    line_counts: &HashMap<String, u64>,
    active_motifs: &[String],
) -> Result<crate::coverage::Coverage, String> {
    let mut eligible: Vec<(String, usize)> = Vec::new();
    let mut non_applicable: usize = 0;
    for p in pids {
        let n = *line_counts.get(p).unwrap_or(&0) as usize;
        match crate::directory::parse_0225_value(p) {
            Some(v) => eligible.push((v, n)),
            None => non_applicable += n,
        }
    }
    let values: Vec<String> = eligible.iter().map(|(v, _)| v.clone()).collect();
    let present = if store.peppol_directory_status()?.is_some() {
        Some(store.directory_present(&values)?)
    } else {
        None
    };
    let ppf = if store.ppf_summary()?.distinct_addr > 0 {
        Some(store.ppf_flags(&values, active_motifs)?)
    } else {
        None
    };
    Ok(crate::coverage::compute(
        &eligible,
        non_applicable,
        present.as_ref(),
        ppf.as_ref(),
    ))
}

/// Sécurisation de la montée en charge à partir d'un scan déjà fait. Gate : les
/// DEUX annuaires doivent être chargés (sinon cœur/pleinement seraient des zéros
/// trompeurs) → `Ok(None)`. Population : lignes du fichier courant, dernier état
/// de résolution connu en base (`load_map`). `ctc_ready` réutilise
/// `output::ctc_status` (parité colonne CSV).
fn securisation_from_scan(
    store: &Store,
    pids: &[String],
    line_counts: &HashMap<String, u64>,
    now: chrono::DateTime<chrono::Utc>,
    active_motifs: &[String],
) -> Result<Option<crate::securisation::Securisation>, String> {
    if store.peppol_directory_status()?.is_none() || store.ppf_summary()?.distinct_addr == 0 {
        return Ok(None);
    }
    let resolutions = store.load_map(pids)?;
    let values: Vec<String> = pids
        .iter()
        .filter_map(|p| crate::directory::parse_0225_value(p))
        .collect();
    let present = store.directory_present(&values)?;
    let ppf = store.ppf_flags(&values, active_motifs)?;

    let mut lines: Vec<crate::securisation::LineFlags> = Vec::with_capacity(pids.len());
    for p in pids {
        let weight = *line_counts.get(p).unwrap_or(&0) as usize;
        let r = resolutions.get(p);
        let in_peppol = r.map(|r| r.exists_in_peppol == Some(true)).unwrap_or(false);
        let ctc_ready = r.map(|r| crate::output::ctc_status(r, now) == "ready").unwrap_or(false);
        let (ppf_usable, in_directory) = match crate::directory::parse_0225_value(p) {
            Some(v) => (ppf.get(&v).map(|f| f.usable).unwrap_or(false), present.contains(&v)),
            None => (false, false),
        };
        lines.push(crate::securisation::LineFlags {
            weight,
            in_peppol,
            ctc_ready,
            ppf_usable,
            in_directory,
        });
    }
    Ok(Some(crate::securisation::compute(&lines)))
}

/// Répartition des lignes par plateforme (PA) à partir d'un scan déjà fait.
/// Population : lignes du fichier courant (`line_counts`), PA du dernier état
/// de résolution connu en base (`load_map`). Regroupement par nom de PA (repli
/// code). Miroir de `securisation_from_scan` ; logique testée dans `repartition`.
fn repartition_from_scan(
    store: &Store,
    pids: &[String],
    line_counts: &HashMap<String, u64>,
) -> Result<crate::repartition::Repartition, String> {
    let resolutions = store.load_map(pids)?;
    let mut entrees: Vec<(Option<String>, u64)> = Vec::with_capacity(pids.len());
    for p in pids {
        let n = *line_counts.get(p).unwrap_or(&0);
        let cle = resolutions
            .get(p)
            .and_then(|r| crate::repartition::pa_key(r.pa_name.as_deref(), r.pa_code.as_deref()));
        entrees.push((cle, n));
    }
    Ok(crate::repartition::compute(&entrees))
}

#[derive(Serialize)]
pub struct PreviewPayload {
    #[serde(flatten)]
    pub preview: csv_io::Preview,
    pub suggested_pid_column: Option<usize>,
}

#[tauri::command]
pub async fn preview_csv(path: String) -> Result<PreviewPayload, String> {
    tokio::task::spawn_blocking(move || {
        let p = csv_io::preview(std::path::Path::new(&path), 5)?;
        let suggested = csv_io::suggest_pid_column(&p);
        Ok(PreviewPayload {
            preview: p,
            suggested_pid_column: suggested,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn set_config(state: State<'_, AppState>, cfg: Config) -> Result<(), String> {
    cfg.validate()?;
    *state.config.lock().unwrap() = Some(cfg);
    Ok(())
}

#[tauri::command]
pub fn load_settings(state: State<'_, AppState>) -> Result<Option<config::Settings>, String> {
    config::load_settings_file(&state.settings_path)
}

#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: config::Settings) -> Result<(), String> {
    config::save_settings_file(&state.settings_path, &settings)
}

/// Mémorise les colonnes désignées pour une signature d'en-têtes, et rend les
/// réglages à jour (l'appelant remplace les siens).
///
/// Prend les réglages courants plutôt que de relire le fichier : au premier
/// lancement il n'existe pas encore, et `Settings` n'a pas de défaut sensé —
/// l'UI, elle, en tient toujours un valide.
#[tauri::command]
pub fn remember_columns(
    state: State<'_, AppState>,
    settings: config::Settings,
    mapping: config::ColumnMapping,
) -> Result<config::Settings, String> {
    let mut s = settings;
    config::memoriser_mapping(&mut s.mappings, mapping);
    config::save_settings_file(&state.settings_path, &s)?;
    Ok(s)
}

/// Some(répertoire) si le mode portable est actif — sert de defaultPath aux
/// dialogues de profils ; None en mode installé (comportement OS inchangé).
#[tauri::command]
pub fn portable_dir() -> Option<String> {
    config::portable_dir_of_current_exe().map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn load_profile(path: String) -> Result<config::Profile, String> {
    config::load_profile_file(Path::new(&path))
}

#[tauri::command]
pub fn save_profile(path: String, profile: config::Profile) -> Result<(), String> {
    config::save_profile_file(Path::new(&path), &profile)
}

#[tauri::command]
pub fn set_proxy_creds(
    state: State<'_, AppState>,
    username: String,
    password: String,
) -> Result<(), String> {
    *state.proxy_creds.lock().unwrap() = Some(ProxyCreds { username, password });
    // Un run actif suspendu pour auth_proxy (407) ne peut pas juste changer
    // de clé : les creds proxy vivent dans le builder reqwest, il faut donc
    // un client entier neuf pour reprendre.
    if let Some(h) = state.run.lock().unwrap().as_ref() {
        let client = state.client()?;
        h.update_client(client);
    }
    Ok(())
}

#[tauri::command]
pub fn update_api_key(state: State<'_, AppState>, key: String) -> Result<(), String> {
    if let Some(cfg) = state.config.lock().unwrap().as_mut() {
        cfg.api.key = key;
    }
    // Un client entier neuf (plutôt que la seule clé) : le canal watch porte
    // ainsi toujours l'état complet, ce qui ferme un entrelacement
    // last-value-wins avec set_proxy_creds (même discipline de verrous : la
    // config est libérée avant de reconstruire le client).
    if let Some(h) = state.run.lock().unwrap().as_ref() {
        let client = state.client()?;
        // update_client lève déjà la suspension système (auth_api/auth_proxy)
        // et relance les workers. On ne touche PAS à set_paused ici : la
        // pause utilisateur (bouton Pause) appartient à l'utilisateur, une
        // nouvelle clé API ne doit pas la lever à sa place.
        h.update_client(client);
    }
    Ok(())
}

#[tauri::command]
pub async fn test_api(state: State<'_, AppState>) -> Result<CallStats, String> {
    let client = state.client()?;
    client.test_key().await.map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct InputStats {
    pub unique: usize,
    pub resolved_ok: usize,
    pub failed: usize,
    pub stale: usize,
    pub missing: usize,
    pub coverage: crate::coverage::Coverage,
}

/// Compare le fichier d'entrée à la base : alimente la popup de reprise et la
/// présélection du mode.
#[tauri::command]
pub async fn analyze_input(state: State<'_, AppState>) -> Result<InputStats, String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    // Scan CSV (500k lignes possibles) + load_map SQLite : bloquants, hors
    // executor tokio.
    tokio::task::spawn_blocking(move || {
        let (_, pids, line_counts) = scan_unique_pids(&input, &cfg.input.pid_column)?;
        let store_g = store.lock().unwrap();
        let known = store_g.load_map(&pids)?;
        let coverage = coverage_from_scan(&store_g, &pids, &line_counts, &cfg.ppf.motifs())?;
        drop(store_g);
        let now = chrono::Utc::now().timestamp();
        let max_age = cfg.api.refresh_days as i64 * 86400;
        let (mut ok, mut failed, mut stale) = (0, 0, 0);
        for p in &pids {
            match known.get(p) {
                None => {}
                Some(r) if r.api_status != "ok" => failed += 1,
                Some(r) if r.resolved_at < now - max_age => stale += 1,
                Some(_) => ok += 1,
            }
        }
        Ok(InputStats {
            unique: pids.len(),
            resolved_ok: ok,
            failed,
            stale,
            missing: pids.len() - ok - failed - stale,
            coverage,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Prérequis du calibrage (mode API) : une clé et un fichier d'entrée.
/// Le message liste TOUT ce qui manque — l'utilisateur ne doit pas découvrir
/// le second prérequis après avoir corrigé le premier.
fn calibration_prerequisites(key: &str, input_path: &str) -> Result<(), String> {
    let missing: Vec<&str> = [
        (key.trim().is_empty(), "une clé API"),
        (
            input_path.trim().is_empty(),
            "un fichier d'entrée (l'échantillon vient de vos adressages)",
        ),
    ]
    .iter()
    .filter_map(|&(absent, label)| absent.then_some(label))
    .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("Calibration impossible : il manque {}.", missing.join(" et ")))
    }
}

#[tauri::command]
pub async fn calibrate_api(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CalibrationReport, String> {
    let cfg = state.current_config()?;
    if cfg.api.mode == ApiMode::Direct {
        // Marteler les SMP distribués pour trouver un plafond n'a pas de
        // sens (et serait impoli) : il n'y a pas de serveur unique à calibrer.
        return Err("Calibration sans objet en mode direct (SMP distribués).".into());
    }
    calibration_prerequisites(&cfg.api.key, &cfg.input.path)?;
    let client = state.client()?;
    let input = state.input_path()?;
    let pid_column = cfg.input.pid_column.clone();
    // Scan CSV bloquant hors executor ; calibrate() reste async ici.
    let mut sample =
        tokio::task::spawn_blocking(move || scan_unique_pids(&input, &pid_column).map(|(_, p, _)| p))
            .await
            .map_err(|e| e.to_string())??;
    sample.truncate(64);
    if sample.is_empty() {
        return Err("Aucun adressage dans le fichier d'entrée.".into());
    }
    state.calibrate_cancel.store(false, Ordering::Relaxed);
    let cancel = state.calibrate_cancel.clone();
    Ok(calibrate(
        &client,
        &sample,
        cfg.api.batch_size as usize,
        cfg.api.concurrency.max(16),
        &cancel,
        |step| {
            let _ = app.emit("calibrate-step", &step);
        },
    )
    .await)
}

/// Arme l'annulation de la calibration en cours (coopérative : le palier en
/// cours se termine). Sans effet si aucune calibration n'est active.
#[tauri::command]
pub fn cancel_calibration(state: State<'_, AppState>) {
    state.calibrate_cancel.store(true, Ordering::Relaxed);
}

#[tauri::command]
pub async fn start_run(
    app: AppHandle,
    state: State<'_, AppState>,
    mode: RunMode,
) -> Result<u64, String> {
    // Refus rapide avant le scan (le garde définitif est plus bas, sous le
    // verrou, car le spawn_blocking introduit un await).
    if state.run.lock().unwrap().is_some() {
        return Err("Un run est déjà en cours.".into());
    }
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let pid_column = cfg.input.pid_column.clone();
    let store = state.store.clone();
    // Scan CSV + compute_todo (load_map SQLite) : bloquants, hors executor.
    let (todo, line_counts) = tokio::task::spawn_blocking(move || {
        let (_, pids, line_counts) = scan_unique_pids(&input, &pid_column)?;
        let now = chrono::Utc::now().timestamp();
        let store = store.lock().unwrap();
        Ok::<_, String>((compute_todo(&mode, &pids, &store, now)?, line_counts))
    })
    .await
    .map_err(|e| e.to_string())??;
    let total = todo.len() as u64;
    let client = state.client()?;
    // Derrière un proxy en mode direct : sonde avant de lancer — un proxy
    // qui refuse le tunnel (créds faux → 403 au CONNECT chez beaucoup de
    // proxys, jamais détectable en 407) ferait labourer tout le fichier
    // en erreurs (run du 15/07/2026).
    if cfg.api.mode == ApiMode::Direct && cfg.api.proxy.is_some() {
        client.preflight_proxy().await?;
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    {
        // Garde définitif : re-vérifie et installe sous LE MÊME verrou
        // (Engine::start est synchrone et rapide : il ne fait que spawner).
        // Sans cela, deux start_run concurrents passés du premier garde
        // créeraient deux moteurs.
        let mut guard = state.run.lock().unwrap();
        if guard.is_some() {
            return Err("Un run est déjà en cours.".into());
        }
        *guard = Some(Arc::new(Engine::start(
            client,
            EngineParams {
                // En direct, chaque adressage a son propre pipeline DNS+SMP :
                // paquets de 1 pour que latences et codes HTTP du cockpit
                // restent par adressage.
                batch_size: if cfg.api.mode == ApiMode::Direct {
                    1
                } else {
                    cfg.api.batch_size as usize
                },
                concurrency: cfg.api.concurrency,
            },
            todo,
            line_counts,
            state.store.clone(),
            tx,
        )));
    }
    // Pont événements moteur → webview.
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = rx.recv().await {
            match ev {
                EngineEvent::Telemetry(s) => {
                    let _ = app.emit("telemetry", &s);
                }
                EngineEvent::Suspended {
                    reason,
                    message,
                    retry_in_s,
                } => {
                    let _ = app.emit(
                        "run-suspended",
                        serde_json::json!({
                            "reason": reason, "message": message, "retry_in_s": retry_in_s
                        }),
                    );
                }
                EngineEvent::Resumed => {
                    let _ = app.emit("run-resumed", serde_json::json!({}));
                }
                EngineEvent::Finished {
                    done,
                    failed,
                    stopped,
                    active_s,
                } => {
                    let _ = app.emit(
                        "run-finished",
                        serde_json::json!({
                            "done": done, "failed": failed, "stopped": stopped,
                            "active_s": active_s
                        }),
                    );
                    break;
                }
            }
        }
    });
    Ok(total)
}

#[tauri::command]
pub fn pause_run(state: State<'_, AppState>, paused: bool) -> Result<(), String> {
    match state.run.lock().unwrap().as_ref() {
        Some(h) => {
            h.set_paused(paused);
            Ok(())
        }
        None => Err("Aucun run en cours.".into()),
    }
}

/// Reprise anticipée d'une suspension système (bannière server_down, bouton
/// « Réessayer maintenant ») : même effet que le timer de backoff du moteur,
/// sans attendre son expiration. `pause_run` ne convient pas ici : il ne
/// pilote que la pause utilisateur, pas la suspension système.
#[tauri::command]
pub fn resume_run(state: State<'_, AppState>) -> Result<(), String> {
    match state.run.lock().unwrap().as_ref() {
        Some(h) => {
            h.resume_system();
            Ok(())
        }
        None => Err("Aucun run en cours.".into()),
    }
}

#[tauri::command]
pub fn stop_run(state: State<'_, AppState>) -> Result<(), String> {
    // Contrat : le slot n'est PAS libéré ici — uniquement via clear_run,
    // appelé par le front à la réception de run-finished. Après request_stop,
    // le moteur draine encore ses requêtes en vol (jusqu'à ~75 s de timeout
    // HTTP) ; le slot occupé fait que le garde de start_run bloque toute
    // relance pendant le drain. Vider le slot ici permettrait un deuxième
    // moteur concurrent, dont le handle serait ensuite effacé par le
    // clear_run déclenché par le run-finished tardif du vieux run.
    match state.run.lock().unwrap().as_ref() {
        Some(h) => {
            h.request_stop();
            Ok(())
        }
        None => Err("Aucun run en cours.".into()),
    }
}

/// À appeler quand run-finished est reçu côté UI, pour libérer le slot.
/// Le run libéré est photographié dans `last_run` (snapshot final + nom du
/// fichier d'entrée) : c'est ce que le rapport HTML exporte.
#[tauri::command]
pub fn clear_run(state: State<'_, AppState>) {
    if let Some(h) = state.run.lock().unwrap().take() {
        let file_name = state
            .input_path()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        *state.last_run.lock().unwrap() = Some(LastRun {
            snapshot: h.telemetry.snapshot(),
            file_name,
        });
    }
}

/// Écrit le rapport HTML du dernier run terminé, à côté du fichier de sortie
/// (mêmes règles de répertoire que generate_output), et rend son chemin.
#[tauri::command]
pub async fn export_report(state: State<'_, AppState>) -> Result<String, String> {
    let (snapshot, file_name) = {
        let last = state.last_run.lock().unwrap();
        let last = last
            .as_ref()
            .ok_or_else(|| String::from("Aucun run terminé à rapporter."))?;
        (last.snapshot.clone(), last.file_name.clone())
    };
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    // Scan CSV + requêtes store : bloquants, hors executor tokio.
    tokio::task::spawn_blocking(move || {
        // Agrégats annuaire/sécurisation sur l'entrée COURANTE, un seul scan.
        // Tolérant : entrée illisible → rapport sans ces sections.
        let (coverage, securisation, repartition) = match scan_unique_pids(&input, &cfg.input.pid_column) {
            Ok((_, pids, line_counts)) => {
                let now_utc = chrono::Utc::now();
                let store_g = store.lock().unwrap();
                let cov = coverage_from_scan(&store_g, &pids, &line_counts, &cfg.ppf.motifs())
                    .unwrap_or(crate::coverage::Coverage::EMPTY);
                let secu =
                    securisation_from_scan(&store_g, &pids, &line_counts, now_utc, &cfg.ppf.motifs())
                        .ok()
                        .flatten();
                let rep = repartition_from_scan(&store_g, &pids, &line_counts).ok();
                (cov, secu, rep)
            }
            Err(_) => (crate::coverage::Coverage::EMPTY, None, None),
        };
        let now = chrono::Local::now();
        let ppf_active_label = cfg.ppf.active_label();
        let html = report::render(&report::ReportData {
            file_name: &file_name,
            date_longue: &report::date_fr_longue(&now),
            date_heure: &now.format("%d/%m/%Y %H:%M").to_string(),
            today: now.date_naive(),
            version: env!("CARGO_PKG_VERSION"),
            snapshot: &snapshot,
            record_plural: cfg.input.record_label.plural(),
            ppf_active_label: &ppf_active_label,
            coverage: &coverage,
            securisation: securisation.as_ref(),
            repartition_pa: repartition.as_ref(),
        });
        let out = resolved_out_dir(&input, &cfg.output.dir).join(format!(
            "{}_rapport.html",
            input.file_stem().unwrap_or_default().to_string_lossy()
        ));
        std::fs::write(&out, html).map_err(|e| format!("écriture du rapport : {e}"))?;
        Ok(out.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Répertoire de sortie effectif : celui des réglages (superpopaul.yaml) ;
/// un chemin relatif (ou vide) se résout contre le dossier du fichier
/// d'entrée — join("") le laisse tel quel.
fn resolved_out_dir(input: &Path, dir: &str) -> PathBuf {
    let d = Path::new(dir);
    if d.is_absolute() {
        d.to_path_buf()
    } else {
        input.parent().unwrap_or(Path::new(".")).join(d)
    }
}

#[tauri::command]
pub async fn generate_output(state: State<'_, AppState>) -> Result<String, String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    // Tout le corps est bloquant (scan CSV, load_map SQLite, écriture CSV) :
    // hors executor tokio.
    tokio::task::spawn_blocking(move || {
        let (meta, pids, _) = scan_unique_pids(&input, &cfg.input.pid_column)?;
        // Contention assumée : pendant un run actif, ce load_map tient le
        // Mutex<Store> et gèle brièvement les upsert_batch des workers (une
        // seule Connection SQLite). Alternative future si ça pique : une 2e
        // connexion lecture seule (le WAL permet lectures // écritures).
        let resolutions = store.lock().unwrap().load_map(&pids)?;
        // Présence annuaire : uniquement si la colonne est demandée ET
        // l'annuaire chargé (sinon None → colonne vide côté output).
        let wants_dir = cfg
            .output
            .columns
            .iter()
            .any(|c| matches!(c, ColumnSpec::Peppol { field: PeppolField::InDirectory }));
        let directory = if wants_dir {
            let s = store.lock().unwrap();
            if s.peppol_directory_status()?.is_some() {
                let vals: Vec<String> = pids
                    .iter()
                    .filter_map(|p| crate::directory::parse_0225_value(p))
                    .collect();
                Some(s.directory_present(&vals)?)
            } else {
                None
            }
        } else {
            None
        };
        // Drapeaux PPF : uniquement si une colonne PPF est demandée ET
        // l'annuaire PPF est non vide (sinon None → colonnes vides). Miroir du
        // gate `directory` ci-dessus.
        let wants_ppf = cfg.output.columns.iter().any(|c| {
            matches!(
                c,
                ColumnSpec::Peppol { field: PeppolField::AnnuairePpf }
                    | ColumnSpec::Peppol { field: PeppolField::PpfActive }
                    | ColumnSpec::Peppol { field: PeppolField::PdpDefinie }
                    | ColumnSpec::Peppol { field: PeppolField::PpfUsable }
            )
        });
        let ppf = if wants_ppf {
            let s = store.lock().unwrap();
            if s.ppf_summary()?.distinct_addr > 0 {
                let ids: Vec<String> = pids
                    .iter()
                    .filter_map(|p| crate::directory::parse_0225_value(p))
                    .collect();
                Some(s.ppf_flags(&ids, &cfg.ppf.motifs())?)
            } else {
                None
            }
        } else {
            None
        };
        let out = resolved_out_dir(&input, &cfg.output.dir)
            .join(output::out_file_name(&input, &cfg.output.suffix));
        let stamp = cfg
            .output
            .timestamp_suffix
            .then(|| chrono::Local::now().format("%Y%m%d-%H%M").to_string());
        let written = output::generate(
            &input,
            &meta,
            &cfg.input.pid_column,
            &cfg.output,
            &resolutions,
            directory.as_ref(),
            ppf.as_ref(),
            &out,
            stamp.as_deref(),
        )?;
        Ok(written.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// Plan de charge (Runs de Facturation)
// ---------------------------------------------------------------------------

/// Scan du fichier courant + jointures, converti en entrées du moteur de plan.
/// Glue impure, non testée unitairement (convention des `*_from_scan`) : toute
/// la logique vit dans `plan.rs`.
fn plan_entrees_from_scan(
    store: &Store,
    input: &Path,
    cfg: &Config,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<crate::plan::LigneEntree>, String> {
    if cfg.input.cf_column.is_empty() || cfg.input.jj_column.is_empty() {
        return Err("désigne les colonnes « compte de facturation » et « jour de cycle » \
                    avant d'établir un plan de charge"
            .into());
    }
    let meta = csv_io::sniff(input)?;
    // Un seul passage pour toutes les colonnes du plan.
    let mut noms: Vec<&str> = vec![
        cfg.input.cf_column.as_str(),
        cfg.input.pid_column.as_str(),
        cfg.input.jj_column.as_str(),
    ];
    let veut_rs = !cfg.input.raison_sociale_column.is_empty();
    if veut_rs {
        noms.push(cfg.input.raison_sociale_column.as_str());
    }
    let cols = csv_io::read_columns(input, &meta, &noms)?;
    let (cfs, pids, jjs) = (&cols[0], &cols[1], &cols[2]);

    let uniques = unique_canonical(pids.clone());
    let resolutions = store.load_map(&uniques)?;
    let valeurs: Vec<String> = uniques
        .iter()
        .filter_map(|p| crate::directory::parse_0225_value(p))
        .collect();
    let present = if store.peppol_directory_status()?.is_some() {
        store.directory_present(&valeurs)?
    } else {
        HashSet::new()
    };
    let ppf = if store.ppf_summary()?.distinct_addr > 0 {
        store.ppf_flags(&valeurs, &cfg.ppf.motifs())?
    } else {
        HashMap::new()
    };

    let mut out = Vec::with_capacity(cfs.len());
    for (i, cf_brut) in cfs.iter().enumerate() {
        let cf = cf_brut.trim();
        if cf.is_empty() {
            continue; // une ligne sans compte de facturation n'est pas planifiable
        }
        let participant = crate::pid::canonical(pids.get(i).map(String::as_str).unwrap_or(""));
        let r = resolutions.get(&participant);
        let pa = r
            .and_then(|r| crate::repartition::pa_key(r.pa_name.as_deref(), r.pa_code.as_deref()))
            .unwrap_or_default();
        // « Résolu » vaut aussi « avec une plateforme identifiée » : les quotas
        // sont par plateforme, un compte sans PA n'y a pas sa place.
        let resolu = r.map(|r| r.api_status == "ok").unwrap_or(false) && !pa.is_empty();
        let ctc_status = r.map(|r| output::ctc_status(r, now)).unwrap_or("").to_string();
        let ctc_ready = ctc_status == "ready";
        let (ppf_usable, in_directory) = match crate::directory::parse_0225_value(&participant) {
            Some(v) => (
                ppf.get(&v).map(|f| f.usable).unwrap_or(false),
                present.contains(&v),
            ),
            None => (false, false),
        };
        out.push(crate::plan::LigneEntree {
            cf: cf.to_string(),
            participant,
            jj_brut: jjs.get(i).cloned().unwrap_or_default(),
            raison_sociale: if veut_rs {
                cols[3].get(i).cloned().unwrap_or_default()
            } else {
                String::new()
            },
            pa,
            resolu,
            ctc_ready,
            ctc_status,
            ppf_usable,
            in_directory,
            resolved_at: r.map(|r| r.resolved_at).unwrap_or(0),
        });
    }
    Ok(out)
}

#[derive(Serialize)]
pub struct RunsImport {
    pub runs: Vec<crate::plan::RunParam>,
    pub erreurs: Vec<String>,
}

/// Parse un `runs.csv`. Les erreurs sont rendues toutes ensemble, avec les
/// lignes valides : corriger un fichier erreur après erreur serait pénible.
#[tauri::command]
pub async fn plan_import_runs(path: String) -> Result<RunsImport, String> {
    tokio::task::spawn_blocking(move || {
        let p = PathBuf::from(&path);
        let meta = csv_io::sniff(&p)?;
        let brut = std::fs::read(&p).map_err(|e| format!("lecture {p:?} : {e}"))?;
        let texte = if meta.encoding == "utf-8" {
            String::from_utf8_lossy(&brut).into_owned()
        } else {
            encoding_rs::WINDOWS_1252.decode(&brut).0.into_owned()
        };
        let (runs, erreurs) = crate::calendrier::parse_runs_csv(&texte);
        Ok(RunsImport {
            runs: runs
                .into_iter()
                .map(|r| crate::plan::RunParam {
                    num: r.num,
                    date: r.date.to_string(),
                    jjs: r.jjs,
                    exclu: r.exclu,
                })
                .collect(),
            erreurs,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Ce que l'écran de plan affiche après un calcul.
#[derive(Serialize)]
pub struct PlanApercu {
    pub funnel: crate::plan::Funnel,
    pub timeline: Vec<crate::timeline::JourTimeline>,
    pub stock_jj: Vec<crate::plan::StockJJ>,
    pub plateformes: Vec<PlateformeApercu>,
    pub avertissements: Vec<String>,
    pub meps: Vec<String>,
    /// Cible effective (celle saisie, ou la taille du pool si elle est vide).
    pub cible: usize,
    pub total: usize,
    pub geles: usize,
    pub epingles: usize,
    pub retires: usize,
}

#[derive(Serialize)]
pub struct PlateformeApercu {
    pub nom: String,
    pub eligibles: usize,
    pub quota: usize,
}

/// Calcule le plan SANS rien écrire. C'est le vrai calcul, pas une
/// approximation : explorer des scénarios ne coûte donc rien, et la
/// persistance ne sert qu'à figer et livrer.
#[tauri::command]
pub async fn plan_preview(
    state: State<'_, AppState>,
    params: crate::plan::PlanParams,
) -> Result<PlanApercu, String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let (apercu, _, _) = calculer_plan(&store, &input, &cfg, &params)?;
        Ok(apercu)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Cœur partagé par l'aperçu et la génération : construit le pool, régénère,
/// et rend l'aperçu, le plan complet et les lignes du fichier d'entrée. Ces
/// dernières remontent pour que la génération écrive le classeur du périmètre
/// sans relire le CSV ni refaire la jointure de résolution.
fn calculer_plan(
    store: &Arc<Mutex<Store>>,
    input: &Path,
    cfg: &Config,
    params: &crate::plan::PlanParams,
) -> Result<
    (
        PlanApercu,
        Vec<crate::plan::LignePlan>,
        Vec<crate::plan::LigneEntree>,
    ),
    String,
> {
    let now = chrono::Utc::now();
    let aujourdhui = chrono::Local::now().date_naive();
    let (runs, debut, fin, meps_fournies) = params.calendrier()?;

    let entrees = {
        let s = store.lock().unwrap();
        plan_entrees_from_scan(&s, input, cfg, now)?
    };
    let (pool, funnel) = crate::plan::construire_pool(&entrees, &params.pa_exclues())?;

    let (meps, mut avertissements) =
        crate::calendrier::completer_meps(&runs, debut, fin, &meps_fournies, params.mep_count);
    // Un mapping fautif vidait l'écran en silence : le funnel tombait à zéro
    // dès la marche des jours de cycle, sans dire que la colonne était en
    // cause. C'est le seul endroit qui connaisse à la fois le funnel et le nom
    // de la colonne désignée.
    avertissements.extend(crate::plan::alerte_colonne_jj(&funnel, &cfg.input.jj_column));
    let utilisables = crate::calendrier::runs_utilisables(&runs, debut, fin, &meps);

    // Plan existant : ce qui doit survivre au re-tirage.
    let ancien = store.lock().unwrap().charger_plan()?;
    let preserves = match &ancien {
        Some((lignes, _)) => crate::plan::Preserves::depuis(lignes, aujourdhui),
        None => crate::plan::Preserves::default(),
    };

    let cible = params.cible.unwrap_or(pool.len() + preserves.consomme());
    let a = crate::plan::regenerer(
        &pool,
        &utilisables,
        &meps,
        params.seed,
        cible,
        &params.rampe,
        &preserves,
    )?;
    avertissements.extend(a.avertissements.clone());

    let stock: std::collections::BTreeMap<String, usize> =
        pool.iter().fold(Default::default(), |mut m, c| {
            *m.entry(c.pa.clone()).or_insert(0) += 1;
            m
        });
    let quotas = crate::plan::quotas_par_pa(cible, &stock);
    let mut plateformes: Vec<PlateformeApercu> = stock
        .iter()
        .map(|(nom, n)| PlateformeApercu {
            nom: nom.clone(),
            eligibles: *n,
            quota: quotas.get(nom).copied().unwrap_or(0),
        })
        .collect();
    plateformes.sort_by(|a, b| b.eligibles.cmp(&a.eligibles).then_with(|| a.nom.cmp(&b.nom)));

    let timeline = crate::timeline::timeline(&runs, debut, fin, &meps, &a.details);
    let stock_jj = crate::plan::stock_par_jj(&pool, &utilisables);

    let actives = a.lignes.iter().filter(|l| !l.retiree()).count();
    let apercu = PlanApercu {
        funnel,
        timeline,
        stock_jj,
        plateformes,
        avertissements,
        meps: meps.iter().map(|m| m.to_string()).collect(),
        cible,
        total: actives,
        geles: preserves.gelees.len(),
        epingles: preserves.epinglees.len(),
        retires: a.lignes.iter().filter(|l| l.retiree()).count(),
    };
    Ok((apercu, a.lignes, entrees))
}

/// Calcule le plan ET l'écrit (lignes + paramètres dans une transaction),
/// puis produit les fichiers par MEP.
#[tauri::command]
pub async fn plan_generate(
    state: State<'_, AppState>,
    params: crate::plan::PlanParams,
) -> Result<PlanGeneration, String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let (apercu, lignes, entrees) = calculer_plan(&store, &input, &cfg, &params)?;
        let horodatage = chrono::Utc::now().timestamp();
        let lignes: Vec<crate::plan::LignePlan> = lignes
            .into_iter()
            .map(|mut l| {
                if l.planned_at == 0 {
                    l.planned_at = horodatage;
                }
                l
            })
            .collect();
        let meta = crate::store::PlanMeta {
            fichier: input
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            hash: sha256_hex(&std::fs::read(&input).map_err(|e| format!("lecture entrée : {e}"))?),
            genere_le: horodatage,
            params_yaml: params.vers_yaml()?,
        };
        store.lock().unwrap().ecrire_plan(&lignes, &meta)?;
        let fichiers = ecrire_fichiers_mep(&input, &cfg, &lignes)?;
        // Le classeur du périmètre part avec les fichiers de livraison : ce
        // qu'on transmet et ce qui le documente restent ainsi cohérents.
        // `ecrire_fichiers_mep` a déjà créé le répertoire de sortie.
        let xlsx = resolved_out_dir(&input, &cfg.output.dir).join(format!(
            "{}_plan_comptes.xlsx",
            input.file_stem().unwrap_or_default().to_string_lossy()
        ));
        crate::plan_xlsx::ecrire(&xlsx, &crate::plan_xlsx::lignes(&entrees, &lignes))?;
        Ok(PlanGeneration { apercu, fichiers })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct PlanGeneration {
    pub apercu: PlanApercu,
    pub fichiers: Vec<FichierMep>,
}

#[derive(Serialize)]
pub struct FichierMep {
    pub chemin: String,
    pub mep_id: usize,
    pub comptes: usize,
}

/// Un fichier par MEP, **cumulatif** (MEP 1..n), comptes nus triés, un par
/// ligne. Les lignes retirées sont exclues de TOUS les fichiers, y compris sur
/// une MEP gelée — c'est l'objet même du retrait.
fn ecrire_fichiers_mep(
    input: &Path,
    cfg: &Config,
    lignes: &[crate::plan::LignePlan],
) -> Result<Vec<FichierMep>, String> {
    let dir = resolved_out_dir(input, &cfg.output.dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("création {dir:?} : {e}"))?;
    let souche = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sortie".into());

    let mut meps: Vec<(usize, String)> = lignes
        .iter()
        .filter(|l| !l.retiree())
        .map(|l| (l.mep_id, l.mep_date.to_string()))
        .collect();
    meps.sort();
    meps.dedup();

    let mut out = Vec::new();
    for (mep_id, mep_date) in meps {
        let mut comptes: Vec<&str> = lignes
            .iter()
            .filter(|l| !l.retiree() && l.mep_id <= mep_id)
            .map(|l| l.cf.as_str())
            .collect();
        comptes.sort_unstable();
        comptes.dedup();
        let chemin = dir.join(format!("{souche}_plan_mep_{mep_id}_{mep_date}.txt"));
        let mut contenu = comptes.join("\n");
        contenu.push('\n');
        std::fs::write(&chemin, contenu).map_err(|e| format!("écriture {chemin:?} : {e}"))?;
        out.push(FichierMep {
            chemin: chemin.display().to_string(),
            mep_id,
            comptes: comptes.len(),
        });
    }
    Ok(out)
}

#[derive(Serialize)]
pub struct PlanEnregistre {
    pub params: crate::plan::PlanParams,
    pub fichier: String,
    pub genere_le: i64,
    /// Vrai si le fichier d'entrée courant n'est pas celui qui a produit le
    /// plan : les lignes gelées peuvent alors ne plus correspondre.
    pub autre_fichier: bool,
}

/// État persisté, au retour sur l'écran de plan.
#[tauri::command]
pub async fn plan_load(state: State<'_, AppState>) -> Result<Option<PlanEnregistre>, String> {
    let input = state.input_path().ok();
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let Some((_, meta)) = store.lock().unwrap().charger_plan()? else {
            return Ok(None);
        };
        let courant = input
            .as_ref()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        Ok(Some(PlanEnregistre {
            params: crate::plan::PlanParams::depuis_yaml(&meta.params_yaml)?,
            autre_fichier: !meta.fichier.is_empty() && meta.fichier != courant,
            fichier: meta.fichier,
            genere_le: meta.genere_le,
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Une ligne du plan telle que l'onglet « Comptes de facturation » l'affiche.
/// L'état d'éligibilité est **recalculé** à chaque lecture, jamais figé : un
/// compte peut être devenu inéligible depuis le tirage.
#[derive(Serialize)]
pub struct LigneRecap {
    pub cf: String,
    pub participant: String,
    pub raison_sociale: String,
    pub jj: u8,
    pub pa: String,
    pub mep_id: usize,
    pub mep_date: String,
    pub run_num: String,
    pub run_date: String,
    pub origine: String,
    pub gelee: bool,
    pub retire_motif: Option<String>,
    /// « eligible » · « ctc_non_pret » · « ppf_non_utilisable » ·
    /// « absent_du_fichier ».
    pub etat: String,
}

/// Un compte du fichier absent du plan, proposable à l'ajout.
#[derive(Serialize)]
pub struct Candidat {
    pub cf: String,
    pub raison_sociale: String,
    pub jj: u8,
    pub pa: String,
    /// Agrégat qui décide du marquage ⚠ : CTC prêt ET PPF utilisable.
    pub eligible: bool,
    /// Adressage sous forme nue (`0225:…`) quand le schéma s'y prête.
    pub participant: String,
    /// `"ready"` | `"later"` | `"expired"` | `""` — jamais aplati.
    pub ctc_status: String,
    pub ppf_usable: bool,
}

fn etat_de(e: Option<&crate::plan::LigneEntree>) -> String {
    match e {
        None => "absent_du_fichier".into(),
        Some(e) if !e.ctc_ready => "ctc_non_pret".into(),
        Some(e) if !e.ppf_usable => "ppf_non_utilisable".into(),
        _ => "eligible".into(),
    }
}

/// Récapitulatif complet du plan. Le filtrage et le tri vivent côté IHM : la
/// volumétrie (quelques milliers de lignes) ne justifie pas de les redescendre.
#[tauri::command]
pub async fn plan_lignes(state: State<'_, AppState>) -> Result<Vec<LigneRecap>, String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let Some((lignes, _)) = store.lock().unwrap().charger_plan()? else {
            return Ok(Vec::new());
        };
        let par_cf = entrees_par_cf(&store, &input, &cfg)?;
        let aujourdhui = chrono::Local::now().date_naive();
        Ok(lignes
            .into_iter()
            .map(|l| LigneRecap {
                etat: etat_de(par_cf.get(&l.cf)),
                gelee: l.gelee(aujourdhui),
                retire_motif: l.retire.as_ref().map(|r| r.motif.clone()),
                origine: match l.origine {
                    crate::plan::Origine::Auto => "auto",
                    crate::plan::Origine::Couverture => "couverture",
                    crate::plan::Origine::Manuel => "manuel",
                }
                .into(),
                mep_date: l.mep_date.to_string(),
                run_date: l.run_date.to_string(),
                cf: l.cf,
                participant: l.participant,
                raison_sociale: l.raison_sociale,
                jj: l.jj,
                pa: l.pa,
                mep_id: l.mep_id,
                run_num: l.run_num,
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn entrees_par_cf(
    store: &Arc<Mutex<Store>>,
    input: &Path,
    cfg: &Config,
) -> Result<HashMap<String, crate::plan::LigneEntree>, String> {
    let now = chrono::Utc::now();
    let s = store.lock().unwrap();
    let entrees = plan_entrees_from_scan(&s, input, cfg, now)?;
    Ok(entrees.into_iter().map(|e| (e.cf.clone(), e)).collect())
}

/// Comptes proposables sur un run : jour de cycle couvert et absents du plan.
///
/// Le filtre par jour de cycle est une contrainte arithmétique — un run ne peut
/// pas facturer un autre jour. En revanche un compte non éligible (CTC non prêt,
/// PPF non utilisable) est proposé et **signalé** : le forcer est un choix assumé.
fn candidats_du_run(
    entrees: &[crate::plan::LigneEntree],
    run: &crate::calendrier::RunFacturation,
    deja_au_plan: &HashSet<String>,
) -> Vec<Candidat> {
    entrees
        .iter()
        .filter(|e| !deja_au_plan.contains(&e.cf))
        .filter_map(|e| {
            let jj = crate::plan::parse_jj(&e.jj_brut)?;
            run.couvre(jj).then(|| Candidat {
                cf: e.cf.clone(),
                raison_sociale: e.raison_sociale.clone(),
                jj,
                pa: e.pa.clone(),
                eligible: e.ctc_ready && e.ppf_usable,
                participant: crate::directory::parse_0225_value(&e.participant)
                    .unwrap_or_else(|| e.participant.clone()),
                ctc_status: e.ctc_status.clone(),
                ppf_usable: e.ppf_usable,
            })
        })
        .collect()
}

/// Comptes proposables sur un run donné. Le run est le point d'entrée : on
/// choisit d'abord où livrer, ensuite quoi y mettre.
#[tauri::command]
pub async fn plan_candidats_run(
    state: State<'_, AppState>,
    run_num: String,
) -> Result<Vec<Candidat>, String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let (lignes, meta) = charger_pour_retouche(&store)?;
        let (runs, _) = calendrier_du_plan(&meta)?;
        // Run inconnu ou écarté : une erreur nommée, pas une liste vide qui
        // ferait croire qu'aucun compte n'est proposable.
        let run = runs
            .iter()
            .find(|r| r.num == run_num)
            .ok_or_else(|| format!("run « {run_num} » inconnu ou écarté du plan"))?;
        let deja: HashSet<String> = lignes.into_iter().map(|l| l.cf).collect();
        let entrees: Vec<crate::plan::LigneEntree> =
            entrees_par_cf(&store, &input, &cfg)?.into_values().collect();
        let mut out = candidats_du_run(&entrees, run, &deja);
        // `entrees_par_cf` rend une table de hachage : sans tri, l'ordre de la
        // fenêtre d'ajout changerait à chaque ouverture.
        out.sort_by(|a, b| a.cf.cmp(&b.cf));
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Charge le plan pour retouche. Un plan absent est une erreur explicite :
/// retoucher ce qui n'existe pas n'a pas de sens.
fn charger_pour_retouche(
    store: &Arc<Mutex<Store>>,
) -> Result<(Vec<crate::plan::LignePlan>, crate::store::PlanMeta), String> {
    store
        .lock()
        .unwrap()
        .charger_plan()?
        .ok_or_else(|| "aucun plan enregistré à retoucher".to_string())
}

/// Réécrit plan ET fichiers. Les deux vont ensemble : laisser les fichiers en
/// arrière les ferait diverger de la base en silence.
fn sauver_apres_retouche(
    store: &Arc<Mutex<Store>>,
    input: &Path,
    cfg: &Config,
    lignes: &[crate::plan::LignePlan],
    meta: &crate::store::PlanMeta,
) -> Result<(), String> {
    store.lock().unwrap().ecrire_plan(lignes, meta)?;
    ecrire_fichiers_mep(input, cfg, lignes)?;
    Ok(())
}

/// Runs et MEP du plan enregistré, tels que la retouche doit les voir.
fn calendrier_du_plan(
    meta: &crate::store::PlanMeta,
) -> Result<(Vec<crate::calendrier::RunFacturation>, Vec<chrono::NaiveDate>), String> {
    let params = crate::plan::PlanParams::depuis_yaml(&meta.params_yaml)?;
    let (runs, debut, fin, fournies) = params.calendrier()?;
    let (meps, _) =
        crate::calendrier::completer_meps(&runs, debut, fin, &fournies, params.mep_count);
    let utilisables = crate::calendrier::runs_utilisables(&runs, debut, fin, &meps);
    Ok((utilisables, meps))
}

#[tauri::command]
pub async fn plan_ajouter(
    state: State<'_, AppState>,
    cfs: Vec<String>,
    run_num: String,
) -> Result<(), String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let (mut lignes, meta) = charger_pour_retouche(&store)?;
        let (runs, meps) = calendrier_du_plan(&meta)?;
        let run = runs
            .iter()
            .find(|r| r.num == run_num)
            .ok_or_else(|| format!("Run de Facturation « {run_num} » inconnu"))?;
        // Les candidats sont ceux du FICHIER, pas du pool : un compte non
        // éligible reste ajoutable (décision assumée).
        let par_cf = entrees_par_cf(&store, &input, &cfg)?;
        let candidats: Vec<crate::plan::CfCandidat> = par_cf
            .values()
            .filter_map(|e| {
                e.jj_brut.trim().parse::<u8>().ok().filter(|j| (1..=31).contains(j)).map(|jj| {
                    crate::plan::CfCandidat {
                        cf: e.cf.clone(),
                        participant: e.participant.clone(),
                        jj,
                        raison_sociale: e.raison_sociale.clone(),
                        pa: e.pa.clone(),
                        in_directory: e.in_directory,
                        resolved_at: e.resolved_at,
                    }
                })
            })
            .collect();
        crate::plan::ajouter(
            &mut lignes,
            &candidats,
            &cfs,
            run,
            &meps,
            chrono::Utc::now().timestamp(),
        )?;
        sauver_apres_retouche(&store, &input, &cfg, &lignes, &meta)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn plan_deplacer(
    state: State<'_, AppState>,
    cfs: Vec<String>,
    run_num: String,
) -> Result<(), String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let (mut lignes, meta) = charger_pour_retouche(&store)?;
        let (runs, meps) = calendrier_du_plan(&meta)?;
        let run = runs
            .iter()
            .find(|r| r.num == run_num)
            .ok_or_else(|| format!("Run de Facturation « {run_num} » inconnu"))?;
        crate::plan::deplacer(&mut lignes, &cfs, run, &meps)?;
        sauver_apres_retouche(&store, &input, &cfg, &lignes, &meta)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn plan_retirer(
    state: State<'_, AppState>,
    cfs: Vec<String>,
    motif: String,
) -> Result<(), String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let (mut lignes, meta) = charger_pour_retouche(&store)?;
        crate::plan::retirer(&mut lignes, &cfs, &motif, chrono::Utc::now().timestamp())?;
        sauver_apres_retouche(&store, &input, &cfg, &lignes, &meta)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn plan_annuler_retrait(
    state: State<'_, AppState>,
    cfs: Vec<String>,
) -> Result<(), String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let (mut lignes, meta) = charger_pour_retouche(&store)?;
        crate::plan::annuler_retrait(&mut lignes, &cfs)?;
        sauver_apres_retouche(&store, &input, &cfg, &lignes, &meta)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Runs compatibles avec un jour de cycle — le sélecteur de l'IHM ne propose
/// que ceux-là (la garde dure est dans le moteur).
#[tauri::command]
pub async fn plan_runs_compatibles(
    state: State<'_, AppState>,
    jj: u8,
) -> Result<Vec<String>, String> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let (_, meta) = charger_pour_retouche(&store)?;
        let (runs, _) = calendrier_du_plan(&meta)?;
        Ok(crate::plan::runs_compatibles(jj, &runs)
            .into_iter()
            .map(|r| r.num.clone())
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Rapport HTML du plan — livrable DISTINCT du rapport de run.
#[tauri::command]
pub async fn plan_rapport(state: State<'_, AppState>) -> Result<String, String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let (lignes, meta) = charger_pour_retouche(&store)?;
        // Pool recalculé au moment du rapport : la comparaison plan vs pool
        // n'a de sens que sur des données fraîches.
        let params = crate::plan::PlanParams::depuis_yaml(&meta.params_yaml)?;
        let entrees = {
            let s = store.lock().unwrap();
            plan_entrees_from_scan(&s, &input, &cfg, chrono::Utc::now())?
        };
        let (pool, _) = crate::plan::construire_pool(&entrees, &params.pa_exclues())?;
        let mut pool_par_pa: std::collections::BTreeMap<String, usize> = Default::default();
        for c in &pool {
            *pool_par_pa.entry(c.pa.clone()).or_insert(0) += 1;
        }
        let mut pool_par_jj: std::collections::BTreeMap<u8, usize> = Default::default();
        for c in &pool {
            *pool_par_jj.entry(c.jj).or_insert(0) += 1;
        }
        // Runs RETENUS : `calendrier_du_plan` applique déjà les trois filtres
        // (exclusion, fenêtre, MEP passée), comme pour `plan_ajouter`.
        let (runs, _meps) = calendrier_du_plan(&meta)?;
        let maintenant = chrono::Local::now();
        let html = crate::plan_report::render(&crate::plan_report::PlanReportData {
            fichier: &meta.fichier,
            date_longue: &report::date_fr_longue(&maintenant),
            version: env!("CARGO_PKG_VERSION"),
            lignes: &lignes,
            aujourdhui: maintenant.date_naive(),
            pool_par_pa: &pool_par_pa,
            pool_par_jj: &pool_par_jj,
            runs: &runs,
        });
        let out = resolved_out_dir(&input, &cfg.output.dir).join(format!(
            "{}_plan.html",
            input.file_stem().unwrap_or_default().to_string_lossy()
        ));
        std::fs::write(&out, html).map_err(|e| format!("écriture du rapport de plan : {e}"))?;
        Ok(out.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Progression émise pendant le chargement de l'annuaire.
/// phase = "download" (done/total en octets) | "parse" (done = lignes, total = None).
#[derive(Clone, Serialize)]
pub struct DirProgress {
    pub phase: &'static str,
    pub done: u64,
    pub total: Option<u64>,
}

/// Progression d'ingestion PPF (phase parse ; pas de download).
#[derive(Clone, Serialize)]
pub struct PpfProgress {
    pub done: u64,
}

/// SHA-256 hexadécimal minuscule du contenu brut d'un fichier.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[derive(Serialize)]
pub struct DirLoadResult {
    pub loaded_at: i64,
    pub count: usize,
}

#[tauri::command]
pub fn directory_status(state: State<'_, AppState>) -> Result<Option<crate::store::DirStatus>, String> {
    state.store.lock().unwrap().peppol_directory_status()
}

/// Parse un fichier annuaire (chemin local ou temporaire de téléchargement) et
/// le charge en base. BLOQUANT (jusqu'à 5,2 M lignes) : à appeler depuis
/// `spawn_blocking`. Émet la progression phase "parse" sur `directory://progress`.
fn parse_and_store_directory(
    path: &Path,
    store: &Arc<Mutex<Store>>,
    source: &str,
    app: &AppHandle,
) -> Result<DirLoadResult, String> {
    let reader = std::io::BufReader::new(
        std::fs::File::open(path).map_err(|e| format!("ouverture {path:?} : {e}"))?,
    );
    let values = crate::directory::stream_0225_values(reader, |lines| {
        let _ = app.emit(
            "directory://progress",
            DirProgress { phase: "parse", done: lines, total: None },
        );
    })?;
    let loaded_at = chrono::Utc::now().timestamp();
    let count = store
        .lock()
        .unwrap()
        .replace_peppol_directory(&values, source, loaded_at)?;
    Ok(DirLoadResult { loaded_at, count })
}

/// Charge un fichier annuaire local (drop / Parcourir). Parsing bloquant hors
/// executor ; progression phase "parse" émise sur `directory://progress`.
#[tauri::command]
pub async fn load_directory_file(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<DirLoadResult, String> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        parse_and_store_directory(Path::new(&path), &store, "file", &app)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Télécharge l'annuaire puis le charge. Progression phase "download" pendant
/// le transfert, puis "parse" pendant l'analyse. Le temporaire est supprimé.
#[tauri::command]
pub async fn download_directory(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DirLoadResult, String> {
    // Proxy éventuel — la config peut être absente (aucun run configuré).
    let (proxy, creds) = {
        let cfg = state.config.lock().unwrap().clone();
        let proxy = cfg.as_ref().and_then(|c| c.api.proxy.as_ref()).map(|p| p.url.clone());
        let creds = state.proxy_creds.lock().unwrap().clone();
        (proxy, creds)
    };
    let app_dl = app.clone();
    let tmp = crate::directory::download_to_temp(
        crate::directory::DIRECTORY_URL,
        proxy.as_deref(),
        creds.as_ref(),
        move |done, total| {
            let _ = app_dl.emit(
                "directory://progress",
                DirProgress { phase: "download", done, total },
            );
        },
    )
    .await?;
    let path = tmp.path().to_path_buf();
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        parse_and_store_directory(&path, &store, "download", &app)
    })
    .await
    .map_err(|e| e.to_string())?;
    drop(tmp); // suppression du temporaire (214 Mo) après parsing
    result
}

/// Charge un fichier PPF : lit le contenu en mémoire (exports de taille
/// modérée — pas 214 Mo comme l'annuaire Peppol), hashe, parse, ingère par
/// upsert cumulatif. Renvoie l'entrée d'historique créée. BLOQUANT →
/// spawn_blocking.
#[tauri::command]
pub async fn load_ppf_file(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<crate::store::PpfFile, String> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| format!("lecture du fichier PPF : {e}"))?;
        let content_hash = sha256_hex(&bytes);
        let file_name = Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let parse = crate::ppf::stream_ppf(std::io::Cursor::new(&bytes), |done| {
            let _ = app.emit("ppf://progress", PpfProgress { done });
        })?;
        store.lock().unwrap().ingest_ppf(
            &file_name,
            &content_hash,
            &parse.rows,
            parse.lines as i64,
            chrono::Utc::now().timestamp(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Historique des fichiers PPF ingérés (le plus récent en tête).
#[tauri::command]
pub fn ppf_files(state: State<'_, AppState>) -> Result<Vec<crate::store::PpfFile>, String> {
    state.store.lock().unwrap().ppf_files()
}

/// Résumé de l'annuaire PPF (adressages distincts, nombre de fichiers).
#[tauri::command]
pub fn ppf_summary(state: State<'_, AppState>) -> Result<crate::store::PpfSummary, String> {
    state.store.lock().unwrap().ppf_summary()
}

/// Vide l'annuaire PPF et son historique.
#[tauri::command]
pub fn reset_ppf(state: State<'_, AppState>) -> Result<(), String> {
    state.store.lock().unwrap().reset_ppf()
}

#[cfg(test)]
mod tests_calibration_prerequisites {
    use super::*;

    #[test]
    fn tout_present_passe() {
        assert!(calibration_prerequisites("K", "data.csv").is_ok());
    }

    #[test]
    fn cle_manquante_le_dit_sans_parler_du_fichier() {
        let e = calibration_prerequisites("  ", "data.csv").unwrap_err();
        assert!(e.contains("clé API"), "{e}");
        assert!(!e.contains("fichier"), "{e}");
    }

    #[test]
    fn fichier_manquant_le_dit_sans_parler_de_la_cle() {
        let e = calibration_prerequisites("K", "").unwrap_err();
        assert!(e.contains("fichier d'entrée"), "{e}");
        assert!(!e.contains("clé"), "{e}");
    }

    #[test]
    fn les_deux_manquants_listent_les_deux() {
        let e = calibration_prerequisites("", " ").unwrap_err();
        assert!(e.contains("clé API"), "{e}");
        assert!(e.contains("fichier d'entrée"), "{e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_valeurs_connues() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// Un run couvrant les jours de cycle 1 et 5.
    fn run_test() -> crate::calendrier::RunFacturation {
        crate::calendrier::RunFacturation {
            num: "R3".into(),
            date: chrono::NaiveDate::from_ymd_opt(2026, 9, 8).unwrap(),
            jjs: vec![1, 5],
            exclu: false,
        }
    }

    fn entree(cf: &str, jj: &str, ctc: &str, ppf: bool) -> crate::plan::LigneEntree {
        crate::plan::LigneEntree {
            cf: cf.into(),
            participant: "0225:1".into(),
            jj_brut: jj.into(),
            raison_sociale: "ACME".into(),
            pa: "Cegedim".into(),
            resolu: true,
            ctc_ready: ctc == "ready",
            ctc_status: ctc.into(),
            ppf_usable: ppf,
            in_directory: true,
            resolved_at: 0,
        }
    }

    #[test]
    fn candidats_run_ne_rend_que_les_jours_de_cycle_couverts() {
        let entrees = vec![
            entree("CF1", "5", "ready", true),
            entree("CF2", "12", "ready", true),
        ];
        let out = candidats_du_run(&entrees, &run_test(), &HashSet::new());
        assert_eq!(out.len(), 1, "le jour 12 n'est pas couvert par ce run");
        assert_eq!(out[0].cf, "CF1");
    }

    #[test]
    fn candidats_run_exclut_les_comptes_deja_au_plan() {
        let entrees = vec![entree("CF1", "5", "ready", true)];
        let deja: HashSet<String> = ["CF1".to_string()].into_iter().collect();
        assert!(candidats_du_run(&entrees, &run_test(), &deja).is_empty());
    }

    #[test]
    fn candidats_run_rend_les_non_eligibles_signales() {
        // Les forcer reste un choix assumé : ils sont proposés ET marqués.
        let entrees = vec![
            entree("CF1", "5", "later", true),
            entree("CF2", "1", "ready", false),
        ];
        let out = candidats_du_run(&entrees, &run_test(), &HashSet::new());
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|c| !c.eligible), "aucun des deux n'est pleinement éligible");
    }

    #[test]
    fn candidats_run_porte_le_statut_ctc_complet() {
        // Le test qui distingue le champ neuf du booléen préexistant : sans lui,
        // rendre `ctc_status` toujours vide passerait inaperçu.
        let entrees = vec![
            entree("CF1", "5", "later", true),
            entree("CF2", "1", "expired", true),
        ];
        let out = candidats_du_run(&entrees, &run_test(), &HashSet::new());
        let statuts: Vec<&str> = out.iter().map(|c| c.ctc_status.as_str()).collect();
        assert!(statuts.contains(&"later") && statuts.contains(&"expired"), "{statuts:?}");
    }

    #[test]
    fn candidats_run_portent_leur_adressage_et_leur_ppf_usable() {
        // Deux champs que rien ne retenait : les remplacer par une chaîne vide
        // ou un `true` en dur ne cassait aucun test.
        let mut prefixe = entree("CF2", "1", "ready", false);
        prefixe.participant = "iso6523-actorid-upis::0225:12345678900012".into();
        let entrees = vec![entree("CF1", "5", "ready", true), prefixe];
        let out = candidats_du_run(&entrees, &run_test(), &HashSet::new());
        let cf1 = out.iter().find(|c| c.cf == "CF1").expect("CF1 absent");
        let cf2 = out.iter().find(|c| c.cf == "CF2").expect("CF2 absent");
        assert_eq!(cf2.participant, "12345678900012", "l'ICD est retiré, comme en base");
        assert_eq!(cf1.participant, "0225:1", "hors schéma connu, la valeur sort telle quelle");
        assert!(cf1.ppf_usable);
        assert!(!cf2.ppf_usable, "le champ suit l'entrée, il n'est pas vrai par construction");
    }

    #[test]
    fn candidats_run_ignore_un_jour_de_cycle_illisible() {
        // Un JJ hors bornes ou non numérique ne correspond à aucun run.
        let entrees = vec![entree("CF1", "zzz", "ready", true), entree("CF2", "99", "ready", true)];
        assert!(candidats_du_run(&entrees, &run_test(), &HashSet::new()).is_empty());
    }
}
