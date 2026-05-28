//! 音訊 capture / write — per-track WAV writer + 平台 capture impl(linux / windows)。

pub mod writer;
pub mod levels;

use serde::{Deserialize, Serialize};

/// 一個 source 的「分類」— 決定預設 visibility + 在 segment 上的 source_kind 欄。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    MeetingSystem,
    MicInternal,
}

/// Segment / 匯出檔的 visibility。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Internal,
}

impl SourceKind {
    pub fn default_visibility(self) -> Visibility {
        match self {
            Self::MeetingSystem => Visibility::Public,
            Self::MicInternal => Visibility::Internal,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::MeetingSystem => "meeting_system",
            Self::MicInternal => "mic_internal",
        }
    }

    pub fn track_name(self) -> &'static str {
        match self {
            Self::MeetingSystem => "system",
            Self::MicInternal => "mic-internal",
        }
    }
}

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "windows")]
pub mod windows;

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// 一個 capture stream — recorder 持有,stop_session 時 set stop_flag。
pub struct CaptureHandle {
    pub source: SourceKind,
    pub writer_handle: JoinHandle<Result<u64, String>>,
    pub signal: Arc<Mutex<SignalMeter>>,
    pub stop_flag: Arc<std::sync::atomic::AtomicBool>,
}

/// 過去 N ms 的 peak + RMS — capsule 用 RMS 判訊號;Record tab VU meter 用 peak。
#[derive(Debug, Clone, Copy, Default)]
pub struct SignalMeter {
    pub peak_rms_db: f32,  // RMS in dB(歷史命名,別動,capsule has_signal 用)
    pub peak_db: f32,      // 瞬時 peak in dB(VU meter peak segment 用)
    pub last_sample_at_unix_ms: u64,
}

impl SignalMeter {
    pub fn has_signal(&self, now_unix_ms: u64) -> bool {
        // 過去 500ms 有 sample 且 peak RMS > -40 dB
        now_unix_ms.saturating_sub(self.last_sample_at_unix_ms) < 500 && self.peak_rms_db > -40.0
    }
}

/// 開啟一個 capture stream 給指定 source,把 samples 寫進指定 WAV path。回 handle。
#[cfg(target_os = "linux")]
pub fn open_capture(
    source: SourceKind,
    out_path: std::path::PathBuf,
) -> Result<CaptureHandle, String> {
    linux::open_capture(source, out_path)
}

#[cfg(target_os = "windows")]
pub fn open_capture(
    source: SourceKind,
    out_path: std::path::PathBuf,
) -> Result<CaptureHandle, String> {
    windows::open_capture(source, out_path)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn open_capture(
    _source: SourceKind,
    _out_path: std::path::PathBuf,
) -> Result<CaptureHandle, String> {
    Err("only linux + windows supported in MVP".into())
}
