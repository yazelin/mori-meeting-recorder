# Phase 2 Live Captions + VAD Streaming — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 mori-meeting-recorder 從「錄完再 batch 轉錄」改成「邊錄邊 VAD-切段即時轉錄」— 即時字幕 + 靜音不送轉錄 + 結束即得稿 + 可調參數。

**Architecture:** audio thread 多餵一份給 `VadChunker`(純邏輯,偵測靜音切點吐 speech 段),speech 段進 `TranscribeWorker`(背景 thread + 佇列)跑 whisper-cli 短段,結果 append 寫 jsonl(single source of truth)+ emit `live-segment` event。Stop 改成 flush + drain + 讀 jsonl 彙整(不再 batch 整檔)。參數走 `RecorderConfig` json,前端加 Live + Settings 兩 tab。

**Tech Stack:** Rust(std::thread + mpsc + serde_json)、whisper-cli batch on short segments、React 18 + TS、Tauri 2 event/command。

**Spec:** `docs/superpowers/specs/2026-05-29-live-captions-vad-design.md`

**Phasing — 5 PR,各自 verify.sh 綠才 merge:**

- **PR A** `feat/recorder-config` — config.rs(TDD,純讀寫)
- **PR B** `feat/vad-chunker` — audio/vad.rs VadChunker(TDD,純邏輯核心)
- **PR C** `feat/transcribe-worker` — transcribe worker + jsonl append + offset(TDD on offset math)
- **PR D** `feat/streaming-recorder` — 接線:audio loop 餵 VAD、recorder start 起 worker、stop 改寫 + config command + live-segment emit(整合,最大風險)
- **PR E** `feat/live-settings-tabs` — 前端 LiveTab + SettingsTab + ExpandedView + i18n(純前端)

PR A/B/C 是純資料 / 純邏輯,無行為改變(不接線),可安全先 merge。PR D 才真正切換錄音行為。PR E 純前端。

**Manual e2e 限制:** 講話對麥驗 Live 字幕需 user 在場。所有 PR 的自動驗證(cargo test / npm build / cargo check / verify.sh)agentic worker 全程跑;manual e2e 清單留最後給 user。

---

## PR A — `feat/recorder-config`

### Task A0: Branch

- [ ] **Step 1**

```bash
cd /home/ct/mori-universe/mori-meeting-recorder
git fetch origin && git checkout main && git pull --ff-only origin main
git checkout -b feat/recorder-config
```

### Task A1: config.rs with TDD

**Files:**
- Create: `src-tauri/src/config.rs`
- Modify: `src-tauri/src/main.rs`(加 `mod config;`)

- [ ] **Step 1: 加 mod 宣告**

在 `src-tauri/src/main.rs` 既有 `mod` 群(找 `mod recorder;` 那區)加一行:

```rust
mod config;
```

- [ ] **Step 2: 寫 failing test(先 struct + 函式簽名 + test,無實作 body)**

Create `src-tauri/src/config.rs`:

```rust
//! Recorder 可調參數 — VAD chunking 行為。存 ~/.mori/meeting-recorder/config.json。
//! 缺檔 / parse fail / 缺欄 → 各自回預設(serde per-field default)。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_silence_split_ms() -> u64 { 600 }
fn default_silence_threshold_db() -> f32 { -45.0 }
fn default_min_speech_secs() -> f32 { 0.5 }
fn default_max_segment_secs() -> f32 { 20.0 }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecorderConfig {
    #[serde(default = "default_silence_split_ms")]
    pub silence_split_ms: u64,
    #[serde(default = "default_silence_threshold_db")]
    pub silence_threshold_db: f32,
    #[serde(default = "default_min_speech_secs")]
    pub min_speech_secs: f32,
    #[serde(default = "default_max_segment_secs")]
    pub max_segment_secs: f32,
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            silence_split_ms: default_silence_split_ms(),
            silence_threshold_db: default_silence_threshold_db(),
            min_speech_secs: default_min_speech_secs(),
            max_segment_secs: default_max_segment_secs(),
        }
    }
}

pub fn config_path() -> PathBuf {
    crate::session_store::default_meetings_dir()
        .parent()
        .map(|p| p.join("meeting-recorder").join("config.json"))
        .unwrap_or_else(|| PathBuf::from("config.json"))
}

pub fn read_config() -> RecorderConfig {
    let path = config_path();
    let s = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return RecorderConfig::default(),
    };
    serde_json::from_str(&s).unwrap_or_default()
}

pub fn write_config(cfg: &RecorderConfig) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir config dir: {e}"))?;
    }
    let s = serde_json::to_string_pretty(cfg).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&path, s).map_err(|e| format!("write config: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values_match_spec() {
        let c = RecorderConfig::default();
        assert_eq!(c.silence_split_ms, 600);
        assert_eq!(c.silence_threshold_db, -45.0);
        assert_eq!(c.min_speech_secs, 0.5);
        assert_eq!(c.max_segment_secs, 20.0);
    }

    #[test]
    fn deserialize_full_json() {
        let json = r#"{"silence_split_ms":800,"silence_threshold_db":-50.0,"min_speech_secs":1.0,"max_segment_secs":30.0}"#;
        let c: RecorderConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.silence_split_ms, 800);
        assert_eq!(c.max_segment_secs, 30.0);
    }

    #[test]
    fn missing_field_falls_back_to_default() {
        // 只給一個欄位,其他三個應回預設
        let json = r#"{"silence_split_ms":900}"#;
        let c: RecorderConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.silence_split_ms, 900);
        assert_eq!(c.silence_threshold_db, -45.0); // default
        assert_eq!(c.min_speech_secs, 0.5);        // default
        assert_eq!(c.max_segment_secs, 20.0);      // default
    }

    #[test]
    fn empty_json_all_defaults() {
        let c: RecorderConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c, RecorderConfig::default());
    }
}
```

- [ ] **Step 3: 跑 test 確認過**

Run: `cd src-tauri && cargo test config`
Expected: 4 tests pass。

- [ ] **Step 4: cargo check 全綠**

