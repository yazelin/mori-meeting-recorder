//! 音訊 capture / write — per-track WAV writer + 平台 capture impl(linux / windows)。

pub mod writer;
pub mod levels;
pub mod vad;
pub mod devices;

use serde::{Deserialize, Serialize};

/// 一個 source 的「分類」— 決定預設 visibility + 在 segment 上的 source_kind 欄。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    MeetingSystem,
    MicInternal,
    MeetingRoom,
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
            Self::MeetingRoom => Visibility::Public,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::MeetingSystem => "meeting_system",
            Self::MicInternal => "mic_internal",
            Self::MeetingRoom => "meeting_room",
        }
    }

    pub fn track_name(self) -> &'static str {
        match self {
            Self::MeetingSystem => "system",
            Self::MicInternal => "mic-internal",
            Self::MeetingRoom => "room",
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
/// 值已經被 audio loop 套過 fast-attack / slow-release smoothing(看 linux.rs / windows.rs)。
#[derive(Debug, Clone, Copy)]
pub struct SignalMeter {
    pub peak_rms_db: f32,  // smoothed RMS in dB(capsule has_signal 用)
    pub peak_db: f32,      // smoothed peak in dB(VU meter peak segment 用)
    pub last_sample_at_unix_ms: u64,
}

impl Default for SignalMeter {
    fn default() -> Self {
        // 預設 DB_FLOOR(-120),第一個 audio chunk 進來時 smooth_db 會 attack snap-up,
        // 不會從 0 dB(full scale)花 1 秒 release 到實際值。
        Self {
            peak_rms_db: -120.0,
            peak_db: -120.0,
            last_sample_at_unix_ms: 0,
        }
    }
}

impl SignalMeter {
    pub fn has_signal(&self, now_unix_ms: u64) -> bool {
        // 過去 500ms 有 sample 且 peak RMS > -40 dB
        now_unix_ms.saturating_sub(self.last_sample_at_unix_ms) < 500 && self.peak_rms_db > -40.0
    }
}

/// 開啟一個 capture stream 給指定 source,把 samples 寫進指定 WAV path。回 handle。
/// 回 (handle, speech_rx)。speech_rx 收 VadChunker 切出的 speech 段,recorder 把它配給
/// transcribe worker。receiver 走 tuple 而非塞進 handle,因為 Receiver 不可 clone、
/// 要 move 出來給 worker thread。
pub type CaptureResult = (CaptureHandle, std::sync::mpsc::Receiver<vad::SpeechSegment>);

/// `pending`:per-track 佇列計數(送進 channel 時 +1,worker 轉完 -1)→ 真實待轉段數。
#[cfg(target_os = "linux")]
pub fn open_capture(
    source: SourceKind,
    out_path: std::path::PathBuf,
    vad_cfg: vad::VadConfig,
    pending: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) -> Result<CaptureResult, String> {
    linux::open_capture(source, out_path, vad_cfg, pending)
}

#[cfg(target_os = "windows")]
pub fn open_capture(
    source: SourceKind,
    out_path: std::path::PathBuf,
    vad_cfg: vad::VadConfig,
    pending: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) -> Result<CaptureResult, String> {
    windows::open_capture(source, out_path, vad_cfg, pending)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn open_capture(
    _source: SourceKind,
    _out_path: std::path::PathBuf,
    _vad_cfg: vad::VadConfig,
    _pending: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) -> Result<CaptureResult, String> {
    Err("only linux + windows supported in MVP".into())
}

#[cfg(test)]
mod source_kind_tests {
    use super::*;

    #[test]
    fn meeting_room_track_name_visibility_and_str() {
        assert_eq!(SourceKind::MeetingRoom.track_name(), "room");
        assert_eq!(SourceKind::MeetingRoom.as_str(), "meeting_room");
        assert_eq!(SourceKind::MeetingRoom.default_visibility(), Visibility::Public);
    }
}
