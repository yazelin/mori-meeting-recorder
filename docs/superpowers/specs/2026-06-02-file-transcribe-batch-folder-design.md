# File Transcribe 批次資料夾 — step 2(設計)

> **Goal**: 在現有「檔案 / Files」單檔轉錄(step 1,PR #80)之上,加「選一個資料夾 →
> 把裡面的音/影檔批次轉成逐字稿」。延續「recorder 成宇宙唯一轉錄入口」的補完;
> desktop 端早於 #140 拔掉整個 Transcribe tab(含批次),這條把批次能力補回 recorder。
>
> **承上**: 設計母文件 [`2026-05-30-file-transcribe-design.md`](2026-05-30-file-transcribe-design.md)
> §94「批次資料夾:本 MVP 不做,留下一個 PR」。本 spec 就是那個 PR(step 2)。
>
> **Plan output**: 本 spec → `writing-plans` → `docs/superpowers/plans/2026-06-02-file-transcribe-batch-folder.md` → 實作。

## 背景

step 1(#80)已 ship 單檔轉錄:`file_transcribe.rs::transcribe_file(input)`(ffmpeg 抽 16kHz
mono WAV → `transcribe::run_whisper(…, &mut None)` 走 whisper-cli 免分塊 → 串接 segment text),
前端 `FilesTab.tsx` 單檔 picker → 轉 → 結果 + copy + 手動「存 .txt」。

批次 = 把這個單檔原語套用到「一個資料夾裡的多個檔」。核心觀察:**單檔原語已經完備,批次
不需要任何新的 whisper / ffmpeg 程式碼**,只需要「列出資料夾內可轉的檔」+「逐檔跑 + 逐檔存 +
進度/取消」的編排層。

## 範圍(yazelin 2026-06-02 拍板)

| # | 決議 | 值 |
|---|---|---|
| 1 | 輸出位置 | **每檔旁存同名 .txt**(`clip.mp3` → `clip.txt` 存原位置),沿用既有 `file_transcribe_save_txt` 行為。 |
| 2 | 子資料夾 | **只轉頂層檔案**,不遞迴子資料夾。 |
| 3 | 併發 | **逐檔序列**(一次一個),不平行 — whisper 重,平行會 CPU/GPU 互打。 |
| 4 | 進度 / 取消 | 逐檔狀態 + 整體進度;跑批次中可**取消**(取消後當前檔跑完即停)。 |
| 5 | 失敗處理 | 單檔失敗 → 標該檔失敗、**續跑下一個**,最後給成功/失敗總結。 |
| 6 | 同名 .txt 已存在 | **覆寫**(跟單檔一致)。「跳過已轉」列 follow-up。 |
| 7 | 做法 | **前端序列 loop 複用既有後端**,而非後端 batch 命令(理由見「做法選擇」)。 |

## 做法選擇

**採 A:前端序列 loop。** 前端拿到資料夾檔案清單後,逐檔 `await` 既有命令
`file_transcribe_one` + `file_transcribe_save_txt`,每檔更新狀態列;取消 = 一個 flag,當前檔跑完
就停。後端幾乎零新 code(只多一個列目錄的命令),進度/取消在 JS 天然好做,逐檔結果自然串流。

**否決 B:後端 batch 命令 + Tauri 事件**(`file_transcribe_batch(folder)` 在 spawn_blocking
跑迴圈、逐檔 emit 事件、後端自己寫 .txt、cancel token 中止)。要多寫迴圈 / 事件 / 取消 plumbing,
等於重做前端 loop 免費給的東西(YAGNI)。**未來若需要 CLI / headless 批次,再走 B**。

## 不混淆:跟單檔 / 會議轉錄的邊界

- 批次只是單檔的編排,**邊界與單檔完全相同**:輸入是現成檔案,輸出 `.txt` 存原檔旁,**不**進
  `~/.mori/meetings/`、沒有 track / visibility / session 概念。
- 跟會議轉錄共用的仍只有 whisper 引擎、language / model 設定、ffmpeg。
- 批次與單檔共用同一條 deps gate(`deps_check`:ffmpeg_ok / whisper_cli_ok / model_ok)。

## 架構

### 後端 `src-tauri/src/file_transcribe.rs`(只加列目錄,不動轉錄)

- 新純函式 `list_supported_in_dir(dir: &Path) -> Result<Vec<PathBuf>, String>`:
  1. `std::fs::read_dir(dir)`(讀不到 → `Err`,含路徑)。
  2. **只頂層**:對每個 entry,`file_type()?.is_file()` 才納入 — 目錄與 symlink 一律跳過
     (`DirEntry::file_type` 不跟隨 symlink,故 symlink 即使指向媒體檔也 is_file()=false;
     symlink 媒體檔屬邊角,列 follow-up)。
  3. `supported_extension(&path)` 過濾(複用既有純函式,白名單 `SUPPORTED_EXTS`)。
  4. 依檔名(`file_name`,大小寫不敏感)排序,回穩定順序。
  - 純函式、可單測(temp dir 餵混合內容)。
- `transcribe_file` / `extract_wav_to_temp` / `supported_extension` / `ffmpeg_present`:**完全不動**。

### 新 Tauri command(`main.rs` 註冊)

- `file_transcribe_list_dir(folder: String) -> Result<Vec<String>, String>`:
  呼 `list_supported_in_dir`,把 `PathBuf` 轉 `String` 回前端。**只列、不轉**。
- `file_transcribe_one(path)` / `file_transcribe_save_txt(source_path, text)`:**複用,不改簽名**。
- 批次無新事件:序列進度由前端逐檔 `await` 自然產生(逐檔一次 IPC,數十檔量級可接受)。

### 前端 `src/tabs/FilesTab.tsx`(升級,沿用自己的 style)

沿用 FilesTab 既有風格:inline style + `.mmr-btn` / `.mmr-btn primary` + recorder 自己的
`var(--text-dim)` / `var(--found-color)` / `var(--danger-color)` token(**不抄 mori-desktop 的
`var(--c-*)`**)、`react-i18next`。

- 既有單檔流程(pick / transcribe / result / copy / 手動 save)**保留不動**。
- 新增「選資料夾」按鈕:`open({ directory: true })` → `file_transcribe_list_dir(folder)` → 得 paths。
- 批次狀態 model:`items: { path, name, status, error?, chars? }[]`,
  `status ∈ 'pending' | 'running' | 'done' | 'error'`;另有 `running`(整批進行中)、
  `cancelRef`(`useRef<boolean>`)。
- 序列 loop(`for` + `await`,不是 `Promise.all`):
  - 每圈先看 `cancelRef.current`,true 就 break(已完成的保留)。
  - 設該列 `running` → `file_transcribe_one(path)` → 成功則 `file_transcribe_save_txt(source_path, text)`
    → 標 `done` + `chars=text.length`;throw → 標 `error` + 存訊息,**continue 下一個**。
- 頂部整體進度:`已完成 x / 共 N(失敗 y)`;`running` 時顯示「取消」鈕(設 `cancelRef.current=true`)。
- **批次自動逐檔存**(不需手動按存 — 不可能按 N 次);單檔流程的手動存鈕維持。
- 空清單(資料夾無支援檔)→ 明確提示,不進 loop。

## 資料流

```
選資料夾
  → file_transcribe_list_dir(folder)        (只頂層 + 副檔名白名單 + 排序)
  → [paths]  ──→ 前端建 items(全 pending)
  → 序列 for path in items:
        cancelRef? → break
        status=running
        file_transcribe_one(path)           (sync 阻塞至完成,複用 step 1)
          ├─ ok  → file_transcribe_save_txt  → status=done(chars)
          └─ err → status=error(原因) → continue
  → 末尾總結:done x / failed y(/ 取消則標已中止)
```

## 錯誤處理

- 資料夾讀不到(權限/不存在)→ `list_dir` `Err` → 前端 callout 明確訊息。
- 資料夾內無支援檔 → 空清單提示(「這個資料夾沒有可轉的音/影檔」),不進 loop。
- deps(ffmpeg / whisper-cli / model)缺 → 同單檔:dep 列紅標 + 「選資料夾」/批次按鈕 disable。
- 單檔 ffmpeg 失敗(無音軌 / 壞檔)/ whisper 失敗 → 該列標 `error` 顯示原因,**不中斷整批**。
- `file_transcribe_save_txt` 失敗(磁碟/權限)→ 同樣標該列 error 續跑。
- 取消:當前檔不強殺(`transcribe_one` 跑完),迴圈在下一圈停;已完成檔案的 .txt 保留。

## 測試(TDD,沿用 recorder cargo test 風格)

- `list_supported_in_dir`(純函式,主力測試):
  - temp dir 放 `a.mp3` / `b.MP4`(大寫)/ `c.txt`(排除)/ `noext`(排除)/ 子資料夾 `sub/`
    內含 `d.wav`(排除,不遞迴)→ 回 `[a.mp3, b.MP4]` 且排序穩定。
  - 空資料夾 → `Ok(vec![])`。
  - 不存在路徑 → `Err`。
- 複用既有單檔測試(`supported_extension` / `extract_wav_to_temp` / smoke)不動。
- 前端:沿用 recorder 既有前端測試慣例(目前以手測為主)。
- `bash scripts/verify.sh`(cargo test + npm run build + cargo check)全綠。
- 真機手測:準備一個資料夾放 2–3 個短音/影檔(含一個故意壞檔驗失敗續跑)→ 選資料夾 → 批轉 →
  驗每檔旁出 `.txt`、進度與失敗計數正確、取消可中止。

## 非目標 / Follow-up

- **遞迴子資料夾**:本 step 只頂層。
- **合併總檔**:不出單一彙整檔。
- **平行轉錄**:序列即可;平行留待有 GPU 佇列再說。
- **跳過已轉**(`.txt` 已存在就略過,用於續跑被取消的批次):MVP 覆寫,跳過列 follow-up。
- **批次走共享 whisper-server**:沿用單檔的 whisper-cli 路徑(`&mut None`);server 路徑要為長檔
  分塊配 60s timeout,非本 step。
- **後端 batch 命令 / CLI headless 批次**(做法 B):未來需要再做。
- **desktop 拔 Transcribe tab(step 3)**:另一條、另一 repo;step 2 完成後批次能力已回到 recorder,
  解掉母文件 §97 的 sequencing 顧慮(批次不再「暫時從宇宙消失」)。

## 驗證

- `bash scripts/verify.sh` 全綠。
- 真機手測清單(見上「測試」末項),逐項通過。
