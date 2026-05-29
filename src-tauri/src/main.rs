#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod audio;
pub mod config;
pub mod diarize;
pub mod exporter;
pub mod manifest;
pub mod postprocess;
pub mod recorder;
pub mod session_store;
pub mod transcribe;
pub mod whisper_discovery;

use recorder::{instance as recorder_instance, RecorderStatus};
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
    LogicalSize, Manager, PhysicalPosition, Size, WebviewUrl, WebviewWindowBuilder,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// 浮動字幕視窗目前是否顯示中。set_captions 設定它;前端 CC 鈕 polling captions_visible
/// 來反映真實狀態(否則錄音 auto-show 開了視窗,CC 鈕不會亮 — 兩邊狀態不同步)。
static CAPTIONS_VISIBLE: AtomicBool = AtomicBool::new(false);

// in-app 模型下載進度(前端 polling download_progress 顯示 % bar)。
static DL_ACTIVE: AtomicBool = AtomicBool::new(false);
static DL_TOTAL: AtomicU64 = AtomicU64::new(0);

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

#[derive(Serialize)]
struct GpuStatus {
    gpu_name: Option<String>, // nvidia-smi 偵測到的 GPU(None = 無 NVIDIA GPU)
    cuda_toolkit: bool,       // nvcc 在不在(能不能 GPU build)— 只 Linux 需要;Windows cuBLAS zip 自帶 runtime
    whisper_gpu_build: bool,  // 現在這支 whisper-cli 是不是 GPU build(旁邊有 ggml-cuda.so/.dll)
    is_windows: bool,         // 決定前端顯示哪套 GPU 啟用步驟(Windows 抓 cuBLAS zip vs Linux build)
}

/// GPU 偵測 — 給 Deps 頁顯示「能不能 GPU 加速 + 缺什麼」。
/// 注意:whisper.cpp 的 GPU 是編譯時決定的,所以這裡只能顯示「硬體/工具齊不齊」+ 指引;
/// 真的要 GPU 還是得用 CUDA 重 build whisper-cli(install script 偵測到 nvcc 會自動編)。
#[tauri::command]
fn gpu_status() -> GpuStatus {
    let gpu_name = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name", "--format=csv,noheader"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        });
    let cuda_toolkit = std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    // 現在這支 whisper-cli 是不是 GPU build:旁邊有沒有 CUDA backend lib。
    // 跨平台:Linux 是 libggml-cuda.so、Windows 是 ggml-cuda.dll → 用 "ggml-cuda" 同時涵蓋。
    let whisper_gpu_build = transcribe::whisper_bin_path()
        .parent()
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .any(|e| e.file_name().to_string_lossy().contains("ggml-cuda"))
        })
        .unwrap_or(false);
    GpuStatus {
        gpu_name,
        cuda_toolkit,
        whisper_gpu_build,
        is_windows: cfg!(target_os = "windows"),
    }
}

#[derive(Serialize)]
struct DownloadProgress {
    active: bool,
    downloaded: u64,
    total: u64,
}

/// 前端 polling 顯示下載 % bar。downloaded = 目前 .part(下載中)或正式檔的大小。
#[tauri::command]
fn download_progress() -> DownloadProgress {
    let path = transcribe::whisper_model_path();
    let part = std::path::PathBuf::from(format!("{}.part", path.display()));
    let downloaded = std::fs::metadata(&part)
        .or_else(|_| std::fs::metadata(&path))
        .map(|m| m.len())
        .unwrap_or(0);
    DownloadProgress {
        active: DL_ACTIVE.load(Ordering::Relaxed),
        downloaded,
        total: DL_TOTAL.load(Ordering::Relaxed),
    }
}

