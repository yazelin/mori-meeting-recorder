# File Transcribe 批次資料夾(step 2)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在現有單檔轉錄(Files 分頁)之上,加「選一個資料夾 → 把頂層音/影檔逐檔轉成逐字稿、各存同名 .txt」。

**Architecture:** 做法 A — 前端序列 loop 複用既有後端。後端只加一個列目錄的純函式 + 一個 Tauri 命令;前端 `FilesTab` 拿到清單後逐檔 `await` 既有 `file_transcribe_one` + `file_transcribe_save_txt`,逐檔更新狀態、可取消、失敗續跑。零新 whisper/ffmpeg 程式碼。

**Tech Stack:** Rust(`std::fs` / `tempfile`)、Tauri v2 command、React + TypeScript(`@tauri-apps/api/core` `invoke`、`@tauri-apps/plugin-dialog` `open`)、react-i18next。

**Spec:** `docs/superpowers/specs/2026-06-02-file-transcribe-batch-folder-design.md`

**Worktree / branch:** `/home/ct/mori-universe/.worktrees/recorder-batch-transcribe` @ `feat/file-transcribe-batch`(已建,off origin/main 1c8cdc4)。

---

## File Structure

| 檔案 | 動作 | 責任 |
|---|---|---|
| `src-tauri/src/file_transcribe.rs` | Modify | 加純函式 `list_supported_in_dir`(只頂層 + 白名單 + 排序)+ 單元測試。轉錄邏輯不動。 |
| `src-tauri/src/main.rs` | Modify | 加 `file_transcribe_list_dir` 命令 + 註冊進 `generate_handler!`。 |
| `src/i18n/locales/en.json` | Modify | `files.*` 加批次相關鍵。 |
| `src/i18n/locales/zh-TW.json` | Modify | `files.*` 加批次相關鍵(對稱)。 |
| `src/tabs/FilesTab.tsx` | Modify | 加「選資料夾」+ 批次清單/進度/取消;單檔流程保留不動。 |

⚠ **Fresh worktree 注意**(memory 既有雷):新 worktree 直接 `cargo test` 會撞 `generate_context!` 需要 `../dist` → **先 `npm run build` 再跑 cargo**。手測要 `npm run tauri dev`(動了 Rust,必須重啟 dev)。

---

## Task 1: 後端 `list_supported_in_dir`(純函式 + 測試)

**Files:**
- Modify: `src-tauri/src/file_transcribe.rs`(import 行 `:10`;測試加在既有 `mod tests` 內,約 `:131`)

- [ ] **Step 1: 改 import 讓 `PathBuf` 可用**

把 `src-tauri/src/file_transcribe.rs:10`
```rust
use std::path::Path;
```
改成
```rust
use std::path::{Path, PathBuf};
```

- [ ] **Step 2: 在 `mod tests` 內寫失敗測試**

在 `src-tauri/src/file_transcribe.rs` 的 `#[cfg(test)] mod tests { … }` 區塊內(既有測試之後)加:
```rust
    #[test]
    fn list_supported_in_dir_top_level_only_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("b.MP4"), b"x").unwrap();
        std::fs::write(root.join("a.mp3"), b"x").unwrap();
        std::fs::write(root.join("c.txt"), b"x").unwrap(); // 排除:非白名單
        std::fs::write(root.join("noext"), b"x").unwrap(); // 排除:無副檔名
        std::fs::create_dir(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("d.wav"), b"x").unwrap(); // 排除:子資料夾不遞迴

        let got = list_supported_in_dir(root).unwrap();
        let names: Vec<String> = got
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.mp3".to_string(), "b.MP4".to_string()]);
    }

    #[test]
    fn list_supported_in_dir_empty_ok() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list_supported_in_dir(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn list_supported_in_dir_missing_errors() {
        let r = list_supported_in_dir(&PathBuf::from("/nonexistent/nope-dir-xyz"));
        assert!(r.is_err());
    }
```

