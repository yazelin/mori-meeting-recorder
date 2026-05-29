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
use zhconv::{zhconv, Variant};
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

/// 簡→台灣正體(zh-Hant-TW)。用純 Rust 的 zhconv(MediaWiki 轉換表 + 台灣詞彙),
/// bundle 在 binary 內 —— 不再依賴外部 opencc 安裝(之前 opencc 沒裝 → 一直是簡體)。
pub fn to_traditional(text: &str) -> String {
    zhconv(text, Variant::ZhTW)
}

/// 只丟「whisper 自己明確標成非語音」的段:整段被括號 / ♪ 包住(SDH 風格的
/// `[keyboard clacking]` / `[Music]` / `[typing]` / `（音樂）`)或空白。
///
/// **刻意不用任何「謝謝大家」式片語黑名單** —— 那些是正常講話內容,用片語過濾會誤殺真的講話。
/// 這裡只靠「whisper 把非語音用括號包起來」這個結構特徵(不是猜講話內容),所以絕不會吃掉真句子。
fn is_noise_segment(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    let first = t.chars().next().unwrap();
    let last = t.chars().last().unwrap();
    matches!(
        (first, last),
        ('[', ']') | ('(', ')') | ('（', '）') | ('【', '】') | ('*', '*') | ('♪', '♪')
    )
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
    // whisper-cli `--output-json-full` 把 JSON 寫到 `<wav>.json`,不是 stdout。
    // ⚠ whisper.cpp 對 CJK 偶爾在 token 邊界切斷多位元組字 → json 含 invalid UTF-8。
    // 用 read(bytes) + from_utf8_lossy(壞 byte 換 �)別整段丟,還能 parse 出大部分文字。
    let json_path = wav.with_extension("wav.json");
    let json = match std::fs::read(&json_path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => {
                let b = e.as_bytes();
                let head = &b[..b.len().min(160)];
                eprintln!(
                    "[whisper] {} not UTF-8 (len={}); lossy-parsing. head: {:?}",
                    json_path.display(),
                    b.len(),
                    String::from_utf8_lossy(head)
                );
                String::from_utf8_lossy(b).into_owned()
            }
        },
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
    // 濾掉非語音雜訊 + whisper 靜音幻覺(不再用 -sns —— 那會逼模型把非語音段瞎掰成真詞)。
    segs.retain(|s| !is_noise_segment(&s.text));
    if traditional {
        for s in &mut segs {
            s.text = to_traditional(&s.text);
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
    pending: Arc<AtomicUsize>,
    done: Arc<AtomicUsize>,
    on_segment: impl Fn(&[Segment]) + Send + 'static,
) -> TranscribeWorker {
    let handle = std::thread::spawn(move || {
        while let Ok(seg) = speech_rx.recv() {
            // pending 在 capture 送進 channel 時就 +1(見 audio/*),這裡只在轉完時 -1。
            // 寫 temp WAV
            let tmp = std::env::temp_dir().join(format!(
                "mori-live-{}-{}.wav",
                kind.as_str(),
                seg.start_offset_ms
            ));
            if let Err(e) = write_wav_16k_mono(&tmp, &seg.samples) {
                eprintln!("live transcribe: write temp wav: {e}");
                pending.fetch_sub(1, Ordering::Relaxed);
                done.fetch_add(1, Ordering::Relaxed);
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
            pending.fetch_sub(1, Ordering::Relaxed);
            done.fetch_add(1, Ordering::Relaxed);
        }
    });
    TranscribeWorker { handle }
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
    fn noise_filter_drops_only_bracketed_nonspeech_never_real_speech() {
        // whisper 用括號 / ♪ 包的非語音標註 + 空白 → 丟
        assert!(is_noise_segment("[keyboard clacking]"));
        assert!(is_noise_segment("[Music]"));
        assert!(is_noise_segment("[typing]"));
        assert!(is_noise_segment("（音樂）"));
        assert!(is_noise_segment("♪ ♪"));
        assert!(is_noise_segment("   "));
        // 任何真講話內容都不丟 —— 「謝謝大家」這種也可能是真的在講,不可用片語過濾誤殺
        assert!(!is_noise_segment("謝謝大家"));
        assert!(!is_noise_segment("謝謝大家收看。下次再見,拜拜。"));
        assert!(!is_noise_segment("我們下週三前要交版本"));
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
    fn to_traditional_converts_simplified_to_taiwan() {
        // zhconv 純 Rust、bundle 在內,簡體一定轉成台灣正體。
        assert_eq!(to_traditional("测试文字"), "測試文字");
        assert_eq!(to_traditional("软件"), "軟體"); // 台灣詞彙(非僅字形)
        // 已是繁體 → 不變
        assert_eq!(to_traditional("會議記錄"), "會議記錄");
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
