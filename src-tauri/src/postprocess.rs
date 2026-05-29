//! 會後後處理:diarize_session_inner — 讀 meeting-info 人員數 → 每軌 diarize_wav
//! → assign_speakers → 標回兩軌 jsonl + 寫 speakers.json。

use crate::diarize::{
    assign_speakers, diarization_models_present, diarize_wav, write_speakers, SpeakerInfo,
    TrackDiarization,
};
use crate::transcribe::{read_segments_jsonl, write_segments_jsonl, Segment};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DiarizeSummary {
    pub num_speakers: usize,
    pub num_segments: usize,
}

// ── 純 relabel helpers(合併 / 逐段改 / 濾出) ────────────────────────────────

/// 把 speaker ∈ merge_ids 的段改成 keep_id(合併講者),其餘不動。
pub fn relabel_merge(mut segs: Vec<Segment>, keep_id: &str, merge_ids: &[String]) -> Vec<Segment> {
    for s in &mut segs {
        if let Some(spk) = &s.speaker {
            if merge_ids.iter().any(|m| m == spk) {
                s.speaker = Some(keep_id.to_string());
            }
        }
    }
    segs
}

/// 把 id == seg_id 的段 speaker 設成 speaker_id(逐段改講者)。
pub fn relabel_one(mut segs: Vec<Segment>, seg_id: &str, speaker_id: &str) -> Vec<Segment> {
    for s in &mut segs {
        if s.id == seg_id {
            s.speaker = Some(speaker_id.to_string());
        }
    }
    segs
}

/// 從 speakers 清單濾掉 drop_ids(合併後移除被併走的講者列)。
pub fn drop_speakers(list: Vec<SpeakerInfo>, drop_ids: &[String]) -> Vec<SpeakerInfo> {
    list.into_iter().filter(|s| !drop_ids.iter().any(|d| d == &s.id)).collect()
}

/// 把標好的 segments 原子寫回各軌 jsonl + 寫 speakers.json。純檔案操作,可單測(不碰引擎/模型)。
pub fn write_labeled_tracks(
    session_root: &std::path::Path,
    labeled: &[Segment],
    speakers: &[SpeakerInfo],
) -> Result<(), String> {
    for (track, jsonl_rel) in [
        ("system", "transcript/system.segments.jsonl"),
        ("mic-internal", "transcript/mic-internal.segments.jsonl"),
    ] {
        let track_segs: Vec<Segment> =
            labeled.iter().filter(|s| s.track.as_str() == track).cloned().collect();
        if track_segs.is_empty() {
            continue; // 該軌沒標到 → 原檔保留不動(刻意,不是 bug)
        }
        write_segments_jsonl(&session_root.join(jsonl_rel), &track_segs)?;
    }
    write_speakers(
        &session_root.join("transcript").join("speakers.json"),
        speakers,
    )?;
    Ok(())
}

