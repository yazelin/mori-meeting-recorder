//! 會後後處理:diarize_session_inner — 讀 meeting-info 人員數 → 每軌 diarize_wav
//! → assign_speakers → 標回兩軌 jsonl + 寫 speakers.json。

use crate::diarize::{
    assign_speakers, diarization_models_present, diarize_wav, write_speakers, TrackDiarization,
};
use crate::transcribe::{append_segments_jsonl, read_segments_jsonl, Segment};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DiarizeSummary {
    pub num_speakers: usize,
    pub num_segments: usize,
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
        // 該軌引擎失敗 → 空 spans(不標 speaker,但不中止整場)
        let spans = diarize_wav(&wav, num_clusters).unwrap_or_default();
        tds.push(TrackDiarization {
            track: track.to_string(),
            spans,
            segments,
        });
    }

    let (labeled, speakers) = assign_speakers(tds);

    // 標回各軌 jsonl(覆寫:刪後重 append,因為 append_segments_jsonl 是 append 語意)
    for (track, jsonl_rel, _wav) in tracks_meta {
        let jsonl = session_root.join(jsonl_rel);
        // Segment.track 值 = kind.track_name() = "system" / "mic-internal"
        let track_segs: Vec<Segment> = labeled
            .iter()
            .filter(|s| s.track.as_str() == *track)
            .cloned()
            .collect();
        if track_segs.is_empty() {
            continue;
        }
        // 覆寫:刪原檔後重建(append 是 open+append 語意)
        let _ = std::fs::remove_file(&jsonl);
        append_segments_jsonl(&jsonl, &track_segs)?;
    }

    write_speakers(
        &session_root.join("transcript").join("speakers.json"),
        &speakers,
    )?;

    Ok(DiarizeSummary {
        num_speakers: speakers.len(),
        num_segments: labeled.len(),
    })
}