Run: `cd src-tauri && cargo check --all-targets 2>&1 | tail -3`
Expected: 0 errors。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/config.rs src-tauri/src/main.rs
git commit -m "feat(config): RecorderConfig with VAD params + serde per-field defaults (TDD)"
```

### Task A2: verify + PR

- [ ] **Step 1**: `bash scripts/verify.sh` 全綠
- [ ] **Step 2**: push + PR + merge

```bash
git push -u origin feat/recorder-config
gh pr create --title "feat(config): RecorderConfig VAD params (PR A of Phase 2)" --body "Phase 2 PR A — config.rs only, no behavior change. RecorderConfig struct + read/write ~/.mori/meeting-recorder/config.json + serde per-field defaults. 4 TDD tests. Spec: docs/superpowers/specs/2026-05-29-live-captions-vad-design.md §4.

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
gh pr merge --squash --delete-branch
```

---

## PR B — `feat/vad-chunker`

### Task B0: Branch

- [ ] **Step 1**

```bash
git checkout main && git pull --ff-only origin main
git checkout -b feat/vad-chunker
```

### Task B1: VadChunker with TDD

**Files:**
- Create: `src-tauri/src/audio/vad.rs`
- Modify: `src-tauri/src/audio/mod.rs`(加 `pub mod vad;`)

- [ ] **Step 1: 加 mod 宣告**

`src-tauri/src/audio/mod.rs` 既有 `pub mod levels;` 下面加:

```rust
pub mod vad;
```

- [ ] **Step 2: 寫 vad.rs(實作 + test 一起,純邏輯 TDD)**

Create `src-tauri/src/audio/vad.rs`:

```rust
//! VAD chunker — 偵測靜音切點,把連續 audio 切成 speech 段。純邏輯,無 IO。
//!
//! 喂法:audio loop 每個 50ms chunk 算好 rms_db 後呼 push()。chunk RMS >= threshold
//! 算有聲,< threshold 算靜音。連續靜音超過 silence_split → 切。speech 段累積到
//! max_segment → 強制切。< min_speech 的段丟掉(去噪)。

const SAMPLE_RATE: u64 = 16_000;

#[derive(Debug, Clone)]
pub struct VadConfig {
    pub silence_split_ms: u64,
    pub silence_threshold_db: f32,
    pub min_speech_secs: f32,
    pub max_segment_secs: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpeechSegment {
    pub samples: Vec<i16>,
    pub start_offset_ms: u64, // 相對該軌第一個 sample 的絕對時間
}

pub struct VadChunker {
    cfg: VadConfig,
    speech_buf: Vec<i16>,
    speech_start_offset_samples: u64,
    total_samples_seen: u64,
    silence_run_samples: u64,
    in_speech: bool,
}

impl VadChunker {
    pub fn new(cfg: VadConfig) -> Self {
        Self {
            cfg,
            speech_buf: Vec::new(),
            speech_start_offset_samples: 0,
            total_samples_seen: 0,
            silence_run_samples: 0,
            in_speech: false,
        }
    }

    fn silence_split_samples(&self) -> u64 {
        self.cfg.silence_split_ms * SAMPLE_RATE / 1000
    }
    fn min_speech_samples(&self) -> u64 {
        (self.cfg.min_speech_secs * SAMPLE_RATE as f32) as u64
    }
    fn max_segment_samples(&self) -> u64 {
        (self.cfg.max_segment_secs * SAMPLE_RATE as f32) as u64
    }

    /// 吃一個 chunk(samples + 已算好的 rms_db)。回傳這次切出的完整 speech 段(0 或 1)。
    pub fn push(&mut self, samples: &[i16], rms_db: f32) -> Option<SpeechSegment> {
        let chunk_start = self.total_samples_seen;
        self.total_samples_seen += samples.len() as u64;
        let is_voice = rms_db >= self.cfg.silence_threshold_db;

        if is_voice {
            if !self.in_speech {
                self.in_speech = true;
                self.speech_start_offset_samples = chunk_start;
                self.speech_buf.clear();
            }
            self.silence_run_samples = 0;
            self.speech_buf.extend_from_slice(samples);
        } else if self.in_speech {
            // 靜音但在 speech 中:尾段靜音也含進去(whisper 較準),累計 silence run
            self.speech_buf.extend_from_slice(samples);
            self.silence_run_samples += samples.len() as u64;
            if self.silence_run_samples >= self.silence_split_samples() {
                return self.cut();
            }
        }
        // max_segment 強制切(就算還在連續講)
        if self.in_speech && self.speech_buf.len() as u64 >= self.max_segment_samples() {
            return self.cut_forced();
        }
        None
    }

    /// 一般切(靜音觸發):吐段 if >= min_speech,然後離開 speech 狀態。
    fn cut(&mut self) -> Option<SpeechSegment> {
        let seg = self.take_if_long_enough();
        self.in_speech = false;
        self.silence_run_samples = 0;
        seg
    }

    /// 強制切(max_segment):吐段,但維持 in_speech,新段從下個 sample 開始。
    fn cut_forced(&mut self) -> Option<SpeechSegment> {
        let seg = self.take_if_long_enough();
        // 維持 in_speech,新段 offset = 目前位置
        self.speech_start_offset_samples = self.total_samples_seen;
        self.silence_run_samples = 0;
        seg
    }

    fn take_if_long_enough(&mut self) -> Option<SpeechSegment> {
        if self.speech_buf.len() as u64 >= self.min_speech_samples() {
            let samples = std::mem::take(&mut self.speech_buf);
            Some(SpeechSegment {
                samples,
                start_offset_ms: self.speech_start_offset_samples * 1000 / SAMPLE_RATE,
            })
        } else {
            self.speech_buf.clear();
            None
        }
    }

