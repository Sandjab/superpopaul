pub mod config;
pub mod pid;
pub mod store;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!())
        .expect("erreur au lancement de Super Popaul");
}
