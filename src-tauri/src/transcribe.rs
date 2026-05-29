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

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

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

/// 優先找 ~/.mori/bin/opencc;其次嘗試 PATH 上的 opencc。
/// 不保證 opencc 存在 — 呼叫端用 Option。
pub fn opencc_bin_path() -> Option<std::path::PathBuf> {
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".mori").join("bin").join("opencc");
        if p.exists() {
            return Some(p);
        }
    }
    // 嘗試 PATH:直接回 Some("opencc"),spawn 失敗就 None(呼叫端處理)
    Some(std::path::PathBuf::from("opencc"))
}

/// 用 opencc 把文字從簡體轉台灣繁體(s2twp.json)。
/// 任何 spawn / IO 錯誤均回 None,呼叫端保留原文。
pub fn to_traditional(text: &str) -> Option<String> {
    let bin = opencc_bin_path()?;
    let mut child = Command::new(&bin)
        .args(["-c", "s2twp.json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    {
        let stdin = child.stdin.as_mut()?;
        stdin.write_all(text.as_bytes()).ok()?;
    }
    let output = child.wait_with_output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// 跑 whisper-cli 對單一 WAV 檔,回 Segments。檔案不存在或 binary 缺則跳過(回空)。
/// `language`:傳給 whisper 的 `-l` 值(e.g. "zh"/"en"/"auto")。
/// `traditional`:若 true 則嘗試用 opencc 把輸出轉台灣繁體;opencc 缺就略過。
pub fn run_whisper(wav: &Path, session_id: &str, kind: SourceKind, language: &str, traditional: bool) -> Vec<Segment> {
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
            "-l",
            language,
            "-sns",
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
    let mut segs = match parse_whisper_json(&json, session_id, kind) {
        Ok(segs) => segs,
        Err(e) => {
            eprintln!("parse whisper json: {e}");
            return vec![];
        }
    };
    if traditional {
        // 嘗試 opencc 轉台灣繁體;opencc 不在就略過(graceful)。
        // 第一次轉失敗時印一次提示(呼叫端 per-segment 失敗各自 None 就略過,不重複 eprintln)。
        let mut warned = false;
        for s in &mut segs {
            match to_traditional(&s.text) {
                Some(t) => s.text = t,
                None if !warned => {
                    eprintln!("[mori] opencc not available or failed — Traditional conversion skipped (install opencc to enable)");
                    warned = true;
                }
                None => {}
            }
        }
    }
    segs
}

/// 把 whisper 跑「短段」出來的 segment(段內相對時間)平移成「整場絕對時間」。
/// offset_ms = 該 speech 段在原始 stream 的起點。
pub fn shift_segments_by_offset(mut segs: Vec<Segment>, offset_ms: u64) -> Vec<Segment> {
    for s in &mut segs {
        s.start_ms += offset_ms;
        s.end_ms += offset_ms;
    }
    segs
}

/// Append segments 到 jsonl(一行一 segment),建立父目錄。跟 Phase 1 batch 格式一致。
pub fn append_segments_jsonl(path: &std::path::Path, segs: &[Segment]) -> Result<(), String> {
    use std::io::Write;
    if segs.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir transcript dir: {e}"))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open jsonl append: {e}"))?;
    for s in segs {
        let line = serde_json::to_string(s).map_err(|e| format!("serialize segment: {e}"))?;
        writeln!(f, "{line}").map_err(|e| format!("write jsonl: {e}"))?;
    }
    Ok(())
}

/// 寫一個 16kHz mono 16-bit WAV(複用 TrackWriter)— transcribe worker 寫 temp 段檔用。
fn write_wav_16k_mono(path: &Path, samples: &[i16]) -> Result<(), String> {
    let mut w = crate::audio::writer::TrackWriter::create(path)?;
    w.push_samples(samples)?;
    w.finalize()
}

/// 背景 transcribe worker — 從 channel 收 SpeechSegment,跑 whisper,append jsonl + emit。
pub struct TranscribeWorker {
    pub handle: std::thread::JoinHandle<()>,
    pub pending: Arc<AtomicUsize>,
}