- [ ] **Step 3: 跑測試確認失敗**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-batch-transcribe/src-tauri && cargo test list_supported_in_dir 2>&1 | tail -20`
（crate 在 `src-tauri/`,無 root Cargo.toml,cargo 一律在 `src-tauri/` 內跑。)
Expected: 編譯失敗 `cannot find function list_supported_in_dir`。

- [ ] **Step 4: 寫最小實作**

在 `src-tauri/src/file_transcribe.rs` 加(放在 `supported_extension` 之後、`ffmpeg_present` 之前即可):
```rust
/// 列出資料夾**頂層**可轉錄的音/影檔:只取一般檔案(目錄與 symlink 跳過,
/// `DirEntry::file_type` 不跟隨 symlink)、`supported_extension` 過濾、依檔名
/// 大小寫不敏感排序。讀不到資料夾回 `Err`。
pub fn list_supported_in_dir(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("read dir {}: {e}", dir.display()))?;
    let mut out: Vec<PathBuf> = Vec::new();
    for entry in rd {
        let entry = entry.map_err(|e| format!("read entry: {e}"))?;
        let ft = entry.file_type().map_err(|e| format!("file type: {e}"))?;
        if !ft.is_file() {
            continue; // 目錄 + symlink 都跳過(不遞迴、symlink 媒體屬 follow-up)
        }
        let path = entry.path();
        if supported_extension(&path) {
            out.push(path);
        }
    }
    out.sort_by_key(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    Ok(out)
}
```

- [ ] **Step 5: 跑測試確認通過**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-batch-transcribe/src-tauri && cargo test list_supported_in_dir 2>&1 | tail -20`
Expected: 3 個測試 PASS。

- [ ] **Step 6: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-batch-transcribe
git add src-tauri/src/file_transcribe.rs
git commit -m "feat(file-transcribe): list_supported_in_dir — 列資料夾頂層可轉檔(批次 step 2 後端)"
```

---

## Task 2: 後端命令 `file_transcribe_list_dir` + 註冊

**Files:**
- Modify: `src-tauri/src/main.rs`（命令加在 `file_transcribe_save_txt` 之後,約 `:96`;註冊加在 `generate_handler!` 內 `file_transcribe_save_txt,` 之後,約 `:914`)

- [ ] **Step 1: 加命令**

在 `src-tauri/src/main.rs` 的 `file_transcribe_save_txt`（`:90-96`）之後加:
```rust
/// 列出資料夾頂層可轉錄的音/影檔路徑(只頂層、副檔名白名單、依檔名排序)。
/// 只列、不轉;轉錄由前端逐檔呼 `file_transcribe_one` + `file_transcribe_save_txt`。
#[tauri::command]
fn file_transcribe_list_dir(folder: String) -> Result<Vec<String>, String> {
    let paths = file_transcribe::list_supported_in_dir(std::path::Path::new(&folder))?;
    Ok(paths.into_iter().map(|p| p.display().to_string()).collect())
}
```

- [ ] **Step 2: 註冊進 invoke handler**

在 `src-tauri/src/main.rs` 的 `generate_handler!`（`:908` 起）內,於 `file_transcribe_save_txt,`（`:914`）之後加一行:
```rust
            file_transcribe_list_dir,
```
（縮排對齊既有命令清單。）

- [ ] **Step 3: 編譯確認**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-batch-transcribe/src-tauri && cargo check 2>&1 | tail -15`
Expected: 編譯通過,無 error。

- [ ] **Step 4: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-batch-transcribe
git add src-tauri/src/main.rs
git commit -m "feat(file-transcribe): file_transcribe_list_dir 命令 + 註冊"
```

---

## Task 3: i18n 批次鍵(en + zh-TW)

**Files:**
- Modify: `src/i18n/locales/en.json`（`files` 物件,`:19-33`）
- Modify: `src/i18n/locales/zh-TW.json`（`files` 物件,`:19-33`）

- [ ] **Step 1: en.json 加鍵**

把 `src/i18n/locales/en.json` 的
```json
    "saved": "Saved to {{path}}"
  },
```
改成
```json
    "saved": "Saved to {{path}}",
    "pick_folder": "Pick folder",
    "batch_start": "Start batch",
    "batch_cancel": "Cancel",
    "batch_progress": "Done {{done}} / {{total}} (failed {{failed}})",
    "no_media": "No transcribable audio/video files in this folder.",
    "status_pending": "Pending",
    "status_running": "Transcribing…",
    "status_done": "Done",
    "status_error": "Failed"
  },
