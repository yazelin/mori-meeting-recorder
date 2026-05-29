# Phase 4 Diarization — Plan A(build spike + crate-independent core)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地講者分離的「crate 無關核心」(資料模型 + `assign_speakers` 對齊 + `speakers.json` + 匯出講者前綴),並用 build spike 選定 onnx diarization crate,為後續 engine 整合鋪路。

**Architecture:** 純函式 `assign_speakers` 把每軌 `SpeakerSpan[]` 對齊到逐字稿段(多數重疊賦值 + mixed 旗標 + 跨軌統一編號 S1..Sn),完全可單測、不依賴任何 onnx crate。資料模型在現有 `Segment` 加兩個 serde-default 欄位(文字不動)。`speakers.json` 存 id→{display,track} 供改名。匯出在現有 `render_md` 加講者前綴。引擎(`diarize_wav`)+ `diarize_session` command + 工作區 UI 由 spike 後的 Plan B/C 處理。

**Tech Stack:** Rust / Tauri 2 / serde / hound;diarization 引擎候選 speakrs / sherpa-onnx-rs / pyannote-rs(onnx,Task 1 spike 選定)。

---

## 範圍與分解(scope check)

本 spec 跨三塊:crate 無關核心(可立即 TDD)、crate 依賴引擎(需先 spike 才能寫出無 placeholder 的程式碼)、前端工作區。故拆成:

- **Plan A(本檔)**:Task 1 build spike + crate 無關核心(資料模型、`assign_speakers`、`speakers.json`、匯出前綴)。全部可現在 TDD、零 placeholder。
- **Plan B(spike 後寫)**:`diarize.rs` 的 `diarize_wav` 引擎(對著選定 crate 的真實 API)+ `diarize_session` command + 模型下載/Deps。
- **Plan C(B 後寫)**:Sessions 分頁升級成會後處理工作區(跑分人 / 改名 / 編輯主題人員 / 重匯出 / 處理 mixed 段)。

所有任務守:`bash scripts/verify.sh` 必綠;短命 branch off main → PR → squash auto-merge;commit 訊息結尾 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。

## File Structure

- `src-tauri/src/transcribe.rs` — `Segment` 加 `speaker` / `speaker_mixed` 欄位(Task 2)。
- `src-tauri/src/diarize.rs`(新)— `SpeakerSpan` / `SpeakerInfo` / `TrackDiarization` 型別 + `assign_speakers` 純函式(Task 3)+ `speakers.json` 讀寫/改名(Task 4)。**本檔 Plan A 階段不含任何 onnx crate 依賴**(引擎 fn 由 Plan B 加入)。
- `src-tauri/src/main.rs` — 註冊 `pub mod diarize;`(Task 3)。
- `src-tauri/src/exporter.rs` — `export` / `render_md` 加講者前綴(Task 5)。
- `src-tauri/src/recorder.rs` — `finalize_session` 呼叫 `export` 處補空 speakers 參數(Task 5,無行為改變)。

---

## Task 1: Build spike — 選定 onnx diarization crate

**這是調查任務,非 TDD。** 產出 = 一個決定 + 記錄,unblock Plan B。可能需要真實機器(GPU)+ 網路抓模型;本機 bash sandbox 對長駐/重 process 會以 exit 144 殺掉,必要時在真機跑。

**Files:**
- Create: `docs/superpowers/notes/2026-05-29-diarization-crate-spike.md`(記錄結果)

- [ ] **Step 1: 準備合成雙人測試 WAV**

做一個 16kHz mono 16-bit 的「兩個不同講者接續講」WAV(例:把兩段不同人的語音串起來,或兩個不同 TTS 聲音)。存 `/tmp/diar-2spk.wav`。記下每個講者大概的時間區間(ground truth)。

- [ ] **Step 2: 逐一評估三個候選 crate**

