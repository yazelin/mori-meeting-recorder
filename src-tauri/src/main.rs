#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod audio;
pub mod exporter;
pub mod manifest;
pub mod recorder;
pub mod session_store;
pub mod transcribe;

use recorder::{instance as recorder_instance, RecorderStatus};
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
    LogicalSize, Manager, Size,
};

#[derive(Debug, Serialize)]
struct DepsCheck {
    whisper_cli_ok: bool,
    whisper_cli_path: String,
    model_ok: bool,
    model_path: String,
}

#[tauri::command]
fn recorder_start(app: tauri::AppHandle) -> Result<String, String> {
    recorder_instance().start_session(app)
}

#[tauri::command]
fn recorder_stop() -> Result<String, String> {
    recorder_instance().stop_session()
}

#[tauri::command]
fn recorder_status() -> RecorderStatus {
    recorder_instance().status()
}

#[tauri::command]
fn deps_check() -> DepsCheck {
    let bin = transcribe::whisper_bin_path();
    let model = transcribe::whisper_model_path();
    DepsCheck {
        whisper_cli_ok: bin.exists() && bin.is_file(),
        whisper_cli_path: bin.to_string_lossy().to_string(),
        model_ok: model.exists()
            && std::fs::metadata(&model)
                .map(|m| m.len() > 40_000_000)
                .unwrap_or(false),
        model_path: model.to_string_lossy().to_string(),
    }
}

#[tauri::command]
fn set_window_mode(window: tauri::Window, mode: String) -> Result<(), String> {
    let (w, h) = match mode.as_str() {
        "collapsed" => (380.0, 44.0),
        "expanded" => (720.0, 480.0),
        other => return Err(format!("unknown mode: {other}")),
    };
    window
        .set_size(Size::Logical(LogicalSize { width: w, height: h }))
        .map_err(|e| format!("set_size: {e}"))
}

#[tauri::command]
fn list_sessions() -> Vec<String> {
    let dir = session_store::default_meetings_dir();
    std::fs::read_dir(&dir)
        .ok()
        .map(|it| {
            it.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
fn open_session_dir(session_id: String) -> Result<(), String> {
    let dir = session_store::default_meetings_dir().join(&session_id);
    if !dir.exists() {
        return Err(format!("not found: {}", dir.display()));
    }
    open_path(&dir.to_string_lossy())
}

#[cfg(target_os = "linux")]
fn open_path(path: &str) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .status()
        .map_err(|e| format!("xdg-open: {e}"))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn open_path(path: &str) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(path)
        .status()
        .map_err(|e| format!("explorer: {e}"))?;
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn open_path(_path: &str) -> Result<(), String> {
    Err("unsupported platform".into())
}

#[allow(dead_code)]
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .setup(|app| {
            // BI-1:啟動時寫 manifest 到 ~/.mori/body-parts/mori.meeting-recorder/manifest.json
            if let Err(e) = manifest::write_on_startup() {
                eprintln!("write manifest: {e}");
            }
            // Tray
            let toggle = MenuItem::with_id(app, "toggle", "顯示 / 隱藏", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "結束", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle, &quit])?;
            let _tray = TrayIconBuilder::new()
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => {
                        if let Some(w) = app.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { .. } = event {
                        if let Some(w) = tray.app_handle().get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            recorder_start,
            recorder_stop,
            recorder_status,
            deps_check,
            set_window_mode,
            list_sessions,
            open_session_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
