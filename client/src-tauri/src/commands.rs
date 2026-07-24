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
use std::collections::HashMap;
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
}