    /// stop 時呼叫,吐剩餘 speech 段(若 >= min_speech)。
    pub fn flush(&mut self) -> Option<SpeechSegment> {
        if self.in_speech {
            let seg = self.take_if_long_enough();
            self.in_speech = false;
            seg
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> VadConfig {
        VadConfig {
            silence_split_ms: 600,
            silence_threshold_db: -45.0,
            min_speech_secs: 0.5,
            max_segment_secs: 20.0,
        }
    }

    // 50ms chunk @16kHz = 800 samples
    const CHUNK: usize = 800;
    fn voice_chunk() -> Vec<i16> { vec![5000; CHUNK] }   // loud
    fn silent_chunk() -> Vec<i16> { vec![0; CHUNK] }

    const VOICE_DB: f32 = -20.0;   // >= -45 → voice
    const SILENT_DB: f32 = -90.0;  // < -45 → silence

    #[test]
    fn speech_then_600ms_silence_cuts_one_segment() {
        let mut v = VadChunker::new(cfg());
        // 1 秒 speech (20 chunks) — 達 min_speech 0.5s
        for _ in 0..20 {
            assert!(v.push(&voice_chunk(), VOICE_DB).is_none());
        }
        // 靜音:600ms = 12 chunks。前 11 個不切,第 12 個切。
        for _ in 0..11 {
            assert!(v.push(&silent_chunk(), SILENT_DB).is_none());
        }
        let seg = v.push(&silent_chunk(), SILENT_DB);
        assert!(seg.is_some(), "12th silent chunk should trigger cut");
        let seg = seg.unwrap();
        assert_eq!(seg.start_offset_ms, 0);
        // samples = 20 voice + 12 silence = 32 chunks(含尾段靜音)
        assert_eq!(seg.samples.len(), 32 * CHUNK);
    }

    #[test]
    fn short_inter_word_silence_does_not_cut() {
        let mut v = VadChunker::new(cfg());
        for _ in 0..20 { v.push(&voice_chunk(), VOICE_DB); }
        // 100ms 靜音 = 2 chunks(< 600ms),不切
        assert!(v.push(&silent_chunk(), SILENT_DB).is_none());
        assert!(v.push(&silent_chunk(), SILENT_DB).is_none());
        // 又有聲,silence_run 應歸零
        assert!(v.push(&voice_chunk(), VOICE_DB).is_none());
    }

    #[test]
    fn too_short_speech_dropped() {
        let mut v = VadChunker::new(cfg());
        // 0.25s speech = 5 chunks(< min_speech 0.5s = 10 chunks)
        for _ in 0..5 { v.push(&voice_chunk(), VOICE_DB); }
        // 600ms 靜音觸發切,但段太短應丟掉(回 None)
        for _ in 0..11 { v.push(&silent_chunk(), SILENT_DB); }
        let seg = v.push(&silent_chunk(), SILENT_DB);
        assert!(seg.is_none(), "sub-min_speech segment should be dropped");
    }

    #[test]
    fn max_segment_forces_cut() {
        let mut v = VadChunker::new(cfg());
        // 連續講 20 秒 = 400 chunks。max_segment 20s = 320000 samples = 400 chunks。
        // 第 400 chunk 後 buf >= max,強制切。
        let mut cut_seen = false;
        for _ in 0..400 {
            if v.push(&voice_chunk(), VOICE_DB).is_some() {
                cut_seen = true;
            }
        }
        assert!(cut_seen, "max_segment should force a cut within 400 chunks");
    }

    #[test]
    fn consecutive_segments_have_increasing_offset() {
        let mut v = VadChunker::new(cfg());
        // 段1:1s speech + 600ms silence
        for _ in 0..20 { v.push(&voice_chunk(), VOICE_DB); }
        for _ in 0..11 { v.push(&silent_chunk(), SILENT_DB); }
        let seg1 = v.push(&silent_chunk(), SILENT_DB).unwrap();
        assert_eq!(seg1.start_offset_ms, 0);
        // 一些靜音間隔(not in speech,不累積)
        for _ in 0..5 { assert!(v.push(&silent_chunk(), SILENT_DB).is_none()); }
        // 段2:1s speech + 600ms silence
        let offset_before_seg2 = (20 + 12 + 5) * CHUNK as u64; // samples seen so far
        for _ in 0..20 { v.push(&voice_chunk(), VOICE_DB); }
        for _ in 0..11 { v.push(&silent_chunk(), SILENT_DB); }
        let seg2 = v.push(&silent_chunk(), SILENT_DB).unwrap();
        let expected_ms = offset_before_seg2 * 1000 / SAMPLE_RATE;
        assert_eq!(seg2.start_offset_ms, expected_ms);
    }

    #[test]
    fn flush_emits_remaining_speech() {
        let mut v = VadChunker::new(cfg());
        for _ in 0..20 { v.push(&voice_chunk(), VOICE_DB); }
        // 沒等到 600ms 靜音就 stop → flush 應吐這段
        let seg = v.flush();
        assert!(seg.is_some());
        assert_eq!(seg.unwrap().samples.len(), 20 * CHUNK);
    }

    #[test]
    fn flush_drops_too_short() {
        let mut v = VadChunker::new(cfg());
        for _ in 0..3 { v.push(&voice_chunk(), VOICE_DB); } // 0.15s < min
        assert!(v.flush().is_none());
    }
}
```

- [ ] **Step 3: 跑 test**

Run: `cd src-tauri && cargo test vad`
Expected: 7 tests pass。如果 max_segment 那題的 chunk 數算不準,微調測試的 chunk 迴圈數(400 chunks × 800 = 320000 = 正好 20s,邊界可能 401,看實作 `>=`)。

- [ ] **Step 4: verify + commit + PR + merge**

```bash
bash scripts/verify.sh
git add src-tauri/src/audio/vad.rs src-tauri/src/audio/mod.rs
git commit -m "feat(vad): VadChunker silence-triggered chunking with absolute offset (TDD)"
git push -u origin feat/vad-chunker
gh pr create --title "feat(vad): VadChunker silence-triggered chunking (PR B of Phase 2)" --body "Phase 2 PR B — audio/vad.rs pure logic, no wiring yet. 7 TDD tests: cut on 600ms silence / no cut on inter-word pause / drop short / max_segment force cut / offset accumulation / flush. Spec §2.

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
gh pr merge --squash --delete-branch
```

---

## PR C — `feat/transcribe-worker`

### Task C0: Branch

```bash
git checkout main && git pull --ff-only origin main
git checkout -b feat/transcribe-worker
```

### Task C1: offset-adjust helper + jsonl append (TDD)

**Files:**
- Modify: `src-tauri/src/transcribe.rs`(加 `shift_segments_by_offset` + `append_segments_jsonl` + tests)

- [ ] **Step 1: 看現有 transcribe.rs Segment + parse**

Run: `grep -n "pub struct Segment\|pub fn parse_whisper_json\|start_ms\|end_ms" src-tauri/src/transcribe.rs | head`
確認 Segment 有 `start_ms: u64` / `end_ms: u64`。

- [ ] **Step 2: 加 offset shift + append helpers + tests**

在 `src-tauri/src/transcribe.rs` 結尾(test mod 之前)加:

```rust
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
```

在 `#[cfg(test)] mod tests` 內加:

```rust
    #[test]
    fn shift_offset_adds_to_both_ends() {
        let segs = vec![Segment {
            id: "s1".into(), session_id: "x".into(), track: "system".into(),
            source_kind: "meeting_system".into(), visibility: "public".into(),
            start_ms: 100, end_ms: 500, text: "hi".into(), is_final: true, confidence: None,
        }];
        let shifted = shift_segments_by_offset(segs, 10_000);
        assert_eq!(shifted[0].start_ms, 10_100);
        assert_eq!(shifted[0].end_ms, 10_500);
    }

    #[test]
    fn append_jsonl_round_trip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("transcript").join("system.segments.jsonl");
        let seg = Segment {
            id: "s1".into(), session_id: "x".into(), track: "system".into(),
            source_kind: "meeting_system".into(), visibility: "public".into(),
            start_ms: 0, end_ms: 1000, text: "a".into(), is_final: true, confidence: None,
        };
        append_segments_jsonl(&path, std::slice::from_ref(&seg)).unwrap();
        append_segments_jsonl(&path, std::slice::from_ref(&seg)).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), 2);
        // 每行可 parse 回 Segment
        for line in content.lines() {
            let _: Segment = serde_json::from_str(line).unwrap();
        }
    }
```

(若 `Segment` 欄位跟上述不符,先 `grep` 看實際欄位再調 test literal。`tempfile` 已是 dev-dep。)

- [ ] **Step 3: 跑 test**

Run: `cd src-tauri && cargo test transcribe`
Expected: 既有 + 2 新 pass。

- [ ] **Step 4: verify + commit + PR + merge**

```bash
bash scripts/verify.sh
git add src-tauri/src/transcribe.rs
git commit -m "feat(transcribe): shift_segments_by_offset + append_segments_jsonl (TDD)"
git push -u origin feat/transcribe-worker
gh pr create --title "feat(transcribe): offset shift + jsonl append helpers (PR C of Phase 2)" --body "Phase 2 PR C — pure helpers, no wiring. shift_segments_by_offset(段內相對→整場絕對) + append_segments_jsonl(邊轉邊落地,格式同 Phase 1). 2 TDD tests. Spec §3.

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
gh pr merge --squash --delete-branch
```

---

## PR D — `feat/streaming-recorder`(整合,最大風險)

### Task D0: Branch

```bash
git checkout main && git pull --ff-only origin main
git checkout -b feat/streaming-recorder
```

### Task D1: CaptureHandle 多帶一個 speech-segment channel

**Files:**
- Modify: `src-tauri/src/audio/mod.rs`、`src-tauri/src/audio/linux.rs`、`src-tauri/src/audio/windows.rs`

設計:audio capture thread 內部建 VadChunker,每個 chunk push,切出的 SpeechSegment 透過 `std::sync::mpsc::Sender<SpeechSegment>` 送出。CaptureHandle 持有對應的 `Receiver`,recorder 把它交給 transcribe worker。

- [ ] **Step 1: open_capture 簽名加 VadConfig 參數 + 回傳帶 segment receiver**

`src-tauri/src/audio/mod.rs` CaptureHandle 加欄位:

```rust
pub struct CaptureHandle {
    pub source: SourceKind,
    pub writer_handle: JoinHandle<Result<u64, String>>,
    pub signal: Arc<Mutex<SignalMeter>>,
    pub stop_flag: Arc<std::sync::atomic::AtomicBool>,
    pub speech_rx: std::sync::mpsc::Receiver<crate::audio::vad::SpeechSegment>, // 新
}
```

`open_capture` wrapper 簽名改:

```rust
#[cfg(target_os = "linux")]
pub fn open_capture(
    source: SourceKind,
    out_path: std::path::PathBuf,
    vad_cfg: crate::audio::vad::VadConfig,
) -> Result<CaptureHandle, String> {
    linux::open_capture(source, out_path, vad_cfg)
}
```

(windows 同樣加參數)

- [ ] **Step 2: linux.rs capture loop 內接 VadChunker**

`src-tauri/src/audio/linux.rs` `open_capture` 加 `vad_cfg` 參數;建 channel;capture thread 內建 `VadChunker::new(vad_cfg)`;每 chunk 算完 rms_db 後:

```rust
                    // VAD:push chunk,切出的段送給 transcribe worker
                    if let Some(seg) = chunker.push(&samples, rms_db) {
                        let _ = speech_tx.send(seg);
                    }
```

stop loop 結束後(while 出來、drop simple 前)flush:

```rust
        if let Some(seg) = chunker.flush() {
            let _ = speech_tx.send(seg);
        }
```

`speech_tx` 是 thread move 進去的 Sender;`speech_rx` 放進回傳的 CaptureHandle。

注意:`rms_db` 變數是 PR #13 smoothing 後的值還是 raw?VAD 應該用 **raw rms**(smoothing 是給 VU 視覺的,VAD 要真實瞬時)。看現有 code:`compute_levels` 回 `(peak_db_raw, rms_db_raw)` 然後 smooth 寫進 SignalMeter。VAD 用 `rms_db_raw`(smooth 前)。

- [ ] **Step 3: windows.rs 同樣接 VadChunker**

`handle_chunk_f32` 需要能拿到 chunker + speech_tx。把它們透過參數或 closure 捕獲傳進去。windows 的 `handle_chunk_f32` 目前是 free fn,改成讓 build_input_stream 的 closure 捕獲 `Arc<Mutex<VadChunker>>` + speech_tx,在算完 rms 後 push。

(windows 無法在 Linux 測,確保 `cargo check` 在 Linux 跳過 windows.rs 即可;textual 對齊 linux 邏輯。)

- [ ] **Step 4: cargo check**

Run: `cd src-tauri && cargo check --all-targets 2>&1 | tail -5`
Expected: 0 errors(會有 unused speech_rx 警告,下個 task 用掉)。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/audio/
git commit -m "feat(audio): capture thread runs VadChunker, emits SpeechSegment via channel"
```

### Task D2: TranscribeWorker

**Files:**
- Modify: `src-tauri/src/transcribe.rs`(加 TranscribeWorker)

- [ ] **Step 1: 加 TranscribeWorker struct**

在 `src-tauri/src/transcribe.rs` 加:

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// 背景 transcribe worker — 從 channel 收 SpeechSegment,跑 whisper,append jsonl + emit。
pub struct TranscribeWorker {
    pub handle: std::thread::JoinHandle<()>,
    pub pending: Arc<AtomicUsize>,
}

/// 啟一個 worker。speech_rx 來自 CaptureHandle;每段轉完呼 on_segment(track, &segments) 給呼叫端 emit。
pub fn spawn_transcribe_worker(
    speech_rx: std::sync::mpsc::Receiver<crate::audio::vad::SpeechSegment>,
    session_id: String,
    kind: crate::audio::SourceKind,
    jsonl_path: std::path::PathBuf,
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
            let raw = run_whisper(&tmp, &session_id, kind);
            let _ = std::fs::remove_file(&tmp);
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
```

`write_wav_16k_mono` — 看 `audio/writer.rs` 是否已有可複用的 WAV 寫入;若無,在 transcribe.rs 加一個簡單 hound-based 或複用 TrackWriter。**先 grep**:

```bash
grep -n "hound\|WavWriter\|fn create\|push_samples\|finalize" src-tauri/src/audio/writer.rs
```

若 writer.rs 用 hound,直接複用:`TrackWriter::create(path)` + `push_samples` + `finalize`。把 `write_wav_16k_mono` 實作成包這三步。

- [ ] **Step 2: cargo check**

Run: `cd src-tauri && cargo check --all-targets 2>&1 | tail -5`

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/transcribe.rs
git commit -m "feat(transcribe): spawn_transcribe_worker — whisper short segs + append jsonl + callback"
```

### Task D3: recorder start/stop 改寫

**Files:**
- Modify: `src-tauri/src/recorder.rs`、`src-tauri/src/main.rs`

- [ ] **Step 1: ActiveSession 加 workers**

`recorder.rs` ActiveSession:

```rust
pub struct ActiveSession {
    pub store: SessionStore,
    pub started_at: DateTime<Local>,
    pub handles: Vec<CaptureHandle>,
    pub workers: Vec<crate::transcribe::TranscribeWorker>, // 新
}
```

- [ ] **Step 2: start_session 起 worker**

`start_session` 內,open_capture 改帶 vad_cfg(從 `crate::config::read_config()` 轉 VadConfig);capture handles 建好後,為每軌 spawn worker。worker 的 `on_segment` callback emit `live-segment`:

```rust
        let cfg = crate::config::read_config();
        let vad_cfg = crate::audio::vad::VadConfig {
            silence_split_ms: cfg.silence_split_ms,
            silence_threshold_db: cfg.silence_threshold_db,
            min_speech_secs: cfg.min_speech_secs,
            max_segment_secs: cfg.max_segment_secs,
        };
        // ... open_capture(kind, out, vad_cfg.clone()) ...

        // 每軌起 transcribe worker
        let mut workers = Vec::new();
        for h in &mut handles {
            // 注意:speech_rx 要 move 出 handle。用 Option<Receiver> 或重構 handle 結構。
        }
```

**Receiver move 問題**:`CaptureHandle.speech_rx` 是 `Receiver`(不可 clone)。spawn worker 要 move 它出來。把 `speech_rx` 改成 `Option<Receiver>`,spawn 時 `.take()`。或在 open_capture 回傳 `(CaptureHandle, Receiver)` tuple。**選後者較乾淨**:open_capture 回 `(CaptureHandle, Receiver<SpeechSegment>)`,start_session 立刻把 receiver 配 worker。CaptureHandle 不存 receiver。

修正 Task D1 Step 1:CaptureHandle **不**加 speech_rx 欄位;改 open_capture 回 tuple。

worker callback:

```rust
            let app = self.app.clone();
            let track_name = match h.source {
                SourceKind::MeetingSystem => "sys",
                SourceKind::MicInternal => "mic",
            };
            let jsonl = store.segments_path(h.source);
            let sid = session_id.clone();
            let worker = crate::transcribe::spawn_transcribe_worker(
                rx, sid, h.source, jsonl,
                move |segs| {
                    if let Some(app) = &app {
                        for s in segs {
                            let _ = app.emit("live-segment", serde_json::json!({
                                "track": track_name,
                                "segment": s,
                            }));
                        }
                    }
                },
            );
            workers.push(worker);
```

- [ ] **Step 3: stop_session 改寫**

```rust
    pub fn stop_session(&self) -> Result<String, String> {
        let mut active_guard = self.active.lock().map_err(|e| e.to_string())?;
        let session = active_guard.take().ok_or("no active session")?;
        drop(active_guard);
        *self.state.lock().map_err(|e| e.to_string())? = State::Transcribing;

        let session_id = session.store.session_id.clone();
        let store = session.store;
        let started_at = session.started_at;

        // 1. 停 capture(stop_flag → capture thread flush VadChunker → 送最後段 → channel 關)
        for h in &session.handles {
            h.stop_flag.store(true, Ordering::Relaxed);
        }
        for h in session.handles {
            let _ = h.writer_handle.join();
        }
        // 2. capture thread 已 drop Sender → worker 的 recv() 收到 Err → loop 結束。join worker(drain 完佇列)。
        for w in session.workers {
            let _ = w.handle.join();
        }
        // 3. 讀回 jsonl 彙整(不再 batch 整檔)
        let sys_segs = read_segments_jsonl(&store.segments_path(SourceKind::MeetingSystem));
        let mic_segs = read_segments_jsonl(&store.segments_path(SourceKind::MicInternal));
        let all_segs: Vec<Segment> = sys_segs.iter().chain(mic_segs.iter()).cloned().collect();

        // 4. export md + timeline(復用 Phase 1 exporter::export)
        let stopped_at = Local::now();
        let meta = /* 同 Phase 1 SessionMeta,segment_count 用 sys_segs.len()/mic_segs.len() */;
        let (pub_md, int_md, timeline) = export(&all_segs, &meta)?;
        std::fs::write(store.public_md_path(), pub_md).map_err(|e| format!("write public.md: {e}"))?;
        std::fs::write(store.internal_md_path(), int_md).map_err(|e| format!("write internal.md: {e}"))?;
        std::fs::write(store.timeline_path(), timeline).map_err(|e| format!("write timeline.json: {e}"))?;

        *self.state.lock().map_err(|e| e.to_string())? = State::Idle;
        Ok(session_id)
    }
```

加 helper `read_segments_jsonl`(在 transcribe.rs):

```rust
pub fn read_segments_jsonl(path: &std::path::Path) -> Vec<Segment> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    content.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
```

注意:stop 前舊 code 用 `tokio::runtime::Runtime::new` + block_on 跑 parallel whisper — **整段刪掉**,改成上面的 read jsonl。

- [ ] **Step 4: levels 計算 — 移除舊 batch 轉錄,確保 segment_count 對**

`exporter::SessionMeta` 的 TrackMeta.segment_count 用 `sys_segs.len()` / `mic_segs.len()`。其餘 meta 欄位同 Phase 1。

- [ ] **Step 5: get_config / set_config command**

`main.rs` 加:

```rust
#[tauri::command]
fn get_config() -> config::RecorderConfig {
    config::read_config()
}

#[tauri::command]
fn set_config(cfg: config::RecorderConfig) -> Result<(), String> {
    config::write_config(&cfg)
}
```

註冊進 `generate_handler!`:加 `get_config, set_config,`。

- [ ] **Step 6: cargo check + test**

Run: `cd src-tauri && cargo check --all-targets 2>&1 | tail -8 && cargo test 2>&1 | tail -5`
Expected: 0 errors;既有 + 新 test 全綠。**recorder_stop 是 async(PR #9)— spawn_blocking 內呼 stop_session,維持不變。**

- [ ] **Step 7: verify + commit + PR + merge**

```bash
bash scripts/verify.sh
git add src-tauri/src/
git commit -m "feat(recorder): streaming transcribe — VAD per-track worker, stop drains + reads jsonl"
git push -u origin feat/streaming-recorder
gh pr create --title "feat(recorder): streaming VAD transcribe pipeline (PR D of Phase 2)" --body "Phase 2 PR D — 接線(最大改動)。audio thread 跑 VadChunker→channel→per-track TranscribeWorker→whisper 短段→append jsonl + emit live-segment。stop 改成 flush+drain worker+讀 jsonl 彙整(刪掉舊 batch 整檔轉錄)。get_config/set_config command。Spec §1/§3/§5.

⚠ 行為切換:此 PR 後錄音是即時轉錄。需 user 手動 e2e 驗(講話對麥看 live-segment + stop 即得稿)。

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
gh pr merge --squash --delete-branch
```

---

## PR E — `feat/live-settings-tabs`(純前端)

### Task E0: Branch

```bash
git checkout main && git pull --ff-only origin main
git checkout -b feat/live-settings-tabs
```

### Task E1: i18n keys

**Files:** `src/i18n/locales/en.json`、`src/i18n/locales/zh-TW.json`

- [ ] **Step 1: 加 keys**

en.json 加(在頂層 object 內):

```json
  "tabs_live": "Live",
  "tabs_settings": "Settings",
  "live": {
    "empty": "Live captions will appear here once recording starts.",
    "pending": "transcribing… ({{n}} pending)",
    "col_sys": "Meeting audio (SYS)",
    "col_mic": "Mic-internal (MIC)"
  },
  "settings": {
    "title": "Transcribe parameters",
    "silence_split": "Silence split threshold",
    "silence_split_hint": "How long a pause counts as a sentence break. Lower = snappier captions but more fragments.",
    "silence_threshold": "Silence volume threshold",
    "silence_threshold_hint": "Below this volume is treated as silence.",
    "min_speech": "Minimum speech length",
    "min_speech_hint": "Sounds shorter than this (noise) are not transcribed.",
    "max_segment": "Max segment length",
    "max_segment_hint": "Force a cut after speaking continuously this long.",
    "default": "default {{v}}",
    "reset": "Reset defaults",
    "save": "Save",
    "saved": "Saved — takes effect next recording"
  }
```

(現有 tabs 在 `tabs` object 內,新增 Live/Settings 對齊既有 key 風格 — 先 grep `"tabs"` 看結構再放對位置。)

zh-TW.json 對應:

```json
  "live": {
    "empty": "開始錄音後,即時字幕會出現在這裡。",
    "pending": "轉錄中…(待處理 {{n}} 段)",
    "col_sys": "會議音訊 (SYS)",
    "col_mic": "內部麥克風 (MIC)"
  },
  "settings": {
    "title": "轉錄參數",
    "silence_split": "靜音切點門檻",
    "silence_split_hint": "一句話講完的停頓多久算切點。調小=字幕更即時但更碎。",
    "silence_threshold": "靜音音量門檻",
    "silence_threshold_hint": "低於此音量視為靜音。",
    "min_speech": "最短語音段",
    "min_speech_hint": "比這短的聲音(雜音)不送轉錄。",
    "max_segment": "安全切點上限",
    "max_segment_hint": "連續講話超過此長度強制切一次。",
    "default": "預設 {{v}}",
    "reset": "還原預設",
    "save": "儲存",
    "saved": "已儲存 — 下次錄音生效"
  }
```

(Live/Settings tab 標籤鍵:看現有 `tabs.record` 等結構,加 `tabs.live` / `tabs.settings`。)

- [ ] **Step 2: build 確認 JSON valid**

Run: `npm run build 2>&1 | tail -3`

- [ ] **Step 3: Commit**

```bash
git add src/i18n/
git commit -m "feat(i18n): live + settings tab keys"
```

### Task E2: LiveColumn + LiveTab

**Files:** Create `src/components/LiveColumn.tsx`、`src/tabs/LiveTab.tsx`;Modify `src/theme.css`

- [ ] **Step 1: LiveColumn component**

```tsx
// src/components/LiveColumn.tsx
//
// 單欄即時字幕滾動(SYS or MIC)。每行時間戳 + 文字,新段從底長出 + auto-scroll。

import { useEffect, useRef } from "react";

export interface LiveSegment {
  start_ms: number;
  text: string;
}

function fmtTs(ms: number): string {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
}

export default function LiveColumn({ title, segments }: { title: string; segments: LiveSegment[] }) {
  const bottomRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [segments.length]);
  return (
    <div className="live-col">
      <div className="live-col-title">{title}</div>
      <div className="live-col-body">
        {segments.map((s, i) => (
          <div key={i} className="live-line">
            <span className="live-ts">{fmtTs(s.start_ms)}</span>
            <span className="live-text">{s.text}</span>
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
```

- [ ] **Step 2: LiveTab**

```tsx
// src/tabs/LiveTab.tsx
//
// 雙欄即時字幕。listen "live-segment" event,依 track 分流到 sys / mic 欄。

import { useEffect, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import LiveColumn, { type LiveSegment } from "../components/LiveColumn";

interface LiveSegmentEvent {
  track: "sys" | "mic";
  segment: { start_ms: number; text: string };
}

export default function LiveTab() {
  const { t } = useTranslation();
  const [sys, setSys] = useState<LiveSegment[]>([]);
  const [mic, setMic] = useState<LiveSegment[]>([]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    (async () => {
      unlisten = await listen<LiveSegmentEvent>("live-segment", (e) => {
        const seg = { start_ms: e.payload.segment.start_ms, text: e.payload.segment.text };
        if (e.payload.track === "sys") setSys((p) => [...p, seg]);
        else setMic((p) => [...p, seg]);
      });
    })();
    return () => { unlisten?.(); };
  }, []);

  const empty = sys.length === 0 && mic.length === 0;
  return (
    <div>
      {empty && <p style={{ color: "var(--text-dim)", fontSize: 12 }}>{t("live.empty")}</p>}
      <div className="live-cols">
        <LiveColumn title={t("live.col_sys")} segments={sys} />
        <LiveColumn title={t("live.col_mic")} segments={mic} />
      </div>
    </div>
  );
}
```

- [ ] **Step 3: CSS**

Append `src/theme.css`:

```css

/* Live captions 雙欄 */
.live-cols { display: flex; gap: 12px; }
.live-col {
  flex: 1; min-width: 0;
  display: flex; flex-direction: column;
  border: 0.5px solid var(--border); border-radius: 10px;
  background: rgba(255,255,255,0.02);
  height: 360px;
}
.live-col-title {
  padding: 8px 10px; font-size: 11px; font-weight: 600;
  color: var(--text-secondary); border-bottom: 0.5px solid var(--border);
}
.live-col-body { flex: 1; overflow-y: auto; padding: 8px 10px; }
.live-line { display: flex; gap: 8px; margin-bottom: 6px; font-size: 12px; line-height: 1.5; }
.live-ts {
  font-family: ui-monospace, "SF Mono", monospace; font-size: 10px;
  color: var(--text-dim); flex-shrink: 0; padding-top: 1px;
}
.live-text { color: var(--text); }
```

- [ ] **Step 4: build + commit**

```bash
npm run build && npx tsc --noEmit
git add src/components/LiveColumn.tsx src/tabs/LiveTab.tsx src/theme.css
git commit -m "feat(live-tab): dual-column live captions via live-segment event"
```

### Task E3: SettingField + SettingsTab

**Files:** Create `src/components/SettingField.tsx`、`src/tabs/SettingsTab.tsx`;Modify `src/theme.css`

- [ ] **Step 1: SettingField**

```tsx
// src/components/SettingField.tsx
//
// 一個參數列:label + 數字 input + 單位 + 預設值提示 + 一行說明。

interface Props {
  label: string;
  hint: string;
  unit: string;
  defaultLabel: string;
  value: number;
  step?: number;
  onChange: (v: number) => void;
}

export default function SettingField({ label, hint, unit, defaultLabel, value, step = 1, onChange }: Props) {
  return (
    <div className="setting-field">
      <div className="setting-field-row">
        <span className="setting-field-label">{label}</span>
        <input
          type="number"
          className="setting-field-input"
          value={value}
          step={step}
          onChange={(e) => onChange(parseFloat(e.target.value))}
        />
        <span className="setting-field-unit">{unit}</span>
        <span className="setting-field-default">{defaultLabel}</span>
      </div>
      <div className="setting-field-hint">{hint}</div>
    </div>
  );
}
```

- [ ] **Step 2: SettingsTab**

```tsx
// src/tabs/SettingsTab.tsx
//
// VAD 轉錄參數設定。get_config 讀,set_config 存。改了下次錄音生效。

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import SettingField from "../components/SettingField";

interface RecorderConfig {
  silence_split_ms: number;
  silence_threshold_db: number;
  min_speech_secs: number;
  max_segment_secs: number;
}

const DEFAULTS: RecorderConfig = {
  silence_split_ms: 600,
  silence_threshold_db: -45,
  min_speech_secs: 0.5,
  max_segment_secs: 20,
};

export default function SettingsTab() {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<RecorderConfig>(DEFAULTS);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    invoke<RecorderConfig>("get_config").then(setCfg).catch(() => setCfg(DEFAULTS));
  }, []);

  const save = async () => {
    try {
      await invoke("set_config", { cfg });
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) { console.error(e); }
  };

  return (
    <div>
      <h3 style={{ marginTop: 0 }}>{t("settings.title")}</h3>
      <SettingField
        label={t("settings.silence_split")} hint={t("settings.silence_split_hint")}
        unit="ms" defaultLabel={t("settings.default", { v: 600 })}
        value={cfg.silence_split_ms} step={50}
        onChange={(v) => setCfg({ ...cfg, silence_split_ms: v })}
      />
      <SettingField
        label={t("settings.silence_threshold")} hint={t("settings.silence_threshold_hint")}
        unit="dB" defaultLabel={t("settings.default", { v: -45 })}
        value={cfg.silence_threshold_db} step={1}
        onChange={(v) => setCfg({ ...cfg, silence_threshold_db: v })}
      />
      <SettingField
        label={t("settings.min_speech")} hint={t("settings.min_speech_hint")}
        unit="s" defaultLabel={t("settings.default", { v: 0.5 })}
        value={cfg.min_speech_secs} step={0.1}
        onChange={(v) => setCfg({ ...cfg, min_speech_secs: v })}
      />
      <SettingField
        label={t("settings.max_segment")} hint={t("settings.max_segment_hint")}
        unit="s" defaultLabel={t("settings.default", { v: 20 })}
        value={cfg.max_segment_secs} step={1}
        onChange={(v) => setCfg({ ...cfg, max_segment_secs: v })}
      />
      <div style={{ display: "flex", gap: 8, marginTop: 14, alignItems: "center" }}>
        <button className="mmr-btn" onClick={() => setCfg(DEFAULTS)}>{t("settings.reset")}</button>
        <button className="mmr-btn primary" onClick={save}>{t("settings.save")}</button>
        {saved && <span style={{ color: "var(--found-color)", fontSize: 11 }}>{t("settings.saved")}</span>}
      </div>
    </div>
  );
}
```

⚠ **Tauri arg camelCase**:`set_config(cfg)` Rust 參數叫 `cfg`,JS invoke 傳 `{ cfg }`。Rust command 參數 `cfg: RecorderConfig` → JS key `cfg`(單字無底線,不變)。確認。

- [ ] **Step 3: CSS**

Append `src/theme.css`:

```css

/* Settings 參數列 */
.setting-field { margin-bottom: 14px; }
.setting-field-row { display: flex; align-items: center; gap: 8px; }
.setting-field-label { font-size: 12px; color: var(--text); min-width: 140px; }
.setting-field-input {
  width: 70px; padding: 4px 8px; font-size: 12px;
  background: var(--btn-bg); color: var(--text);
  border: 0.5px solid var(--border); border-radius: 6px;
  font-family: ui-monospace, monospace;
}
.setting-field-unit { font-size: 11px; color: var(--text-secondary); }
.setting-field-default { font-size: 10px; color: var(--text-dim); margin-left: auto; }
.setting-field-hint { font-size: 10.5px; color: var(--text-dim); margin-top: 3px; line-height: 1.4; }
```

- [ ] **Step 4: build + commit**

```bash
npm run build && npx tsc --noEmit
git add src/components/SettingField.tsx src/tabs/SettingsTab.tsx src/theme.css
git commit -m "feat(settings-tab): VAD param editor with get_config/set_config"
```

### Task E4: ExpandedView 接 2 個新 tab

**Files:** `src/ExpandedView.tsx`

- [ ] **Step 1: 看現有 tab 結構**

Run: `cat src/ExpandedView.tsx`
找到 tab 定義(陣列或 switch)+ tab body 渲染。

- [ ] **Step 2: 加 Live + Settings**

import:

```tsx
import LiveTab from "./tabs/LiveTab";
import SettingsTab from "./tabs/SettingsTab";
```

tab 清單加 `live` / `settings`(順序:record / live / sessions / deps / settings),body switch 加對應 case。tab 標籤用 `t("tabs.live")` / `t("tabs.settings")`(或 E1 定的鍵名)。

- [ ] **Step 3: build + tsc**

```bash
npm run build && npx tsc --noEmit
```

- [ ] **Step 4: commit + PR + merge**

```bash
git add src/ExpandedView.tsx
git commit -m "feat(ui): wire Live + Settings tabs into ExpandedView"
git push -u origin feat/live-settings-tabs
gh pr create --title "feat(ui): Live captions + Settings tabs (PR E of Phase 2)" --body "Phase 2 PR E — 純前端。LiveTab(雙欄即時字幕 listen live-segment)+ SettingsTab(VAD 參數 get_config/set_config)+ ExpandedView 接 5 tab + i18n. Spec §6.

需 user 手動 e2e 驗 Live 字幕滾動 + Settings 存讀。

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
gh pr merge --squash --delete-branch
```

---

## Manual e2e 清單(留給 user 明早)

全部 5 PR merge 後,**重啟 tauri dev**(Rust 改了),逐項驗:

1. **即時字幕**:展開 → Live tab → 開始錄音 → 播 YouTube + 對麥講話 → SYS 左欄 / MIC 右欄一句句冒出字幕
2. **VAD 切點**:講一句停一下 → 字幕一句句出現(不是整段);靜音時段不冒空行
3. **結束即得稿**:Stop → 幾乎立刻完成(不卡 transcribing);Sessions tab 新卡 segs > 0;開資料夾看 public.md 不空
4. **參數可調**:Settings tab → 改 silence_split_ms → 儲存 → 下次錄音切點變化
5. **當機不丟稿**(可選):錄一段後 `pkill -f mori-meeting-recorder` → 看 `~/.mori/meetings/<最新>/transcript/*.segments.jsonl` 已有先前句子
6. **降級**:若 whisper deps 在 → 正常;Live tab 不會空轉

任何項不對 → 回報,逐項修。

---

## Self-Review Notes

| Spec 段 | 對應 PR/Task |
|---|---|
| §1 架構(三路 + stop 流程) | PR D(D1 audio 餵 VAD / D2 worker / D3 stop 改寫) |
| §1 jsonl single source of truth | PR C(append helper)+ PR D(stop 讀 jsonl) |
| §2 VadChunker + 4 參數 | PR B |
| §2 時間軸 offset | PR B(offset 累加)+ PR C(shift) |
| §3 TranscribeWorker + pending | PR D(D2) |
| §4 RecorderConfig | PR A |
| §5 get/set_config + live-segment emit | PR D(D3) |
| §6 LiveTab / SettingsTab / 5 tab | PR E |
| §7 測試 | A1/B1/C1 TDD;manual e2e 清單 |

**已知風險已標**:
- PR D Receiver move — plan 內已修正成 open_capture 回 tuple(不存 receiver 在 handle)
- VAD 用 raw rms(smooth 前)— D1 Step 2 已註明
- windows.rs 無法 Linux 測 — D1 Step 3 textual 對齊
- recorder_stop async(PR #9)維持 — D3 Step 6 已註明
- pending counter 顯示(Live tab "轉錄中 N 段")— spec 有,plan E2 LiveTab 暫未接 pending UI;**列為 E 的可選補強**,核心字幕不靠它

**Tauri 2 gotchas 已套用**([[reference_tauri2_gotchas]]):
- worker 用 std::thread(不是 tokio::spawn),無 runtime context 問題
- live-segment emit + jsonl 雙路(emit 漏不影響檔案,呼應 emit-listen-race)
- capability:emit 已在 PR1 的 capabilities/default.json 給了 core:event:allow-emit

**Branch 命名** feat/* 對齊 [[mori-branch-naming]];trunk-based 短命 branch [[feedback_trunk_based_auto_merge]]。
