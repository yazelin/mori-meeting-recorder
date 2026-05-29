# 聲紋註冊 + 認人 Implementation Plan(v1, recorder)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use `- [ ]`.
> **以 sonnet 跑**(opus classifier 可能不穩);沿用既有原子寫 / Segment.speaker / speakers.json / diarize。守 `bash scripts/verify.sh` 綠、短命 branch→PR→squash、commit 尾 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。

**Goal:** recorder 能註冊多人聲紋(可攜 `~/.mori` 庫,帶模型標記)+ 分人時逐群比對 → 自動標真名。

**Architecture:** `voiceprint.rs` = registry(serde,model-tagged)+ 純比對(cosine / best_match,可單測)+ embedding 抽取(sherpa-onnx `SpeakerEmbeddingExtractor`,沿用分人那顆 3D-Speaker,無 Python)。commands 管註冊/列表/刪/改名。diarize_session 比對每群 → 命中改 speakers.json display。recorder 新「人員」分頁錄音 + 名單。

**真實 sherpa-onnx API(已抓):**
```rust
use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};
let config = SpeakerEmbeddingExtractorConfig { model: Some(path), num_threads: 1, debug: false, provider: Some("cpu".into()) };
let extractor = SpeakerEmbeddingExtractor::create(&config) /* Option */;
let stream = extractor.create_stream() /* Option */;
stream.accept_waveform(sample_rate, &samples_f32);
stream.input_finished();
let emb: Option<Vec<f32>> = extractor.compute(&stream);
let dim = extractor.dim();
```
> build 時若 create/create_stream/compute 是 `Result` 而非 `Option`,照 compiler 改 `.ok_or_else`/`?`(diarize 那邊是 Option,大概率一致)。

---

## File Structure
- `src-tauri/src/voiceprint.rs`(新):registry 型別/IO + `cosine`/`best_match`(純)+ `embed_samples`/`embed_wav_file`(sherpa)+ enroll/list/remove helper。
- `src-tauri/src/main.rs`:commands `enroll_voice_start` / `enroll_voice_finish` / `list_voiceprints` / `remove_voiceprint` / `rename_voiceprint` / `voiceprint_models_present` + 註冊 handler。
- `src-tauri/src/recorder.rs`:enroll 錄音(沿用 `VoiceCapture`,錄 mic 到 temp WAV,不轉錄)。
- `src-tauri/src/postprocess.rs`:`diarize_session_inner` 尾端加「比對命中改 display」。
- `src/tabs/PeopleTab.tsx`(新)+ `ExpandedView.tsx`(加分頁)+ i18n。

---

## Task 1: registry + 純比對(TDD 核心)

**Files:** Create `src-tauri/src/voiceprint.rs`;Modify `src-tauri/src/main.rs`(`pub mod voiceprint;`)

- [ ] **Step 1: 型別 + 純函式 + registry IO(先讓它編譯)**
```rust
//! 聲紋註冊 + 認人。registry 在 ~/.mori/voiceprints/registry.json(可攜、帶 embedding 模型標記)。
//! 嵌入用分人那顆 sherpa-onnx 3D-Speaker(無 Python);比對用自家 cosine(純、可測)。
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SUPPORTED_VOICEPRINT_VERSION: u32 = 1;
/// 目前 embedding 模型識別(= ~/.mori/models/3dspeaker-eres2net-zh.onnx)。跨機器共用聲紋必須同此值。
pub const EMB_MODEL: &str = "3dspeaker-eres2net-zh";
pub const MATCH_THRESHOLD: f32 = 0.7;
fn default_version() -> u32 { 1 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person { pub id: String, pub name: String, pub samples: Vec<Vec<f32>> }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default = "default_version")] pub contract_version: u32,
    pub embedding_model: String,
    pub people: Vec<Person>,
}

pub fn registry_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".mori").join("voiceprints").join("registry.json")
}

/// cosine 相似度(同長度;零向量回 0)。
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() { return 0.0; }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na * nb) }
}

/// query 對每個人取其 samples 的最大 cosine;最佳 ≥ threshold → Some(該人)。空/都不夠像 → None。
pub fn best_match<'a>(query: &[f32], people: &'a [Person], threshold: f32) -> Option<&'a Person> {
    people.iter()
        .map(|p| (p, p.samples.iter().map(|e| cosine(query, e)).fold(f32::MIN, f32::max)))
        .filter(|(_, s)| *s >= threshold)
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(p, _)| p)
}

/// 讀 registry;缺/壞/版本太新/模型不符 → None(視為無註冊)。
pub fn read_registry() -> Option<Registry> {
    let s = std::fs::read_to_string(registry_path()).ok()?;
    let reg: Registry = serde_json::from_str(&s).ok()?;
    if reg.contract_version > SUPPORTED_VOICEPRINT_VERSION { return None; }
    if reg.embedding_model != EMB_MODEL {
        eprintln!("[voiceprint] registry model {} != {EMB_MODEL} — ignoring", reg.embedding_model);
        return None;
    }
    Some(reg)
}

/// 讀回供「寫入」用(不做模型 gate,讓 enroll 能初始化/沿用)。缺 → 新的空 registry(本機模型)。
pub fn load_or_new() -> Registry {
    std::fs::read_to_string(registry_path()).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| Registry { contract_version: 1, embedding_model: EMB_MODEL.to_string(), people: vec![] })
}

pub fn write_registry(reg: &Registry) -> Result<(), String> {
    let path = registry_path();
    if let Some(p) = path.parent() { std::fs::create_dir_all(p).map_err(|e| format!("mkdir: {e}"))?; }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(reg).map_err(|e| e.to_string())?).map_err(|e| format!("write tmp: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))
}
```
`main.rs` 加 `pub mod voiceprint;`。

