use crate::api::{ApiClient, CallStats, ProxyCreds};
use crate::config::{self, Config};
use crate::csv_io;
use crate::modes::{compute_todo, RunMode};
use crate::output;
use crate::pid::unique_canonical;
use crate::resolver::{calibrate, CalibrationReport, Engine, EngineEvent, EngineParams, RunHandle};
use crate::store::Store;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    /// (répertoire du YAML si chargé/sauvé — base des chemins relatifs, config)
    pub config: Mutex<Option<(Option<PathBuf>, Config)>>,
    pub proxy_creds: Mutex<Option<ProxyCreds>>,
    pub run: Mutex<Option<Arc<RunHandle>>>,
}

impl AppState {
    pub fn new(store: Store) -> Self {
        AppState {
            store: Arc::new(Mutex::new(store)),
            config: Mutex::new(None),
            proxy_creds: Mutex::new(None),
            run: Mutex::new(None),
        }
    }

    fn current_config(&self) -> Result<(Option<PathBuf>, Config), String> {
        self.config
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "Aucune configuration active.".into())
    }

    fn input_path(&self) -> Result<PathBuf, String> {
        let (base, cfg) = self.current_config()?;
        Ok(match base {
            Some(dir) => config::resolve_relative(&dir.join("x.yaml"), &cfg.input.path),
            None => PathBuf::from(&cfg.input.path),
        })
    }

    fn client(&self) -> Result<ApiClient, String> {
        let (_, cfg) = self.current_config()?;
        let creds = self.proxy_creds.lock().unwrap().clone();
        ApiClient::new(
            &cfg.api.url,
            &cfg.api.key,
            cfg.api.proxy.as_ref().map(|p| p.url.as_str()),
            creds.as_ref(),
        )
    }

    /// PIDs uniques canoniques du fichier d'entrée (lecture complète).
    fn unique_pids(&self) -> Result<Vec<String>, String> {
        let (_, cfg) = self.current_config()?;
        let path = self.input_path()?;
        let meta = csv_io::sniff(&path)?;
        let vals = csv_io::read_column(&path, &meta, &cfg.input.pid_column)?;
        Ok(unique_canonical(vals))
    }
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
    let mut guard = state.config.lock().unwrap();
    let base = guard.as_ref().and_then(|(b, _)| b.clone());
    *guard = Some((base, cfg));
    Ok(())
}

#[tauri::command]
pub fn load_config(state: State<'_, AppState>, path: String) -> Result<Config, String> {
    let p = PathBuf::from(&path);
    let cfg = config::load(&p)?;
    *state.config.lock().unwrap() = Some((p.parent().map(PathBuf::from), cfg.clone()));
    Ok(cfg)
}

#[tauri::command]
pub fn save_config(state: State<'_, AppState>, path: String, cfg: Config) -> Result<(), String> {
    let p = PathBuf::from(&path);
    config::save(&p, &cfg)?;
    *state.config.lock().unwrap() = Some((p.parent().map(PathBuf::from), cfg));
    Ok(())
}

#[tauri::command]
pub fn set_proxy_creds(state: State<'_, AppState>, username: String, password: String) {
    *state.proxy_creds.lock().unwrap() = Some(ProxyCreds { username, password });
}

#[tauri::command]
pub fn update_api_key(state: State<'_, AppState>, key: String) -> Result<(), String> {
    if let Some((_, cfg)) = state.config.lock().unwrap().as_mut() {
        cfg.api.key = key.clone();
    }
    if let Some(h) = state.run.lock().unwrap().as_ref() {
        // update_key lève déjà la suspension système (auth_api/auth_proxy) et
        // relance les workers. On ne touche PAS à set_paused ici : la pause
        // utilisateur (bouton Pause) appartient à l'utilisateur, une nouvelle
        // clé API ne doit pas la lever à sa place.
        h.update_key(&key);
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
}

/// Compare le fichier d'entrée à la base : alimente la popup de reprise et la
/// présélection du mode.
#[tauri::command]
pub async fn analyze_input(state: State<'_, AppState>) -> Result<InputStats, String> {
    let (_, cfg) = state.current_config()?;
    let pids = state.unique_pids()?;
    let known = state.store.lock().unwrap().load_map(&pids)?;
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
    })
}

#[tauri::command]
pub async fn calibrate_api(state: State<'_, AppState>) -> Result<CalibrationReport, String> {
    let (_, cfg) = state.current_config()?;
    let client = state.client()?;
    let mut sample = state.unique_pids()?;
    sample.truncate(64);
    if sample.is_empty() {
        return Err("Aucun adressage dans le fichier d'entrée.".into());
    }
    Ok(calibrate(
        &client,
        &sample,
        cfg.api.batch_size as usize,
        cfg.api.concurrency.max(16),
    )
    .await)
}

#[tauri::command]
pub async fn start_run(
    app: AppHandle,
    state: State<'_, AppState>,
    mode: RunMode,
) -> Result<u64, String> {
    if state.run.lock().unwrap().is_some() {
        return Err("Un run est déjà en cours.".into());
    }
    let (_, cfg) = state.current_config()?;
    let pids = state.unique_pids()?;
    let now = chrono::Utc::now().timestamp();
    let todo = {
        let store = state.store.lock().unwrap();
        compute_todo(&mode, &pids, &store, now)?
    };
    let total = todo.len() as u64;
    let client = state.client()?;
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let handle = Arc::new(Engine::start(
        client,
        EngineParams {
            batch_size: cfg.api.batch_size as usize,
            concurrency: cfg.api.concurrency,
        },
        todo,
        state.store.clone(),
        tx,
    ));
    *state.run.lock().unwrap() = Some(handle);
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
                } => {
                    let _ = app.emit(
                        "run-finished",
                        serde_json::json!({
                            "done": done, "failed": failed, "stopped": stopped
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

#[tauri::command]
pub fn stop_run(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.run.lock().unwrap();
    match guard.as_ref() {
        Some(h) => {
            h.request_stop();
            *guard = None;
            Ok(())
        }
        None => Err("Aucun run en cours.".into()),
    }
}

/// À appeler quand run-finished est reçu côté UI, pour libérer le slot.
#[tauri::command]
pub fn clear_run(state: State<'_, AppState>) {
    *state.run.lock().unwrap() = None;
}

#[tauri::command]
pub async fn generate_output(state: State<'_, AppState>) -> Result<String, String> {
    let (base, cfg) = state.current_config()?;
    let input = state.input_path()?;
    let meta = csv_io::sniff(&input)?;
    let pids = state.unique_pids()?;
    let resolutions = state.store.lock().unwrap().load_map(&pids)?;
    let out = match &base {
        Some(dir) => config::resolve_relative(&dir.join("x.yaml"), &cfg.output.path),
        None => PathBuf::from(&cfg.output.path),
    };
    let stamp = cfg
        .output
        .timestamp_suffix
        .then(|| chrono::Local::now().format("%Y%m%d-%H%M").to_string());
    let written = output::generate(
        &input,
        &meta,
        &cfg.input.pid_column,
        &cfg.output.columns,
        &resolutions,
        &out,
        stamp.as_deref(),
    )?;
    Ok(written.display().to_string())
}
