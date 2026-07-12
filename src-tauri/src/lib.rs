pub mod api;
pub mod config;
pub mod csv_io;
pub mod modes;
pub mod pid;
pub mod store;
pub mod telemetry;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!())
        .expect("erreur au lancement de Super Popaul");
}