- [ ] **Step 2: 失敗測試 → 跑失敗**
```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn person(id: &str, name: &str, samples: Vec<Vec<f32>>) -> Person { Person { id: id.into(), name: name.into(), samples } }

    #[test]
    fn cosine_basic() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0); // 長度不符
    }

    #[test]
    fn best_match_picks_nearest_above_threshold() {
        let people = vec![
            person("p1", "亞澤", vec![vec![1.0, 0.0]]),
            person("p2", "老闆", vec![vec![0.0, 1.0]]),
        ];
        assert_eq!(best_match(&[0.95, 0.05], &people, 0.7).map(|p| p.name.as_str()), Some("亞澤"));
        assert!(best_match(&[0.7, 0.7], &people, 0.99).is_none()); // 都不夠像
        assert!(best_match(&[1.0, 0.0], &[], 0.7).is_none());      // 空名單
    }

    #[test]
    fn registry_round_trip_and_gates() {
        // (用 load_or_new/parse 純邏輯;registry_path 真檔 IO 不在純測)
        let reg = Registry { contract_version: 1, embedding_model: EMB_MODEL.into(), people: vec![person("p1","甲",vec![vec![0.1,0.2]])] };
        let s = serde_json::to_string(&reg).unwrap();
        let back: Registry = serde_json::from_str(&s).unwrap();
        assert_eq!(back.people[0].name, "甲");
        // 版本太新 / 模型不符 由 read_registry gate(見實作);此處驗 parse 不爆
        let newer = r#"{"contract_version":2,"embedding_model":"x","people":[]}"#;
        let _: Registry = serde_json::from_str(newer).unwrap();
    }
}
```
Run: `cd src-tauri && cargo test --release voiceprint` → 先失敗(fn 未定義)。

- [ ] **Step 3: 實作即 Step 1 的內容 → 跑通過**

Run: `cargo test --release voiceprint` → PASS。

- [ ] **Step 4: Commit**
```bash
git add src-tauri/src/voiceprint.rs src-tauri/src/main.rs
git commit -m "feat(voiceprint): registry + cosine/best_match pure core (TDD)"
```

---

## Task 2: embedding 抽取(sherpa-onnx)

**Files:** Modify `src-tauri/src/voiceprint.rs`

- [ ] **Step 1: 實作 `embed_samples` / `embed_wav_file`**
```rust
/// 從 f32 樣本算聲紋向量(sherpa-onnx 3D-Speaker,= 分人那顆 emb 模型)。模型缺 → Err。
pub fn embed_samples(samples: &[f32], sample_rate: u32) -> Result<Vec<f32>, String> {
    use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};
    let model = crate::diarize::emb_model_path();
    if !model.exists() { return Err("embedding model not installed".into()); }
    let config = SpeakerEmbeddingExtractorConfig {
        model: Some(model.to_string_lossy().to_string()),
        num_threads: 1, debug: false, provider: Some("cpu".to_string()),
    };
    let extractor = SpeakerEmbeddingExtractor::create(&config).ok_or("create extractor")?;
    let stream = extractor.create_stream().ok_or("create stream")?;
    stream.accept_waveform(sample_rate, samples);
    stream.input_finished();
    extractor.compute(&stream).ok_or_else(|| "compute embedding failed".to_string())
}

/// 讀整個 WAV 算聲紋(註冊用;16k mono)。
pub fn embed_wav_file(path: &std::path::Path) -> Result<Vec<f32>, String> {
    use sherpa_onnx::Wave;
    let wave = Wave::read(&path.to_string_lossy()).ok_or_else(|| format!("read wave {}", path.display()))?;
    embed_samples(wave.samples(), wave.sample_rate())
}
```
> build 時依 compiler 確認 `create`/`create_stream`/`compute`/`Wave::read` 是 Option(用 `.ok_or`)還是 Result(用 `?` + map_err);其餘照 API。

