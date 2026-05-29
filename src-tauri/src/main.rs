#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod audio;
pub mod config;
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
    LogicalSize, Manager, PhysicalPosition, Size, WebviewUrl, WebviewWindowBuilder,
};
use std::sync::atomic::{AtomicBool, Ordering};

/// 浮動字幕視窗目前是否顯示中。set_captions 設定它;前端 CC 鈕 polling captions_visible
/// 來反映真實狀態(否則錄音 auto-show 開了視窗,CC 鈕不會亮 — 兩邊狀態不同步)。
static CAPTIONS_VISIBLE: AtomicBool = AtomicBool::new(false);

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
async fn recorder_stop() -> Result<String, String> {
    // stop_session 內 block_on whisper.cpp 轉錄(秒~分鐘),sync command 會卡
    // Tauri runtime worker → 前端 polling 整個排在後面 → UI 凍。把 sync 重活丟
    // 進 spawn_blocking,Tauri 主 runtime 繼續處理 recorder_status polling。
    tauri::async_runtime::spawn_blocking(|| recorder_instance().stop_session())
        .await
        .map_err(|e| format!("join blocking task: {e}"))?
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
fn get_config() -> config::RecorderConfig {
    config::read_config()
}

#[tauri::command]
fn set_config(cfg: config::RecorderConfig) -> Result<(), String> {
    config::write_config(&cfg)
}

#[tauri::command]
fn set_meeting_info(topic: String, participants: String) -> Result<(), String> {
    recorder_instance().set_meeting_info(topic, participants)
}

#[tauri::command]
fn voice_input_start() -> Result<(), String> {
    recorder_instance().voice_input_start()
}

#[tauri::command]
async fn voice_input_stop() -> Result<String, String> {
    // whisper 轉錄是 blocking → spawn_blocking 不卡 UI(同 recorder_stop)
    tauri::async_runtime::spawn_blocking(|| recorder_instance().voice_input_stop())
        .await
        .map_err(|e| format!("join voice: {e}"))?
}

/// 找現有 caption 視窗,沒有就建一個(Rust 端建窗比 JS WebviewWindow.getByLabel 可靠;
/// 靜態 tauri.conf 定義在某些 Wayland 環境沒被建/show 沒效)。帶 log 方便抓問題。
fn ensure_caption_window(app: &tauri::AppHandle, label: &str) -> Result<tauri::WebviewWindow, String> {
    if let Some(w) = app.get_webview_window(label) {
        return Ok(w);
    }
    eprintln!("[captions] {label}: creating");
    // transparent(false):透明視窗在這台 Wayland 即使 is_visible=true 也看不到 → 用不透明,
    // 一定有可見表面。位置在 show 時相對主視窗重設(見 set_captions),這裡只給初始值。
    WebviewWindowBuilder::new(app, label, WebviewUrl::default())
        .title(label)
        .inner_size(360.0, 220.0)
        .position(40.0, 120.0)
        .decorations(false)
        .transparent(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
        .map_err(|e| format!("{label} build: {e}"))
}

/// 顯示 / 隱藏兩個浮動字幕視窗。顯示時擺在「主視窗正下方」—— user 把主視窗拖到哪
/// (含多螢幕)字幕就跟到哪,不會被擺到看不到的角落。前端 CC 鈕 + 錄音 auto-show 都呼這個。
#[tauri::command]
fn set_captions(app: tauri::AppHandle, visible: bool) -> Result<(), String> {
    let (base_x, base_y) = app
        .get_webview_window("main")
        .and_then(|m| {
            let p = m.outer_position().ok()?;
            let s = m.outer_size().ok()?;
            Some((p.x, p.y + s.height as i32 + 8))
        })
        .unwrap_or((40, 120));
    for (i, label) in ["caption-sys", "caption-mic"].into_iter().enumerate() {
        match ensure_caption_window(&app, label) {
            Ok(w) => {
                if visible {
                    let x = base_x + (i as i32) * 368;
                    let _ = w.set_position(PhysicalPosition::new(x, base_y));
                    let _ = w.show();
                    let _ = w.set_focus();
                } else {
                    let _ = w.hide();
                }
                eprintln!(
                    "[captions] {label}: visible={visible} is_visible={:?} pos={:?} size={:?}",
                    w.is_visible().ok(),
                    w.outer_position().ok(),
                    w.inner_size().ok()
                );
            }
            Err(e) => eprintln!("[captions] {label}: {e}"),
        }
    }
    CAPTIONS_VISIBLE.store(visible, Ordering::Relaxed);
    Ok(())
}

/// 前端 CC 鈕 polling 用 — 浮動字幕視窗目前是否顯示。
#[tauri::command]
fn captions_visible() -> bool {
    CAPTIONS_VISIBLE.load(Ordering::Relaxed)
}

#[tauri::command]
fn set_window_mode(window: tauri::Window, mode: String) -> Result<(), String> {
    let (w, h) = match mode.as_str() {
        "collapsed" => (380.0, 44.0),
        "expanded" => (720.0, 620.0),
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
fn list_sessions_detailed() -> Vec<session_store::SessionSummary> {
    let dir = session_store::default_meetings_dir();
    let mut summaries: Vec<session_store::SessionSummary> = std::fs::read_dir(&dir)
        .ok()
        .map(|it| {
            it.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if !name.starts_with("meeting-") { return None; }
                    if !e.path().is_dir() { return None; }
                    Some(session_store::read_session_summary(&name, &dir))
                })
                .collect()
        })
        .unwrap_or_default();
    summaries.sort_by(|a, b| {
        match (a.corrupt, b.corrupt) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => b.started_at.cmp(&a.started_at),
        }
    });
    summaries
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

            // 預先建好兩個浮動字幕視窗(hidden)。在 startup 建,避免「建完馬上 show」的 race
            // (build().visible(false) 後立刻 show 時 is_visible 還是 false,視窗沒真的出來)。
            // 等 set_captions 被呼叫時 → 已 found existing → show 可靠生效。
            let _ = ensure_caption_window(app.handle(), "caption-sys");
            let _ = ensure_caption_window(app.handle(), "caption-mic");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            recorder_start,
            recorder_stop,
            recorder_status,
            deps_check,
            get_config,
            set_config,
            set_meeting_info,
            voice_input_start,
            voice_input_stop,
            set_captions,
            captions_visible,
            set_window_mode,
            list_sessions,
            list_sessions_detailed,
            open_session_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
