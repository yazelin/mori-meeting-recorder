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
    #[serde(default)]
    pub speaker: Option<String>,
    #[serde(default)]
    pub speaker_mixed: bool,
    /// 決議依據 / 內部補充:true → 這段在 internal.md 末尾加入「決議依據 / 內部補充」區塊。
    /// public.md 完全不受影響(hard rule #3)。
    #[serde(default)]
    pub supplement: bool,
}

/// 把 whisper 輸出壓成單行:whisper 會在內部斷句處塞 `\n`(一段含多句 → 多行),
/// 直接顯示會讓即時字幕/匯出一段變好幾行。收斂所有空白(含 `\n`)成單一空格。
fn normalize_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
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
            text: normalize_text(&r.text),
            is_final: true,
            confidence: r.confidence,
            speaker: None,
            speaker_mixed: false,
            supplement: false,
        })
        .collect();
    Ok(segs)
}

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const WHISPER_BIN: &str = "whisper-cli";

pub fn whisper_bin_path() -> std::path::PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("bin").join(WHISPER_BIN))
        .unwrap_or_else(|| std::path::PathBuf::from(WHISPER_BIN))
}

/// 模型檔依設定的 model 名解析:`~/.mori/models/ggml-<model>.bin`(small / large-v3-turbo)。
/// deps_check 與 run_whisper 都走這個,所以換模型後兩邊一致。
pub fn whisper_model_path() -> std::path::PathBuf {
    let filename = format!("ggml-{}.bin", crate::config::read_config().model);
    dirs::home_dir()
        .map(|h| h.join(".mori").join("models").join(&filename))
        .unwrap_or_else(|| std::path::PathBuf::from(filename))
}

/// 簡→台灣正體。用純 Rust 的 ferrous-opencc(bundle OpenCC 官方字典,s2twp 含台灣詞彙
/// 片語,如 软件→軟體),零外部安裝。converter 載字典較重 → OnceLock 只建一次後重用;
/// 萬一建失敗(理論上不會,字典 bundle 在內)就回原文。
pub fn to_traditional(text: &str) -> String {
    use ferrous_opencc::config::BuiltinConfig;
    use ferrous_opencc::OpenCC;
    static CC: std::sync::OnceLock<Option<OpenCC>> = std::sync::OnceLock::new();
    match CC
        .get_or_init(|| OpenCC::from_config(BuiltinConfig::S2twp).ok())
        .as_ref()
    {
        Some(cc) => cc.convert(text),
        None => text.to_string(),
    }
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
/// 轉錄單一 WAV clip,回後處理完的 Segments。
/// `server`:本場「一次解析、快取」的共享 whisper-server(None = 走 cli;Some = 走 server)。
/// 引擎選擇(auto/cli/server + 驗活)在開場時做一次(見 recorder.rs),不在這裡每段重做。
///
/// **sticky fallback**:server 端任何失敗(連線 / 非 200 / malformed json)→ 設 `*server = None`,
/// 本場之後一律 cli。否則 server 若中途掛掉,會變成「每段都先等 timeout 再 fallback」,比純 cli 還慢
/// —— 比原本「每段重驗」更糟,所以只認賠一次就改用 cli。**standalone-first**:沒 server 永遠能跑(契約 §3.3)。
///
/// noise filter + 繁體一律在這層做 → server / cli 兩路輸出後處理完全一致。
pub fn run_whisper(
    wav: &Path,
    session_id: &str,
    kind: SourceKind,
    language: &str,
    traditional: bool,
    server: &mut Option<crate::whisper_discovery::WhisperServerDescriptor>,
) -> Vec<Segment> {
    let mut segs = match server.as_ref() {
        Some(desc) => match run_whisper_server(desc, wav, session_id, kind, language) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[whisper] server failed ({e}); using cli for the rest of this session");
                *server = None; // sticky:本場放棄 server,後續 clip 不再每段等 timeout
                run_whisper_cli(wav, session_id, kind, language)
            }
        },
        None => run_whisper_cli(wav, session_id, kind, language),
    };
    // 濾掉非語音雜訊(不再用 -sns —— 那會逼模型把非語音段瞎掰成真詞)。
    segs.retain(|s| !is_noise_segment(&s.text));
    if traditional {
        for s in &mut segs {
            s.text = to_traditional(&s.text);
        }
    }
    segs
}

