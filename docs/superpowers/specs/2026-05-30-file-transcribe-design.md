# File Transcribe — 把「轉現成檔案」搬進 recorder(設計)

> **Goal**: 讓 mori-meeting-recorder 成為森林宇宙**唯一的轉錄入口**。現有「錄會議 → 轉錄」不變;
> 新增一個「檔案 / Files」分頁,讓使用者把**手上已有的音檔/影片**轉成逐字稿。
> 之後 mori-desktop 整個 Transcribe tab 移除(另一個 repo、另一條 PR)。
>
> **Plan output**: 本 spec → `writing-plans` → `docs/superpowers/plans/2026-05-30-file-transcribe.md` → 實作。

## 背景

mori-desktop 的 Transcribe tab 一直有兩種「轉現成檔案」模式:單檔、批次資料夾(走 `mori-core::transcribe_media`:
ffmpeg 抽 16kHz mono WAV → 長檔切塊 → whisper)。會議模式已於 `chore/shed-meeting-mode`(PR #138)移出。
recorder 目前只會轉**它自己錄下來的** session 音訊,沒有「丟一個現成檔案來轉」的入口。

yazelin 拍板:recorder 收下這個能力,desktop 最終整個 Transcribe tab 拔掉。

## 範圍(yazelin 2026-05-30 拍板)

| # | 決議 | 值 |
|---|---|---|
| 1 | MVP 範圍 | **只做單檔**(picker / drag-drop 一個檔)。批次資料夾留 follow-up。 |
| 2 | 格式支援 | **加 ffmpeg dep** → 全格式對等 desktop(wav/mp3/m4a/flac/mp4/mkv/webm)。 |
| 3 | 分頁名 | **檔案 / Files**(跟「錄會議」區隔)。 |
| 4 | PR 切法 | **分兩 PR**:本 repo 加檔案轉錄(本 spec)→ 驗過 → 才開 desktop 拔 tab 的 PR。 |

## 不混淆:跟會議轉錄的邊界

- 檔案轉錄的輸入是**現成檔案**,輸出**不**進 `~/.mori/meetings/`(那是會議專用、有 visibility 邊界)。
- 檔案轉錄沒有 `track / source_kind / visibility` 概念,也不產生 session。它就是「檔案 → 文字」。
- 共用的只有 whisper 引擎(共享 whisper-server)、language/model 設定、ffmpeg。

## 架構

### 後端 `src-tauri/src/file_transcribe.rs`(新模組)

filesystem + subprocess,跟 `recorder.rs` 的 session 生命週期解耦。全 **sync**(沿用 recorder transcribe 鏈 sync 風格)。

- `extract_wav_to_temp(input: &Path) -> Result<NamedTempFile, String>` — port desktop `extract_wav_bytes` 的 ffmpeg 參數(`ffmpeg -hide_banner -loglevel error -i <input> -vn -ar 16000 -ac 1 -c:a pcm_s16le -f wav <temp.wav>`),用 `std::process::Command`(sync),輸出寫 `tempfile::NamedTempFile`。
- `supported_extension(path: &Path) -> bool` — 純函式,白名單 `wav/mp3/m4a/flac/mp4/mkv/webm/ogg/aac`(可測)。
- `ffmpeg_present() -> bool` — `ffmpeg -version` exit 0(deps 檢查用)。
- `transcribe_file(input: &Path) -> Result<FileTranscript, String>`:
  1. `extract_wav_to_temp(input)`。
  2. `cfg = config::read_config()`(拿 language / traditional)。
  3. **`transcribe::run_whisper(temp.path(), &label, SourceKind::MicInternal, &cfg.language, cfg.traditional, &mut None)`** — 傳 `&mut None` 直走 whisper-cli 路徑(`None` 分支),**避開共享 server 的 60s per-call timeout**;whisper-cli 原生處理任意長度檔,MVP 免手動分塊。沿用既有 noise filter + 繁體 s2twp 後處理。`kind`/`session_id` 標記丟棄,只取 `Segment.text` 以空白串接。
  4. duration 用 `hound` 讀 temp WAV(recorder 已有 hound dep);回 `FileTranscript { source_path, text, duration_secs }`。

**reuse 而非複製**:whisper 一律走既有 `transcribe::run_whisper`,零新 whisper 程式碼。共享 whisper-**server** 路徑(需為長檔分塊配 60s timeout)列 follow-up,非 MVP。

### 新 Tauri commands(`main.rs` 註冊)

- `file_transcribe_check_deps() -> FileTranscribeDeps { ffmpeg_ok, whisper_cli_ok, model_ok }`(複用既有 `deps_check` 的 whisper/model 檢查 + 新 ffmpeg)。
- `file_transcribe_one(path: String) -> Result<FileTranscript, String>`(sync command,阻塞至轉完;UI 顯示 spinner)。
- `file_transcribe_save_txt(source_path: String, text: String) -> Result<String, String>`(存 `<name>.txt` 在原檔旁)。
- MVP 無分塊 → 無進度事件,UI 用「轉錄中」spinner。(批次 / 分塊進度列 follow-up。)

### 前端 `src/tabs/FilesTab.tsx`(新分頁)

- 加進 ExpandedView 的 tab 列(現 Record/Live/Sessions/People/Deps/Settings → +Files)。
- 移植 desktop TranscribeTab 的**單檔**部分:deps 狀態列、檔案 picker + drag-drop、語言下拉(沿用 recorder config.language 預設)、轉錄按鈕(deps 紅標時 disable)、轉錄中 spinner、結果文字 + copy + 「存 .txt」。
- **用 recorder 自己的 css**(`src/theme.css` token,不抄 desktop var(--c-*))。
- i18n 走 recorder 既有 i18n。

### Deps(新增 ffmpeg)

- install scripts(`scripts/install-whisper-{linux,windows}.{sh,ps1}` 或新 `install-ffmpeg-*`)加 ffmpeg/ffprobe 安裝。
- DepsTab 加 ffmpeg 一列(`file_transcribe_check_deps`)。
- 對齊「bundle deps in repo」硬規矩。

## 資料流

```
使用者選檔
  → file_transcribe_one(path)            (sync,阻塞至完成,UI 顯示 spinner)
  → ffmpeg extract 16kHz mono WAV        → tempfile NamedTempFile
  → run_whisper(temp, …, &mut None)      → whisper-cli(原生處理任意長度)→ noise filter + 繁體 → Vec<Segment>
  → 串接 segment.text(空白接連)
  → FileTranscript 回 UI(顯示 + copy + 可選存 .txt 旁邊)
```

## 錯誤處理

- ffmpeg/ffprobe 缺 → check_deps 紅標 + 擋轉錄按鈕(同 desktop)。
- ffmpeg 失敗(無音軌/壞檔)→ 明確錯誤訊息回 UI。
- whisper-server 不可達 → `run_whisper` 既有 CLI fallback 接手(沿用)。

## 測試(TDD,沿用 recorder cargo test 風格)

- `chunk_wav`:邊界(剛好 5min / 略多 / 空)→ 純函式單測。
- `extract_wav_bytes`:整合測試餵一個小 fixture(repo 內小音檔)驗 WAV header + 非空;ffmpeg 缺則 `#[ignore]`。
- `file_transcribe_check_deps`:ffmpeg 在/不在兩路。
- 副檔名過濾(supported exts)純函式單測。
- 前端:沿用 recorder 既有前端測試慣例(若有)。

## 非目標 / Follow-up

- **批次資料夾**:本 MVP 不做,留下一個 PR。
- **desktop 拔 Transcribe tab**:**另一條 PR(step 3)**。⚠ 注意:本 MVP 只搬單檔;desktop 那條 PR 若要把「批次」也拔掉,等於批次能力暫時從宇宙消失 → 要嘛 desktop PR 先只拔單檔留批次,要嘛先把批次也搬進 recorder。此 sequencing 留到 step 3 再跟 yazelin 確認。
- in-process Rust decoder(symphonia)取代 ffmpeg:desktop 已評估並否決(`transcribe_media.rs` 註解),本 spec 沿用 ffmpeg subprocess。
- 檔案轉錄的 body-interface manifest interface 擴充:MVP 不動 manifest。

## 驗證

- `bash scripts/verify.sh`(cargo test + npm run build + cargo check)全綠。
- 真機手測:選一個 mp3/mp4 → 轉 → 出逐字稿(清單見實作 plan 結尾)。