/// 對一場 session 跑分人:讀人員數→num_clusters、每軌 diarize_wav + assign_speakers、
/// 把 speaker 標回兩軌 jsonl(覆寫)+ 寫 speakers.json。模型缺 → Err。
pub fn diarize_session_inner(
    session_root: &std::path::Path,
    num_clusters: Option<usize>,
) -> Result<DiarizeSummary, String> {
    if !diarization_models_present() {
        return Err("diarization models not installed".to_string());
    }

    // track_name() → "system" / "mic-internal" (matches SourceKind::track_name)
    let tracks_meta: &[(&str, &str, &str)] = &[
        (
            "system",
            "transcript/system.segments.jsonl",
            "audio/system.wav",
        ),
        (
            "mic-internal",
            "transcript/mic-internal.segments.jsonl",
            "audio/mic-internal.wav",
        ),
    ];

    let mut tds: Vec<TrackDiarization> = Vec::new();
    for (track, jsonl_rel, wav_rel) in tracks_meta {
        let jsonl = session_root.join(jsonl_rel);
        let wav = session_root.join(wav_rel);
        let segments = read_segments_jsonl(&jsonl);
        if segments.is_empty() || !wav.exists() {
            continue; // 空軌跳過
        }
        let spans = match diarize_wav(&wav, num_clusters) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("diarize_wav [{track}]: {e} — skipping speaker labels for this track");
                Vec::new()
            }
        };
        tds.push(TrackDiarization {
            track: track.to_string(),
            spans,
            segments,
        });
    }

    let (labeled, speakers) = assign_speakers(tds);

    write_labeled_tracks(session_root, &labeled, &speakers)?;

    // 若有可用聲紋庫(模型相符)→ 逐群比對 → 命中改 speakers.json 的 display 成真名。
    if let Some(reg) = crate::voiceprint::read_registry() {
        identify_speakers(session_root, &labeled, &reg);
    }

    // 記下這場分人用了哪兩個模型(best-effort,失敗不影響分人結果)。
    stamp_diar_models(session_root);

    Ok(DiarizeSummary {
        num_speakers: speakers.len(),
        num_segments: labeled.len(),
    })
}

/// 把分人用的 segmentation / embedding 模型名寫進 timeline.json 的 SessionMeta(best-effort)。
/// 為了可重現/debug:將來換模型,回頭看舊紀錄才知道哪場用哪個分的。讀不到/壞檔就略過。
fn stamp_diar_models(session_root: &std::path::Path) {
    fn model_name(p: std::path::PathBuf) -> Option<String> {
        p.file_stem().map(|s| s.to_string_lossy().into_owned())
    }
    let path = session_root.join("timeline.json");
    let Ok(s) = std::fs::read_to_string(&path) else { return };
    let Ok(mut meta) = serde_json::from_str::<crate::exporter::SessionMeta>(&s) else { return };
    meta.diarize_seg_model = model_name(crate::diarize::seg_model_path());
    meta.diarize_emb_model = model_name(crate::diarize::emb_model_path());
    if let Ok(body) = serde_json::to_string_pretty(&meta) {
        let _ = std::fs::write(&path, body);
    }
}

/// 讀一場 session 兩軌 jsonl 合併(依 start_ms 排序),給工作區顯示。
pub fn read_session_segments(session_root: &std::path::Path) -> Vec<crate::transcribe::Segment> {
    let mut all = crate::transcribe::read_segments_jsonl(
        &session_root.join("transcript/system.segments.jsonl"),
    );
    all.extend(crate::transcribe::read_segments_jsonl(
        &session_root.join("transcript/mic-internal.segments.jsonl"),
    ));
    all.sort_by_key(|s| s.start_ms);
    all
}

/// 用目前 jsonl(已含 speaker)+ speakers.json 重新匯出 meeting.public/internal.md。
/// meta 由 timeline.json 還原(就是序列化過的 SessionMeta)。
pub fn reexport_session(session_root: &std::path::Path) -> Result<(), String> {
    let segs = read_session_segments(session_root);
    let speakers = crate::diarize::read_speakers(
        &session_root.join("transcript/speakers.json"),
    );
    let meta_json = std::fs::read_to_string(session_root.join("timeline.json"))
        .map_err(|e| format!("read timeline.json: {e}"))?;
    let meta: crate::exporter::SessionMeta =
        serde_json::from_str(&meta_json).map_err(|e| format!("parse timeline.json: {e}"))?;
    let (pub_md, int_md, timeline) = crate::exporter::export(&segs, &meta, &speakers)?;
    std::fs::write(session_root.join("meeting.public.md"), pub_md)
        .map_err(|e| format!("write public: {e}"))?;
    std::fs::write(session_root.join("meeting.internal.md"), int_md)
        .map_err(|e| format!("write internal: {e}"))?;
    std::fs::write(session_root.join("timeline.json"), timeline)
        .map_err(|e| format!("write timeline: {e}"))?;
    Ok(())
}