```

- [ ] **Step 2: zh-TW.json 加對稱鍵**

把 `src/i18n/locales/zh-TW.json` 的
```json
    "saved": "已存到 {{path}}"
  },
```
改成
```json
    "saved": "已存到 {{path}}",
    "pick_folder": "選資料夾",
    "batch_start": "開始批次轉錄",
    "batch_cancel": "取消",
    "batch_progress": "完成 {{done}} / {{total}}(失敗 {{failed}})",
    "no_media": "這個資料夾沒有可轉的音/影檔。",
    "status_pending": "待處理",
    "status_running": "轉錄中…",
    "status_done": "完成",
    "status_error": "失敗"
  },
```

- [ ] **Step 3: 驗 JSON 合法**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-batch-transcribe && node -e "JSON.parse(require('fs').readFileSync('src/i18n/locales/en.json','utf8'));JSON.parse(require('fs').readFileSync('src/i18n/locales/zh-TW.json','utf8'));console.log('json ok')"`
Expected: `json ok`。

- [ ] **Step 4: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-batch-transcribe
git add src/i18n/locales/en.json src/i18n/locales/zh-TW.json
git commit -m "feat(file-transcribe): i18n 批次轉錄字串(en + zh-TW)"
```

---

## Task 4: 前端 `FilesTab` 批次 UI

**Files:**
- Modify: `src/tabs/FilesTab.tsx`

> 沿用 FilesTab 既有風格(inline style + `.mmr-btn` / `.mmr-btn primary` + recorder 自己的 `var(--…)` token,**不抄** mori-desktop `var(--c-*)`)。單檔流程(`pick`/`transcribe`/`result`/copy/手動 save)**完全保留**。recorder 前端無單元測試框架 → 本 Task 以 `npm run build`(tsc 型別)+ 手測 把關。

- [ ] **Step 1: import 加 `useRef`**

把 `src/tabs/FilesTab.tsx:1`
```tsx
import { useEffect, useState } from "react";
```
改成
```tsx
import { useEffect, useRef, useState } from "react";
```

- [ ] **Step 2: 加 `BatchItem` type**

在 `FileTranscript` type 之後(約 `:18`,`MEDIA_EXTS` 之前)加:
```tsx
type BatchItem = {
  path: string;
  name: string;
  status: "pending" | "running" | "done" | "error";
  error?: string;
  chars?: number;
};
```

- [ ] **Step 3: 加批次 state**

在 `const [copied, setCopied] = useState(false);`（約 `:33`)之後加:
```tsx
  // 批次資料夾轉錄
  const [items, setItems] = useState<BatchItem[]>([]);
  const [batchRunning, setBatchRunning] = useState(false);
  const cancelRef = useRef(false);
```

- [ ] **Step 4: 加進度衍生值**

在 `const depsOk = …;`（約 `:40`)之後加:
```tsx
  const batchDone = items.filter((it) => it.status === "done").length;
  const batchErr = items.filter((it) => it.status === "error").length;
```

- [ ] **Step 5: 加 `pickFolder` / `runBatch` / `cancelBatch`**

在 `transcribe` 函式之後(約 `:58`,`copy` 之前)加:
```tsx
  const pickFolder = async () => {
    setErr(null); setResult(null); setSavedAt(null); setItems([]);
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel !== "string") return;
    try {
      const paths = await invoke<string[]>("file_transcribe_list_dir", { folder: sel });
      if (paths.length === 0) { setErr(t("files.no_media")); return; }
      setItems(paths.map((p) => ({ path: p, name: p.split(/[\\/]/).pop() || p, status: "pending" })));
    } catch (e: any) {
      setErr(String(e));
    }
  };

  const runBatch = async () => {
    if (!items.length || batchRunning) return;
    cancelRef.current = false;
    setBatchRunning(true); setErr(null);
    const snapshot = items;
    for (let i = 0; i < snapshot.length; i++) {
      if (cancelRef.current) break;
      if (snapshot[i].status === "done") continue;
      setItems((prev) => prev.map((it, j) => (j === i ? { ...it, status: "running" } : it)));
      try {
        const r = await invoke<FileTranscript>("file_transcribe_one", { path: snapshot[i].path });
        await invoke<string>("file_transcribe_save_txt", { sourcePath: r.source_path, text: r.text });
        setItems((prev) => prev.map((it, j) => (j === i ? { ...it, status: "done", chars: r.text.length } : it)));
      } catch (e: any) {
        setItems((prev) => prev.map((it, j) => (j === i ? { ...it, status: "error", error: String(e) } : it)));
      }
    }
    setBatchRunning(false);
  };

  const cancelBatch = () => { cancelRef.current = true; };
