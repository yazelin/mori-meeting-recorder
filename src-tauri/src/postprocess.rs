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

    Ok(DiarizeSummary {
        num_speakers: speakers.len(),
        num_segments: labeled.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcribe::read_segments_jsonl;

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
}
