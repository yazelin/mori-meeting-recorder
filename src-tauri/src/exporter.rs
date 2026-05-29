//! 從 Vec<Segment> + SessionMeta 產生 meeting.public.md / meeting.internal.md / timeline.json。
//! 純函式 — IO 由呼叫端(recorder.rs)做。

use crate::audio::SourceKind;
use crate::transcribe::Segment;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SessionMeta {
    pub schema_version: u32,
    pub session_id: String,
    pub started_at: String,
    pub stopped_at: String,
    pub duration_secs: u64,
    pub tracks: Vec<TrackMeta>,
    pub exports: Exports,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackMeta {
    pub name: String,
    pub source_kind: String,
    pub visibility: String,
    pub audio_path: String,
    pub transcript_path: String,
    pub segment_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Exports {
    pub public: String,
    pub internal: String,
}

/// 把 ms 轉成 hh:mm:ss 字串(供 markdown 顯示)。
fn fmt_ts(ms: u64) -> String {
    let total = ms / 1000;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

/// 從 segments + meta 產生 (public_md, internal_md, timeline_json) 三條字串。
pub fn export(
    segments: &[Segment],
    meta: &SessionMeta,
    speakers: &[crate::diarize::SpeakerInfo],
) -> Result<(String, String, String), String> {
    let public_md = render_md(segments, "public", &format!(
        "# Meeting Notes — {}\n\n> Source: meeting_system. Mic-internal not included.\n\n",
        meta.started_at
    ), speakers);
    let internal_md = render_md(segments, "internal", &format!(
        "# Meeting — 內部備忘 — {}\n\n> 包含 mic-internal segments(本機麥克風)。**內部用途,不對外發。**\n\n",
        meta.started_at
    ), speakers);
    let timeline = serde_json::to_string_pretty(meta).map_err(|e| format!("timeline json: {e}"))?;
    Ok((public_md, internal_md, timeline))
}

fn render_md(
    segments: &[Segment],
    visibility: &str,
    header: &str,
    speakers: &[crate::diarize::SpeakerInfo],
) -> String {
    let mut out = String::from(header);
    let mut filtered: Vec<&Segment> = segments.iter().filter(|s| s.visibility == visibility).collect();
    filtered.sort_by_key(|s| s.start_ms);
    if filtered.is_empty() {
        out.push_str("_(no segments)_\n");
        return out;
    }
    for s in filtered {
        let internal_prefix = if visibility == "internal" && s.source_kind == "mic_internal" {
            "(內部)"
        } else {
            ""
        };
        let speaker_prefix = match &s.speaker {
            Some(id) => speakers
                .iter()
                .find(|sp| &sp.id == id)
                .map(|sp| format!("{}: ", sp.display))
                .unwrap_or_default(),
            None => String::new(),
        };
        out.push_str(&format!("[{}] {}{}{}\n", fmt_ts(s.start_ms), internal_prefix, speaker_prefix, s.text));
    }
    out
}

/// 算出 segments 對應每個 track 的數量 — 用來填 TrackMeta.segment_count。
pub fn segment_count(segments: &[Segment], kind: SourceKind) -> usize {
    segments.iter().filter(|s| s.source_kind == kind.as_str()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(id: &str, source: &str, vis: &str, start: u64, text: &str) -> Segment {
        Segment {
            id: id.into(),
            session_id: "t".into(),
            track: if source == "meeting_system" { "system".into() } else { "mic-internal".into() },
            source_kind: source.into(),
            visibility: vis.into(),
            start_ms: start,
            end_ms: start + 1000,
            text: text.into(),
            is_final: true,
            confidence: None,
            speaker: None,
            speaker_mixed: false,
        }
    }

    fn meta(session_id: &str) -> SessionMeta {
        SessionMeta {
            schema_version: 1,
            session_id: session_id.into(),
            started_at: "2026-05-28T14:30:00+08:00".into(),
            stopped_at: "2026-05-28T15:15:00+08:00".into(),
            duration_secs: 2700,
            tracks: vec![],
            exports: Exports {
                public: "meeting.public.md".into(),
                internal: "meeting.internal.md".into(),
            },
        }
    }

    #[test]
    fn public_md_only_contains_public_visibility() {
        let segs = vec![
            seg("s1", "meeting_system", "public", 1000, "客戶說的"),
            seg("s2", "mic_internal", "internal", 2000, "我方策略"),
            seg("s3", "meeting_system", "public", 3000, "客戶又說的"),
        ];
        let (pub_md, _, _) = export(&segs, &meta("t"), &[]).unwrap();
        assert!(pub_md.contains("客戶說的"));
        assert!(pub_md.contains("客戶又說的"));
        assert!(!pub_md.contains("我方策略"));
    }

    #[test]
    fn internal_md_only_contains_internal_with_prefix() {
        let segs = vec![
            seg("s1", "meeting_system", "public", 1000, "客戶說的"),
            seg("s2", "mic_internal", "internal", 2000, "我方策略"),
        ];
        let (_, int_md, _) = export(&segs, &meta("t"), &[]).unwrap();
        assert!(int_md.contains("(內部)我方策略"));
        assert!(!int_md.contains("客戶說的"));
    }

    #[test]
    fn timeline_json_is_valid_json_with_session_id() {
        let (_, _, tl) = export(&[], &meta("meeting-x"), &[]).unwrap();
        let v: serde_json::Value = serde_json::from_str(&tl).unwrap();
        assert_eq!(v["session_id"], "meeting-x");
        assert_eq!(v["schema_version"], 1);
    }

    #[test]
    fn empty_segments_produce_no_segments_placeholder() {
        let (pub_md, int_md, _) = export(&[], &meta("t"), &[]).unwrap();
        assert!(pub_md.contains("(no segments)"));
        assert!(int_md.contains("(no segments)"));
    }

    #[test]
    fn segments_sorted_by_start_ms_in_output() {
        let segs = vec![
            seg("s1", "meeting_system", "public", 5000, "後面"),
            seg("s2", "meeting_system", "public", 1000, "前面"),
        ];
        let (pub_md, _, _) = export(&segs, &meta("t"), &[]).unwrap();
        let pos_qian = pub_md.find("前面").unwrap();
        let pos_hou = pub_md.find("後面").unwrap();
        assert!(pos_qian < pos_hou);
    }

    #[test]
    fn fmt_ts_formats_hours_minutes_seconds() {
        assert_eq!(fmt_ts(0), "00:00:00");
        assert_eq!(fmt_ts(123_000), "00:02:03");
        assert_eq!(fmt_ts(3_723_000), "01:02:03");
    }

    #[test]
    fn segment_count_filters_by_source_kind() {
        let segs = vec![
            seg("s1", "meeting_system", "public", 0, "a"),
            seg("s2", "mic_internal", "internal", 0, "b"),
            seg("s3", "meeting_system", "public", 0, "c"),
        ];
        assert_eq!(segment_count(&segs, SourceKind::MeetingSystem), 2);
        assert_eq!(segment_count(&segs, SourceKind::MicInternal), 1);
    }

    #[test]
    fn render_md_prefixes_speaker_display_when_present() {
        use crate::diarize::SpeakerInfo;
        let mut s = seg("a", "meeting_system", "public", 1000, "你好");
        s.speaker = Some("S1".into());
        let speakers = vec![SpeakerInfo { id: "S1".into(), display: "亞澤".into(), track: "system".into() }];
        let (public_md, _, _) = export(&[s], &meta("m"), &speakers).unwrap();
        assert!(public_md.contains("亞澤: 你好"), "got: {public_md}");
    }

    #[test]
    fn render_md_no_prefix_when_no_speaker() {
        let s = seg("a", "meeting_system", "public", 1000, "你好");
        let (public_md, _, _) = export(&[s], &meta("m"), &[]).unwrap();
        assert!(public_md.contains("] 你好"), "got: {public_md}");
        assert!(!public_md.contains(": 你好"));
    }

    #[test]
    fn internal_mic_segment_with_speaker_shows_both_prefixes() {
        use crate::diarize::SpeakerInfo;
        let mut s = seg("b", "mic_internal", "internal", 1000, "私聊");
        s.speaker = Some("S2".into());
        let speakers = vec![SpeakerInfo { id: "S2".into(), display: "亞澤".into(), track: "mic-internal".into() }];
        let (_, int_md, _) = export(&[s], &meta("m"), &speakers).unwrap();
        assert!(int_md.contains("(內部)亞澤: 私聊"), "got: {int_md}");
    }
}
