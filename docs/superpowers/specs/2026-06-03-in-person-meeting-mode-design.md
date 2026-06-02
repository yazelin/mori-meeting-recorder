# 現場會議模式 — 第二種會議錄音模式(設計)

> **Goal**: 在現有「線上會議」雙軌錄音(系統音 + 麥克風,visibility 分流 public/internal)之外,
> 新增「**現場會議**」模式:實體會議、沒開視訊、沒有系統音,**只收麥克風/房間音**,那一軌就是
> 整場會議紀錄,輸出**單一 `meeting.md`**。使用者可在 Record 分頁**快速切換**兩種模式。
>
> **Plan output**: 本 spec → `writing-plans` → `docs/superpowers/plans/2026-06-03-in-person-meeting-mode.md` → 實作。

## 背景

現有「線上會議 / Observer」模式:雙軌 capture —— `MeetingSystem`(系統輸出,視訊對方聲,
visibility=public)+ `MicInternal`(本機麥,我方私聊,visibility=internal)→ 停止後 whisper 轉錄
→ visibility 分流匯出 `meeting.public.md`(只 system)/ `meeting.internal.md`(含 mic)。

實體會議時:沒開網路視訊 → **沒有系統音**;所有人在同一個會議室,靠麥克風收整個房間。此時
「公開 vs 內部」的軌道區隔不存在 —— 麥收到的就是整場會議,一份紀錄就好。

**架構觀察(已查證,see Explore 2026-06-03)**:雙軌其實是 `recorder.rs:158` 一個寫死的迴圈
`[MeetingSystem, MicInternal]`;每個音源**獨立**開(各自 thread / WAV / transcribe worker)。
VAD、轉錄、語者分離、exporter 都**與軌數無關**。所以「只收麥單軌」底層幾乎已支援,主要缺
mode 開關 + 單檔輸出語意 + UI 切換。

## 範圍(yazelin 2026-06-03 拍板)

| # | 決議 | 值 |
|---|---|---|
| 1 | 模式數 | 兩種:`online`(線上會議,現狀)/ `in_person`(現場會議,新增) |
| 2 | 現場音源 | **只收麥克風/房間音**,無系統音軌;**用系統預設輸入裝置**(指定裝置=另一份 spec) |
| 3 | 現場輸出 | **單一 `meeting.md`**(room 軌全部逐字稿);**不**產 public/internal 兩檔 |
| 4 | 軌表示 | 新增 `SourceKind::MeetingRoom`(track 名 `room`,visibility=public)(做法 A) |
| 5 | 快速切換 | RecordTab 開始鈕上方 segmented 切換(線上/現場);改了寫 config;**錄音中鎖住** |
| 6 | 預設記憶 | `recording_mode` 存 config,重開 app 記得上次選的(預設 `online`) |
| 7 | 語者分離 | 不動;現場 `room.wav` 單軌走**現有手動「分人」工作區**(房間多人一支麥最受用) |
| 8 | 內部補充 flag | 是線上模式(public/internal 分流)概念 → **現場模式不適用**,meeting.md 即全部內容 |

## 做法選擇:現場那一軌怎麼表示

**採 A:新增 `SourceKind::MeetingRoom`**(track_name `room`、visibility=public)。現場 session =
單軌 `[MeetingRoom]`,`open_capture` 的 `MeetingRoom` 分支跟 mic 一樣抓**預設輸入裝置**。檔案
`audio/room.wav`、`transcript/room.segments.jsonl`。語意誠實、檔名自明。代價:enum 多一 variant,
需補幾處 match(`track_name` / `default_visibility` / `open_capture` linux+windows),機械式。

**否決 B:複用 `MicInternal`**(改 visibility→public + 輸出走單檔)。少改 code,但 `mic-internal.wav`
卻是整場房間錄音 → 誤導;visibility override 是特例,資料模型不誠實。**未來不轉向 B**。

## 不混淆:跟線上模式的邊界

