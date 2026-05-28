#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod exporter;
mod session_store;
mod transcribe;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = tauri::Manager::get_webview_window(app, "main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
