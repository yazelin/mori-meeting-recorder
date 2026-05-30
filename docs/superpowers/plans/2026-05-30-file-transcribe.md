# File Transcribe Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 mori-meeting-recorder 加一個「檔案 / Files」分頁,把任意現成音/影檔轉成逐字稿,讓 recorder 成為宇宙唯一轉錄入口。

**Architecture:** 新 sync 模組 `file_transcribe.rs`:ffmpeg 抽 16kHz mono WAV → temp 檔 → 複用既有 `transcribe::run_whisper(…, &mut None)`(直走 whisper-cli,原生處理任意長度,免分塊)→ 串接 segment text。3 個 Tauri command + 前端 FilesTab + DepsCheck 加 ffmpeg + install script 加 ffmpeg。

**Tech Stack:** Rust(Tauri 2、std::process::Command、tempfile、hound)、React/TS、whisper.cpp CLI、ffmpeg。

**Spec:** `docs/superpowers/specs/2026-05-30-file-transcribe-design.md`

**基準:** 全程在隔離 worktree `feat/file-transcribe`(基於含 #79 的 main)。每個 task 後 `cd src-tauri && cargo test`(前端 task 用 `npm run build`)。

---

### Task 1: `file_transcribe.rs` — 純函式 + 結構(supported_extension / FileTranscript / ffmpeg_present)

**Files:**
- Create: `src-tauri/src/file_transcribe.rs`
- Modify: `src-tauri/src/main.rs`(加 `mod file_transcribe;`)
- Modify: `src-tauri/Cargo.toml`(確認 `tempfile` 在 `[dependencies]`,不是只在 dev)

- [ ] **Step 1: 確認 tempfile 是 runtime dep**

Run: `grep -A40 "^\[dependencies\]" src-tauri/Cargo.toml | grep tempfile`
若只出現在 `[dev-dependencies]`,把 `tempfile = "3"` 加到 `[dependencies]`。

- [ ] **Step 2: 建檔 + 寫 failing 測試(supported_extension)**

建 `src-tauri/src/file_transcribe.rs`:

```rust
//! 檔案轉錄:把任意現成音/影檔(非 session 錄音)轉成逐字稿。
//! ffmpeg 抽 16kHz mono WAV → temp → 複用 transcribe::run_whisper(cli 路徑)。
//! 跟 recorder.rs 的 session 生命週期完全解耦;無 visibility / track 概念。

use std::path::Path;
use serde::Serialize;

/// 支援的副檔名(小寫,不含點)。ffmpeg 能解的常見音/影格式。
const SUPPORTED_EXTS: &[&str] = &[
    "wav", "mp3", "m4a", "flac", "ogg", "aac", "opus", "wma", // 音
    "mp4", "mkv", "webm", "mov", "avi", // 影(抽音軌)
];

/// 副檔名白名單判斷(大小寫不敏感)。
pub fn supported_extension(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => SUPPORTED_EXTS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

/// 單檔轉錄結果(回給前端)。
#[derive(Serialize, Debug, Clone)]
pub struct FileTranscript {
    pub source_path: String,
    pub text: String,
    pub duration_secs: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn supported_extension_accepts_audio_and_video() {
        assert!(supported_extension(&PathBuf::from("a/b.mp3")));
        assert!(supported_extension(&PathBuf::from("a/b.MP4")));
        assert!(supported_extension(&PathBuf::from("x.wav")));
    }

    #[test]
    fn supported_extension_rejects_others() {
        assert!(!supported_extension(&PathBuf::from("a/b.txt")));
        assert!(!supported_extension(&PathBuf::from("noext")));
    }
}
```

在 `src-tauri/src/main.rs` 模組宣告區(其他 `mod xxx;` 旁)加 `mod file_transcribe;`。

- [ ] **Step 3: 跑測試確認 PASS**

Run: `cd src-tauri && cargo test --bin mori-meeting-recorder file_transcribe::tests`
Expected: 2 passed。(若 crate 無 bin-test 設定,用 `cargo test file_transcribe`)

- [ ] **Step 4: 加 ffmpeg_present()**

在 `file_transcribe.rs` 加(`tests` mod 之前):

```rust
/// ffmpeg 在 PATH 且可執行(`ffmpeg -version` exit 0)。deps 檢查用。
pub fn ffmpeg_present() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
```

- [ ] **Step 5: 跑 cargo check 確認編譯**

Run: `cd src-tauri && cargo check`
Expected: Finished(無 error)。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/file_transcribe.rs src-tauri/src/main.rs src-tauri/Cargo.toml
git commit -m "feat(file-transcribe): module scaffold — supported_extension + FileTranscript + ffmpeg_present"
```

---

### Task 2: ffmpeg 抽取 + transcribe_file(複用 run_whisper)

**Files:**
- Modify: `src-tauri/src/file_transcribe.rs`
- Test fixture: 用既有 `src-tauri/tests/fixtures/`(若無小音檔則 ffmpeg 整合測試標 `#[ignore]`)

- [ ] **Step 1: 寫 extract_wav_to_temp**

在 `file_transcribe.rs` 加:

```rust
use tempfile::NamedTempFile;

/// 用 ffmpeg 把任意輸入抽成 16kHz mono PCM WAV,寫到 temp 檔,回 handle
/// (drop 時自動刪)。參數對齊 mori-desktop transcribe_media::extract_wav_bytes。
pub fn extract_wav_to_temp(input: &Path) -> Result<NamedTempFile, String> {
    if !input.exists() {
        return Err(format!("file not found: {}", input.display()));
    }
    let tmp = tempfile::Builder::new()
        .suffix(".wav")
        .tempfile()
        .map_err(|e| format!("create temp wav: {e}"))?;
    let status = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(input)
        .args(["-vn", "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le", "-f", "wav"])
        .arg(tmp.path())
        .status()
        .map_err(|e| format!("spawn ffmpeg — 確認系統有裝 ffmpeg: {e}"))?;
    if !status.success() {
        return Err(format!("ffmpeg failed (exit {:?}) on {}", status.code(), input.display()));
    }
    let size = std::fs::metadata(tmp.path()).map(|m| m.len()).unwrap_or(0);
    if size < 44 {
        return Err(format!("ffmpeg produced empty WAV — 來源可能沒音軌或損壞: {}", input.display()));
    }
    Ok(tmp)
}
```

- [ ] **Step 2: 寫 transcribe_file(複用 run_whisper)**

```rust
use crate::audio::SourceKind;

/// 單檔轉錄主入口。ffmpeg 抽 WAV → run_whisper(cli 路徑)→ 串接 text。
pub fn transcribe_file(input: &Path) -> Result<FileTranscript, String> {
    let tmp = extract_wav_to_temp(input)?;
    let cfg = crate::config::read_config();
    let label = input
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    // &mut None → run_whisper 直走 whisper-cli(避開共享 server 60s timeout;
    // cli 原生處理任意長度檔)。kind/session_id 標記丟棄,只取 text。
    let segs = crate::transcribe::run_whisper(
        tmp.path(),
        &label,
        SourceKind::MicInternal,
        &cfg.language,
        cfg.traditional,
        &mut None,
    );
    let text = segs
        .iter()
        .map(|s| s.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let duration_secs = wav_duration_secs(tmp.path());
    Ok(FileTranscript {
        source_path: input.display().to_string(),
        text,
        duration_secs,
    })
}

/// 讀 temp WAV 算秒數(hound,recorder 既有 dep)。讀不到回 0。
fn wav_duration_secs(wav: &Path) -> f32 {
    match hound::WavReader::open(wav) {
        Ok(r) => {
            let spec = r.spec();
            let frames = r.len() as f32 / (spec.channels.max(1) as f32);
            frames / (spec.sample_rate.max(1) as f32)
        }
        Err(_) => 0.0,
    }
}
```

- [ ] **Step 3: cargo check 確認編譯**

Run: `cd src-tauri && cargo check`
Expected: Finished。若 `run_whisper` 簽章對不上(#79 後),依實際簽章調整參數(本 plan 基於 main@23c8b3d 的 `run_whisper(wav: &Path, session_id, kind, language, traditional, server: &mut Option<…>)`)。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/file_transcribe.rs
git commit -m "feat(file-transcribe): ffmpeg extract + transcribe_file reusing run_whisper (cli path)"
```

---

### Task 3: Tauri commands + DepsCheck 加 ffmpeg

**Files:**
- Modify: `src-tauri/src/main.rs`(加 3 command + 註冊 + DepsCheck.ffmpeg_ok)

- [ ] **Step 1: DepsCheck 加 ffmpeg_ok**

在 `main.rs` 的 `struct DepsCheck { whisper_cli_ok, model_ok, … }` 加欄位 `ffmpeg_ok: bool`,並在 `fn deps_check()` 回傳時設 `ffmpeg_ok: file_transcribe::ffmpeg_present()`。

- [ ] **Step 2: 加 3 個 command**

在 main.rs commands 區(其他 `#[tauri::command]` 旁)加:

```rust
#[derive(serde::Serialize)]
struct FileTranscribeDeps {
    ffmpeg_ok: bool,
    whisper_cli_ok: bool,
    model_ok: bool,
}

#[tauri::command]
fn file_transcribe_check_deps() -> FileTranscribeDeps {
    let bin = transcribe::whisper_bin_path();
    let model = transcribe::whisper_model_path();
    FileTranscribeDeps {
        ffmpeg_ok: file_transcribe::ffmpeg_present(),
        whisper_cli_ok: bin.exists() && bin.is_file(),
        model_ok: model.exists(),
    }
}

#[tauri::command]
fn file_transcribe_one(path: String) -> Result<file_transcribe::FileTranscript, String> {
    file_transcribe::transcribe_file(std::path::Path::new(&path))
}

#[tauri::command]
fn file_transcribe_save_txt(source_path: String, text: String) -> Result<String, String> {
    let src = std::path::Path::new(&source_path);
    let out = src.with_extension("txt");
    std::fs::write(&out, text).map_err(|e| format!("write {}: {e}", out.display()))?;
    Ok(out.display().to_string())
}
```

- [ ] **Step 3: 註冊到 invoke_handler**

在 `tauri::generate_handler![…]` 清單加:`file_transcribe_check_deps, file_transcribe_one, file_transcribe_save_txt`。

- [ ] **Step 4: cargo test 確認編譯 + 既有測試不破**

Run: `cd src-tauri && cargo test`
Expected: 全 pass(含 Task1 的 2 個新測試)。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "feat(file-transcribe): tauri commands (check_deps/one/save_txt) + DepsCheck ffmpeg_ok"
```

---

### Task 4: 前端 FilesTab + ExpandedView 註冊 + i18n

**Files:**
- Create: `src/tabs/FilesTab.tsx`
- Modify: `src/ExpandedView.tsx`(Tab union + 按鈕 + render)
- Modify: `src/i18n/index.ts`(`tabs.files` + `files.*` keys,zh-TW + en)

- [ ] **Step 1: 讀既有樣板**

Read `src/tabs/SettingsTab.tsx`(版面/css class 慣例)+ `src/i18n/index.ts`(key 結構 + `useTranslation` 用法)。FilesTab 沿用相同 css token,**不抄 desktop var(--c-*)**。

- [ ] **Step 2: 建 FilesTab.tsx**

單檔流程:`@tauri-apps/plugin-dialog` 的 `open()` 選檔 → `invoke("file_transcribe_one", { path })` → 顯示結果。元件:
- mount 時 `invoke("file_transcribe_check_deps")`,任一 false → 顯示紅標 + disable 轉錄。
- 「選檔」按鈕(open dialog,filters 音/影副檔名)。
- 選定後顯示檔名 + 「開始轉錄」按鈕。
- 轉錄中:disable + spinner 文案 `t("files.transcribing")`。
- 完成:結果 textarea + 「複製」+「存 .txt」(`invoke("file_transcribe_save_txt", { sourcePath, text })`)。
- 錯誤:`t("files.error", { e })`。

(完整 JSX 依 SettingsTab/desktop TranscribeTab FileMode 移植;invoke 名與參數如上,參數走 Tauri auto-camelCase:`sourcePath`。)

- [ ] **Step 3: ExpandedView 註冊分頁**

`src/ExpandedView.tsx`:
- `type Tab` union 加 `"files"`。
- import `FilesTab from "./tabs/FilesTab"`。
- tab 按鈕列加 `<button className={\`tab-btn ${tab === "files" ? "active" : ""}\`} onClick={() => setTab("files")}>{t("tabs.files")}</button>`(放在 deps 之前或 settings 之後,依視覺)。
- render 區加 `{tab === "files" && <FilesTab />}`。

- [ ] **Step 4: i18n keys**

`src/i18n/index.ts` zh-TW + en 各加:`tabs.files`(「檔案」/「Files」)、`files.title`、`files.hint`、`files.pick`、`files.start`、`files.transcribing`、`files.copy`、`files.save_txt`、`files.saved`、`files.error`、`files.deps_*`(ffmpeg/whisper/model 紅標文案)。

- [ ] **Step 5: 前端 build 驗證**

Run: `npm run build`
Expected: tsc + vite build 成功,無 type error。

- [ ] **Step 6: Commit**

```bash
git add src/tabs/FilesTab.tsx src/ExpandedView.tsx src/i18n/index.ts
git commit -m "feat(file-transcribe): Files tab UI + ExpandedView wiring + i18n"
```

---

### Task 5: Deps — install script 加 ffmpeg + DepsTab ffmpeg 列

**Files:**
- Modify: `scripts/install-whisper-linux.sh` + `scripts/install-whisper-windows.ps1`(加 ffmpeg 安裝/檢查)
- Modify: `src/tabs/DepsTab.tsx`(顯示 ffmpeg 狀態)

- [ ] **Step 1: install script 加 ffmpeg**

`install-whisper-linux.sh`:加一段確保 ffmpeg(`command -v ffmpeg || sudo apt-get install -y ffmpeg`,跟既有 apt 風格一致)。
`install-whisper-windows.ps1`:加 `winget install --id Gyan.FFmpeg -e`(或既有 winget 風格)+ PATH 提示。

- [ ] **Step 2: DepsTab 顯示 ffmpeg**

`DepsTab.tsx`:呼 `file_transcribe_check_deps`(或既有 `deps_check` 已含 `ffmpeg_ok`),多渲染一列 ffmpeg ✓/✗ + 安裝提示。

- [ ] **Step 3: build 驗證**

Run: `npm run build && cd src-tauri && cargo check`
Expected: 都成功。

- [ ] **Step 4: Commit**

```bash
git add scripts/ src/tabs/DepsTab.tsx
git commit -m "feat(file-transcribe): bundle ffmpeg dep (install scripts + DepsTab row)"
```

---

### Task 6: 完整驗證 + manual checklist

- [ ] **Step 1: verify.sh 全綠**

Run: `bash scripts/verify.sh`
Expected: cargo test + npm run build + cargo check 全 pass。

- [ ] **Step 2: 真機 smoke(可選,自動環境)**

若有 ffmpeg + whisper-cli + model:`file_transcribe_one` 對一個小 mp3 → 非空 text。否則列入 manual checklist。

- [ ] **Step 3: 整理 manual-test checklist(交付 yazelin)**

寫進 PR body + 回報。

---

## Self-Review

**Spec coverage:** 範圍(單檔)✓ Task2-4;ffmpeg dep ✓ Task5;檔案分頁名「檔案」✓ Task4;reuse run_whisper ✓ Task2;不進 ~/.mori/meetings ✓(存 .txt 旁邊,Task3);輸出 copy/save ✓ Task4。
**Placeholder scan:** 後端全給實際 code;前端 JSX 因量大走「移植 SettingsTab/TranscribeFileMode + 明確 invoke 名/參數」——非 placeholder,是明確的 port 指示 + 介面契約。
**Type consistency:** `FileTranscript{source_path,text,duration_secs}`、command 名 `file_transcribe_{check_deps,one,save_txt}`、`run_whisper(&Path, &str, SourceKind, &str, bool, &mut Option)` 全程一致。
