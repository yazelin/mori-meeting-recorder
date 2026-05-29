# Phase 4 — 講者分離(Speaker Diarization)+ 會後處理工作區 設計

- 日期:2026-05-29
- 狀態:設計已 co-review(brainstorming 四段全通過),待寫 implementation plan
- 範圍:mori-meeting-recorder 的「會後處理工作區」容器 + 它的第一個功能「講者分離」

## 1. 目標與背景

現況:主幹(雙軌擷取 → VAD 即時轉錄 → 匯出原稿)可動。匯出的 `meeting.public/internal.md` 是**逐字稿原稿**(時間戳 + 原始 whisper 文字),且**不分人**:

- 兩軌只是「音源」之分:`meeting_system`(遠端所有人混音 / 現場一桌人)vs `mic_internal`(本地)。
- 同一軌裡的多個講者**混在一條音訊**,whisper 只做語音→文字、不標誰講的。

目標:會議結束後,把**多個講者分開**並標到逐字稿,讓會議紀錄能呈現「誰講了什麼」(後續校正、總結、決議/待辦歸屬的基礎)。

## 2. 已拍板決策(brainstorm 結果)

| 主題 | 決策 |
|------|------|
| 時機 | **會後批次、on-demand**(不在 stop 時自動跑),當「會後處理」第一步 |
| Pipeline | **方案 B**:完整 segmentation → embedding → clustering(onnx,無 Python) |
| 命名 | **匿名 `講者N` + 事後改名**;**不自動標「你」**(只分人員) |
| 場景 | **線上 + 現場都要顧** → 兩軌都要能分人(含現場一桌人段內換人) |
| 對齊 | **方案 X**:標註現有 jsonl 主字稿,**不重新轉錄**(守「live jsonl 是主字稿」規矩) |
| 容器 | **同 app 的分頁** —— 現有 Sessions 分頁升級成「會後處理工作區」(**不**做獨立 app) |
| Session 模型 | **一段錄音 = 一場會**(暫停/繼續、多段合併留 v2) |
| 引擎/模型 | onnx crate(speakrs / sherpa-onnx-rs / pyannote-rs,plan step 0 build spike 選定);模型放 `~/.mori/models/`,GPU 走 onnxruntime CUDA EP / 否則 CPU |
| 失敗策略 | graceful:模型缺/引擎錯 → 跳過、逐字稿維持不標人、不 hard-fail(standalone-first) |

與剛落地的共享 whisper-server 同哲學:本地共享模型(`~/.mori`)、可選 backend、graceful、standalone-first。

## 3. 架構

### 3.1 容器:Sessions 分頁 → 會後處理工作區

Sessions 分頁從「列表」升級為工作區入口:列出歷史會議 → 點一場進入該場的處理畫面(diarization → 改名 → [未來:校正 → 總結] → 匯出)。所有功能吃**同一份** `~/.mori/meetings/<session>/` 資料 + 同一套模型,不另開 app。

一段錄音(start→stop)= 一個 session = 一場會。多段合併 / 暫停繼續為 v2。

### 3.2 Diarization pipeline(每軌一次)

1. 對每軌非空 WAV(`audio/system.wav` / `audio/mic-internal.wav`)跑 onnx diarization 引擎 → `SpeakerSpan { start_ms, end_ms, speaker_local }[]`(含段內換人、重疊處理)。
2. **跨軌統一編號**:一個人只出現在一軌(遠端在 sys、本地在 mic),直接接續編號:sys 群 → `S1..Sk`、mic 群 → `S(k+1)..Sn`,成一份統一講者清單。
3. **對齊(方案 X,不重轉)**:把講者時間軸套到現有 `transcript/*.segments.jsonl`,每段以**重疊時間最多**的講者賦值(`speaker=Sx`)。一段橫跨 ≥2 講者(段內換人)且**次多講者的重疊顯著**(門檻:次多講者重疊 > 1s 或 > 該段 30%,取小者)→ 仍給多數者,但標 `speaker_mixed=true` 供手動切。VAD 多在停頓處切,故多數情況 clip 邊界已對齊換人,重疊為例外。

### 3.3 資料模型(只「加掛」,文字不動)

`Segment`(`transcribe.rs`)新增兩欄(serde 預設、向後相容):

