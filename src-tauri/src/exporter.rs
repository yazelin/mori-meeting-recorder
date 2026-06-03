//! 從 Vec<Segment> + SessionMeta 產生 meeting.public.md / meeting.internal.md / timeline.json。
//! 純函式 — IO 由呼叫端(recorder.rs)做。

use crate::audio::SourceKind;
use crate::transcribe::Segment;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub schema_version: u32,
    pub session_id: String,
    pub started_at: String,
    pub stopped_at: String,
    pub duration_secs: u64,
    pub tracks: Vec<TrackMeta>,
    pub exports: Exports,
    /// 這場當時用的 whisper 轉錄模型(stop 時記,e.g. "small" / "large-v3-turbo")。
    /// 記下來是為了可重現/debug:將來換更大模型,回頭看舊紀錄才知道哪場用哪個跑的。
    #[serde(default)]
    pub transcribe_model: String,
    /// 分人用的 segmentation / embedding 模型名(跑分人時記;沒分過 = None)。
    #[serde(default)]
    pub diarize_seg_model: Option<String>,
    #[serde(default)]
    pub diarize_emb_model: Option<String>,
    /// 錄音模式("online" / "in_person")。舊 timeline.json 無此欄 → serde default 空字串。
    #[serde(default)]
    pub recording_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackMeta {
    pub name: String,
    pub source_kind: String,
    pub visibility: String,
    pub audio_path: String,
    pub transcript_path: String,
    pub segment_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
/// public_md:只含 visibility=public 的段(hard rule #3 — supplement 完全不進此檔)。
/// internal_md:所有段 + 若有 supplement=true 的段,末尾附加「決議依據 / 內部補充」區塊。
pub fn export(
    segments: &[Segment],
    meta: &SessionMeta,
    speakers: &[crate::diarize::SpeakerInfo],
) -> Result<(String, String, String), String> {
    let public_md = render_md(segments, "public", &format!(
        "# Meeting Notes — {}\n\n> Source: meeting_system. Mic-internal not included.\n\n",
        meta.started_at
    ), speakers);
    let mut internal_md = render_md(segments, "internal", &format!(
        "# Meeting — 內部備忘 — {}\n\n> 包含 mic-internal segments(本機麥克風)。**內部用途,不對外發。**\n\n",
        meta.started_at
    ), speakers);
    // 決議依據 / 內部補充:任何軌 supplement=true 的段,依 start_ms 排序,附在 internal_md 末尾。
    // public_md 絕對不加(hard rule #3)。
    let mut supplement_segs: Vec<&Segment> = segments.iter().filter(|s| s.supplement).collect();
    if !supplement_segs.is_empty() {
        supplement_segs.sort_by_key(|s| s.start_ms);
        internal_md.push_str("\n## 決議依據 / 內部補充\n\n");
        for s in supplement_segs {
            internal_md.push_str(&format!("[{}] {}\n", fmt_ts(s.start_ms), s.text));
        }
    }
    let timeline = serde_json::to_string_pretty(meta).map_err(|e| format!("timeline json: {e}"))?;
    Ok((public_md, internal_md, timeline))
}

/// 現場模式單檔匯出:room 軌(visibility=public)全部段 → 單一 meeting.md。
/// 現場無「客戶 / 我方」之分 → 不產 public/internal、無補充區塊。回 (meeting_md, timeline_json)。
pub fn export_single(
    segments: &[Segment],
    meta: &SessionMeta,
    speakers: &[crate::diarize::SpeakerInfo],
) -> Result<(String, String), String> {
    let meeting_md = render_md(
        segments,
        "public",
        &format!("# 會議記錄 — {}\n\n> 現場會議(單一收音來源)。\n\n", meta.started_at),
        speakers,
    );
    let timeline = serde_json::to_string_pretty(meta).map_err(|e| format!("timeline json: {e}"))?;
    Ok((meeting_md, timeline))
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
            supplement: false,
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
            transcribe_model: "small".into(),
            diarize_seg_model: None,
            diarize_emb_model: None,
            recording_mode: "in_person".into(),
        }
    }

    #[test]
    fn session_meta_model_fields_round_trip_and_default() {
        // 新欄位 serialize→deserialize 保留
        let mut m = meta("x");
        m.transcribe_model = "large-v3-turbo".into();
        m.diarize_seg_model = Some("pyannote-segmentation-3-0".into());
        let s = serde_json::to_string(&m).unwrap();
        let back: SessionMeta = serde_json::from_str(&s).unwrap();
        assert_eq!(back.transcribe_model, "large-v3-turbo");
        assert_eq!(back.diarize_seg_model.as_deref(), Some("pyannote-segmentation-3-0"));
        // 舊 timeline.json(沒這些欄位)→ serde default(空字串 / None),不報錯
        let old = r#"{"schema_version":1,"session_id":"x","started_at":"t","stopped_at":"t","duration_secs":1,"tracks":[],"exports":{"public":"","internal":""}}"#;
        let parsed: SessionMeta = serde_json::from_str(old).unwrap();
        assert_eq!(parsed.transcribe_model, "");
        assert_eq!(parsed.diarize_seg_model, None);
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

    #[test]
    fn supplement_true_appears_in_internal_section_and_never_in_public() {
        // 決議依據 / 內部補充:supplement=true 的段出現在 internal_md 末尾區塊,public_md 完全不含。
        let mut s_pub = seg("p1", "meeting_system", "public", 1000, "客戶主張");
        s_pub.supplement = true; // 即使是 public 軌,supplement 仍只進 internal
        let s_int = seg("i1", "mic_internal", "internal", 3000, "我方私聊");
        let mut s_supp = seg("i2", "mic_internal", "internal", 2000, "這是決議依據");
        s_supp.supplement = true;
        let segs = vec![s_pub, s_int, s_supp];
        let (pub_md, int_md, _) = export(&segs, &meta("t"), &[]).unwrap();

        // ── public.md HARD RULE #3:supplement section 絕對不出現 ──
        assert!(!pub_md.contains("決議依據 / 內部補充"),
            "public.md must NOT contain supplement section, got:\n{pub_md}");
        assert!(!pub_md.contains("這是決議依據"),
            "public.md must NOT contain supplement text, got:\n{pub_md}");

        // ── internal.md:有 supplement 段時出現區塊標題 ──
        assert!(int_md.contains("## 決議依據 / 內部補充"),
            "internal.md should contain supplement section header, got:\n{int_md}");
        // 兩個 supplement=true 的段都進區塊(依 start_ms:2000 先,1000 後)
        assert!(int_md.contains("這是決議依據"),
            "internal.md should contain supplement text, got:\n{int_md}");
        assert!(int_md.contains("客戶主張"),
            "internal.md supplement section should contain public-track supplement seg, got:\n{int_md}");

        // ── supplement section 排在 start_ms 升序 ──
        // 在 supplement 區塊標題之後取子字串,避免 主體段落(start_ms=2000)干擾排序驗證。
        let supp_section = int_md.split("## 決議依據 / 內部補充").nth(1)
            .expect("supplement section header should exist in internal_md");
        let pos_1000_in_supp = supp_section.find("客戶主張").unwrap();
        let pos_2000_in_supp = supp_section.find("這是決議依據").unwrap();
        // start_ms 1000(客戶主張)< start_ms 2000(這是決議依據)→ 前者應先出現
        assert!(pos_1000_in_supp < pos_2000_in_supp,
            "supplement entries should be sorted by start_ms (1000 before 2000)");

        // ── 非 supplement 的內部段只在主體,不出現在 supplement 區塊 ──
        // (「我方私聊」應在主體,不該也出現在補充區塊 — 這裡驗它確實存在且不是補充)
        assert!(int_md.contains("我方私聊"),
            "internal.md should still contain non-supplement internal seg, got:\n{int_md}");
    }

    #[test]
    fn supplement_section_omitted_when_none_flagged() {
        // 全部 supplement=false(預設)→ internal.md 不出現「決議依據 / 內部補充」區塊。
        let segs = vec![
            seg("s1", "meeting_system", "public", 1000, "正常段"),
            seg("m1", "mic_internal", "internal", 2000, "內部段"),
        ];
        let (pub_md, int_md, _) = export(&segs, &meta("t"), &[]).unwrap();
        assert!(!int_md.contains("決議依據 / 內部補充"),
            "internal.md should NOT contain supplement section when none flagged, got:\n{int_md}");
        assert!(!pub_md.contains("決議依據 / 內部補充"),
            "public.md should never contain supplement section, got:\n{pub_md}");
    }

    #[test]
    fn export_single_room_segments_into_single_md() {
        let mut s1 = seg("r1", "meeting_room", "public", 1000, "大家好");
        s1.track = "room".into();
        let mut s2 = seg("r2", "meeting_room", "public", 2000, "開始開會");
        s2.track = "room".into();
        let (meeting_md, timeline) = export_single(&[s1, s2], &meta("m"), &[]).unwrap();
        assert!(meeting_md.contains("大家好"));
        assert!(meeting_md.contains("開始開會"));
        assert!(meeting_md.contains("會議記錄"));
        // 不走 public/internal 分流 header
        assert!(!meeting_md.contains("Mic-internal not included"));
        let v: serde_json::Value = serde_json::from_str(&timeline).unwrap();
        assert_eq!(v["session_id"], "m");
        assert_eq!(v["recording_mode"], "in_person");
    }
}