/// 對每個講者(群)取其最長一段、切該軌 WAV 那段音訊算聲紋 → best_match → 命中改 speakers.json display。
fn identify_speakers(session_root: &std::path::Path, labeled: &[crate::transcribe::Segment], reg: &crate::voiceprint::Registry) {
    use std::collections::HashMap;
    // 每個 speaker id → 最長段(track + start_ms + end_ms)
    let mut longest: HashMap<String, &crate::transcribe::Segment> = HashMap::new();
    for s in labeled {
        if let Some(spk) = &s.speaker {
            let dur = s.end_ms.saturating_sub(s.start_ms);
            longest.entry(spk.clone())
                .and_modify(|cur| { if dur > cur.end_ms.saturating_sub(cur.start_ms) { *cur = s; } })
                .or_insert(s);
        }
    }
    let sp_path = session_root.join("transcript").join("speakers.json");
    let mut speakers = crate::diarize::read_speakers(&sp_path);
    let mut changed = false;
    for (spk_id, seg) in longest {
        let wav_rel = if seg.track == "system" { "audio/system.wav" } else { "audio/mic-internal.wav" };
        let wav = session_root.join(wav_rel);
        let emb = match read_wav_slice_f32(&wav, seg.start_ms, seg.end_ms) {
            Some((s, sr)) => match crate::voiceprint::embed_samples(&s, sr) { Ok(e) => e, Err(_) => continue },
            None => continue,
        };
        if let Some(p) = crate::voiceprint::best_match(&emb, &reg.people, crate::voiceprint::MATCH_THRESHOLD) {
            if let Some(si) = speakers.iter_mut().find(|x| x.id == spk_id) {
                si.display = p.name.clone();
                changed = true;
            }
        }
    }
    if changed { let _ = crate::diarize::write_speakers(&sp_path, &speakers); }
}

