//! ~/.mori/meetings/<session-id>/ 目錄佈局 + path getters。純函式 + filesystem。

use crate::audio::SourceKind;
use std::path::{Path, PathBuf};

pub struct SessionStore {
    pub session_id: String,
    pub root: PathBuf,
}

impl SessionStore {
    /// 從既有 session root 建 store(不建目錄)。session_id 取 root 末段 dir 名,
    /// 供只需要 path getter 的 caller(如摘要 writer / read_summary_md)用,
    /// 避免在外面重組 struct literal + 多餘 clone。
    pub fn from_root(root: PathBuf) -> Self {
        let session_id = root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        Self { session_id, root }
    }

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
    pub fn meeting_md_path(&self) -> PathBuf { self.root.join("meeting.md") }
    pub fn timeline_path(&self) -> PathBuf { self.root.join("timeline.json") }

    // ── 摘要產物(§4.3):跟逐字稿匯出檔並列,一眼可分。 ──
    pub fn summary_public_md_path(&self) -> PathBuf { self.root.join("meeting.summary.public.md") }
    pub fn summary_internal_md_path(&self) -> PathBuf { self.root.join("meeting.summary.internal.md") }
    pub fn summary_audit_path(&self) -> PathBuf { self.root.join("summary.audit.jsonl") }
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

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SessionSummary {
    pub id: String,
    pub started_at: String,
    pub duration_secs: u64,
    pub public_segs: u32,
    pub internal_segs: u32,
    pub preview: Option<String>,
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub organized: bool,
    pub corrupt: bool,
}

pub fn read_session_summary(id: &str, base: &std::path::Path) -> SessionSummary {
    let store = SessionStore { session_id: id.to_string(), root: base.join(id) };
    let timeline_path = store.timeline_path();
    let timeline_str = match std::fs::read_to_string(&timeline_path) {
        Ok(s) => s,
        Err(_) => {
            return SessionSummary {
                id: id.to_string(),
                started_at: String::new(),
                duration_secs: 0,
                public_segs: 0,
                internal_segs: 0,
                preview: None,
                topic: String::new(),
                organized: false,
                corrupt: true,
            };
        }
    };
    let v: serde_json::Value = match serde_json::from_str(&timeline_str) {
        Ok(v) => v,
        Err(_) => {
            return SessionSummary {
                id: id.to_string(),
                started_at: String::new(),
                duration_secs: 0,
                public_segs: 0,
                internal_segs: 0,
                preview: None,
                topic: String::new(),
                organized: false,
                corrupt: true,
            };
        }
    };
    let started_at = v.get("started_at").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let duration_secs = v.get("duration_secs").and_then(|x| x.as_u64()).unwrap_or(0);

    let (public_segs, internal_segs) = count_segments_by_visibility(&store.root);
    // 現場模式只產 meeting.md(無 public.md)→ 預覽改讀 meeting.md。
    let recording_mode = v.get("recording_mode").and_then(|x| x.as_str()).unwrap_or("online");
    let preview = if recording_mode == "in_person" {
        read_public_md_preview(&store.meeting_md_path())
    } else {
        read_public_md_preview(&store.public_md_path())
    };

    let topic = std::fs::read_to_string(store.root.join("meeting-info.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("topic").and_then(|x| x.as_str()).map(|s| s.to_string()))
        .unwrap_or_default();
    let organized = std::fs::read_to_string(store.root.join("session-state.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("organized").and_then(|x| x.as_bool()))
        .unwrap_or(false);

    SessionSummary {
        id: id.to_string(),
        started_at,
        duration_secs,
        public_segs,
        internal_segs,
        preview,
        topic,
        organized,
        corrupt: false,
    }
}

/// 寫該場 session-state.json 的 organized 旗標(獨立檔,不碰 meeting-info.json)。
pub fn write_organized(session_root: &std::path::Path, organized: bool) -> Result<(), String> {
    let body = serde_json::to_string_pretty(&serde_json::json!({ "organized": organized }))
        .map_err(|e| e.to_string())?;
    std::fs::write(session_root.join("session-state.json"), body)
        .map_err(|e| format!("write session-state.json: {e}"))
}

/// 全文搜尋:掃 meetings_dir 下每場匯出的逐字稿 md(meeting.md / public / internal),
/// query(去空白、小寫)子字串命中 → 收 session id。空 query → 空清單。讀檔失敗該場跳過。
pub fn search_fulltext(meetings_dir: &std::path::Path, query: &str) -> Vec<String> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    let Ok(rd) = std::fs::read_dir(meetings_dir) else { return hits };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("meeting-") || !entry.path().is_dir() {
            continue;
        }
        let root = entry.path();
        for md in ["meeting.md", "meeting.public.md", "meeting.internal.md"] {
            if let Ok(text) = std::fs::read_to_string(root.join(md)) {
                if text.to_lowercase().contains(&q) {
                    hits.push(name.clone());
                    break;
                }
            }
        }
    }
    hits.sort();
    hits
}

fn count_segments_by_visibility(session_dir: &std::path::Path) -> (u32, u32) {
    let transcript_dir = session_dir.join("transcript");
    let entries = match std::fs::read_dir(&transcript_dir) {
        Ok(e) => e,
        Err(_) => return (0, 0),
    };
    let mut pub_count = 0_u32;
    let mut int_count = 0_u32;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") { continue; }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for line in content.lines() {
            if line.trim().is_empty() { continue; }
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            match v.get("visibility").and_then(|x| x.as_str()) {
                Some("public")   => pub_count += 1,
                Some("internal") => int_count += 1,
                _ => {}
            }
        }
    }
    (pub_count, int_count)
}

fn read_public_md_preview(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        if trimmed.starts_with('#') { continue; }
        if trimmed.starts_with('>') { continue; }
        if trimmed.starts_with("_(") { return None; }
        let mut s = trimmed.to_string();
        if s.chars().count() > 120 {
            s = s.chars().take(120).collect::<String>() + "…";
        }
        return Some(s);
    }
    None
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