/// 在 app 內下載「目前 Settings 選的模型」(curl → `<path>.bin.part`,完成後 rename)。
/// 先 HEAD 取總大小給進度 bar。下載是 blocking → spawn_blocking,UI 仍可 polling 進度。
#[tauri::command]
async fn download_model() -> Result<(), String> {
    if DL_ACTIVE.swap(true, Ordering::Relaxed) {
        return Err("download already in progress".into());
    }
    let result = tauri::async_runtime::spawn_blocking(|| -> Result<(), String> {
        let path = transcribe::whisper_model_path();
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "ggml-small.bin".into());
        let url =
            format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{filename}");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir models: {e}"))?;
        }
        DL_TOTAL.store(0, Ordering::Relaxed);
        if let Ok(out) = std::process::Command::new("curl").args(["-sIL", &url]).output() {
            let headers = String::from_utf8_lossy(&out.stdout).to_lowercase();
            if let Some(len) = headers
                .lines()
                .filter_map(|l| l.strip_prefix("content-length:"))
                .filter_map(|v| v.trim().parse::<u64>().ok())
                .last()
            {
                DL_TOTAL.store(len, Ordering::Relaxed);
            }
        }
        let part = format!("{}.part", path.display());
        let status = std::process::Command::new("curl")
            .args(["-L", "--fail", "-o", &part, &url])
            .status()
            .map_err(|e| format!("spawn curl: {e}"))?;
        if !status.success() {
            let _ = std::fs::remove_file(&part);
            return Err(format!("curl 失敗({status})— 檢查網路 / 磁碟空間"));
        }
        std::fs::rename(&part, &path).map_err(|e| format!("rename: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("join download: {e}"));
    DL_ACTIVE.store(false, Ordering::Relaxed);
    result?
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

// ── C1: 分人模型安裝狀態 + 下載 ────────────────────────────────────────────────

/// 兩個分人模型(seg + emb)都在才回 true — DepsTab 用來顯示「已安裝」狀態。
#[tauri::command]
fn diar_models_present() -> bool {
    diarize::diarization_models_present()
}

/// 下載分人模型(seg tar.bz2 + emb onnx)到 ~/.mori/models/。
/// 進度走既有 DL_ACTIVE / DL_TOTAL statics(前端 polling download_progress)。
/// 寫到 tmp 再 rename,中途失敗不留殘檔。
/// Linux:呼叫系統 tar 解 .tar.bz2;Windows:TODO — tar.exe 在 Win10 1803+ 有,但需驗證。
#[tauri::command]
async fn download_diar_models() -> Result<(), String> {
    if DL_ACTIVE.swap(true, Ordering::Relaxed) {
        return Err("download already in progress".into());
    }
    let result = tauri::async_runtime::spawn_blocking(|| -> Result<(), String> {
        let seg_url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2";
        let emb_url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx";

        let seg_dest = diarize::seg_model_path();
        let emb_dest = diarize::emb_model_path();

        if let Some(parent) = seg_dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir models: {e}"))?;
        }

        // ── Step 1: seg tar.bz2 — HEAD 取大小 → curl 下載 → 系統 tar 解壓 → rename ──
        // emb 也下,估 total = seg+emb 各半(近似夠用);seg 和 emb 分開 phase。
        DL_TOTAL.store(0, Ordering::Relaxed);
        // 先 HEAD seg URL 取大小,給 DL_TOTAL(emb 大小追加)
        let mut total_bytes = 0u64;
        for url in [seg_url, emb_url] {
            if let Ok(out) = std::process::Command::new("curl").args(["-sIL", url]).output() {
                let headers = String::from_utf8_lossy(&out.stdout).to_lowercase();
                if let Some(len) = headers
                    .lines()
                    .filter_map(|l| l.strip_prefix("content-length:"))
                    .filter_map(|v| v.trim().parse::<u64>().ok())
                    .last()
                {
                    total_bytes += len;
                }
            }
        }
        DL_TOTAL.store(total_bytes, Ordering::Relaxed);

        // Download seg tar.bz2
        let seg_tar_part = seg_dest.with_extension("onnx.tar.bz2.part");
        let status = std::process::Command::new("curl")
            .args(["-L", "--fail", "-o", &seg_tar_part.to_string_lossy(), seg_url])
            .status()
            .map_err(|e| format!("spawn curl (seg): {e}"))?;
        if !status.success() {
            let _ = std::fs::remove_file(&seg_tar_part);
            return Err(format!("curl seg failed ({status}) — 檢查網路 / 磁碟空間"));
        }

        // Extract model.onnx from the tar.bz2 using system tar
        // The archive contains sherpa-onnx-pyannote-segmentation-3-0/model.onnx
        let extract_dir = seg_dest.parent().unwrap().join("_seg_extract_tmp");
        let _ = std::fs::remove_dir_all(&extract_dir);
        std::fs::create_dir_all(&extract_dir).map_err(|e| format!("mkdir extract dir: {e}"))?;

        #[cfg(target_os = "windows")]
        {
            // Windows 10 1803+ ships tar.exe; if absent, this will error clearly.
            // TODO(windows): verify tar.exe on target machine; fallback to bzip2+tar crates if needed.
            let st = std::process::Command::new("tar")
                .args([
                    "-xjf",
                    &seg_tar_part.to_string_lossy(),
                    "-C",
                    &extract_dir.to_string_lossy(),
                    "--strip-components=1",
                ])
                .status()
                .map_err(|e| format!("tar (win): {e}"))?;
            if !st.success() {
                let _ = std::fs::remove_file(&seg_tar_part);
                let _ = std::fs::remove_dir_all(&extract_dir);
                return Err("tar extract failed on Windows — ensure tar.exe is available (Win10 1803+)".into());
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            // Linux/macOS: system tar + bzip2
            let st = std::process::Command::new("tar")
                .args([
                    "-xjf",
                    &seg_tar_part.to_string_lossy(),
                    "-C",
                    &extract_dir.to_string_lossy(),
                    "--strip-components=1",
                ])
                .status()
                .map_err(|e| format!("tar (unix): {e}"))?;
            if !st.success() {
                let _ = std::fs::remove_file(&seg_tar_part);
                let _ = std::fs::remove_dir_all(&extract_dir);
                return Err(format!("tar extract failed ({st}) — check bzip2 is installed"));
            }
        }

        // find model.onnx in extracted dir (--strip-components=1 puts it at extract_dir/model.onnx)
        let extracted_model = extract_dir.join("model.onnx");
        if !extracted_model.exists() {
            // Search one level deeper just in case strip-components didn't apply as expected
            let found = std::fs::read_dir(&extract_dir)
                .ok()
                .and_then(|mut rd| {
                    rd.find_map(|e| {
                        let e = e.ok()?;
                        let candidate = e.path().join("model.onnx");
                        if candidate.exists() { Some(candidate) } else { None }
                    })
                });
            match found {
                Some(f) => std::fs::rename(&f, &seg_dest)
                    .map_err(|e| format!("rename seg model: {e}"))?,
                None => {
                    let _ = std::fs::remove_file(&seg_tar_part);
                    let _ = std::fs::remove_dir_all(&extract_dir);
                    return Err("model.onnx not found inside tar archive".into());
                }
            }
        } else {
            std::fs::rename(&extracted_model, &seg_dest)
                .map_err(|e| format!("rename seg model: {e}"))?;
        }
        let _ = std::fs::remove_file(&seg_tar_part);
        let _ = std::fs::remove_dir_all(&extract_dir);

        // ── Step 2: emb onnx — direct download ──────────────────────────────────
        let emb_part = emb_dest.with_extension("onnx.part");
        let status = std::process::Command::new("curl")
            .args(["-L", "--fail", "-o", &emb_part.to_string_lossy(), emb_url])
            .status()
            .map_err(|e| format!("spawn curl (emb): {e}"))?;
        if !status.success() {
            let _ = std::fs::remove_file(&emb_part);
            return Err(format!("curl emb failed ({status}) — 檢查網路 / 磁碟空間"));
        }
        std::fs::rename(&emb_part, &emb_dest).map_err(|e| format!("rename emb: {e}"))?;

        Ok(())
    })
    .await
    .map_err(|e| format!("join download_diar_models: {e}"));
    DL_ACTIVE.store(false, Ordering::Relaxed);
    result?
}

// ── C2: 工作區後端 command ───────────────────────────────────────────────────

/// 讀一場 session 的逐字稿(兩軌 jsonl 合併,依 start_ms 排序)。
#[tauri::command]
fn read_session_transcript(session_id: String) -> Vec<transcribe::Segment> {
    postprocess::read_session_segments(
        &session_store::default_meetings_dir().join(&session_id),
    )
}

/// 讀一場 session 的講者清單(speakers.json → Vec<SpeakerInfo>)。
#[tauri::command]
fn read_speakers_cmd(session_id: String) -> Vec<diarize::SpeakerInfo> {
    diarize::read_speakers(
        &session_store::default_meetings_dir()
            .join(&session_id)
            .join("transcript/speakers.json"),
    )
}

/// 改一場 session 某講者的顯示名(只動 speakers.json)。
#[tauri::command]
fn rename_speaker_cmd(session_id: String, id: String, display: String) -> Result<(), String> {
    diarize::rename_speaker(
        &session_store::default_meetings_dir()
            .join(&session_id)
            .join("transcript/speakers.json"),
        &id,
        &display,
    )
}

/// 讀 meeting-info.json(topic + participants);缺檔 → 回 {topic:"",participants:""}。
#[tauri::command]
fn read_meeting_info(session_id: String) -> serde_json::Value {
    let p = session_store::default_meetings_dir()
        .join(&session_id)
        .join("meeting-info.json");
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({"topic": "", "participants": ""}))
}

/// 寫 meeting-info.json(topic + participants)到指定 session。
#[tauri::command]
fn set_meeting_info_for(
    session_id: String,
    topic: String,
    participants: String,
) -> Result<(), String> {
    let root = session_store::default_meetings_dir().join(&session_id);
    let body = serde_json::to_string_pretty(
        &serde_json::json!({"topic": topic, "participants": participants}),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(root.join("meeting-info.json"), body)
        .map_err(|e| format!("write meeting-info: {e}"))
}

/// 用目前 jsonl(已含 speaker)+ speakers.json + timeline.json 重新匯出
/// meeting.public/internal.md(spawn_blocking 不卡 UI)。
#[tauri::command]
async fn reexport_session(session_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        postprocess::reexport_session(
            &session_store::default_meetings_dir().join(&session_id),
        )
    })
    .await
    .map_err(|e| format!("join reexport_session: {e}"))?
}

