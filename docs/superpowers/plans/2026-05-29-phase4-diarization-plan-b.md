# Phase 4 Diarization — Plan B(sherpa-onnx 引擎 + diarize_session command + 模型 Deps)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Steps use checkbox (`- [ ]`).
>
> **⚠ 必須在真機執行**:本 plan 連結 `sherpa-onnx`(onnxruntime),且需網路抓 crates.io —— 開發 sandbox 的 crates.io 被擋(403)、且常駐/重 process 會被殺。Linux build / GPU / Windows 由執行者真機確認。

**Goal:** 把 Plan A 的 crate 無關核心接上真正的 onnx 引擎:`diarize_wav`(sherpa-onnx)+ `diarize_session` Tauri command(吃 meeting-info 人員數當 num_clusters)+ 模型下載/Deps,讓「會後對一場會跑分人 → 標回 jsonl + speakers.json」端到端可動。

**Architecture:** `diarize.rs` 既有純函式 `assign_speakers`(Plan A)之上,新增 `diarize_wav(wav, num_clusters) -> Vec<SpeakerSpan>`(包 sherpa-onnx)。`diarize_session` command 讀 meeting-info 算人員數 → 每軌 `diarize_wav` → `assign_speakers` → 寫回兩軌 jsonl(加 speaker)+ 寫 `speakers.json` → emit 進度。模型(segmentation + embedding onnx)放 `~/.mori/models/`,Deps 下載。

**Tech Stack:** Rust / Tauri 2 / `sherpa-onnx` 1.13(crates.io,features `static`)/ hound / serde。

**已驗(spike,見 spec §3.7):** sherpa-onnx 對中文四人樣本給定 num_clusters=4 分得乾淨;CPU RTF 0.076(13x realtime,GPU 可選 provider=cuda);自動 threshold 易過切 → 用人員數當 num_clusters。

**真實 API(取自 crate `rust-api-examples/examples/offline_speaker_diarization.rs`):**
```rust
use sherpa_onnx::{
    FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
    OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
    SpeakerEmbeddingExtractorConfig, Wave,
};
let config = OfflineSpeakerDiarizationConfig {
    segmentation: OfflineSpeakerSegmentationModelConfig {
        pyannote: OfflineSpeakerSegmentationPyannoteModelConfig { model: Some("seg.onnx".into()) },
        ..Default::default()
    },
    embedding: SpeakerEmbeddingExtractorConfig { model: Some("emb.onnx".into()), ..Default::default() },
    clustering: FastClusteringConfig { num_clusters: 4, ..Default::default() },
    ..Default::default()
};
let sd = OfflineSpeakerDiarization::create(&config)?;     // Result
let wave = Wave::read("a.wav")?;                          // .sample_rate(), .samples() -> &[f32]
let result = sd.process(wave.samples())?;                 // result.num_speakers(), .num_segments(), .sort_by_start_time()
for s in result.sort_by_start_time() { /* s.start, s.end (f32 秒), s.speaker (i32) */ }
```

---

## File Structure

- `src-tauri/Cargo.toml` — 加 `sherpa-onnx`(Task B1)。
- `src-tauri/src/diarize.rs` — 加 `diarize_wav` + 模型路徑 helper(Task B2)。Plan A 的純函式不動。
- `src-tauri/src/main.rs` — 註冊 `diarize_session` command + `download_progress`/`gpu_status` 旁邊(Task B3)。
- `src-tauri/src/recorder.rs` 或 `src-tauri/src/session.rs`(視現況)— `diarize_session` orchestration(Task B3)。放哪以「能拿到 session 路徑 + emit」為準;若 recorder.rs 已大,開新 `src-tauri/src/postprocess.rs`。
- `src-tauri/capabilities/*.json` — 若新 command 要前端呼叫,加進 capability(Task B3)。
- Deps:`download_model` 既有機制擴充(Task B4)。

---

## Task B1: 加 sherpa-onnx 依賴 + 確認 build

**Files:** Modify `src-tauri/Cargo.toml`

- [ ] **Step 1: 加依賴**