對 `speakrs`、`sherpa-onnx-rs`(k2-fsa 的 Rust API)、`pyannote-rs` 各開一個 scratch crate(或本 repo 的暫時 `examples/diar_spike.rs`),逐一:
1. `cargo add <crate>`,`cargo build` — 記錄能否在 Linux build(onnxruntime 連結是否痛)。
2. 抓該 crate 需要的 segmentation + speaker-embedding onnx 模型,放 `~/.mori/models/`。
3. 對 `/tmp/diar-2spk.wav` 跑 diarization,印出 `(start, end, speaker)` spans。
4. 記錄:能否吃 GPU(CUDA EP)、span 是否合理分出 2 人且時間對得上 ground truth、模型大小、API 易用度、跨平台(Windows)風險。

- [ ] **Step 3: 記錄決定**

在 spike notes 寫下:選哪個 crate、它的 `diarize_wav` 呼叫形狀(輸入/輸出型別)、需要的模型檔名 + 下載 URL + 大小、GPU 開關方式、Windows 風險。若三個都不行 → 記錄退方案 A(每 VAD clip 一個講者,只需 embedding 模型 + 我們自己分群)。

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/notes/2026-05-29-diarization-crate-spike.md
git commit -m "docs(diarize): build spike — pick onnx diarization crate"
```

**Gate:** Plan B 不開始,直到本 task 產出「選定的 crate + 其 API 形狀 + 模型來源」。Plan A 的 Task 2–5 不依賴本 task,可平行做。

---

## Task 2: `Segment` 加 `speaker` / `speaker_mixed` 欄位

**Files:**
- Modify: `src-tauri/src/transcribe.rs`(`Segment` struct + 所有建構處)
- Test: `src-tauri/src/transcribe.rs`(`#[cfg(test)] mod tests`)

- [ ] **Step 1: 寫失敗測試(舊 jsonl 無欄位 → 預設)**

加到 transcribe.rs 的 tests:

```rust
    #[test]
    fn segment_speaker_fields_default_when_absent() {
        // 舊 jsonl(沒有 speaker / speaker_mixed)要能反序列化,欄位回預設
        let line = r#"{"id":"s1","session_id":"x","track":"system","source_kind":"meeting_system","visibility":"public","start_ms":0,"end_ms":1000,"text":"hi","is_final":true}"#;
        let s: Segment = serde_json::from_str(line).unwrap();
        assert_eq!(s.speaker, None);
        assert!(!s.speaker_mixed);
    }
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd src-tauri && cargo test --release segment_speaker_fields_default_when_absent`
Expected: 編譯失敗(`Segment` 無 `speaker` / `speaker_mixed` 欄位)。

- [ ] **Step 3: 加欄位到 `Segment`**

在 `src-tauri/src/transcribe.rs` 的 `Segment` struct,於 `confidence` 後加:

```rust
    #[serde(default)]
    pub speaker: Option<String>,
    #[serde(default)]
    pub speaker_mixed: bool,
```

- [ ] **Step 4: 補齊所有 `Segment { .. }` 建構處**

下列每個建構 `Segment` 的地方都加 `speaker: None,` 與 `speaker_mixed: false,`:
1. `parse_whisper_json` 的 `.map(|(i, r)| Segment { .. })`。
2. `parse_server_json` 的 `Ok(Some(Segment { .. }))`。
3. tests 的 `sample_seg()` helper。

(`exporter.rs` 的 test helper `seg()` 在 Task 5 一起補。)

- [ ] **Step 5: 跑測試確認通過 + 全測試綠**

Run: `cd src-tauri && cargo test --release transcribe`
Expected: PASS(含新測試;舊測試不變)。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/transcribe.rs
git commit -m "feat(diarize): add speaker/speaker_mixed fields to Segment (serde default, back-compat)"
```

---

## Task 3: `diarize.rs` — 型別 + `assign_speakers` 純函式(核心)

**Files:**
- Create: `src-tauri/src/diarize.rs`
- Modify: `src-tauri/src/main.rs`(加 `pub mod diarize;`)
- Test: `src-tauri/src/diarize.rs`(`#[cfg(test)] mod tests`)

