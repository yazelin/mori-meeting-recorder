# Phase 4 — Plan C(會後處理工作區)+ B4(模型下載 UX)

> **For agentic workers:** subagent-driven。守 `bash scripts/verify.sh` 綠;短命 branch → PR → squash;commit 尾 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。前端沿用既有 `.mori-*` / `var(--*)` theme,別寫死顏色。UI 無單測,`npm run build` 為 gate。

**Goal:** 讓講者分離從頭到尾在 UI 跑得起來:Deps 能下載分人模型(B4);Sessions 分頁點一場會進工作區 → 跑分人 → 從人員清單改名 → 帶講者前綴重匯出。

**Architecture:** 後端補幾個薄 Tauri command(包已測的 diarize/postprocess/exporter/diarize::speakers)+ B4 模型下載(沿用 `download_model`/`download_progress` 機制)。前端:DepsTab 加分人模型區塊;SessionsTab 從「列表+開資料夾」升級成「列表 → 點進工作區」。

**已具備:** `diarize_session`(#66)、`diarize::{read_speakers,rename_speaker,write_speakers,participant_count}`、`exporter::export(segs, meta, speakers)`、`list_sessions_detailed`/`open_session_dir`/`read_session_summary`、`download_model`/`download_progress`(whisper 模型下載範式)。

---

## File Structure
- `src-tauri/src/main.rs` — 新 command:`download_diar_models`、`diar_models_present`、`read_speakers_cmd`、`rename_speaker_cmd`、`read_meeting_info`、`set_meeting_info_for`、`reexport_session`、`read_session_transcript`。註冊進 handler。
- `src-tauri/src/postprocess.rs` 或 `session_store.rs` — `reexport_session` / transcript 讀取的 helper(純檔案,可測)。
- `src/tabs/DepsTab.tsx` — 分人模型區塊(B4 UI)。
- `src/tabs/SessionsTab.tsx` + 新 `src/tabs/SessionWorkspace.tsx`(或 components/)— 工作區。
- `src/components/MeetingCard.tsx` — onOpen 改成進工作區(或加「整理」鈕)。
- locale(若有 i18n key)。

---

## Task C1（B4）：下載分人模型 + Deps 狀態

**Files:** `src-tauri/src/main.rs`(command)、`src/tabs/DepsTab.tsx`

- [ ] **Step 1: `diar_models_present` command**
```rust
#[tauri::command]
fn diar_models_present() -> bool {
    crate::diarize::diarization_models_present()
}
```

- [ ] **Step 2: `download_diar_models` command(沿用 download_progress 機制)**

對齊既有 `download_model`(看 main.rs 的 `DL_ACTIVE`/`DL_TOTAL` statics + `download_progress`)。下載兩個檔到 `diarize::seg_model_path()` / `emb_model_path()`:
- seg:`https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2` → 解 tar.bz2 取內部 `model.onnx` → 存成 `seg_model_path()`。
- emb:`https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx` → 存成 `emb_model_path()`。
用 `ureq`(已是依賴)下載;tar.bz2 解壓:加 `bzip2`/`tar` crate 或呼叫系統 `tar`(對齊 repo 既有做法 —— 若 install 腳本用系統 tar,command 可 `std::process::Command::new("tar")`;Windows 無 tar 時退用 crate)。寫到 tmp 再 rename。回 `Result<(), String>`,進度走既有 `download_progress`。

- [ ] **Step 3: DepsTab 加「講者分離模型」區塊**

`DepsTab.tsx`:呼 `diar_models_present` 顯示 已安裝/未安裝;未安裝給「下載」鈕呼 `download_diar_models` + 既有進度條。沿用既有 deps 區塊樣式（`.mori-*`）。

- [ ] **Step 4: verify + commit**(`bash scripts/verify.sh`)

---

## Task C2：工作區後端 command

**Files:** `src-tauri/src/main.rs`（commands）；`src-tauri/src/postprocess.rs`（reexport/transcript helper + 純測）

- [ ] **Step 1: helper（純檔案,可測）— 讀 transcript + reexport**

在 postprocess.rs:
```rust
/// 讀一場 session 兩軌 jsonl 合併(依 start_ms 排序),給工作區顯示。
pub fn read_session_segments(session_root: &std::path::Path) -> Vec<crate::transcribe::Segment> {
    let mut all = crate::transcribe::read_segments_jsonl(&session_root.join("transcript/system.segments.jsonl"));
    all.extend(crate::transcribe::read_segments_jsonl(&session_root.join("transcript/mic-internal.segments.jsonl")));
    all.sort_by_key(|s| s.start_ms);
    all
}

/// 用目前 jsonl(已含 speaker)+ speakers.json 重新匯出 meeting.public/internal.md。
/// meta 由 timeline.json 還原(就是序列化過的 SessionMeta)。
pub fn reexport_session(session_root: &std::path::Path) -> Result<(), String> {
    let segs = read_session_segments(session_root);
    let speakers = crate::diarize::read_speakers(&session_root.join("transcript/speakers.json"));
    let meta_json = std::fs::read_to_string(session_root.join("timeline.json"))
        .map_err(|e| format!("read timeline.json: {e}"))?;
    let meta: crate::exporter::SessionMeta = serde_json::from_str(&meta_json)
        .map_err(|e| format!("parse timeline.json: {e}"))?;
    let (pub_md, int_md, timeline) = crate::exporter::export(&segs, &meta, &speakers)?;
    std::fs::write(session_root.join("meeting.public.md"), pub_md).map_err(|e| format!("write public: {e}"))?;
    std::fs::write(session_root.join("meeting.internal.md"), int_md).map_err(|e| format!("write internal: {e}"))?;
    std::fs::write(session_root.join("timeline.json"), timeline).map_err(|e| format!("write timeline: {e}"))?;
    Ok(())
}
```
> 確認 `SessionMeta` 有 `#[derive(Deserialize)]`(目前只有 `Serialize`)→ **Task C2 需給 `SessionMeta`/`TrackMeta`/`Exports` 加 `Deserialize`**(timeline.json 還原用)。加上去。
> 純測:temp dir 寫假 timeline.json + 兩軌 jsonl(其中含 speaker)+ speakers.json → `reexport_session` → 讀回 public.md 確認含講者前綴、internal.md 含 mic。

- [ ] **Step 2: commands**
```rust
#[tauri::command]
fn read_session_transcript(session_id: String) -> Vec<crate::transcribe::Segment> {
    crate::postprocess::read_session_segments(&crate::session_store::default_meetings_dir().join(&session_id))
}
#[tauri::command]
fn read_speakers_cmd(session_id: String) -> Vec<crate::diarize::SpeakerInfo> {
    crate::diarize::read_speakers(&crate::session_store::default_meetings_dir().join(&session_id).join("transcript/speakers.json"))
}
#[tauri::command]
fn rename_speaker_cmd(session_id: String, id: String, display: String) -> Result<(), String> {
    crate::diarize::rename_speaker(&crate::session_store::default_meetings_dir().join(&session_id).join("transcript/speakers.json"), &id, &display)
}
#[tauri::command]
fn read_meeting_info(session_id: String) -> serde_json::Value {
    let p = crate::session_store::default_meetings_dir().join(&session_id).join("meeting-info.json");
    std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({"topic":"","participants":""}))
}
#[tauri::command]
fn set_meeting_info_for(session_id: String, topic: String, participants: String) -> Result<(), String> {
    let root = crate::session_store::default_meetings_dir().join(&session_id);
    let body = serde_json::to_string_pretty(&serde_json::json!({"topic":topic,"participants":participants})).map_err(|e| e.to_string())?;
    std::fs::write(root.join("meeting-info.json"), body).map_err(|e| format!("write meeting-info: {e}"))
}
#[tauri::command]
async fn reexport_session(session_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::postprocess::reexport_session(&crate::session_store::default_meetings_dir().join(&session_id))
    }).await.map_err(|e| format!("join: {e}"))?
}
```
全部註冊進 `generate_handler!`。

- [ ] **Step 3: verify + commit**

---

## Task C3：SessionsTab → 工作區 UI

**Files:** `src/tabs/SessionsTab.tsx`、新 `src/tabs/SessionWorkspace.tsx`、`src/components/MeetingCard.tsx`

- [ ] **Step 1: SessionsTab 加「選中一場 → 顯示工作區」狀態**

`SessionsTab` 加 `const [openId, setOpenId] = useState<string|null>(null)`。MeetingCard 點擊(或加「整理」鈕)→ `setOpenId(s.id)`。openId 有值時 render `<SessionWorkspace sessionId={openId} onBack={()=>setOpenId(null)} />`,否則 render 既有列表。保留 `open_session_dir`(開資料夾)當次要動作。

- [ ] **Step 2: `SessionWorkspace.tsx`**

載入:`read_meeting_info` / `read_speakers_cmd` / `read_session_transcript`(invoke,注意 camelCase `sessionId`)。畫面(沿用 `.mori-tab*` / `var(--*)`):
1. 返回鈕 + 標題(meeting 時間)。
2. **主題 / 人員**輸入框(預填 read_meeting_info),「儲存」→ `set_meeting_info_for`。
3. **「分人」鈕** → `diarize_session({sessionId})`;先檢查 `diar_models_present`,沒裝就提示去 Deps 下載。跑完重載 speakers + transcript。轉圈/disable 防重複點。
4. **講者清單**:每個 `SpeakerInfo` 一列,顯示 `id` + 一個可編輯的顯示名(預設值,或下拉用「人員」字串拆出的名字當選項)。改 → `rename_speaker_cmd` → 重載。
5. **逐字稿**:`read_session_transcript` 的段,每段顯示 `[時間] 講者顯示名: text`(speaker→display 用 speakers map;`speaker_mixed` 的段標個記號提示「可能多位講者」)。
6. **「重新匯出」鈕** → `reexport_session` → toast 成功。

- [ ] **Step 3: i18n key（若 repo 有 i18n,沿用既有 namespace；proper noun 不進 locale）**

- [ ] **Step 4: `npm run build` + verify + commit**

---

## Self-Review
- **Spec coverage**:B4 下載+Deps(C1)；工作區命令 read/rename/info/reexport/transcript(C2)；工作區 UI 跑分人+改名+重匯出(C3)。對齊 spec §3.1(Sessions→工作區)/§3.3(人員清單餵改名)/§3.6(匯出前綴)。
- **依賴前置**:C2 需給 `SessionMeta` 等加 `Deserialize`(timeline.json 還原)——已在 C2 Step1 標明。
- **無 placeholder**:command 程式碼齊;UI 結構+行為齊,Tauri 註冊/theme 沿用既有 pattern(skill 允許)。tar.bz2 解壓方式給了兩條路(系統 tar / crate)依 repo 既有慣例選。
- **Type consistency**:`SpeakerInfo`/`Segment`/`SessionMeta`/`read_speakers`/`rename_speaker`/`export` 皆既有;新 command 薄包一致。
