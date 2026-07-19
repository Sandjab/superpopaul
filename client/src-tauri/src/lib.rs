pub mod api;
pub mod commands;
pub mod config;
pub mod coverage;
pub mod ctc;
pub mod csv_io;
pub mod direct;
pub mod directory;
pub mod modes;
pub mod output;
pub mod pid;
pub mod ppf;
pub mod resolver;
pub mod report;
pub mod store;
pub mod telemetry;

use tauri::Manager;

/// Relève la limite soft de descripteurs de fichiers à 8192 (bornée par la
/// hard limit) et rend la valeur effective. Le défaut launchd d'une app GUI
/// macOS est 256 : intenable en mode direct — une socket UDP par lookup
/// hickory + le pool de connexions SMP (EMFILE constaté le 2026-07-14 à
/// concurrence 128). 8192 reste sous OPEN_MAX (10240), au-delà duquel
/// setrlimit échoue sur macOS quand la hard limit est infinie.
#[cfg(unix)]
fn raise_fd_limit() -> u64 {
    unsafe {
        let mut lim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) != 0 {
            return 0;
        }
        let target = 8192.min(lim.rlim_max);
        if lim.rlim_cur < target {
            lim.rlim_cur = target;
            let _ = libc::setrlimit(libc::RLIMIT_NOFILE, &lim);
            let _ = libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim);
        }
        lim.rlim_cur
    }
}

pub fn run() {
    #[cfg(unix)]
    raise_fd_limit();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Mode portable (Windows) : données à côté de l'exe si marqueur
            // ou base déjà présents — cf. config::portable_dir.
            let dir = match config::portable_dir_of_current_exe() {
                Some(d) => d,
                None => app.path().app_data_dir()?,
            };
            let store = store::Store::open(&dir.join("superpopaul.db"))
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            app.manage(commands::AppState::new(store, dir.join("superpopaul.yaml")));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::preview_csv,
            commands::set_config,
            commands::load_settings,
            commands::save_settings,
            commands::portable_dir,
            commands::load_profile,
            commands::save_profile,
            commands::set_proxy_creds,
            commands::update_api_key,
            commands::test_api,
            commands::analyze_input,
            commands::calibrate_api,
            commands::cancel_calibration,
            commands::start_run,
            commands::pause_run,
            commands::resume_run,
            commands::stop_run,
            commands::clear_run,
            commands::generate_output,
            commands::export_report,
            commands::directory_status,
            commands::load_directory_file,
            commands::download_directory,
            commands::load_ppf_file,
            commands::ppf_files,
            commands::ppf_summary,
            commands::reset_ppf
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de Super Popaul");
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn limite_fd_relevee_a_8192_minimum() {
        // On se met d'abord dans la peau d'une app GUI macOS (soft = 256,
        // le défaut launchd) : sans ça le test hériterait du soft élevé du
        // shell et ne prouverait rien. Si ce test échoue, le mode direct
        // retombe en « Too many open files » à forte concurrence
        // (incident du 2026-07-14 à concurrence 128).
        unsafe {
            let mut lim = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            assert_eq!(libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim), 0);
            lim.rlim_cur = 256;
            assert_eq!(libc::setrlimit(libc::RLIMIT_NOFILE, &lim), 0);
        }
        let effective = super::raise_fd_limit();
        assert!(effective >= 8192, "limite fd effective : {effective}");
    }
}