在 `[dependencies]` 加(對齊 rust-api-examples 的用法,`static` 把 onnxruntime 靜態連進來,免 runtime .so 部署):
```toml
# 講者分離(onnx,無 Python)。static = 靜態連 onnxruntime(CPU)。GPU 之後用 provider=cuda(需 CUDA-enabled build)。
sherpa-onnx = { version = "1.13", default-features = false, features = ["static"] }
```

- [ ] **Step 2: 確認能 build(真機)**

Run: `cd src-tauri && cargo build 2>&1 | tail -20`
Expected: 編譯成功。**若 onnxruntime 連結失敗**(常見於缺 cmake / C++ toolchain):記錄錯誤,裝對應系統依賴(spike notes)。Windows 另確認 `static` vs `shared`。

- [ ] **Step 3: Commit**
```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "build(diarize): add sherpa-onnx (onnx speaker diarization, static)"
```

---

## Task B2: `diarize_wav` 引擎 + 模型路徑

**Files:** Modify `src-tauri/src/diarize.rs`;Test: 同檔 `#[cfg(test)]`

- [ ] **Step 1: 模型路徑 helper + 失敗測試**

加到 diarize.rs:
```rust
use std::path::{Path, PathBuf};

/// segmentation 模型路徑(下載時 rename 成這個固定名)。
pub fn seg_model_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".mori").join("models").join("pyannote-segmentation-3-0.onnx")
}
/// speaker embedding 模型路徑。
pub fn emb_model_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".mori").join("models").join("3dspeaker-eres2net-zh.onnx")
}
/// 兩個模型都在才算裝好。
pub fn diarization_models_present() -> bool {
    seg_model_path().exists() && emb_model_path().exists()
}
```
測試(純路徑,可單測):
```rust
    #[test]
    fn diar_model_paths_under_mori_models() {
        assert!(seg_model_path().to_string_lossy().contains(".mori/models") || seg_model_path().to_string_lossy().contains(".mori\\models"));
        assert!(emb_model_path().ends_with("3dspeaker-eres2net-zh.onnx"));
    }
```

- [ ] **Step 2: 跑測試確認失敗 → 實作 → 通過**

Run fail: `cd src-tauri && cargo test --release diar_model_paths` → 未定義。實作上面三個 fn → PASS。

- [ ] **Step 3: 實作 `diarize_wav`(真實 sherpa-onnx API)**

加到 diarize.rs:
```rust
/// 對單一 WAV 跑 sherpa-onnx 講者分離。`num_clusters`:Some(n>0) 用已知人數(品質最佳,
/// 來自 meeting-info 人員數);None → 自動(cluster threshold,易過/欠切,使用者改名時修)。
/// 回 SpeakerSpan(該軌 local speaker id)。模型缺 / 引擎錯 → Err(caller 視為「該軌不標」)。
pub fn diarize_wav(wav: &Path, num_clusters: Option<usize>) -> Result<Vec<SpeakerSpan>, String> {
    use sherpa_onnx::{
        FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
        OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
        SpeakerEmbeddingExtractorConfig, Wave,
    };
    let seg = seg_model_path();
    let emb = emb_model_path();
    if !seg.exists() || !emb.exists() {
        return Err("diarization models not installed".to_string());
    }
    // num_clusters>0 用已知人數;否則 -1 = 交給 threshold(起點 0.7,spike 觀察 0.5 過切)。
    let clustering = match num_clusters {
        Some(n) if n > 0 => FastClusteringConfig { num_clusters: n as i32, ..Default::default() },
        _ => FastClusteringConfig { num_clusters: -1, threshold: 0.7, ..Default::default() },
    };
    let config = OfflineSpeakerDiarizationConfig {
        segmentation: OfflineSpeakerSegmentationModelConfig {
            pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                model: Some(seg.to_string_lossy().to_string()),
            },
            ..Default::default()
        },
        embedding: SpeakerEmbeddingExtractorConfig {
            model: Some(emb.to_string_lossy().to_string()),
            ..Default::default()
        },
        clustering,
        ..Default::default()
    };
    let sd = OfflineSpeakerDiarization::create(&config).map_err(|e| format!("create diarizer: {e:?}"))?;
    let wave = Wave::read(&wav.to_string_lossy()).map_err(|e| format!("read wave {}: {e:?}", wav.display()))?;
    if sd.sample_rate() != wave.sample_rate() {
        return Err(format!("sample-rate mismatch: model {} vs wav {}", sd.sample_rate(), wave.sample_rate()));
    }
    let result = sd.process(wave.samples()).map_err(|e| format!("diarize: {e:?}"))?;
    let spans = result
        .sort_by_start_time()
        .into_iter()
        .map(|s| SpeakerSpan {
            start_ms: (s.start.max(0.0) * 1000.0) as u64,
            end_ms: (s.end.max(0.0) * 1000.0) as u64,
            speaker_local: s.speaker.max(0) as usize,
        })
        .collect();
    Ok(spans)
}
```
> **真機 build 時確認(3rd-party 細節,compiler 會即時報)**:`FastClusteringConfig` 的第二欄是否叫 `threshold`(否則照編譯錯誤改名);`Wave::read` 參數型別(`&str` vs `&Path`);`s.speaker` 整數型別。其餘照 example 一字不差。

