//! ~/.mori/meetings/<session-id>/ 目錄佈局 + path getters。純函式 + filesystem。

use crate::audio::SourceKind;
use std::path::{Path, PathBuf};

pub struct SessionStore {
    pub session_id: String,
    pub root: PathBuf,
}

impl SessionStore {
    /// 建出 `<base>/<session_id>/{audio,transcript}/` 並回 store。
    pub fn create(session_id: &str, base: &Path) -> Result<Self, String> {
        let root = base.join(session_id);
        std::fs::create_dir_all(root.join("audio")).map_err(|e| format!("mkdir audio: {e}"))?;
        std::fs::create_dir_all(root.join("transcript")).map_err(|e| format!("mkdir transcript: {e}"))?;
        Ok(Self { session_id: session_id.to_string(), root })
    }

    pub fn audio_path(&self, kind: SourceKind) -> PathBuf {
        self.root.join("audio").join(format!("{}.wav", kind.track_name()))
    }

    pub fn segments_path(&self, kind: SourceKind) -> PathBuf {
        self.root.join("transcript").join(format!("{}.segments.jsonl", kind.track_name()))
    }

    pub fn public_md_path(&self) -> PathBuf { self.root.join("meeting.public.md") }
    pub fn internal_md_path(&self) -> PathBuf { self.root.join("meeting.internal.md") }
    pub fn timeline_path(&self) -> PathBuf { self.root.join("timeline.json") }
}

/// 預設 base dir = `~/.mori/meetings/`。
pub fn default_meetings_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("meetings"))
        .unwrap_or_else(|| PathBuf::from(".mori/meetings"))
}

/// 產生新 session id:`meeting-YYYYMMDD-HHMMSS`(local time)。
pub fn new_session_id(now: chrono::DateTime<chrono::Local>) -> String {
    format!("meeting-{}", now.format("%Y%m%d-%H%M%S"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::TempDir;

    #[test]
    fn create_makes_audio_and_transcript_dirs() {
        let tmp = TempDir::new().unwrap();
        let s = SessionStore::create("meeting-test", tmp.path()).unwrap();
        assert!(s.root.join("audio").is_dir());
        assert!(s.root.join("transcript").is_dir());
    }

    #[test]
    fn path_getters_return_expected_layout() {
        let tmp = TempDir::new().unwrap();
        let s = SessionStore::create("meeting-x", tmp.path()).unwrap();
        assert_eq!(s.audio_path(SourceKind::MeetingSystem), tmp.path().join("meeting-x/audio/system.wav"));
        assert_eq!(s.audio_path(SourceKind::MicInternal), tmp.path().join("meeting-x/audio/mic-internal.wav"));
        assert_eq!(s.segments_path(SourceKind::MeetingSystem), tmp.path().join("meeting-x/transcript/system.segments.jsonl"));
        assert_eq!(s.public_md_path(), tmp.path().join("meeting-x/meeting.public.md"));
        assert_eq!(s.internal_md_path(), tmp.path().join("meeting-x/meeting.internal.md"));
        assert_eq!(s.timeline_path(), tmp.path().join("meeting-x/timeline.json"));
    }

    #[test]
    fn session_id_has_meeting_prefix_and_timestamp() {
        let now = chrono::Local.with_ymd_and_hms(2026, 5, 28, 14, 30, 0).unwrap();
        let id = new_session_id(now);
        assert_eq!(id, "meeting-20260528-143000");
    }
}
