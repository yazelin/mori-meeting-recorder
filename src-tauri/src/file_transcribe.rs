//! 檔案轉錄:把任意現成音/影檔(非 session 錄音)轉成逐字稿。
//!
//! 流程:ffmpeg 抽 16kHz mono PCM WAV → temp 檔 → 複用 `transcribe::run_whisper`
//! 的 whisper-cli 路徑(傳 `&mut None` 直走 cli,避開共享 server 的 60s per-call
//! timeout;cli 原生處理任意長度檔,免手動分塊)→ 串接 segment text。
//!
//! 跟 `recorder.rs` 的 session 生命週期完全解耦 — 沒有 visibility / track 概念,
//! 不產生 session,輸出也不進 `~/.mori/meetings/`(那是會議專用)。

use std::path::{Path, PathBuf};

use serde::Serialize;
use tempfile::NamedTempFile;

use crate::audio::SourceKind;

/// 支援的副檔名(小寫,不含點)。ffmpeg 能解的常見音/影格式。
const SUPPORTED_EXTS: &[&str] = &[
    "wav", "mp3", "m4a", "flac", "ogg", "aac", "opus", "wma", // 音
    "mp4", "mkv", "webm", "mov", "avi", // 影(抽音軌)
];

/// 副檔名白名單判斷(大小寫不敏感)。
pub fn supported_extension(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => SUPPORTED_EXTS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

/// 列出資料夾**頂層**可轉錄的音/影檔:只取一般檔案(目錄與 symlink 跳過,
/// `DirEntry::file_type` 不跟隨 symlink)、`supported_extension` 過濾、依檔名
/// 大小寫不敏感排序。讀不到資料夾回 `Err`。
pub fn list_supported_in_dir(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("read dir {}: {e}", dir.display()))?;
    let mut out: Vec<PathBuf> = Vec::new();
    for entry in rd {
        let entry = entry.map_err(|e| format!("read entry: {e}"))?;
        let ft = entry.file_type().map_err(|e| format!("file type: {e}"))?;
        if !ft.is_file() {
            continue; // 目錄 + symlink 都跳過(不遞迴、symlink 媒體屬 follow-up)
        }
        let path = entry.path();
        if supported_extension(&path) {
            out.push(path);
        }
    }
    out.sort_by_key(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    Ok(out)
}

/// ffmpeg 在 PATH 且可執行(`ffmpeg -version` exit 0)。deps 檢查用。
pub fn ffmpeg_present() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 單檔轉錄結果(回給前端)。
#[derive(Serialize, Debug, Clone)]
pub struct FileTranscript {
    pub source_path: String,
    pub text: String,
    pub duration_secs: f32,
}

/// 用 ffmpeg 把任意輸入抽成 16kHz mono PCM WAV,寫到 temp 檔,回 handle
/// (drop 時自動刪)。參數對齊 mori-desktop `transcribe_media::extract_wav_bytes`。
pub fn extract_wav_to_temp(input: &Path) -> Result<NamedTempFile, String> {
    if !input.exists() {
        return Err(format!("file not found: {}", input.display()));
    }
    let tmp = tempfile::Builder::new()
        .suffix(".wav")
        .tempfile()
        .map_err(|e| format!("create temp wav: {e}"))?;
    let status = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(input)
        .args([
            "-vn", // 影片檔只拿音軌
            "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le", "-f", "wav",
        ])
        .arg(tmp.path())
        .status()
        .map_err(|e| format!("spawn ffmpeg — 確認系統有裝 ffmpeg: {e}"))?;
    if !status.success() {
        return Err(format!(
            "ffmpeg failed (exit {:?}) on {}",
            status.code(),
            input.display()
        ));
    }
    let size = std::fs::metadata(tmp.path()).map(|m| m.len()).unwrap_or(0);
    if size < 44 {
        return Err(format!(
            "ffmpeg produced empty WAV — 來源可能沒音軌或損壞: {}",
            input.display()
        ));
    }
    Ok(tmp)
}

/// 讀 WAV 算秒數(hound,recorder 既有 dep)。讀不到回 0。
fn wav_duration_secs(wav: &Path) -> f32 {
    match hound::WavReader::open(wav) {
        Ok(r) => {
            let spec = r.spec();
            let frames = r.len() as f32 / (spec.channels.max(1) as f32);
            frames / (spec.sample_rate.max(1) as f32)
        }
        Err(_) => 0.0,
    }
}

/// 單檔轉錄主入口。ffmpeg 抽 WAV → run_whisper(cli 路徑)→ 串接 text。
pub fn transcribe_file(input: &Path) -> Result<FileTranscript, String> {
    let tmp = extract_wav_to_temp(input)?;
    let cfg = crate::config::read_config();
    let label = input
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    // &mut None → run_whisper 直走 whisper-cli(避開共享 server 60s timeout;cli
    // 原生處理任意長度檔)。kind / session_id 標記丟棄,只取 text。noise filter +
    // 繁體 s2twp 由 run_whisper 內部一律處理。
    let segs = crate::transcribe::run_whisper(
        tmp.path(),
        &label,
        SourceKind::MicInternal,
        &cfg.language,
        cfg.traditional,
        &mut None,
    );
    let text = segs
        .iter()
        .map(|s| s.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let duration_secs = wav_duration_secs(tmp.path());
    Ok(FileTranscript {
        source_path: input.display().to_string(),
        text,
        duration_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn supported_extension_accepts_audio_and_video() {
        assert!(supported_extension(&PathBuf::from("a/b.mp3")));
        assert!(supported_extension(&PathBuf::from("a/b.MP4")));
        assert!(supported_extension(&PathBuf::from("x.wav")));
        assert!(supported_extension(&PathBuf::from("dir/clip.FLAC")));
    }

    #[test]
    fn supported_extension_rejects_others() {
        assert!(!supported_extension(&PathBuf::from("a/b.txt")));
        assert!(!supported_extension(&PathBuf::from("noext")));
        assert!(!supported_extension(&PathBuf::from("a/b.pdf")));
    }

    #[test]
    fn extract_wav_to_temp_errors_on_missing_file() {
        let r = extract_wav_to_temp(&PathBuf::from("/nonexistent/nope.mp3"));
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("file not found"));
    }

    /// 真機 smoke:對一個真語音檔跑完整 transcribe_file(ffmpeg → whisper-cli → text)。
    /// 預設 ignore(需 ffmpeg + ~/.mori/bin/whisper-cli + model + 真檔)。跑法:
    ///   FT_SMOKE_AUDIO=/path/clip.mp3 cargo test --release transcribe_file_smoke -- --ignored --nocapture
    #[test]
    #[ignore]
    fn transcribe_file_smoke() {
        let path = std::env::var("FT_SMOKE_AUDIO")
            .expect("set FT_SMOKE_AUDIO to a real speech audio file path");
        let r = transcribe_file(std::path::Path::new(&path)).expect("transcribe_file should succeed");
        eprintln!(
            "\n=== transcript ({:.1}s) ===\n{}\n=== end ===\n",
            r.duration_secs, r.text
        );
        assert!(!r.text.trim().is_empty(), "transcript should be non-empty for speech audio");
    }

    #[test]
    fn list_supported_in_dir_top_level_only_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("b.MP4"), b"x").unwrap();
        std::fs::write(root.join("a.mp3"), b"x").unwrap();
        std::fs::write(root.join("c.txt"), b"x").unwrap(); // 排除:非白名單
        std::fs::write(root.join("noext"), b"x").unwrap(); // 排除:無副檔名
        std::fs::create_dir(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("d.wav"), b"x").unwrap(); // 排除:子資料夾不遞迴

        let got = list_supported_in_dir(root).unwrap();
        let names: Vec<String> = got
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.mp3".to_string(), "b.MP4".to_string()]);
    }

    #[test]
    fn list_supported_in_dir_empty_ok() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list_supported_in_dir(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn list_supported_in_dir_missing_errors() {
        let r = list_supported_in_dir(&PathBuf::from("/nonexistent/nope-dir-xyz"));
        assert!(r.is_err());
    }
}