```rust
pub speaker: Option<String>,      // 穩定講者 id "S1".."Sn";未對齊到 → None
pub speaker_mixed: bool,          // 預設 false;true = 橫跨 ≥2 講者,待手動切
```

新檔 `transcript/speakers.json`:

```json
{ "S1": { "display": "講者1", "track": "system" },
  "S2": { "display": "講者2", "track": "mic-internal" } }
```

`id → { display, track }`。**改名只動這個檔**(逐字稿段不重寫,改名乾淨可逆)。`display` 預設 `講者N`。

主題 / 人員續用 `meeting-info.json`(每場一份):**開始錄音時可填、會後工作區也能填/改**(兩邊寫同一檔)。工作區「講者改名」的下拉**用 `meeting-info` 的人員清單當選項**(輸入過的人員 = 改名選單)。

### 3.4 元件邊界(各一個清楚職責)

- `diarize.rs` — 引擎邊界:`fn diarize_wav(wav: &Path, hint: Option<usize>) -> Result<Vec<SpeakerSpan>>`。實際 onnx crate 藏在後面,可換。模型路徑走 `~/.mori/models/`。
- `assign_speakers`(純函式)— `(per_track_spans, segments) -> (labeled_segments, speaker_table)`:跨軌統一編號 + 多數對齊 + `speaker_mixed`。**完全可單測,不需模型**。
- `speakers.json` 讀寫 + rename。
- `diarize_session(session_id)` Tauri command — 串接:load WAV + jsonl → 每軌 `diarize_wav` → `assign_speakers` → 寫回 jsonl + 寫 `speakers.json` → emit 進度。背景跑。
- 工作區 UI(前端,Sessions 分頁升級)。

### 3.5 模型 / Deps / GPU

`segmentation.onnx` + `speaker-embedding.onnx` 放 `~/.mori/models/`;Deps 分頁顯示有無 + 下載按鈕;install 腳本抓(來源用所選 crate 的模型 release)。GPU 透過 onnxruntime CUDA EP,否則 CPU —— 比照 whisper 的 GPU 故事。

### 3.6 匯出

`meeting.public/internal.md` 有講者時,每段前綴顯示名(`亞澤: …`);改名後可重匯出。`speaker_mixed` 段在工作區標示出來供手動處理。

## 4. 錯誤處理(全 graceful)

- 模型未裝 → `diarize_session` 回明確狀態(Deps 提示下載),逐字稿維持不標人。
- 引擎錯 / WAV 壞 → log + 跳過該軌,另一軌照跑。
- 空 / 靜音軌 → 無 span → 不標。
- 對齊:無重疊 span → `speaker=None`;橫跨 ≥2 → 多數 + `speaker_mixed=true`。
- 重跑 diarization → 重算 `S1..Sn` 並**重置 `speakers.json` 顯示名(已改的名字會被洗掉)** → 重跑前警告。v1 先如此;「保留改名」留後續。

## 5. 測試

- **`assign_speakers` 純邏輯(可單測核心,不需模型)**:多數分配、`speaker_mixed` 旗標、無重疊→None、跨軌統一編號、空輸入。
- 引擎 wrapper:`#[ignore]` 整合測試,用合成雙人 WAV + 模型,手動跑(比照 whisper round-trip)。
- `speakers.json` round-trip;匯出前綴渲染;`meeting-info` 人員 → 改名選項。

## 6. 範圍

**v1(本 spec)**:diarization 全鏈 + 工作區(開一場會 → 跑分人 → 看標好的稿 → 從人員清單改名 → 編輯主題/人員 → 帶講者前綴重匯出 → 手動處理 mixed 段)。

**明確不在 v1(各自之後另開 spec)**:文字校正(LLM 6 步)、總結 / action items(LLM)、暫停/繼續、多段合併、聲紋註冊(跨會議自動認人)、live 即時標人。

## 7. Plan 要處理的開放項

- **Step 0 build spike**:在 speakrs / sherpa-onnx-rs / pyannote-rs 中選定 —— 確認能 build + bundle 模型 + 吃 GPU + 對合成雙人 WAV 跑得出合理 span。選不出再退方案 A(每 VAD clip 一個講者)。
- onnx runtime 依賴對 Tauri build / 跨平台(Linux + Windows)的影響。
- 模型來源 URL + 大小 + install 腳本整合(比照 whisper 模型下載)。