- [ ] **Step 2: #[ignore] 整合測試**
```rust
    #[test]
    #[ignore]
    fn embed_wav_real() {
        // EMB_WAV=/path/to.wav cargo test --release embed_wav_real -- --ignored --nocapture
        let p = std::env::var("EMB_WAV").expect("set EMB_WAV");
        let e = embed_wav_file(std::path::Path::new(&p)).expect("embed");
        assert!(!e.is_empty());
        eprintln!("dim={}", e.len());
    }
```

- [ ] **Step 3: verify + Commit**
```bash
cd .. && bash scripts/verify.sh
git add src-tauri/src/voiceprint.rs
git commit -m "feat(voiceprint): embed_samples/embed_wav_file via sherpa-onnx extractor"
```

---

## Task 3: enroll 錄音 + commands

**Files:** Modify `src-tauri/src/recorder.rs`(enroll 錄音)、`src-tauri/src/main.rs`(commands + handler)

- [ ] **Step 1: recorder enroll 錄音(沿用 VoiceCapture,不轉錄)**

`recorder.rs` 已有 `VoiceCapture`(mic→temp WAV)+ `voice_input_start`/`voice_input_stop`。加一對「只錄不轉」的:`enroll_record_start(&self)`(等同 voice_input_start,錄到 `~/.mori/voiceprints/enroll-temp.wav` 或 temp_dir 的固定名)、`enroll_record_stop(&self) -> Result<PathBuf,String>`(停 capture、回 temp WAV 路徑、**不刪不轉錄**)。可直接複用 voice_input 的 VoiceCapture 欄位(若 voice 與 enroll 不會同時,共用 `self.voice`;否則加 `self.enroll`)。實作對齊既有 voice_input_start/stop。

- [ ] **Step 2: commands**
```rust
#[tauri::command] fn voiceprint_models_present() -> bool { crate::diarize::emb_model_path().exists() }

#[tauri::command] fn enroll_voice_start() -> Result<(), String> { crate::recorder::instance().enroll_record_start() }

/// 停止錄音 → 嵌入 temp WAV → 累加進 registry 的該 name(沒有就新建)→ 清 temp。
#[tauri::command]
fn enroll_voice_finish(name: String) -> Result<(), String> {
    let wav = crate::recorder::instance().enroll_record_stop()?;
    let emb = crate::voiceprint::embed_wav_file(&wav)?;
    let _ = std::fs::remove_file(&wav);
    let mut reg = crate::voiceprint::load_or_new();
    match reg.people.iter_mut().find(|p| p.name == name) {
        Some(p) => p.samples.push(emb),
        None => reg.people.push(crate::voiceprint::Person {
            id: format!("p{}", reg.people.len() + 1), name, samples: vec![emb],
        }),
    }
    crate::voiceprint::write_registry(&reg)
}

#[derive(serde::Serialize)] struct VoiceprintInfo { id: String, name: String, sample_count: usize }
#[tauri::command]
fn list_voiceprints() -> Vec<VoiceprintInfo> {
    crate::voiceprint::load_or_new().people.into_iter()
        .map(|p| VoiceprintInfo { id: p.id, name: p.name, sample_count: p.samples.len() })
        .collect()
}
#[tauri::command]
fn remove_voiceprint(id: String) -> Result<(), String> {
    let mut reg = crate::voiceprint::load_or_new();
    reg.people.retain(|p| p.id != id);
    crate::voiceprint::write_registry(&reg)
}
#[tauri::command]
fn rename_voiceprint(id: String, name: String) -> Result<(), String> {
    let mut reg = crate::voiceprint::load_or_new();
    if let Some(p) = reg.people.iter_mut().find(|p| p.id == id) { p.name = name; }
    crate::voiceprint::write_registry(&reg)
}
```
全部註冊進 `generate_handler!`。`VoiceprintInfo` 不回傳 embeddings(只 metadata)。

- [ ] **Step 3: verify + Commit**
```bash
cd .. && bash scripts/verify.sh
git add src-tauri/src/recorder.rs src-tauri/src/main.rs
git commit -m "feat(voiceprint): enroll record + commands (enroll/list/remove/rename/models_present)"
```