    #[test]
    fn read_session_summary_missing_timeline_returns_corrupt() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("meeting-x")).unwrap();
        let s = read_session_summary("meeting-x", tmp.path());
        assert!(s.corrupt);
        assert_eq!(s.id, "meeting-x");
        assert_eq!(s.public_segs, 0);
    }

    fn write_test_file(path: &std::path::Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn read_session_summary_happy_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_dir = tmp.path().join("meeting-x");
        write_test_file(
            &session_dir.join("timeline.json"),
            r#"{"schema_version":1,"session_id":"meeting-x","started_at":"2026-05-28T14:30:00+08:00","stopped_at":"2026-05-28T15:00:00+08:00","duration_secs":1800,"tracks":[],"exports":{"public":"","internal":"","timeline":""}}"#,
        );
        write_test_file(
            &session_dir.join("transcript").join("system.segments.jsonl"),
            r#"{"id":"s1","session_id":"meeting-x","track":"system","source_kind":"meeting_system","visibility":"public","start_ms":0,"end_ms":1000,"text":"a","is_final":true}
{"id":"s2","session_id":"meeting-x","track":"system","source_kind":"meeting_system","visibility":"public","start_ms":1000,"end_ms":2000,"text":"b","is_final":true}
"#,
        );
        write_test_file(
            &session_dir.join("transcript").join("mic-internal.segments.jsonl"),
            r#"{"id":"m1","session_id":"meeting-x","track":"mic-internal","source_kind":"mic_internal","visibility":"internal","start_ms":500,"end_ms":1500,"text":"c","is_final":true}
"#,
        );
        write_test_file(
            &session_dir.join("meeting.public.md"),
            "# Meeting Notes — 2026-05-28 14:30\n\n> Source: meeting_system.\n\n客戶要求三週後上線\n再說\n",
        );

        let s = read_session_summary("meeting-x", tmp.path());
        assert!(!s.corrupt);
        assert_eq!(s.id, "meeting-x");
        assert_eq!(s.started_at, "2026-05-28T14:30:00+08:00");
        assert_eq!(s.duration_secs, 1800);
        assert_eq!(s.public_segs, 2);
        assert_eq!(s.internal_segs, 1);
        assert_eq!(s.preview.as_deref(), Some("客戶要求三週後上線"));
    }

    #[test]
    fn read_session_summary_empty_public_md_yields_none_preview() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_dir = tmp.path().join("m");
        write_test_file(
            &session_dir.join("timeline.json"),
            r#"{"schema_version":1,"session_id":"m","started_at":"2026-05-28T14:30:00+08:00","stopped_at":"2026-05-28T14:30:01+08:00","duration_secs":1,"tracks":[],"exports":{"public":"","internal":"","timeline":""}}"#,
        );
        write_test_file(
            &session_dir.join("meeting.public.md"),
            "# Meeting Notes — empty\n\n> Source: meeting_system.\n\n_(no segments)_\n",
        );
        let s = read_session_summary("m", tmp.path());
        assert!(!s.corrupt);
        assert_eq!(s.preview, None);
    }

    #[test]
    fn write_organized_roundtrips_and_keeps_topic() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let root = base.join("meeting-z");
        std::fs::create_dir_all(root.join("transcript")).unwrap();
        std::fs::write(root.join("timeline.json"),
            r#"{"schema_version":1,"session_id":"meeting-z","started_at":"t","stopped_at":"t","duration_secs":1,"tracks":[],"exports":{"public":"","internal":""}}"#).unwrap();
        std::fs::write(root.join("meeting-info.json"), r#"{"topic":"專案A","participants":""}"#).unwrap();

        write_organized(&root, true).unwrap();
        let s = read_session_summary("meeting-z", base);
        assert!(s.organized);
        assert_eq!(s.topic, "專案A"); // 分檔,topic 不受影響

        write_organized(&root, false).unwrap();
        assert!(!read_session_summary("meeting-z", base).organized);
    }

    #[test]
    fn search_fulltext_matches_md_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        for (id, md, text) in [
            ("meeting-a", "meeting.public.md", "討論 Roadmap 與排程"),
            ("meeting-b", "meeting.md", "現場閒聊"),
        ] {
            let root = base.join(id);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join(md), text).unwrap();
        }
        let hits = search_fulltext(base, "roadmap"); // 小寫 query 命中大寫 Roadmap
        assert_eq!(hits, vec!["meeting-a".to_string()]);
        assert!(search_fulltext(base, "不存在的字").is_empty());
        assert!(search_fulltext(base, "   ").is_empty()); // 空白 query
    }

    #[test]
    fn read_session_summary_includes_topic_and_organized() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let root = base.join("meeting-x");
        std::fs::create_dir_all(root.join("transcript")).unwrap();
        std::fs::write(root.join("timeline.json"),
            r#"{"schema_version":1,"session_id":"meeting-x","started_at":"2026-06-03T10:00:00+08:00","stopped_at":"t","duration_secs":60,"tracks":[],"exports":{"public":"","internal":""}}"#).unwrap();
        std::fs::write(root.join("meeting-info.json"), r#"{"topic":"季度檢討","participants":"甲,乙"}"#).unwrap();
        std::fs::write(root.join("session-state.json"), r#"{"organized":true}"#).unwrap();

        let s = read_session_summary("meeting-x", base);
        assert_eq!(s.topic, "季度檢討");
        assert!(s.organized);
        assert!(!s.corrupt);
    }

    #[test]
    fn read_session_summary_defaults_when_info_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let root = base.join("meeting-y");
        std::fs::create_dir_all(root.join("transcript")).unwrap();
        std::fs::write(root.join("timeline.json"),
            r#"{"schema_version":1,"session_id":"meeting-y","started_at":"t","stopped_at":"t","duration_secs":1,"tracks":[],"exports":{"public":"","internal":""}}"#).unwrap();
        // 無 meeting-info.json / session-state.json
        let s = read_session_summary("meeting-y", base);
        assert_eq!(s.topic, "");
        assert!(!s.organized);
    }
}