- [ ] **Step 1: 建檔 + 型別 + 函式骨架(先讓它編譯但邏輯空)**

Create `src-tauri/src/diarize.rs`:

```rust
//! 講者分離的 crate 無關核心:型別 + assign_speakers 對齊(純函式,可單測)。
//! 引擎(diarize_wav,依賴選定的 onnx crate)由 Plan B 於本檔加入。

use crate::transcribe::Segment;
use serde::{Deserialize, Serialize};

/// 一個講者-同質時間段(引擎輸出);speaker_local = 該軌內的本地群 id(0-based)。
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakerSpan {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker_local: usize,
}

/// 統一後的講者(跨軌 S1..Sn);display 預設「講者N」,track = 來源軌。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeakerInfo {
    pub id: String,
    pub display: String,
    pub track: String,
}

/// 一軌的 diarization 輸入:該軌名、spans、該軌 segments。
pub struct TrackDiarization {
    pub track: String,
    pub spans: Vec<SpeakerSpan>,
    pub segments: Vec<Segment>,
}

// mixed 門檻:次多講者重疊 >= min(1s, 30% 段長) → 標 speaker_mixed(取小者,短段更易被標)。
const MIXED_MIN_MS: u64 = 1000;
const MIXED_MIN_FRAC: f64 = 0.30;

/// 把每軌 spans 對齊到該軌 segments:多數重疊賦 speaker(統一 S1..Sn),次多顯著 → speaker_mixed。
/// 跨軌統一編號(一人只在一軌)。回 (標好的 segments, 講者表)。
pub fn assign_speakers(tracks: Vec<TrackDiarization>) -> (Vec<Segment>, Vec<SpeakerInfo>) {
    let _ = (MIXED_MIN_MS, MIXED_MIN_FRAC);
    let _ = tracks;
    (Vec::new(), Vec::new())
}
```

Add to `src-tauri/src/main.rs`(在其他 `pub mod` 附近):

```rust
pub mod diarize;
```

- [ ] **Step 2: 寫失敗測試(四個情境)**