// ── C3: 分人修正 command ────────────────────────────────────────────────────────

/// 合併講者:把 merge_ids 的段全改成 keep_id(兩軌)+ 從 speakers.json 移除 merge_ids。
#[tauri::command]
fn merge_speakers(session_id: String, keep_id: String, merge_ids: Vec<String>) -> Result<(), String> {
    let root = session_store::default_meetings_dir().join(&session_id);
    for jsonl_rel in ["transcript/system.segments.jsonl", "transcript/mic-internal.segments.jsonl"] {
        let path = root.join(jsonl_rel);
        let segs = transcribe::read_segments_jsonl(&path);
        if segs.is_empty() {
            continue;
        }
        let relabeled = postprocess::relabel_merge(segs, &keep_id, &merge_ids);
        transcribe::write_segments_jsonl(&path, &relabeled)?;
    }
    let sp_path = root.join("transcript").join("speakers.json");
    let kept = postprocess::drop_speakers(diarize::read_speakers(&sp_path), &merge_ids);
    diarize::write_speakers(&sp_path, &kept)
}

/// 逐段改講者:把指定 track 的 seg_id 那段 speaker 設成 speaker_id(讀 jsonl → relabel_one → 原子寫回)。
#[tauri::command]
fn set_segment_speaker(session_id: String, track: String, seg_id: String, speaker_id: String) -> Result<(), String> {
    let root = session_store::default_meetings_dir().join(&session_id);
    let jsonl_rel = match track.as_str() {
        "system" => "transcript/system.segments.jsonl",
        "mic-internal" => "transcript/mic-internal.segments.jsonl",
        other => return Err(format!("unknown track: {other}")),
    };
    let path = root.join(jsonl_rel);
    let segs = transcribe::read_segments_jsonl(&path);
    let relabeled = postprocess::relabel_one(segs, &seg_id, &speaker_id);
    transcribe::write_segments_jsonl(&path, &relabeled)
}