- 現場模式**沒有** system 軌,也**沒有** public/internal 分流。`room` 軌 visibility 雖標 public,
  但匯出走**單檔 `meeting.md`** 路徑,不產 `meeting.public.md` / `meeting.internal.md`。
- 線上模式**完全不變**(雙軌 + public/internal 兩檔 + 內部補充 flag)。
- 兩模式共用:capture thread 機制、VAD、whisper 引擎、language/model 設定、diarization 工作區、
  session 目錄結構(`~/.mori/meetings/<id>/`)。

## 架構

### 後端 `src-tauri/src/audio/mod.rs`(SourceKind 增補)

- `SourceKind` 加 `MeetingRoom`(現於 `mod.rs:9-15`,僅 `MeetingSystem` / `MicInternal`)。
- `track_name`:`MeetingRoom => "room"`。
- `default_visibility`(現 `mod.rs:25-31`):`MeetingRoom => Visibility::Public`。

### 後端 `src-tauri/src/audio/{linux,windows}.rs`(open_capture 分支)

- `open_capture` 對 `MeetingRoom` 的取源 = **跟 `MicInternal` 同一條**(預設輸入裝置):
  Linux `Simple::new(..., Direction::Record, source_name=None, ...)`;Windows `host.default_input_device()`。
- 不新增裝置列舉/選擇(另一份 spec)。

### 後端 `src-tauri/src/recorder.rs`(模式參數化)

- 新純函式 `sources_for_mode(mode: &str) -> Vec<SourceKind>`:
  `"in_person" => vec![MeetingRoom]`;其餘(含 `"online"`)`=> vec![MeetingSystem, MicInternal]`。可單測。
- `start_session`:把 `recorder.rs:158` 寫死的 `[MeetingSystem, MicInternal]` 換成
  `sources_for_mode(&cfg.recording_mode)`。迴圈其餘(獨立開源 / thread / WAV / transcribe worker)不動。
- `finalize_session`(現 `recorder.rs:287-348`,`SessionMeta.tracks` 寫死 2 軌於 `:317-334`):
  改成**依實際抓到的 handles 動態建** `tracks`;`SessionMeta` 加 `recording_mode` 欄位。
- `stop_session` 讀 segment 檔處(現假設 sys+mic 都在)改成**容缺**:只讀實際存在的軌的 jsonl。

### 後端 `src-tauri/src/exporter.rs`(依模式分流)

- `SessionMeta` 加 `#[serde(default)] recording_mode: String`,寫進 `timeline.json`。
- 匯出依模式:
  - `online` → 維持現有 `export()`(`public.md` + `internal.md` + `timeline.json`,`exporter.rs:53-81`)。
  - `in_person` → 新增 `export_single()`:把所有 room 段渲染成單一 `meeting.md`(沿用 `render_md`),
    **不**產 public/internal;`timeline.json` 照常(含 `recording_mode="in_person"`)。
- `finalize_session` 依 `recording_mode` 呼對應匯出函式並寫對應檔。

### 後端 `src-tauri/src/config.rs`(模式欄位)

- `RecorderConfig` 加 `#[serde(default = "default_recording_mode")] recording_mode: String`
  (`default_recording_mode() -> "online"`)。現有 `get_config`/`set_config` 命令即可讀寫,免新命令。

### 前端 `src/tabs/RecordTab.tsx`(快速切換 + 單軌呈現)

- 開始控制列**上方**加 segmented 切換:`線上會議` / `現場會議`,綁 `config.recording_mode`
  (`get_config` 讀、改時 `set_config` 寫)。**錄音中(state≠idle)disable**,不能中途切。
- 軌 pill:`online` 顯示 SYS(會議音訊)+ MIC(內部麥)兩顆;`in_person` 只顯示一顆「現場/房間」。
  依當前模式(idle 看 config、recording 看 session 實際軌)決定顯示哪些 `TrackPanel`。