/// 16kHz mono WAV 的長度(ms)。共享 server 回 plain `{text}`(無 per-word offset),
/// 所以整個 clip 視為「一段」,end_ms 用這個算;讀不到回 0。
fn wav_duration_ms(wav: &Path) -> u64 {
    match hound::WavReader::open(wav) {
        Ok(r) => {
            let spec = r.spec();
            let frames = r.len() as u64 / (spec.channels.max(1) as u64);
            frames * 1000 / (spec.sample_rate.max(1) as u64)
        }
        Err(_) => 0,
    }
}

/// 把共享 whisper-server 的 baseline plain-json 回應(`{"text": "..."}`,契約 §2)轉成
/// 「整個 clip 一段」的 raw Segment(start=0、end=clip 長度;之後由 caller 平移成絕對時間)。
///
/// 回傳語義刻意分三種(影響上層要不要 fallback cli):
/// - `Err`  = json 解不開(server 回了非預期內容)→ run_whisper_server 往上拋 → fallback cli。
/// - `Ok(None)` = text 去空白後為空(**真靜音**,whisper 對無語音 clip 的正常輸出)→ 不產段、
///   **不 fallback**(否則每段靜音都會白跑一次 cli)。
/// - `Ok(Some)` = 有內容。noise filter / 繁體一律在 run_whisper 那層做,這裡不碰。
fn parse_server_json(
    json: &str,
    clip_duration_ms: u64,
    session_id: &str,
    kind: SourceKind,
) -> Result<Option<Segment>, String> {
    #[derive(Deserialize)]
    struct Resp {
        text: String,
    }
    let resp: Resp = serde_json::from_str(json).map_err(|e| format!("parse server json: {e}"))?;
    let text = normalize_text(&resp.text);
    if text.is_empty() {
        return Ok(None);
    }
    let visibility = kind.default_visibility();
    Ok(Some(Segment {
        id: "seg_001".to_string(),
        session_id: session_id.to_string(),
        track: kind.track_name().to_string(),
        source_kind: kind.as_str().to_string(),
        visibility: match visibility {
            Visibility::Public => "public".to_string(),
            Visibility::Internal => "internal".to_string(),
        },
        start_ms: 0,
        end_ms: clip_duration_ms,
        text,
        is_final: true,
        confidence: None,
        speaker: None,
        speaker_mixed: false,
        supplement: false,
    }))
}

/// POST 一個 WAV clip 到共享 whisper-server 的 `/inference`(multipart/form-data,
/// 契約 §2 baseline:`response_format=json` → plain `{text}`)→ raw segments。
/// 任何失敗(連線 / 非 200 / parse)回 Err,讓 transcribe_raw fallback cli(standalone-first)。
/// multipart 手刻(ureq 無 multipart):固定長 boundary,短 audio clip 撞 boundary 機率可忽略。
fn run_whisper_server(
    desc: &crate::whisper_discovery::WhisperServerDescriptor,
    wav: &Path,
    session_id: &str,
    kind: SourceKind,
    language: &str,
) -> Result<Vec<Segment>, String> {
    let wav_bytes = std::fs::read(wav).map_err(|e| format!("read wav: {e}"))?;
    let duration_ms = wav_duration_ms(wav);
    let boundary = "----morimeetingrecorderFormBoundary7MA4YWxkTrZu0gW";
    let mut body: Vec<u8> = Vec::with_capacity(wav_bytes.len() + 512);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"clip.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&wav_bytes);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\n{language}\r\n").as_bytes(),
    );
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"response_format\"\r\n\r\njson\r\n").as_bytes(),
    );
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    // 60s(原 120s):clip 上限 20s 音訊,即使慢機 CPU 跑 large-v3-turbo 也夠;太長會讓中途掛掉的
    // server 把第一段卡到天荒地老才 fallback。配合 sticky fallback,最多認賠這一段就改 cli。
    let resp = ureq::post(&desc.inference_url())
        .set("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
        .timeout(std::time::Duration::from_secs(60))
        .send_bytes(&body)
        .map_err(|e| format!("POST {}: {e}", desc.inference_url()))?;
    if resp.status() != 200 {
        // body 常帶診斷(模型 OOM / 載入失敗等),截 200 字一起回,讓 fallback log 看得到原因。
        let status = resp.status();
        let snippet: String = resp.into_string().unwrap_or_default().chars().take(200).collect();
        return Err(format!("status {status}: {snippet}"));
    }
    let json = resp.into_string().map_err(|e| format!("read resp: {e}"))?;
    // malformed json → `?` 往上變 Err → fallback cli;空 text → Ok(None) → 空 Vec(真靜音,不 fallback)。
    Ok(parse_server_json(&json, duration_ms, session_id, kind)?
        .map(|s| vec![s])
        .unwrap_or_default())
}