/// 讀 WAV 在 [start_ms,end_ms] 的樣本(f32)並回傳實際 sample rate。讀不到/空 → None。
fn read_wav_slice_f32(wav: &std::path::Path, start_ms: u64, end_ms: u64) -> Option<(Vec<f32>, i32)> {
    let mut r = hound::WavReader::open(wav).ok()?;
    let sr = r.spec().sample_rate as i32;
    let sr_u64 = sr as u64;
    let lo = (start_ms * sr_u64 / 1000) as usize;
    let hi = (end_ms * sr_u64 / 1000) as usize;
    let all: Vec<i16> = r.samples::<i16>().filter_map(|x| x.ok()).collect();
    if lo >= all.len() || hi <= lo { return None; }
    let hi = hi.min(all.len());
    Some((all[lo..hi].iter().map(|&v| v as f32 / 32768.0).collect(), sr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcribe::read_segments_jsonl;

    fn seg_with(id: &str, track: &str, speaker: Option<&str>) -> Segment {
        Segment {
            id: id.into(), session_id: "m".into(), track: track.into(),
            source_kind: if track == "system" { "meeting_system".into() } else { "mic_internal".into() },
            visibility: "public".into(), start_ms: 0, end_ms: 1000, text: "x".into(),
            is_final: true, confidence: None,
            speaker: speaker.map(|s| s.to_string()), speaker_mixed: false,
        }
    }

    #[test]
    fn relabel_merge_reassigns_merge_ids_to_keep() {
        let segs = vec![
            seg_with("a", "system", Some("S1")),
            seg_with("b", "system", Some("S3")),
            seg_with("c", "system", Some("S2")),
            seg_with("d", "system", None),
        ];
        let out = relabel_merge(segs, "S1", &["S3".to_string()]);
        assert_eq!(out[0].speaker.as_deref(), Some("S1")); // 本來就是 keep,不動
        assert_eq!(out[1].speaker.as_deref(), Some("S1")); // S3 → S1
        assert_eq!(out[2].speaker.as_deref(), Some("S2")); // 不在 merge 清單,不動
        assert_eq!(out[3].speaker, None);                   // None 不動
    }

    #[test]
    fn relabel_one_only_changes_matching_seg_id() {
        let segs = vec![seg_with("a", "system", Some("S1")), seg_with("b", "system", Some("S1"))];
        let out = relabel_one(segs, "b", "S2");
        assert_eq!(out[0].speaker.as_deref(), Some("S1"));
        assert_eq!(out[1].speaker.as_deref(), Some("S2"));
    }

    #[test]
    fn drop_speakers_filters_listed_ids() {
        let list = vec![
            SpeakerInfo { id: "S1".into(), display: "甲".into(), track: "system".into() },
            SpeakerInfo { id: "S3".into(), display: "乙".into(), track: "system".into() },
        ];
        let out = drop_speakers(list, &["S3".to_string()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "S1");
    }

    fn make_seg(track: &str, speaker: Option<&str>) -> Segment {
        Segment {
            id: "s1".into(),
            session_id: "test".into(),
            track: track.into(),
            source_kind: "meeting_system".into(),
            visibility: "public".into(),
            start_ms: 0,
            end_ms: 1000,
            text: "labeled text".into(),
            is_final: true,
            confidence: None,
            speaker: speaker.map(|s| s.to_string()),
            speaker_mixed: false,
        }
    }

    #[test]
    fn write_labeled_tracks_atomic_overwrite_and_speakers_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();

        // Pre-write an OLD segment into system jsonl to confirm overwrite semantics
        let transcript_dir = root.join("transcript");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        let old_seg = make_seg("system", None);
        let system_jsonl = transcript_dir.join("system.segments.jsonl");
        std::fs::write(&system_jsonl, serde_json::to_string(&old_seg).unwrap() + "\n").unwrap();

        // Call write_labeled_tracks with a NEW labeled segment (speaker=Some("S1"))
        let labeled = vec![make_seg("system", Some("S1"))];
        let speakers = vec![SpeakerInfo {
            id: "S1".into(),
            display: "講者1".into(),
            track: "system".into(),
        }];
        write_labeled_tracks(root, &labeled, &speakers).unwrap();

        // system.segments.jsonl should now have exactly 1 line and it's the NEW labeled segment
        let segs = read_segments_jsonl(&system_jsonl);
        assert_eq!(segs.len(), 1, "should have exactly 1 segment after overwrite");
        assert_eq!(segs[0].speaker.as_deref(), Some("S1"), "segment should have new speaker label");
        assert_eq!(segs[0].text, "labeled text");

        // speakers.json should exist
        let speakers_json = transcript_dir.join("speakers.json");
        assert!(speakers_json.exists(), "speakers.json should exist");
    }

    /// TDD pure test: reexport_session reads jsonl + speakers.json + timeline.json →
    /// writes meeting.public.md containing the speaker display name prefix.
    #[test]
    fn reexport_session_writes_public_md_with_speaker_prefix() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();

        // --- setup transcript dir ---
        let transcript_dir = root.join("transcript");
        std::fs::create_dir_all(&transcript_dir).unwrap();

        // system segment with speaker=Some("S1"), visibility=public
        let sys_seg = Segment {
            id: "s1".into(),
            session_id: "meeting-test".into(),
            track: "system".into(),
            source_kind: "meeting_system".into(),
            visibility: "public".into(),
            start_ms: 1000,
            end_ms: 3000,
            text: "這是系統音訊".into(),
            is_final: true,
            confidence: None,
            speaker: Some("S1".into()),
            speaker_mixed: false,
        };
        std::fs::write(
            transcript_dir.join("system.segments.jsonl"),
            serde_json::to_string(&sys_seg).unwrap() + "\n",
        )
        .unwrap();

        // mic-internal segment without speaker, visibility=internal
        let mic_seg = Segment {
            id: "m1".into(),
            session_id: "meeting-test".into(),
            track: "mic-internal".into(),
            source_kind: "mic_internal".into(),
            visibility: "internal".into(),
            start_ms: 2000,
            end_ms: 4000,
            text: "這是內部麥克風".into(),
            is_final: true,
            confidence: None,
            speaker: None,
            speaker_mixed: false,
        };
        std::fs::write(
            transcript_dir.join("mic-internal.segments.jsonl"),
            serde_json::to_string(&mic_seg).unwrap() + "\n",
        )
        .unwrap();

        // speakers.json: S1 → 亞澤
        let speakers = vec![SpeakerInfo {
            id: "S1".into(),
            display: "亞澤".into(),
            track: "system".into(),
        }];
        crate::diarize::write_speakers(&transcript_dir.join("speakers.json"), &speakers).unwrap();

        // timeline.json: minimal SessionMeta
        let meta = crate::exporter::SessionMeta {
            schema_version: 1,
            session_id: "meeting-test".into(),
            started_at: "2026-05-30T10:00:00+08:00".into(),
            stopped_at: "2026-05-30T10:30:00+08:00".into(),
            duration_secs: 1800,
            tracks: vec![],
            exports: crate::exporter::Exports {
                public: "meeting.public.md".into(),
                internal: "meeting.internal.md".into(),
            },
            transcribe_model: "small".into(),
            diarize_seg_model: None,
            diarize_emb_model: None,
        };
        std::fs::write(
            root.join("timeline.json"),
            serde_json::to_string_pretty(&meta).unwrap(),
        )
        .unwrap();

        // --- act ---
        reexport_session(root).unwrap();

        // --- assert ---
        let public_md = std::fs::read_to_string(root.join("meeting.public.md")).unwrap();
        assert!(
            public_md.contains("亞澤: "),
            "public.md should contain speaker display prefix '亞澤: ', got:\n{public_md}"
        );
        assert!(
            public_md.contains("這是系統音訊"),
            "public.md should contain segment text, got:\n{public_md}"
        );
        // mic-internal is internal visibility → must NOT appear in public.md
        assert!(
            !public_md.contains("這是內部麥克風"),
            "public.md must not contain internal segment, got:\n{public_md}"
        );

        // internal.md should contain mic segment (no speaker prefix)
        let internal_md = std::fs::read_to_string(root.join("meeting.internal.md")).unwrap();
        assert!(
            internal_md.contains("這是內部麥克風"),
            "internal.md should contain mic segment, got:\n{internal_md}"
        );
    }

    /// read_session_segments merges both tracks sorted by start_ms.
    #[test]
    fn read_session_segments_merges_and_sorts() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let transcript_dir = root.join("transcript");
        std::fs::create_dir_all(&transcript_dir).unwrap();

        let seg_early = Segment {
            id: "m1".into(),
            session_id: "x".into(),
            track: "mic-internal".into(),
            source_kind: "mic_internal".into(),
            visibility: "internal".into(),
            start_ms: 500,
            end_ms: 1500,
            text: "早".into(),
            is_final: true,
            confidence: None,
            speaker: None,
            speaker_mixed: false,
        };
        let seg_late = Segment {
            id: "s1".into(),
            session_id: "x".into(),
            track: "system".into(),
            source_kind: "meeting_system".into(),
            visibility: "public".into(),
            start_ms: 2000,
            end_ms: 3000,
            text: "晚".into(),
            is_final: true,
            confidence: None,
            speaker: None,
            speaker_mixed: false,
        };
        std::fs::write(
            transcript_dir.join("system.segments.jsonl"),
            serde_json::to_string(&seg_late).unwrap() + "\n",
        )
        .unwrap();
        std::fs::write(
            transcript_dir.join("mic-internal.segments.jsonl"),
            serde_json::to_string(&seg_early).unwrap() + "\n",
        )
        .unwrap();

        let segs = read_session_segments(root);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].start_ms, 500, "first segment should be the mic-internal (earliest)");
        assert_eq!(segs[1].start_ms, 2000, "second segment should be the system (later)");
    }
}