/// 對一場 session 跑分人後處理:讀 meeting-info 人員數 → num_clusters → 每軌 diarize_wav
/// → assign_speakers → 標回兩軌 jsonl + 寫 speakers.json。
/// 耗時(實時因子約 0.08x CPU)→ spawn_blocking 不卡 UI(同 recorder_stop 模式)。
#[tauri::command]
async fn diarize_session(session_id: String) -> Result<postprocess::DiarizeSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = session_store::default_meetings_dir().join(&session_id);
        // 讀 meeting-info.json 取人員字串 → participant_count → num_clusters
        let info = std::fs::read_to_string(root.join("meeting-info.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
        let participants = info
            .as_ref()
            .and_then(|v| v.get("participants"))
            .and_then(|p| p.as_str())
            .unwrap_or("");
        let num_clusters = diarize::participant_count(participants);
        postprocess::diarize_session_inner(&root, num_clusters)
    })
    .await
    .map_err(|e| format!("join diarize_session: {e}"))?
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
            download_model,
            download_progress,
            gpu_status,
            set_window_mode,
            list_sessions,
            list_sessions_detailed,
            open_session_dir,
            diarize_session,
            // C1: diarization model management
            diar_models_present,
            download_diar_models,
            // C2: workspace backend commands
            read_session_transcript,
            read_speakers_cmd,
            rename_speaker_cmd,
            read_meeting_info,
            set_meeting_info_for,
            reexport_session,
            // C3: diarization correction commands
            merge_speakers,
            set_segment_speaker,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