/// whisper-cli per-call:spawn whisper-cli + parse `--output-json-full` sidecar → 原始 segments。
fn run_whisper_cli(wav: &Path, session_id: &str, kind: SourceKind, language: &str) -> Vec<Segment> {
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
    match parse_whisper_json(&json, session_id, kind) {
        Ok(segs) => segs,
        Err(e) => {
            eprintln!("parse whisper json: {e}");
            vec![]
        }
    }
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

/// 原子覆寫整個 jsonl(tmp+rename,一行一 segment)。diarization 標回稿用 —— 取代「刪檔再 append」
/// 的資料遺失風險(append 失敗時原稿不會先被刪掉)。
pub fn write_segments_jsonl(path: &std::path::Path, segs: &[Segment]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir transcript dir: {e}"))?;
    }
    let mut body = String::new();
    for s in segs {
        body.push_str(&serde_json::to_string(s).map_err(|e| format!("serialize segment: {e}"))?);
        body.push('\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("write tmp jsonl: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename jsonl: {e}"))
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
    try_server: bool,
    on_segment: impl Fn(&[Segment]) + Send + 'static,
) -> TranscribeWorker {
    let handle = std::thread::spawn(move || {
        // server 解析「lazy + warmup 容忍」:還沒接上前每段重試 reachable_server()(recorder 開場 autostart
        // 的 supervisor 可能還在載模型),一接上就固定用它。一旦「用過的 server」又失敗(run_whisper sticky
        // 把它設 None)→ server_disabled,本場後續永久落 cli,不再重試(避免每段重撞掛掉的 server)。
        let mut server: Option<crate::whisper_discovery::WhisperServerDescriptor> = None;
        let mut server_disabled = !try_server;
        while let Ok(seg) = speech_rx.recv() {
            if !server_disabled && server.is_none() {
                server = crate::whisper_discovery::reachable_server();
            }
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
            let had_server = server.is_some();
            let raw = run_whisper(&tmp, &session_id, kind, &language, traditional, &mut server);
            if had_server && server.is_none() {
                server_disabled = true; // 用過的 server 失敗(sticky)→ 本場永久落 cli,不再重試
            }
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

    #[test]
    fn normalize_text_collapses_internal_newlines_to_single_line() {
        // whisper 內部斷句的 \n 不該變成多行字幕
        assert_eq!(normalize_text("揹著海下山\n遠觀天山\n啊"), "揹著海下山 遠觀天山 啊");
        assert_eq!(normalize_text("  hello \n world  "), "hello world");
        assert_eq!(normalize_text("一句"), "一句");
        assert!(!normalize_text("a\nb\nc").contains('\n'));
    }

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
            speaker: None,
            speaker_mixed: false,
            supplement: false,
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
    fn write_jsonl_atomic_overwrite() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("transcript").join("system.segments.jsonl");
        let seg = sample_seg();
        // 第一次寫:2 個 segment
        write_segments_jsonl(&path, &[seg.clone(), seg.clone()]).unwrap();
        let segs1 = read_segments_jsonl(&path);
        assert_eq!(segs1.len(), 2, "first write should have 2 segments");
        // 第二次寫(覆寫):仍然只有 2 個,不是 4(驗 overwrite 語意)
        write_segments_jsonl(&path, &[seg.clone(), seg.clone()]).unwrap();
        let segs2 = read_segments_jsonl(&path);
        assert_eq!(segs2.len(), 2, "overwrite should still have 2 segments, not 4");
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
    fn parse_server_json_makes_one_segment_spanning_the_clip() {
        // baseline plain {text}:整個 clip 一段,start=0、end=clip 長度。whisper 常前綴空白 → trim。
        let seg = parse_server_json(r#"{"text":" 我們下週三前要交版本。"}"#, 4200, "m1", SourceKind::MeetingSystem)
            .unwrap()
            .unwrap();
        assert_eq!(seg.id, "seg_001");
        assert_eq!(seg.session_id, "m1");
        assert_eq!(seg.track, "system");
        assert_eq!(seg.source_kind, "meeting_system");
        assert_eq!(seg.visibility, "public");
        assert_eq!(seg.start_ms, 0);
        assert_eq!(seg.end_ms, 4200);
        assert_eq!(seg.text, "我們下週三前要交版本。"); // 前綴空白被 trim
        assert!(seg.is_final);
    }

    #[test]
    fn parse_server_json_mic_internal_visibility() {
        let seg = parse_server_json(r#"{"text":"私心話"}"#, 1000, "x", SourceKind::MicInternal)
            .unwrap()
            .unwrap();
        assert_eq!(seg.visibility, "internal");
        assert_eq!(seg.track, "mic-internal");
    }

    #[test]
    fn parse_server_json_empty_text_is_ok_none_not_error() {
        // 空 / 純空白 = 真靜音 → Ok(None):不產段、**不**讓上層 fallback cli(否則每段靜音白跑 cli)。
        assert!(parse_server_json(r#"{"text":""}"#, 1000, "x", SourceKind::MeetingSystem).unwrap().is_none());
        assert!(parse_server_json(r#"{"text":"   "}"#, 1000, "x", SourceKind::MeetingSystem).unwrap().is_none());
    }

    #[test]
    fn parse_server_json_keeps_bracketed_text_for_upstream_noise_filter() {
        // 括號非語音標註在這層「不」過濾(交給 run_whisper 的 is_noise_segment 統一處理,兩引擎一致)。
        let seg = parse_server_json(r#"{"text":"[Music]"}"#, 800, "x", SourceKind::MeetingSystem)
            .unwrap()
            .unwrap();
        assert_eq!(seg.text, "[Music]");
    }

    #[test]
    fn parse_server_json_corrupt_is_err_so_caller_falls_back() {
        // malformed json = server 回非預期 → Err → run_whisper_server 往上拋 → fallback cli。
        assert!(parse_server_json("{ not json", 1000, "x", SourceKind::MeetingSystem).is_err());
    }

    #[test]
    fn wav_duration_ms_matches_sample_count() {
        // 16000 frames @ 16kHz mono = 1000ms
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("dur.wav");
        let signal: Vec<i16> = vec![0; 16_000];
        write_wav_16k_mono(&path, &signal).unwrap();
        assert_eq!(wav_duration_ms(&path), 1000);
        // 讀不到 → 0(讓 end_ms 退化成 0,不會 panic)
        assert_eq!(wav_duration_ms(std::path::Path::new("/nonexistent.wav")), 0);
    }

    /// 真打活的 whisper-server,驗手刻 multipart 確實被 cpp-httplib 接受(200 + {text})。
    /// 預設 `#[ignore]`(不進 verify.sh / CI)。手動跑:
    ///   WHISPER_SERVER_PORT=38099 cargo test --release server_post_round_trip -- --ignored --nocapture
    #[test]
    #[ignore]
    fn server_post_round_trip() {
        let port: u16 = std::env::var("WHISPER_SERVER_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .expect("set WHISPER_SERVER_PORT to a running whisper-server");
        let desc = crate::whisper_discovery::WhisperServerDescriptor {
            contract_version: 1,
            host: "127.0.0.1".into(),
            port,
            model: "small".into(),
            pid: 0,
            started_at: "test".into(),
            inference_path: "/inference".into(),
        };
        // 1 秒靜音 clip:重點是驗 transport(200 + 合法 {text} json),不是轉錄品質。
        let tmp = tempfile::TempDir::new().unwrap();
        let wav = tmp.path().join("clip.wav");
        write_wav_16k_mono(&wav, &vec![0i16; 16_000]).unwrap();
        let segs = run_whisper_server(&desc, &wav, "itest", SourceKind::MeetingSystem, "zh")
            .expect("POST /inference should round-trip 200 + json");
        // 靜音可能回空段或 [BLANK_AUDIO];能 Ok 回傳就證明 multipart 格式被接受、json 解得開。
        eprintln!("server_post_round_trip segs = {segs:?}");
        for s in &segs {
            assert_eq!(s.end_ms, 1000); // wav_duration_ms 對 1s clip
            assert_eq!(s.source_kind, "meeting_system");
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

    #[test]
    fn segment_speaker_fields_default_when_absent() {
        // 舊 jsonl(沒有 speaker / speaker_mixed)要能反序列化,欄位回預設
        let line = r#"{"id":"s1","session_id":"x","track":"system","source_kind":"meeting_system","visibility":"public","start_ms":0,"end_ms":1000,"text":"hi","is_final":true}"#;
        let s: Segment = serde_json::from_str(line).unwrap();
        assert_eq!(s.speaker, None);
        assert!(!s.speaker_mixed);
    }

    #[test]
    fn segment_supplement_defaults_false_when_absent_from_old_jsonl() {
        // 舊 jsonl(沒有 supplement 欄位)反序列化 → supplement 應為 false(back-compat)。
        let line = r#"{"id":"s1","session_id":"x","track":"system","source_kind":"meeting_system","visibility":"public","start_ms":0,"end_ms":1000,"text":"hi","is_final":true}"#;
        let s: Segment = serde_json::from_str(line).unwrap();
        assert!(!s.supplement, "supplement should default to false for old jsonl without the field");
    }
}