加到 diarize.rs 末:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn seg(track: &str, source: &str, vis: &str, start: u64, end: u64) -> Segment {
        Segment {
            id: format!("{track}-{start}"),
            session_id: "m".into(),
            track: track.into(),
            source_kind: source.into(),
            visibility: vis.into(),
            start_ms: start,
            end_ms: end,
            text: "x".into(),
            is_final: true,
            confidence: None,
            speaker: None,
            speaker_mixed: false,
        }
    }

    #[test]
    fn single_speaker_segment_gets_label_not_mixed() {
        let td = TrackDiarization {
            track: "system".into(),
            spans: vec![SpeakerSpan { start_ms: 0, end_ms: 5000, speaker_local: 0 }],
            segments: vec![seg("system", "meeting_system", "public", 1000, 2000)],
        };
        let (segs, speakers) = assign_speakers(vec![td]);
        assert_eq!(segs[0].speaker.as_deref(), Some("S1"));
        assert!(!segs[0].speaker_mixed);
        assert_eq!(speakers, vec![SpeakerInfo { id: "S1".into(), display: "講者1".into(), track: "system".into() }]);
    }

    #[test]
    fn two_speaker_segment_is_mixed_and_takes_majority() {
        // 段 0..2000:講者0 佔 0..1600(多數)、講者1 佔 1600..2000(400ms);
        // 段長 2000 → 門檻 min(1000, 600)=600;次多 400 < 600 → 不 mixed
        let td = TrackDiarization {
            track: "system".into(),
            spans: vec![
                SpeakerSpan { start_ms: 0, end_ms: 1600, speaker_local: 0 },
                SpeakerSpan { start_ms: 1600, end_ms: 5000, speaker_local: 1 },
            ],
            segments: vec![seg("system", "meeting_system", "public", 0, 2000)],
        };
        let (segs, _) = assign_speakers(vec![td]);
        assert_eq!(segs[0].speaker.as_deref(), Some("S1")); // 多數 = local 0 = S1
        assert!(!segs[0].speaker_mixed);

        // 段 0..2000:講者0 佔 0..1000、講者1 佔 1000..2000(1000ms);門檻 min(1000,600)=600;次多 1000>=600 → mixed
        let td2 = TrackDiarization {
            track: "system".into(),
            spans: vec![
                SpeakerSpan { start_ms: 0, end_ms: 1000, speaker_local: 0 },
                SpeakerSpan { start_ms: 1000, end_ms: 5000, speaker_local: 1 },
            ],
            segments: vec![seg("system", "meeting_system", "public", 0, 2000)],
        };
        let (segs2, _) = assign_speakers(vec![td2]);
        assert!(segs2[0].speaker_mixed);
    }

    #[test]
    fn no_overlap_segment_has_no_speaker() {
        let td = TrackDiarization {
            track: "system".into(),
            spans: vec![SpeakerSpan { start_ms: 0, end_ms: 1000, speaker_local: 0 }],
            segments: vec![seg("system", "meeting_system", "public", 5000, 6000)],
        };
        let (segs, _) = assign_speakers(vec![td]);
        assert_eq!(segs[0].speaker, None);
        assert!(!segs[0].speaker_mixed);
    }

    #[test]
    fn cross_track_numbering_is_unified_and_continues() {
        let sys = TrackDiarization {
            track: "system".into(),
            spans: vec![
                SpeakerSpan { start_ms: 0, end_ms: 1000, speaker_local: 0 },
                SpeakerSpan { start_ms: 1000, end_ms: 2000, speaker_local: 1 },
            ],
            segments: vec![seg("system", "meeting_system", "public", 0, 1000)],
        };
        let mic = TrackDiarization {
            track: "mic-internal".into(),
            spans: vec![SpeakerSpan { start_ms: 0, end_ms: 1000, speaker_local: 0 }],
            segments: vec![seg("mic-internal", "mic_internal", "internal", 0, 1000)],
        };
        let (_segs, speakers) = assign_speakers(vec![sys, mic]);
        // sys 兩群 → S1,S2;mic 一群 → S3
        assert_eq!(speakers.iter().map(|s| s.id.clone()).collect::<Vec<_>>(), vec!["S1", "S2", "S3"]);
        assert_eq!(speakers[2].track, "mic-internal");
    }
}
```

- [ ] **Step 3: 跑測試確認失敗**

Run: `cd src-tauri && cargo test --release diarize`
Expected: FAIL(assign_speakers 回空)。

- [ ] **Step 4: 實作 `assign_speakers`**

把 Step 1 的 `assign_speakers` 本體換成:

```rust
pub fn assign_speakers(tracks: Vec<TrackDiarization>) -> (Vec<Segment>, Vec<SpeakerInfo>) {
    use std::collections::HashMap;
    let mut speakers: Vec<SpeakerInfo> = Vec::new();
    let mut out: Vec<Segment> = Vec::new();
    let mut next_global = 1usize;

    for td in tracks {
        // 本地 id → 全域 S{n}(本地 id 排序,決定性)
        let mut local_ids: Vec<usize> = td.spans.iter().map(|s| s.speaker_local).collect();
        local_ids.sort_unstable();
        local_ids.dedup();
        let mut map: HashMap<usize, String> = HashMap::new();
        for lid in local_ids {
            let gid = format!("S{next_global}");
            speakers.push(SpeakerInfo {
                id: gid.clone(),
                display: format!("講者{next_global}"),
                track: td.track.clone(),
            });
            map.insert(lid, gid);
            next_global += 1;
        }

        for mut s in td.segments {
            let mut overlap: HashMap<usize, u64> = HashMap::new();
            for span in &td.spans {
                let lo = s.start_ms.max(span.start_ms);
                let hi = s.end_ms.min(span.end_ms);
                if hi > lo {
                    *overlap.entry(span.speaker_local).or_insert(0) += hi - lo;
                }
            }
            if overlap.is_empty() {
                s.speaker = None;
                s.speaker_mixed = false;
            } else {
                let mut v: Vec<(usize, u64)> = overlap.into_iter().collect();
                v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                s.speaker = map.get(&v[0].0).cloned();
                let dur = s.end_ms.saturating_sub(s.start_ms).max(1);
                let threshold = ((dur as f64 * MIXED_MIN_FRAC) as u64).min(MIXED_MIN_MS);
                let second = v.get(1).map(|x| x.1).unwrap_or(0);
                s.speaker_mixed = second > 0 && second >= threshold;
            }
            out.push(s);
        }
    }
    (out, speakers)
}
```

- [ ] **Step 5: 跑測試確認通過**

Run: `cd src-tauri && cargo test --release diarize`
Expected: PASS(四個測試)。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/diarize.rs src-tauri/src/main.rs
git commit -m "feat(diarize): SpeakerSpan/SpeakerInfo + assign_speakers pure reconciliation (TDD core)"
```