```

- [ ] **Step 6: 加「選資料夾」按鈕**

在按鈕列(`pick` + `transcribe` 兩鈕的 `<div>`,約 `:98-105`)內,於 transcribe 按鈕之後加:
```tsx
        <button className="mmr-btn" onClick={pickFolder} disabled={busy || batchRunning}>{t("files.pick_folder")}</button>
```

- [ ] **Step 7: 加批次清單/進度/取消區塊**

在單檔 `result` 區塊之後(約 `:127`,component return 最外層 `</div>`(`:128`)之前)加:
```tsx
      {items.length > 0 && (
        <div style={{ marginTop: 14 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 8 }}>
            {batchRunning ? (
              <button className="mmr-btn" onClick={cancelBatch}>{t("files.batch_cancel")}</button>
            ) : (
              <button className="mmr-btn primary" onClick={runBatch} disabled={!depsOk}>{t("files.batch_start")}</button>
            )}
            <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>
              {t("files.batch_progress", { done: batchDone, total: items.length, failed: batchErr })}
            </span>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 2, maxHeight: 200, overflowY: "auto" }}>
            {items.map((it) => (
              <div key={it.path} style={{ display: "flex", justifyContent: "space-between", gap: 8, fontSize: 11, padding: "2px 0" }}>
                <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={it.error ?? it.path}>{it.name}</span>
                <span style={{ flexShrink: 0, color: it.status === "error" ? "var(--danger-color)" : it.status === "done" ? "var(--found-color)" : "var(--text-dim)" }}>
                  {it.status === "pending" && t("files.status_pending")}
                  {it.status === "running" && t("files.status_running")}
                  {it.status === "done" && `${t("files.status_done")}${typeof it.chars === "number" ? ` (${it.chars})` : ""}`}
                  {it.status === "error" && t("files.status_error")}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
```

- [ ] **Step 8: 型別 + build 確認**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-batch-transcribe && npm run build 2>&1 | tail -15`
Expected: tsc 無型別錯、vite build 成功。

- [ ] **Step 9: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-batch-transcribe
git add src/tabs/FilesTab.tsx
git commit -m "feat(file-transcribe): FilesTab 批次資料夾 — 選資料夾 + 逐檔進度/取消/續跑"
```

---

## Task 5: 全量驗證 + 真機手測

**Files:** 無(驗證)

- [ ] **Step 1: verify.sh 全綠(先 build 再驗,避 fresh-worktree dist 雷)**

Run:
```bash
cd /home/ct/mori-universe/.worktrees/recorder-batch-transcribe
npm run build && bash scripts/verify.sh 2>&1 | tail -25
```
Expected: cargo test 全 PASS(含 Task 1 三個新測試)、npm run build 成功、cargo check 乾淨。

- [ ] **Step 2: 真機手測(需 `npm run tauri dev`,動了 Rust 要重啟 dev)**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-batch-transcribe
# 準備測試資料夾
mkdir -p /tmp/mmr-batch-test
# 放 2-3 個短音/影檔(可從既有錄音或 ~/.mori/meetings/ 複製)+ 1 個故意壞檔:
#   cp <某短音檔> /tmp/mmr-batch-test/a.mp3 ; cp ... b.m4a
#   printf 'not audio' > /tmp/mmr-batch-test/broken.mp3
npm run tauri dev
```
手測檢查清單:
  - 切到「檔案 / Files」分頁,deps 三列全綠。
  - 按「選資料夾」→ 選 `/tmp/mmr-batch-test` → 清單只列頂層支援檔(壞檔也列,因副檔名合法)。
  - 按「開始批次轉錄」→ 逐列由「待處理」→「轉錄中…」→「完成 (字數)」;壞檔變「失敗」且**不中斷**後續。
  - 頂部進度 `完成 x / 共 N(失敗 y)` 數字正確。
  - 每個成功檔旁出現同名 `.txt`(`ls /tmp/mmr-batch-test/*.txt`)。
  - 跑批次中按「取消」→ 當前檔跑完後停,剩餘維持「待處理」。
  - ⚠ 若「選資料夾」按了沒反應(silent fail)→ 查 `src-tauri/capabilities/*.json` 是否含 dialog open 權限(單檔 picker 已用同權限,理論上免改;見 memory tauri2 capability silent-fail 雷)。

- [ ] **Step 3: push + 開 PR(auto-merge)**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-batch-transcribe
git push -u origin feat/file-transcribe-batch
gh pr create --fill --base main --head feat/file-transcribe-batch
gh pr merge --auto --squash
```
（recorder repo 無 CI;auto-merge 會在無 required check 時直接合,或依 repo 設定。確認 PR 後告知 yazelin。）

- [ ] **Step 4: 清 worktree(merge 後)**

```bash
cd /home/ct/mori-universe/mori-meeting-recorder
git worktree remove /home/ct/mori-universe/.worktrees/recorder-batch-transcribe
```

---

## Self-Review

**Spec coverage**(對 `2026-06-02-file-transcribe-batch-folder-design.md` 逐條):
- 範圍#1 每檔旁存 .txt → Task 4 Step 5 `runBatch` 呼 `file_transcribe_save_txt`。✅
- 範圍#2 只頂層 → Task 1 `list_supported_in_dir` `is_file()` 跳目錄 + 測試驗子資料夾排除。✅
- 範圍#3 序列 → Task 4 `runBatch` `for`+`await`。✅
- 範圍#4 進度/取消 → Task 4 Step 4 衍生值 + Step 7 進度列 + `cancelRef`/`cancelBatch`。✅
- 範圍#5 失敗續跑 → Task 4 `runBatch` `catch` 標 error 後 loop 續行。✅
- 範圍#6 覆寫 → 複用 `file_transcribe_save_txt`(`fs::write` 覆寫)。✅
- 範圍#7 做法 A → 後端只加 list_dir,轉錄複用。✅
- 架構/後端 `list_supported_in_dir` → Task 1。✅ 命令 → Task 2。✅
- 架構/前端 FilesTab 升級 → Task 4。✅
- 錯誤處理(讀不到/空/deps/單檔失敗/取消)→ Task 1 Err + Task 4 `no_media`/dep gate(`disabled={!depsOk}`)/catch/cancelRef。✅
- 測試(`list_supported_in_dir` 三案 + verify.sh + 手測)→ Task 1 + Task 5。✅
- 非目標(遞迴/合併/平行/跳過已轉/server 路徑/B 命令/step 3)→ 計劃皆未觸及,符合。✅

**Placeholder scan:** 無 TBD/TODO;每個 code step 都有完整 code;命令與函式名前後一致。✅

**Type consistency:**
- Rust:`list_supported_in_dir(&Path) -> Result<Vec<PathBuf>, String>`(Task 1 定義)= Task 2 命令呼叫簽名一致;`PathBuf` import 在 Task 1 Step 1 補。✅
- 命令名 `file_transcribe_list_dir` 在 Task 2(定義 + 註冊)與 Task 4 Step 5(`invoke<string[]>("file_transcribe_list_dir", { folder })`)一致;參數 `folder: String` ↔ JS `{ folder }`(Tauri v2 auto-camelCase 對單字無影響)。✅
- `file_transcribe_one` 回 `FileTranscript { source_path, text, duration_secs }`;Task 4 `save_txt` 傳 `{ sourcePath: r.source_path, text: r.text }`(既有單檔同樣用 `sourcePath` camelCase,已驗證可行)。✅
- i18n 鍵(`files.pick_folder`/`batch_start`/`batch_cancel`/`batch_progress`/`no_media`/`status_*`)Task 3 定義 = Task 4 引用一致。✅
