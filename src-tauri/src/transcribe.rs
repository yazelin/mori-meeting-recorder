//! 轉錄:shell-out whisper.cpp CLI + parse `--output-json-full` 輸出。
//! 本檔的 `parse_whisper_json` 是純函式;spawn 邏輯(run_whisper)在 Task 9 補。

use crate::audio::{SourceKind, Visibility};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub id: String,
    pub session_id: String,
    pub track: String,
    pub source_kind: String,
    pub visibility: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub is_final: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

/// 把 whisper.cpp `--output-json-full` 解析成 Segments。
/// `session_id` / `kind` 由呼叫端帶進來(parser 不知道這些)。
pub fn parse_whisper_json(
    json: &str,
    session_id: &str,
    kind: SourceKind,
) -> Result<Vec<Segment>, String> {
    #[derive(Deserialize)]
    struct Root {
        transcription: Vec<RawSeg>,
    }
    #[derive(Deserialize)]
    struct RawSeg {
        offsets: Offsets,
        text: String,
        #[serde(default)]
        confidence: Option<f64>,
    }
    #[derive(Deserialize)]
    struct Offsets {
        from: u64,
        to: u64,
    }

    let root: Root = serde_json::from_str(json).map_err(|e| format!("parse: {e}"))?;
    let visibility = kind.default_visibility();
    let segs = root
        .transcription
        .into_iter()
        .enumerate()
        .map(|(i, r)| Segment {
            id: format!("seg_{:03}", i + 1),
            session_id: session_id.to_string(),
            track: kind.track_name().to_string(),
            source_kind: kind.as_str().to_string(),
            visibility: match visibility {
                Visibility::Public => "public".to_string(),
                Visibility::Internal => "internal".to_string(),
            },
            start_ms: r.offsets.from,
            end_ms: r.offsets.to,
            text: r.text.trim().to_string(),
            is_final: true,
            confidence: r.confidence,
        })
        .collect();
    Ok(segs)
}

use std::path::Path;
use std::process::Command;

const WHISPER_BIN: &str = "whisper-cli";
const WHISPER_MODEL_FILENAME: &str = "ggml-small.bin";

pub fn whisper_bin_path() -> std::path::PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("bin").join(WHISPER_BIN))
        .unwrap_or_else(|| std::path::PathBuf::from(WHISPER_BIN))
}

pub fn whisper_model_path() -> std::path::PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("models").join(WHISPER_MODEL_FILENAME))
        .unwrap_or_else(|| std::path::PathBuf::from(WHISPER_MODEL_FILENAME))
}

/// 跑 whisper-cli 對單一 WAV 檔,回 Segments。檔案不存在或 binary 缺則跳過(回空)。
pub fn run_whisper(wav: &Path, session_id: &str, kind: SourceKind) -> Vec<Segment> {
    if !wav.exists() {
        return vec![];
    }
    let bin = whisper_bin_path();
    let model = whisper_model_path();
    if !bin.exists() || !model.exists() {
        eprintln!("whisper deps missing — skipping transcribe");
        return vec![];
    }
    let output = match Command::new(&bin)
        .args([
            "-m",
            &model.to_string_lossy(),
            "-f",
            &wav.to_string_lossy(),
            "--output-json-full",
            "--no-prints",
        ])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("spawn whisper-cli: {e}");
            return vec![];
        }
    };
    if !output.status.success() {
        eprintln!("whisper-cli exited {}: {}", output.status, String::from_utf8_lossy(&output.stderr));
        return vec![];
    }
    // whisper-cli `--output-json-full` 把 JSON 寫到 `<wav>.json`,不是 stdout
    let json_path = wav.with_extension("wav.json");
    let json = match std::fs::read_to_string(&json_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read whisper json {}: {e}", json_path.display());
            return vec![];
        }
    };
    match parse_whisper_json(&json, session_id, kind) {
        Ok(segs) => segs,
        Err(e) => {
            eprintln!("parse whisper json: {e}");
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/whisper-small.json");

    #[test]
    fn parses_fixture_into_two_segments() {
        let segs = parse_whisper_json(FIXTURE, "meeting-test", SourceKind::MeetingSystem).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].id, "seg_001");
        assert_eq!(segs[0].session_id, "meeting-test");
        assert_eq!(segs[0].track, "system");
        assert_eq!(segs[0].source_kind, "meeting_system");
        assert_eq!(segs[0].visibility, "public");
        assert_eq!(segs[0].start_ms, 1500);
        assert_eq!(segs[0].end_ms, 4200);
        assert_eq!(segs[0].text, "我們希望下週三前看到版本。");
        assert!(segs[0].is_final);
        assert_eq!(segs[0].confidence, Some(-0.142));
    }

    #[test]
    fn mic_internal_gets_internal_visibility() {
        let segs = parse_whisper_json(FIXTURE, "x", SourceKind::MicInternal).unwrap();
        assert_eq!(segs[0].visibility, "internal");
        assert_eq!(segs[0].source_kind, "mic_internal");
        assert_eq!(segs[0].track, "mic-internal");
    }

    #[test]
    fn corrupt_json_returns_err() {
        assert!(parse_whisper_json("{ not json", "x", SourceKind::MeetingSystem).is_err());
    }

    #[test]
    fn empty_transcription_returns_empty_vec() {
        let json = r#"{"transcription": []}"#;
        let segs = parse_whisper_json(json, "x", SourceKind::MeetingSystem).unwrap();
        assert!(segs.is_empty());
    }
}