- done 畫面:`in_person` 顯示輸出為 `meeting.md`(非 public/internal)。
- 沿用 RecordTab 既有 inline style + recorder 自己的 `var(--…)` token,不抄 mori-desktop。

### i18n `src/i18n/locales/{en,zh-TW}.json`

- 加鍵:`record.mode_label` / `record.mode_online` / `record.mode_in_person` / 「現場/房間」軌 label /
  done 畫面單檔字串。兩語系對稱。

### manifest `src-tauri/src/manifest.rs`

- description 一行更新:雙軌(線上)+ 單軌(現場)會議錄音。

### 語者分離(`diarize.rs` / Sessions 工作區,沿用但須確認軌來源)

- diarize 演算法 / 手動「分人」流程(輸入人數 → 按鈕 → 分)**不變**。
- **須確認的整合點**:diarize_session / 工作區要**依 session 實際 `tracks` 找音檔**(現場 = `room.wav`),
  而非寫死 `system.wav` / `mic-internal.wav`;若目前寫死,改成依 `SessionMeta.tracks` 逐軌跑,
  否則現場 session 按「分人」會找不到音檔。實作時先查 diarize_session 的取檔邏輯。

## 資料流(現場模式)

```
RecordTab 選「現場會議」(寫 config.recording_mode="in_person")
  → recorder_start
  → sources_for_mode("in_person") = [MeetingRoom]
  → open_capture(MeetingRoom) = 預設輸入裝置 → audio/room.wav(獨立 thread)
  → VAD 切段 → whisper 轉錄 → transcript/room.segments.jsonl(+ 即時字幕)
  → recorder_stop → finalize_session
  → recording_mode="in_person" → export_single(room 段) → meeting.md
  → (可選)Sessions 工作區「分人」→ diarize room.wav → 帶講者重匯出
```

## 錯誤處理

- 現場麥開失敗(無輸入裝置/被佔用)→ session 明確報錯(現場無備援軌,不像線上還有另一軌)。
- 模式切換在錄音中 disable,避免中途換軌語意混亂。
- `stop_session` 容缺讀 segment:現場只有 room.jsonl,缺 system/mic 不報錯。
- 線上模式行為與檔案產出**零變動**(回歸風險點:確認 online 仍出 public/internal 兩檔)。

## 測試(TDD,沿用 recorder cargo test 風格)

- `sources_for_mode`:`"in_person"→[MeetingRoom]`、`"online"→[MeetingSystem,MicInternal]`、
  未知字串 → 落 online 預設。純函式單測。
- `SourceKind::MeetingRoom`:`track_name()=="room"`、`default_visibility()==Public`。單測。
- `export_single`:給一組 room 段 → `meeting.md` 非空、含全部段、**不**產 public/internal;
  空段 → 合理空輸出。單測。
- `finalize_session` 動態建軌(現場 1 軌 / 線上 2 軌)→ 整合測或薄單測。
- **回歸**:線上模式 export 仍產 public/internal 兩檔(既有測試應仍綠)。
- 前端:沿用 recorder 既有前端慣例(手測為主)。
- `bash scripts/verify.sh` 全綠。
- 真機手測:切現場 → 對房間講話 → 單一 ROOM pill 亮 → Stop → 驗 `~/.mori/meetings/<id>/meeting.md`
  寫對、無 public/internal;切回線上 → 仍雙軌雙檔。

## 非目標 / Follow-up

- **收音裝置選擇**(列舉輸入裝置 + 下拉 + 持久化 + 接進 capture)：**另一份 spec**(順便給線上麥用)。
- 現場模式錄完**自動觸發** diarization:不做,沿用手動「分人」。
- 會議室多麥混音 / 陣列麥 beamforming:不做。
- 線上↔現場 的 session 後期互轉:不做。
- 單檔 vs 雙檔以外的第三種輸出語意:不做。

## 驗證

- `bash scripts/verify.sh` 全綠。
- 真機手測清單(見上「測試」末項),逐項通過(動了 Rust → 重啟 tauri dev)。