/// 啟一個 worker。speech_rx 來自 open_capture 回傳的 tuple;每段轉完呼 on_segment(&segments) 給呼叫端 emit。
/// `language`: 傳給 whisper 的 `-l` 值(e.g. "zh"/"en"/"auto")。
/// `traditional`: 是否嘗試用 opencc 轉台灣繁體。
pub fn spawn_transcribe_worker(
    speech_rx: std::sync::mpsc::Receiver<crate::audio::vad::SpeechSegment>,
    session_id: String,
    kind: crate::audio::SourceKind,
    jsonl_path: std::path::PathBuf,
    language: String,
    traditional: bool,
    on_segment: impl Fn(&[Segment]) + Send + 'static,
) -> TranscribeWorker {
    let pending = Arc::new(AtomicUsize::new(0));
    let pending_thread = pending.clone();
    let handle = std::thread::spawn(move || {
        while let Ok(seg) = speech_rx.recv() {
            pending_thread.fetch_add(1, Ordering::Relaxed);
            // 寫 temp WAV
            let tmp = std::env::temp_dir().join(format!(
                "mori-live-{}-{}.wav",
                kind.as_str(),
                seg.start_offset_ms
            ));
            if let Err(e) = write_wav_16k_mono(&tmp, &seg.samples) {
                eprintln!("live transcribe: write temp wav: {e}");
                pending_thread.fetch_sub(1, Ordering::Relaxed);
                continue;
            }
            let raw = run_whisper(&tmp, &session_id, kind, &language, traditional);
            let _ = std::fs::remove_file(&tmp);
            // whisper-cli 另外寫了 <wav>.json sidecar,一併清掉
            let _ = std::fs::remove_file(tmp.with_extension("wav.json"));
            let shifted = shift_segments_by_offset(raw, seg.start_offset_ms);
            if !shifted.is_empty() {
                if let Err(e) = append_segments_jsonl(&jsonl_path, &shifted) {
                    eprintln!("live transcribe: append jsonl: {e}");
                }
                on_segment(&shifted);
            }
            pending_thread.fetch_sub(1, Ordering::Relaxed);
        }
    });
    TranscribeWorker { handle, pending }
}

/// 讀回 jsonl 成 Vec<Segment>(stop 時彙整用)。缺檔回空。壞行跳過。
pub fn read_segments_jsonl(path: &std::path::Path) -> Vec<Segment> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/whisper-small.json");

    fn sample_seg() -> Segment {
        Segment {
            id: "s1".into(),
            session_id: "x".into(),
            track: "system".into(),
            source_kind: "meeting_system".into(),
            visibility: "public".into(),
            start_ms: 100,
            end_ms: 500,
            text: "hi".into(),
            is_final: true,
            confidence: None,
        }
    }

    #[test]
    fn shift_offset_adds_to_both_ends() {
        let shifted = shift_segments_by_offset(vec![sample_seg()], 10_000);
        assert_eq!(shifted[0].start_ms, 10_100);
        assert_eq!(shifted[0].end_ms, 10_500);
    }

    #[test]
    fn append_jsonl_round_trip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("transcript").join("system.segments.jsonl");
        let seg = sample_seg();
        append_segments_jsonl(&path, std::slice::from_ref(&seg)).unwrap();
        append_segments_jsonl(&path, std::slice::from_ref(&seg)).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), 2);
        for line in content.lines() {
            let _: Segment = serde_json::from_str(line).unwrap();
        }
    }

    #[test]
    fn read_jsonl_skips_blank_and_bad_lines() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("t.jsonl");
        let good = serde_json::to_string(&sample_seg()).unwrap();
        std::fs::write(&path, format!("{good}\n\nnot json\n{good}\n")).unwrap();
        let segs = read_segments_jsonl(&path);
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn read_jsonl_missing_file_returns_empty() {
        let segs = read_segments_jsonl(std::path::Path::new("/nonexistent/x.jsonl"));
        assert!(segs.is_empty());
    }

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

    #[test]
    fn to_traditional_does_not_panic() {
        // to_traditional either returns Some(String) (opencc present) or None (absent).
        // It must never panic regardless of opencc availability.
        let result = to_traditional("测试文字");
        // Either None (no opencc) or Some with a non-empty string — never panic.
        if let Some(s) = result {
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn write_wav_16k_mono_round_trip() {
        use hound::WavReader;
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("live-seg.wav");
        let signal: Vec<i16> = (0..3200_i16).map(|i| i * 10).collect();
        write_wav_16k_mono(&path, &signal).unwrap();
        let mut r = WavReader::open(&path).unwrap();
        assert_eq!(r.spec().channels, 1);
        assert_eq!(r.spec().sample_rate, 16_000);
        assert_eq!(r.spec().bits_per_sample, 16);
        let read_back: Vec<i16> = r.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(read_back, signal);
    }
}