---

## Task 4: `speakers.json` 讀寫 + 改名

**Files:**
- Modify: `src-tauri/src/diarize.rs`
- Test: `src-tauri/src/diarize.rs`(tests)

- [ ] **Step 1: 寫失敗測試(round-trip + rename)**

加到 diarize.rs tests:

```rust
    #[test]
    fn speakers_json_round_trip_and_rename() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("speakers.json");
        let speakers = vec![
            SpeakerInfo { id: "S1".into(), display: "講者1".into(), track: "system".into() },
            SpeakerInfo { id: "S2".into(), display: "講者2".into(), track: "mic-internal".into() },
        ];
        write_speakers(&path, &speakers).unwrap();
        let read = read_speakers(&path);
        assert_eq!(read, speakers);

        rename_speaker(&path, "S1", "亞澤").unwrap();
        let read2 = read_speakers(&path);
        assert_eq!(read2.iter().find(|s| s.id == "S1").unwrap().display, "亞澤");
        assert_eq!(read2.iter().find(|s| s.id == "S2").unwrap().display, "講者2");
    }

    #[test]
    fn read_speakers_missing_file_is_empty() {
        assert!(read_speakers(std::path::Path::new("/nonexistent/speakers.json")).is_empty());
    }
```

(確認 `src-tauri/Cargo.toml` 的 `[dev-dependencies]` 已有 `tempfile = "3"` — 已存在。)

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd src-tauri && cargo test --release speakers_json`
Expected: FAIL(`write_speakers` / `read_speakers` / `rename_speaker` 未定義)。

- [ ] **Step 3: 實作讀寫 + 改名**

加到 diarize.rs(型別區後、tests 前):

```rust
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Serialize, Deserialize)]
struct SpeakerEntry {
    display: String,
    track: String,
}

