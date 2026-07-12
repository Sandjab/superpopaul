pub mod api;
pub mod commands;
pub mod config;
pub mod csv_io;
pub mod modes;
pub mod output;
pub mod pid;
pub mod resolver;
pub mod store;
pub mod telemetry;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let store = store::Store::open(&dir.join("superpopaul.db"))
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            app.manage(commands::AppState::new(store));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::preview_csv,
            commands::set_config,
            commands::load_config,
            commands::save_config,
            commands::set_proxy_creds,
            commands::update_api_key,
            commands::test_api,
            commands::analyze_input,
            commands::calibrate_api,
            commands::start_run,
            commands::pause_run,
            commands::resume_run,
            commands::stop_run,
            commands::clear_run,
            commands::generate_output
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de Super Popaul");
}