- [ ] **Step 4: #[ignore] 整合測試(需模型,真機手動跑)**
```rust
    /// 需 ~/.mori/models 的兩個 diar 模型 + 一個多人 wav。手動:
    ///   DIAR_WAV=/path/0-four-speakers-zh.wav cargo test --release diarize_wav_real -- --ignored --nocapture
    #[test]
    #[ignore]
    fn diarize_wav_real() {
        let wav = std::env::var("DIAR_WAV").expect("set DIAR_WAV");
        let spans = diarize_wav(std::path::Path::new(&wav), Some(4)).expect("diarize");
        eprintln!("got {} spans", spans.len());
        assert!(!spans.is_empty());
        let speakers: std::collections::BTreeSet<usize> = spans.iter().map(|s| s.speaker_local).collect();
        assert!(speakers.len() >= 2, "expected ≥2 speakers, got {}", speakers.len());
    }
```

- [ ] **Step 5: verify + Commit**

Run: `cd .. && bash scripts/verify.sh`(`cargo test` 不含 #[ignore],仍須綠)。
```bash
git add src-tauri/src/diarize.rs
git commit -m "feat(diarize): diarize_wav engine via sherpa-onnx (num_clusters from caller, graceful on missing models)"
```

---

## Task B3: `diarize_session` command(串接 + 人員數→num_clusters)

**Files:** Create/Modify orchestration in `src-tauri/src/recorder.rs`(或新 `postprocess.rs`);Modify `src-tauri/src/main.rs`(註冊 command);Modify capability json。Test: 對「人員字串→人數」純函式單測。

- [ ] **Step 1: 人員字串→人數 純函式 + 失敗測試**

meeting-info 的 `participants` 是使用者填的字串(逗號/頓號/換行分隔)。在 diarize.rs 加:
```rust
/// 從 meeting-info 的人員字串數人數(逗號 , 、頓號 、 、分號 ; 、換行 皆分隔);空 → None。
pub fn participant_count(participants: &str) -> Option<usize> {
    let n = participants
        .split(|c| c == ',' || c == '、' || c == ';' || c == '\n' || c == '，')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .count();
    if n > 0 { Some(n) } else { None }
}
```
測試:
```rust
    #[test]
    fn participant_count_counts_names() {
        assert_eq!(participant_count("亞澤, 老闆、阿明\n小美"), Some(4));
        assert_eq!(participant_count("  "), None);
        assert_eq!(participant_count(""), None);
        assert_eq!(participant_count("只有我"), Some(1));
    }
```
跑失敗 → 實作 → 通過。

- [ ] **Step 2: `diarize_session` orchestration**

在能拿到 session 路徑的地方(參考 `recorder.rs` 用 `SessionStore` / `session_store::default_meetings_dir()` 組 `<meetings>/<session_id>/`;音檔 `audio/system.wav`、`audio/mic-internal.wav`;逐字稿 `transcript/system.segments.jsonl`、`transcript/mic-internal.segments.jsonl`;`meeting-info.json`)實作:

```rust
use crate::transcribe::{read_segments_jsonl, append_segments_jsonl, Segment};
use crate::diarize::{diarize_wav, assign_speakers, participant_count, write_speakers, TrackDiarization, diarization_models_present};

#[derive(serde::Serialize)]
pub struct DiarizeSummary { pub num_speakers: usize, pub num_segments: usize }

/// 對一場 session 跑分人:讀人員數→num_clusters、每軌 diarize_wav + assign_speakers、
/// 把 speaker 標回兩軌 jsonl(覆寫)+ 寫 speakers.json。模型缺 → Err。
pub fn diarize_session_inner(session_root: &std::path::Path, num_clusters: Option<usize>) -> Result<DiarizeSummary, String> {
    if !diarization_models_present() {
        return Err("diarization models not installed".to_string());
    }
    let tracks_meta = [
        ("system", "transcript/system.segments.jsonl", "audio/system.wav"),
        ("mic-internal", "transcript/mic-internal.segments.jsonl", "audio/mic-internal.wav"),
    ];
    let mut tds: Vec<TrackDiarization> = Vec::new();
    for (track, jsonl_rel, wav_rel) in tracks_meta {
        let jsonl = session_root.join(jsonl_rel);
        let wav = session_root.join(wav_rel);
        let segments = read_segments_jsonl(&jsonl);
        if segments.is_empty() || !wav.exists() {
            continue; // 空軌跳過
        }
        let spans = diarize_wav(&wav, num_clusters).unwrap_or_default(); // 該軌引擎失敗 → 不標(空 spans)
        tds.push(TrackDiarization { track: track.to_string(), spans, segments });
    }
    let (labeled, speakers) = assign_speakers(tds);
    // 標回各軌 jsonl(覆寫:同 track 的 labeled segments 重寫該檔)
    for (track, jsonl_rel, _wav) in tracks_meta {
        let jsonl = session_root.join(jsonl_rel);
        let track_segs: Vec<Segment> = labeled.iter().filter(|s| s.track == track).cloned().collect();
        if track_segs.is_empty() { continue; }
        let _ = std::fs::remove_file(&jsonl); // 覆寫:刪後重 append(append_segments_jsonl 是 append 語意)
        append_segments_jsonl(&jsonl, &track_segs)?;
    }
    write_speakers(&session_root.join("transcript").join("speakers.json"), &speakers)?;
    Ok(DiarizeSummary { num_speakers: speakers.len(), num_segments: labeled.len() })
}
```
> 注意 `Segment.track` 值:Plan A 的 `assign_speakers` 用傳入的 `TrackDiarization.track` 字串編號,但回傳的 `Segment.track` 是 segment 自己的(`"system"` / `"mic-internal"`,見 `SourceKind::track_name()`)。上面 filter 用 `s.track == track` 需與 segment 實際 track 值一致(`"system"` / `"mic-internal"`)—— 實作時確認 jsonl 內 `track` 欄位值,必要時改 filter 條件。

- [ ] **Step 3: Tauri command + 註冊 + capability**

```rust
#[tauri::command]
fn diarize_session(session_id: String) -> Result<crate::recorder::DiarizeSummary, String> {
    let root = crate::session_store::default_meetings_dir().join(&session_id);
    // 讀 meeting-info 人員數
    let info = std::fs::read_to_string(root.join("meeting-info.json")).ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    let participants = info.as_ref().and_then(|v| v.get("participants")).and_then(|p| p.as_str()).unwrap_or("");
    let num_clusters = crate::diarize::participant_count(participants);
    crate::recorder::diarize_session_inner(&root, num_clusters)
}
```
在 `main.rs` 的 `tauri::generate_handler![...]` 加入 `diarize_session`。若前端要呼叫,於對應 `capabilities/*.json` 的 permissions 加 `"core:event:default"` 已有則僅需把 command 放進 handler(Tauri 2 command 預設可被前端 invoke,除非有 capability 限制 —— 對齊現有 `download_model` 等 command 的註冊方式)。背景跑(慢):command 內用 `tauri::async_runtime::spawn_blocking` 包 `diarize_session_inner`,或前端接受同步等待 + 轉圈;**對齊現有耗時 command 的做法**。

- [ ] **Step 4: 跑純函式測試 + verify + Commit**

Run: `cd src-tauri && cargo test --release participant_count && cd .. && bash scripts/verify.sh`
```bash
git add src-tauri/src/recorder.rs src-tauri/src/diarize.rs src-tauri/src/main.rs src-tauri/capabilities/*.json
git commit -m "feat(diarize): diarize_session command — participant-count→num_clusters, label both tracks + write speakers.json"
```

---

## Task B4: 模型下載 + Deps 顯示

**Files:** Modify Deps 下載機制(對齊既有 `download_model` / `DepsTab.tsx`);Modify install 腳本。

- [ ] **Step 1: 下載 + 安裝兩個模型到固定路徑**

新增一個下載動作(命令或擴充 `download_model`),抓並 rename 到 `seg_model_path()` / `emb_model_path()`:
- segmentation:`https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2` → 解開取 `model.onnx` → 存成 `~/.mori/models/pyannote-segmentation-3-0.onnx`。
- embedding:`https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx` → 存成 `~/.mori/models/3dspeaker-eres2net-zh.onnx`。

對齊既有 whisper 模型下載的 progress / 寫檔模式(`download_model` / `download_progress` statics,見 `main.rs`)。

- [ ] **Step 2: Deps 分頁顯示「講者分離模型」狀態 + 下載鈕**

`DepsTab.tsx` 加一個區塊:呼 `diarization_models_present`(新加一個輕量 query command 回 bool,或併進既有 deps_check 回傳)顯示「已安裝 / 未安裝」+ 下載按鈕 + 進度條。對齊既有 deps 區塊樣式(沿用 `.mori-*` class,別寫死 rgba)。

- [ ] **Step 3: install 腳本帶上(可選一鍵)**

`scripts/install-whisper-linux.sh` / `.ps1` 末尾加可選步驟下載這兩個 diar 模型到 `~/.mori/models/`(與 whisper 模型同處)。

- [ ] **Step 4: verify + Commit**
```bash
bash scripts/verify.sh
git add -A && git commit -m "feat(diarize): download + Deps status for diarization onnx models"
```

---

## Self-Review

- **Spec coverage**:引擎 `diarize_wav`(B2,真實 sherpa-onnx API,num_clusters)✓;`diarize_session` 串接 + 人員數→num_clusters + 標回 jsonl + speakers.json(B3)✓;模型 Deps/下載(B4)✓;GPU = provider(spec §3.7,v1 用 CPU,B 不強制 GPU — 已足夠)✓;graceful 模型缺(B2/B3 Err)✓。assign_speakers / speakers.json / Segment 欄位 / 匯出前綴 = Plan A(已 merged #62)。
- **Placeholder scan**:無 TODO/TBD。三處標明「真機 build 時依 compiler 確認 3rd-party 欄位名」= 第三方 API 細節(非自家邏輯空白),compiler 即時報、example 已給最可能寫法。Tauri command 註冊/capability/Deps UI 標明「對齊既有 download_model 模式」= 既有 codebase 模式(skill 允許)。
- **Type consistency**:`SpeakerSpan` / `SpeakerInfo` / `TrackDiarization` / `assign_speakers` / `write_speakers` 皆 Plan A 已定;`diarize_wav(&Path, Option<usize>) -> Result<Vec<SpeakerSpan>,String>` 與 B3 呼叫一致;`participant_count(&str)->Option<usize>` 與 B3 一致;`Segment.track` 過濾的注意已標。
- **真機限制**:全 plan 不可在開發 sandbox build(crates.io 擋 + onnxruntime),必須真機執行 + 確認 build/GPU/Windows。