---

## Task 4: 分人時比對命中 → 自動標真名

**Files:** Modify `src-tauri/src/postprocess.rs`(`diarize_session_inner` 尾端)

- [ ] **Step 1: 比對後改 display**

在 `write_labeled_tracks` 之後、`stamp_diar_models` 附近加(best-effort,失敗不影響分人):
```rust
    // 若有可用聲紋庫(模型相符)→ 逐群比對 → 命中改 speakers.json 的 display 成真名。
    if let Some(reg) = crate::voiceprint::read_registry() {
        identify_speakers(session_root, &labeled, &reg);
    }
```
新 helper:
```rust
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
            Some(s) => match crate::voiceprint::embed_samples(&s, 16_000) { Ok(e) => e, Err(_) => continue },
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

/// 讀 16k mono WAV 在 [start_ms,end_ms] 的樣本(f32)。讀不到/空 → None。
fn read_wav_slice_f32(wav: &std::path::Path, start_ms: u64, end_ms: u64) -> Option<Vec<f32>> {
    let mut r = hound::WavReader::open(wav).ok()?;
    let sr = r.spec().sample_rate as u64;
    let lo = (start_ms * sr / 1000) as usize;
    let hi = (end_ms * sr / 1000) as usize;
    let all: Vec<i16> = r.samples::<i16>().filter_map(|x| x.ok()).collect();
    if lo >= all.len() || hi <= lo { return None; }
    let hi = hi.min(all.len());
    Some(all[lo..hi].iter().map(|&v| v as f32 / 32768.0).collect())
}
```
(`hound` 已是依賴。)

- [ ] **Step 2: verify + Commit**
```bash
cd .. && bash scripts/verify.sh
git add src-tauri/src/postprocess.rs
git commit -m "feat(voiceprint): identify clusters in diarize_session — auto-name matched speakers"
```

---

## Task 5: 前端「人員」分頁

**Files:** Create `src/tabs/PeopleTab.tsx`;Modify `src/tabs/ExpandedView.tsx`(加分頁)+ i18n locales

- [ ] **Step 1: PeopleTab**

新分頁(沿用 `.mori-tab*` / `var(--*)`):
- 上方:`voiceprint_models_present` false → 提示去 Deps 下載分人模型(共用同一顆);true 才顯示錄音區。
- **錄音註冊**:名字輸入 + 「開始錄音」(`enroll_voice_start`)→ 計時(建議錄 ~30s)→「完成」(`enroll_voice_finish({ name })`)→ 重載名單。錄音中 disable 防重複。
- **名單**:`list_voiceprints()` → 每人一列(名字 + `sample_count` 樣本 + 「補錄」=同名再 enroll_voice_start/finish + 改名 `rename_voiceprint` + 刪 `remove_voiceprint`)。
- 說明文字:「錄好的人,做會議紀錄分人時會自動標上名字」。

- [ ] **Step 2: ExpandedView 加分頁**

在 `ExpandedView.tsx` 的分頁列加「人員」(對齊既有 tab 定義),render `<PeopleTab/>`。i18n key 加進 zh-TW + en(兩邊同步)。

- [ ] **Step 3: build + verify + Commit**
```bash
npm run build && bash scripts/verify.sh
git add src/tabs/PeopleTab.tsx src/tabs/ExpandedView.tsx src/i18n/locales/*.json
git commit -m "feat(voiceprint): People tab — enroll/list/rename/delete voiceprints"
```

---

## Self-Review
- **Spec coverage**:可攜 registry + 模型標記(T1)✓;embedding 用分人同模型(T2)✓;註冊累加 + 名單(T3)✓;分人逐群比對自動命名 + opt-in/graceful(T4,read_registry None→跳過)✓;UI 分頁(T5)✓;跨機器同模型硬前提(T1 `EMB_MODEL` gate)✓。NAS/AgentOS = 後續(registry 可攜已備)。
- **Placeholder scan**:無 TODO;backend 完整碼;sherpa Option/Result 在 build 時依 compiler 微調(第三方細節,已標)。前端沿用既有 tab/Select/token pattern。
- **Type consistency**:`Person{id,name,samples}` / `Registry{contract_version,embedding_model,people}` / `cosine` / `best_match` / `embed_samples`/`embed_wav_file` / `read_registry`/`load_or_new`/`write_registry` 跨 task 一致;commands camelCase(`enrollVoiceFinish({name})` 等)對應前端。
- **原子寫**:`write_registry`(tmp+rename)、`write_speakers`(既有)。
