//! Integration test:不跑真 cpal / whisper(那些是 manual e2e),測「對給定 fake segments,
//! exporter + segments JSONL 寫對位置 + visibility 對」這條鏈。

use mori_meeting_recorder::audio::SourceKind;
use mori_meeting_recorder::exporter::{export, Exports, SessionMeta, TrackMeta};
use mori_meeting_recorder::session_store::SessionStore;
use mori_meeting_recorder::transcribe::Segment;
use tempfile::TempDir;

fn fake_seg(kind: SourceKind, idx: u64, text: &str) -> Segment {
    Segment {
        id: format!("seg_{:03}", idx),
        session_id: "meeting-test".into(),
        track: kind.track_name().into(),
        source_kind: kind.as_str().into(),
        visibility: match kind.default_visibility() {
            mori_meeting_recorder::audio::Visibility::Public => "public".into(),
            mori_meeting_recorder::audio::Visibility::Internal => "internal".into(),
        },
        start_ms: idx * 1000,
        end_ms: idx * 1000 + 500,
        text: text.into(),
        is_final: true,
        confidence: None,
        speaker: None,
        speaker_mixed: false,
    }
}

#[test]
fn end_to_end_exporter_chain_writes_correct_files() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::create("meeting-test", tmp.path()).unwrap();
    let segs = vec![
        fake_seg(SourceKind::MeetingSystem, 1, "客戶說 A"),
        fake_seg(SourceKind::MicInternal, 2, "我方私聊"),
        fake_seg(SourceKind::MeetingSystem, 3, "客戶說 B"),
    ];
    let meta = SessionMeta {
        schema_version: 1,
        session_id: "meeting-test".into(),
        started_at: "2026-05-28T14:30:00+08:00".into(),
        stopped_at: "2026-05-28T15:15:00+08:00".into(),
        duration_secs: 2700,
        tracks: vec![
            TrackMeta {
                name: "system".into(),
                source_kind: "meeting_system".into(),
                visibility: "public".into(),
                audio_path: "audio/system.wav".into(),
                transcript_path: "transcript/system.segments.jsonl".into(),
                segment_count: 2,
            },
            TrackMeta {
                name: "mic-internal".into(),
                source_kind: "mic_internal".into(),
                visibility: "internal".into(),
                audio_path: "audio/mic-internal.wav".into(),
                transcript_path: "transcript/mic-internal.segments.jsonl".into(),
                segment_count: 1,
            },
        ],
        exports: Exports {
            public: "meeting.public.md".into(),
            internal: "meeting.internal.md".into(),
        },
    };
    let (pub_md, int_md, timeline) = export(&segs, &meta).unwrap();
    std::fs::write(store.public_md_path(), &pub_md).unwrap();
    std::fs::write(store.internal_md_path(), &int_md).unwrap();
    std::fs::write(store.timeline_path(), &timeline).unwrap();

    let pub_read = std::fs::read_to_string(store.public_md_path()).unwrap();
    let int_read = std::fs::read_to_string(store.internal_md_path()).unwrap();
    assert!(pub_read.contains("客戶說 A"));
    assert!(pub_read.contains("客戶說 B"));
    assert!(!pub_read.contains("我方私聊"));
    assert!(int_read.contains("(內部)我方私聊"));
    assert!(!int_read.contains("客戶說 A"));
}