/// 寫 speakers.json:物件 id→{display,track}(BTreeMap 穩定排序),原子寫(tmp+rename)。
pub fn write_speakers(path: &Path, speakers: &[SpeakerInfo]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let map: BTreeMap<String, SpeakerEntry> = speakers
        .iter()
        .map(|s| (s.id.clone(), SpeakerEntry { display: s.display.clone(), track: s.track.clone() }))
        .collect();
    let body = serde_json::to_string_pretty(&map).map_err(|e| format!("serialize: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("write tmp: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename: {e}"))
}

/// 讀 speakers.json → Vec<SpeakerInfo>(依 id 排序)。缺檔/壞檔 → 空。
pub fn read_speakers(path: &Path) -> Vec<SpeakerInfo> {
    let s = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let map: BTreeMap<String, SpeakerEntry> = match serde_json::from_str(&s) {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };
    map.into_iter()
        .map(|(id, e)| SpeakerInfo { id, display: e.display, track: e.track })
        .collect()
}

/// 改某講者的顯示名(只動 speakers.json)。找不到 id → Err。
pub fn rename_speaker(path: &Path, id: &str, new_display: &str) -> Result<(), String> {
    let mut speakers = read_speakers(path);
    let found = speakers.iter_mut().find(|s| s.id == id);
    match found {
        Some(s) => {
            s.display = new_display.to_string();
            write_speakers(path, &speakers)
        }
        None => Err(format!("speaker id not found: {id}")),
    }
}
```

- [ ] **Step 4: 跑測試確認通過**

Run: `cd src-tauri && cargo test --release speakers`
Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/diarize.rs
git commit -m "feat(diarize): speakers.json read/write + rename (atomic, id->display/track map)"
```

---

## Task 5: 匯出加講者前綴

**Files:**
- Modify: `src-tauri/src/exporter.rs`(`export` / `render_md` 簽章 + 邏輯 + test helper)
- Modify: `src-tauri/src/recorder.rs`(`finalize_session` 呼叫 `export` 處補空 speakers)
- Test: `src-tauri/src/exporter.rs`(tests)

- [ ] **Step 1: 寫失敗測試(有/無講者前綴)**

加到 exporter.rs tests(用既有 `seg()` helper,Step 3 會幫它加 speaker 欄位):

```rust
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
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd src-tauri && cargo test --release exporter`
Expected: FAIL(`export` 簽章只有兩參數;`seg` 缺 speaker 欄位)。

- [ ] **Step 3: 改 `export` / `render_md` 簽章 + 邏輯,並補 test helper**

在 `src-tauri/src/exporter.rs`:

(a) `export` 簽章加 `speakers: &[crate::diarize::SpeakerInfo]`:

```rust
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
```

(b) `render_md` 加 `speakers` 參數 + 前綴邏輯:

```rust
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
```

(c) test helper `seg()` 補欄位(在 `confidence: None,` 後加):

```rust
            confidence: None,
            speaker: None,
            speaker_mixed: false,
```

- [ ] **Step 4: 更新 `recorder.rs` 的 `export` 呼叫(stop 時還沒分人 → 傳空)**

在 `src-tauri/src/recorder.rs` 的 `finalize_session`,把:

```rust
        let (pub_md, int_md, timeline) = export(&all_segs, &meta)?;
```

改成:

```rust
        let (pub_md, int_md, timeline) = export(&all_segs, &meta, &[])?;
```

- [ ] **Step 5: 跑測試確認通過 + 全測試綠**

Run: `cd src-tauri && cargo test --release exporter && cargo test --release`
Expected: PASS(含兩新測試;recorder/其他不變)。

- [ ] **Step 6: verify.sh + Commit**

```bash
cd .. && bash scripts/verify.sh   # 必綠
git add src-tauri/src/exporter.rs src-tauri/src/recorder.rs
git commit -m "feat(diarize): export speaker-name prefix in meeting markdown (empty = unchanged)"
```

---

## Self-Review

- **Spec coverage(Plan A 範圍)**:資料模型(Task 2 ✓)、`assign_speakers` 多數+mixed+跨軌編號(Task 3 ✓)、speakers.json + 改名(Task 4 ✓)、匯出前綴(Task 5 ✓)、build spike 選 crate(Task 1 ✓)。引擎 `diarize_wav` / `diarize_session` command / 模型 Deps / 工作區 UI = Plan B/C(spike 後),已於範圍段標明。
- **Placeholder scan**:無 TBD/TODO;每個 code step 有完整程式碼;Task 1 是調查任務(本來就無 code),產出明確(crate 決定 + API 形狀)。
- **Type consistency**:`SpeakerSpan`/`SpeakerInfo`/`TrackDiarization` 定義(Task 3)與 Task 4(`SpeakerInfo` 同型)、Task 5(`crate::diarize::SpeakerInfo`)一致;`assign_speakers` 回 `(Vec<Segment>, Vec<SpeakerInfo>)` 與 command 用法(Plan B)相容;`Segment.speaker: Option<String>` / `speaker_mixed: bool`(Task 2)被 Task 3/5 一致使用。
- **mixed 門檻** spec 與 plan 一致(`min(1s, 30%段長)`,取小者)。
