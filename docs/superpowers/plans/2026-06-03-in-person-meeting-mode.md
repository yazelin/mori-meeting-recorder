# 現場會議模式(core)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增「現場會議」錄音模式:只收一支麥/房間音(單軌),輸出單一 `meeting.md`,RecordTab 可快速切換線上/現場。

**Architecture:** 做法 A —— 新增 `SourceKind::MeetingRoom`(track `room`、visibility public、取預設輸入裝置)。`sources_for_mode(mode)` 把寫死的雙軌迴圈參數化;現場單軌在內部沿用「mic 通道」(mic_progress / "mic" live lane / levels.mic),所以 status payload 不變。`finalize_session` 依模式動態建 tracks 並分流匯出(線上→`export()` 兩檔;現場→`export_single()`→`meeting.md`)。

**Tech Stack:** Rust(libpulse / cpal / serde)、Tauri v2 command、React + TS、react-i18next。

**Spec:** `docs/superpowers/specs/2026-06-03-in-person-meeting-mode-design.md`

**Worktree / branch:** `/home/ct/mori-universe/.worktrees/recorder-in-person-mode` @ `feat/in-person-meeting-mode`(off origin/main `d42f47c`)。

⚠ **Fresh-worktree 雷**:cargo 在 `src-tauri/` 內跑(無 root Cargo.toml);Tauri `generate_context!` 需要 `dist/` → **先 `npm run build` 再 cargo**。手測要 `npm run tauri dev`(動了 Rust 必須重啟 dev)。

---

## File Structure

| 檔案 | 動作 | 責任 |
|---|---|---|
| `src-tauri/src/config.rs` | Modify | 加 `recording_mode` 欄位(預設 `"online"`) |
| `src-tauri/src/audio/mod.rs` | Modify | `SourceKind::MeetingRoom` + track_name/visibility/as_str |
| `src-tauri/src/audio/linux.rs` | Modify | `pick_source` 的 `MeetingRoom` 分支(預設輸入) |
| `src-tauri/src/audio/windows.rs` | Modify | `pick_device` + `default_config` 的 `MeetingRoom` 分支(預設輸入) |
| `src-tauri/src/exporter.rs` | Modify | `SessionMeta.recording_mode` 欄位 + `export_single()`(單檔) |
| `src-tauri/src/session_store.rs` | Modify | `meeting_md_path()` |
| `src-tauri/src/recorder.rs` | Modify | `sources_for_mode()` + start_session 參數化 + ActiveSession.mode + finalize 動態建軌/分流匯出 |
| `src/tabs/RecordTab.tsx` | Modify | 模式 segmented 切換(錄音中鎖)+ 依模式顯示軌 pill |
| `src/i18n/locales/{en,zh-TW}.json` | Modify | mode / room 字串 |
| `src-tauri/src/manifest.rs` | Modify | description 一行 |

**本 plan 範圍外(follow-up,已識別檔案)**:
- **現場 session 的分人/校正**:`postprocess.rs:74-116,222`(`write_labeled_tracks` / `diarize_session_inner` 的 `[system, mic-internal]` 硬清單 + `wav_rel` 三元判斷)+ `main.rs:699/721/736/752`(`merge_speakers` 迴圈 + 三個 `match track.as_str()`)須認得 `"room"` / `audio/room.wav` / `transcript/room.segments.jsonl`。本 plan **不做**;現場 session 按「分人」目前會是「找不到 sys/mic 軌 → 跳過、0 講者」的安全 no-op(不 crash)。
- **現場摘要**:summary pipeline 假設 public/internal 雙摘要,現場單源語意不同 → follow-up。

---

## Task 1: config 加 `recording_mode`

**Files:**
- Modify: `src-tauri/src/config.rs`(default fns 區 `:7-48`、struct `:50-81`、Default impl `:83-102`)

- [ ] **Step 1: 加 default fn**

在 `src-tauri/src/config.rs` 的 `default_summary_ollama_base_url`(`:46-48`)之後加:
```rust
fn default_recording_mode() -> String {
    // "online" = 雙軌(系統 + 麥,visibility 分流);"in_person" = 單軌房間麥 → 單一 meeting.md。
    "online".to_string()
}
```

- [ ] **Step 2: 加 struct 欄位**

在 `RecorderConfig` 的 `summary_force_local_default`(`:79-80`)之後、結構大括號內加:
```rust
    #[serde(default = "default_recording_mode")]
    pub recording_mode: String,
```

- [ ] **Step 3: 加進 Default impl**

在 `impl Default for RecorderConfig` 的 `summary_force_local_default: false,`(`:99`)之後加:
```rust
            recording_mode: default_recording_mode(),
```

- [ ] **Step 4: 編譯確認**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode/src-tauri && cargo check 2>&1 | tail -8`
Expected: 通過(`RecorderConfig` derive 的測試若有也應仍綠)。

- [ ] **Step 5: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode
git add src-tauri/src/config.rs
git commit -m "feat(recorder): config.recording_mode (online/in_person, 預設 online)"
```

---

## Task 2: `SourceKind::MeetingRoom`(+ 平台取源分支)

**Files:**
- Modify: `src-tauri/src/audio/mod.rs`(enum `:12-15`、impl `:25-46`、tests 區末)
- Modify: `src-tauri/src/audio/linux.rs`(`pick_source` `:30-35`)
- Modify: `src-tauri/src/audio/windows.rs`(`pick_device` `:16-26`、`default_config` match `:35-42`)

- [ ] **Step 1: 寫失敗測試(mod.rs tests)**

在 `src-tauri/src/audio/mod.rs` 檔尾加一個測試模組(若已有 `#[cfg(test)] mod tests` 就加進去):
```rust
#[cfg(test)]
mod source_kind_tests {
    use super::*;

    #[test]
    fn meeting_room_track_name_visibility_and_str() {
        assert_eq!(SourceKind::MeetingRoom.track_name(), "room");
        assert_eq!(SourceKind::MeetingRoom.as_str(), "meeting_room");
        assert_eq!(SourceKind::MeetingRoom.default_visibility(), Visibility::Public);
    }
}
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode/src-tauri && cargo test meeting_room_track_name 2>&1 | tail -15`
Expected: 編譯失敗 `no variant named MeetingRoom`。

- [ ] **Step 3: 加 enum variant + 三個 match 分支**

`src-tauri/src/audio/mod.rs`:enum(`:12-15`)加 `MeetingRoom,`:
```rust
pub enum SourceKind {
    MeetingSystem,
    MicInternal,
    MeetingRoom,
}
```
`default_visibility`(`:26-31`)加:
```rust
            Self::MeetingRoom => Visibility::Public,
```
`as_str`(`:33-38`)加:
```rust
            Self::MeetingRoom => "meeting_room",
```
`track_name`(`:40-45`)加:
```rust
            Self::MeetingRoom => "room",
```

- [ ] **Step 4: Linux `pick_source` 分支**

`src-tauri/src/audio/linux.rs` 的 `pick_source`(`:30-35`)加 `MeetingRoom => Ok(None)`(同 mic 走預設輸入):
```rust
pub fn pick_source(source: SourceKind) -> Result<Option<String>, String> {
    match source {
        SourceKind::MicInternal => Ok(None),
        SourceKind::MeetingRoom => Ok(None),
        SourceKind::MeetingSystem => pick_system_monitor().map(Some),
    }
}
```

- [ ] **Step 5: Windows `pick_device` + `default_config` 分支**

`src-tauri/src/audio/windows.rs` 的 `pick_device`(`:16-26`)加:
```rust
        SourceKind::MeetingRoom => host
            .default_input_device()
            .ok_or_else(|| "no default input device".into()),
```
`open_capture` 內 `default_config` 的 match(`:35-42`)加:
```rust
        SourceKind::MeetingRoom => device
            .default_input_config()
            .map_err(|e| format!("default_input_config: {e}"))?,
```

- [ ] **Step 6: 跑測試確認通過**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode/src-tauri && cargo test meeting_room_track_name 2>&1 | tail -15`
Expected: PASS。(Windows 分支在 Linux 上 `cfg` 掉,不影響本機編譯;靠 cargo check 文法把關。)

- [ ] **Step 7: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode
git add src-tauri/src/audio/mod.rs src-tauri/src/audio/linux.rs src-tauri/src/audio/windows.rs
git commit -m "feat(recorder): SourceKind::MeetingRoom (room 軌, public, 預設輸入裝置)"
```

---

## Task 3: exporter — `recording_mode` 欄位 + `export_single()`

**Files:**
- Modify: `src-tauri/src/exporter.rs`(`SessionMeta` `:8-26`、新函式、tests `:142-158` 的 `meta()` helper)

- [ ] **Step 1: 寫失敗測試**

在 `src-tauri/src/exporter.rs` 的 `#[cfg(test)] mod tests` 內加:
```rust
    #[test]
    fn export_single_room_segments_into_single_md() {
        let mut s1 = seg("r1", "meeting_room", "public", 1000, "大家好");
        s1.track = "room".into();
        let mut s2 = seg("r2", "meeting_room", "public", 2000, "開始開會");
        s2.track = "room".into();
        let (meeting_md, timeline) = export_single(&[s1, s2], &meta("m"), &[]).unwrap();
        assert!(meeting_md.contains("大家好"));
        assert!(meeting_md.contains("開始開會"));
        assert!(meeting_md.contains("會議記錄"));
        // 不走 public/internal 分流 header
        assert!(!meeting_md.contains("Mic-internal not included"));
        let v: serde_json::Value = serde_json::from_str(&timeline).unwrap();
        assert_eq!(v["session_id"], "m");
        assert_eq!(v["recording_mode"], "in_person");
    }
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode/src-tauri && cargo test export_single_room 2>&1 | tail -15`
Expected: 失敗(`export_single` 不存在 + `meta()` 無 `recording_mode` 欄位)。

- [ ] **Step 3: SessionMeta 加 `recording_mode` 欄位**

`src-tauri/src/exporter.rs` 的 `SessionMeta`,在 `diarize_emb_model`(`:24-25`)之後加:
```rust
    /// 錄音模式("online" / "in_person")。舊 timeline.json 無此欄 → serde default 空字串。
    #[serde(default)]
    pub recording_mode: String,
```

- [ ] **Step 4: tests 的 `meta()` helper 補欄位**

`src-tauri/src/exporter.rs` 測試裡的 `meta()`(`:142-158`),在 `diarize_emb_model: None,` 之後加:
```rust
            recording_mode: "in_person".into(),
```

- [ ] **Step 5: 加 `export_single()`**

在 `export()`(`:56-81`)之後加:
```rust
/// 現場模式單檔匯出:room 軌(visibility=public)全部段 → 單一 meeting.md。
/// 現場無「客戶 / 我方」之分 → 不產 public/internal、無補充區塊。回 (meeting_md, timeline_json)。
pub fn export_single(
    segments: &[Segment],
    meta: &SessionMeta,
    speakers: &[crate::diarize::SpeakerInfo],
) -> Result<(String, String), String> {
    let meeting_md = render_md(
        segments,
        "public",
        &format!("# 會議記錄 — {}\n\n> 現場會議(單一收音來源)。\n\n", meta.started_at),
        speakers,
    );
    let timeline = serde_json::to_string_pretty(meta).map_err(|e| format!("timeline json: {e}"))?;
    Ok((meeting_md, timeline))
}
```

- [ ] **Step 6: 補既有 SessionMeta 建構處(暫填,保持 build 綠)**

加 `SessionMeta.recording_mode` 後,`recorder.rs:311` 既有的 `SessionMeta { … }` 建構會缺欄編不過。
在 `src-tauri/src/recorder.rs` finalize 內該建構的 `diarize_emb_model: None,`(`:341`)之後**暫加**一行:
```rust
            recording_mode: "online".into(), // Task 5 改成依 session.recording_mode 動態分流
```
(Task 5 會整段改寫 finalize,這行屆時被取代。)

- [ ] **Step 7: 跑測試確認通過**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode/src-tauri && cargo test 2>&1 | tail -20`
Expected: 全綠(含 `export_single_room` 新測試 + exporter 既有 public/internal 回歸)。

- [ ] **Step 8: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode
git add src-tauri/src/exporter.rs src-tauri/src/recorder.rs
git commit -m "feat(recorder): SessionMeta.recording_mode + export_single (現場單檔 meeting.md)"
```

---

## Task 4: `sources_for_mode()` + start_session 參數化 + ActiveSession.mode

**Files:**
- Modify: `src-tauri/src/recorder.rs`(`ActiveSession` `:22-29`、新函式、`start_session` `:115-241`)

- [ ] **Step 1: 寫失敗測試**

在 `src-tauri/src/recorder.rs` 檔尾(若無 tests 模組則新增):
```rust
#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn sources_for_mode_picks_tracks() {
        assert_eq!(
            sources_for_mode("online"),
            vec![SourceKind::MeetingSystem, SourceKind::MicInternal]
        );
        assert_eq!(sources_for_mode("in_person"), vec![SourceKind::MeetingRoom]);
        // 未知字串 → 落 online 預設
        assert_eq!(
            sources_for_mode("bogus"),
            vec![SourceKind::MeetingSystem, SourceKind::MicInternal]
        );
    }
}
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode/src-tauri && cargo test sources_for_mode 2>&1 | tail -15`
Expected: 失敗(`sources_for_mode` 不存在)。

- [ ] **Step 3: 加 `sources_for_mode()`**

在 `src-tauri/src/recorder.rs` 的 `impl Recorder {` **之前**(top-level fn)加:
```rust
/// 依錄音模式回傳要開的音源清單。線上=系統+麥(雙軌);現場=單一房間麥。
pub fn sources_for_mode(mode: &str) -> Vec<SourceKind> {
    match mode {
        "in_person" => vec![SourceKind::MeetingRoom],
        _ => vec![SourceKind::MeetingSystem, SourceKind::MicInternal],
    }
}
```

- [ ] **Step 4: ActiveSession 加 `recording_mode`**

`ActiveSession`(`:22-29`)在 `transcribe_model: String,` 之後加:
```rust
    /// 這場的錄音模式("online" / "in_person"),finalize 依此分流匯出。
    pub recording_mode: String,
```

- [ ] **Step 5: start_session 參數化音源迴圈 + room→mic 通道**

`src-tauri/src/recorder.rs` `start_session`:
(a) 在 `let transcribe_model = cfg.model.clone();`(`:136`)之後加:
```rust
        let recording_mode = cfg.recording_mode.clone();
```
(b) 把迴圈頭(`:158`)`for kind in [SourceKind::MeetingSystem, SourceKind::MicInternal] {` 改成:
```rust
        for kind in sources_for_mode(&recording_mode) {
```
(c) `prog` 的 match(`:160-163`)加 `MeetingRoom`(現場單軌沿用 mic 進度通道):
```rust
            let prog = match kind {
                SourceKind::MeetingSystem => &self.sys_progress,
                SourceKind::MicInternal | SourceKind::MeetingRoom => &self.mic_progress,
            };
```
(d) `track` 的 match(`:167-170`)加 `MeetingRoom`(沿用 "mic" live lane):
```rust
                    let track = match kind {
                        SourceKind::MeetingSystem => "sys",
                        SourceKind::MicInternal | SourceKind::MeetingRoom => "mic",
                    };
```
(e) `*active_guard = Some(ActiveSession { … })`(`:203-209`)補欄位 —— 在 `transcribe_model,` 之後加:
```rust
            recording_mode,
```

- [ ] **Step 6: 跑測試 + 編譯**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode/src-tauri && cargo test sources_for_mode 2>&1 | tail -15`
Expected: `sources_for_mode` 測試 PASS(finalize 的 SessionMeta 仍可能因缺欄報錯 → Task 5 修;若 build 被擋,先做 Task 5)。

- [ ] **Step 7: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode
git add src-tauri/src/recorder.rs
git commit -m "feat(recorder): sources_for_mode + start_session 依模式開軌 (現場單軌走 mic 通道)"
```

---

## Task 5: finalize_session 動態建軌 + 依模式分流匯出

**Files:**
- Modify: `src-tauri/src/session_store.rs`(path helpers `:40-42`)
- Modify: `src-tauri/src/recorder.rs`(`finalize_session` `:287-348`)

- [ ] **Step 1: SessionStore 加 `meeting_md_path()`**

`src-tauri/src/session_store.rs` 在 `internal_md_path`(`:41`)之後加:
```rust
    pub fn meeting_md_path(&self) -> PathBuf { self.root.join("meeting.md") }
```

- [ ] **Step 2: 改寫 finalize_session(動態建軌 + 分流匯出)**

把 `src-tauri/src/recorder.rs` 的 `finalize_session`(`:287-348`)整段 body(從 `let store = session.store;` 起到 `Ok(())`)改成:
```rust
        let store = session.store;
        let started_at = session.started_at;
        let transcribe_model = session.transcribe_model;
        let recording_mode = session.recording_mode;
        let session_id = store.session_id.clone();

        // 1. capture thread flush；2. worker drain。
        for h in session.handles {
            let _ = h.writer_handle.join();
        }
        for w in session.workers {
            let _ = w.handle.join();
        }

        let stopped_at = Local::now();
        let duration_secs = (stopped_at - started_at).num_seconds().max(0) as u64;

        // 依模式讀軌 + 建 tracks + 匯出。
        if recording_mode == "in_person" {
            // 現場:單一 room 軌 → 單一 meeting.md。
            let room_segs = crate::transcribe::read_segments_jsonl(
                &store.segments_path(SourceKind::MeetingRoom),
            );
            let meta = SessionMeta {
                schema_version: 1,
                session_id: session_id.clone(),
                started_at: started_at.to_rfc3339(),
                stopped_at: stopped_at.to_rfc3339(),
                duration_secs,
                tracks: vec![TrackMeta {
                    name: "room".into(),
                    source_kind: "meeting_room".into(),
                    visibility: "public".into(),
                    audio_path: "audio/room.wav".into(),
                    transcript_path: "transcript/room.segments.jsonl".into(),
                    segment_count: room_segs.len(),
                }],
                exports: Exports {
                    public: "meeting.md".into(),
                    internal: String::new(),
                },
                transcribe_model,
                diarize_seg_model: None,
                diarize_emb_model: None,
                recording_mode,
            };
            let (meeting_md, timeline) = crate::exporter::export_single(&room_segs, &meta, &[])?;
            std::fs::write(store.meeting_md_path(), meeting_md)
                .map_err(|e| format!("write meeting.md: {e}"))?;
            std::fs::write(store.timeline_path(), timeline)
                .map_err(|e| format!("write timeline.json: {e}"))?;
            return Ok(());
        }

        // 線上:雙軌 → public/internal 兩檔(行為與既有相同)。
        let sys_segs = crate::transcribe::read_segments_jsonl(
            &store.segments_path(SourceKind::MeetingSystem),
        );
        let mic_segs = crate::transcribe::read_segments_jsonl(
            &store.segments_path(SourceKind::MicInternal),
        );
        let all_segs: Vec<Segment> = sys_segs.iter().chain(mic_segs.iter()).cloned().collect();
        let meta = SessionMeta {
            schema_version: 1,
            session_id: session_id.clone(),
            started_at: started_at.to_rfc3339(),
            stopped_at: stopped_at.to_rfc3339(),
            duration_secs,
            tracks: vec![
                TrackMeta {
                    name: "system".into(),
                    source_kind: "meeting_system".into(),
                    visibility: "public".into(),
                    audio_path: "audio/system.wav".into(),
                    transcript_path: "transcript/system.segments.jsonl".into(),
                    segment_count: sys_segs.len(),
                },
                TrackMeta {
                    name: "mic-internal".into(),
                    source_kind: "mic_internal".into(),
                    visibility: "internal".into(),
                    audio_path: "audio/mic-internal.wav".into(),
                    transcript_path: "transcript/mic-internal.segments.jsonl".into(),
                    segment_count: mic_segs.len(),
                },
            ],
            exports: Exports {
                public: "meeting.public.md".into(),
                internal: "meeting.internal.md".into(),
            },
            transcribe_model,
            diarize_seg_model: None,
            diarize_emb_model: None,
            recording_mode,
        };
        let (pub_md, int_md, timeline) = export(&all_segs, &meta, &[])?;
        std::fs::write(store.public_md_path(), pub_md).map_err(|e| format!("write public.md: {e}"))?;
        std::fs::write(store.internal_md_path(), int_md).map_err(|e| format!("write internal.md: {e}"))?;
        std::fs::write(store.timeline_path(), timeline).map_err(|e| format!("write timeline.json: {e}"))?;
        Ok(())
```

- [ ] **Step 3: 全量編譯 + 既有測試回歸**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode/src-tauri && cargo test 2>&1 | tail -20`
Expected: 全綠 —— 含 Task 2/3/4 新測試 + exporter 既有 public/internal 回歸測試(線上模式行為不變)。

- [ ] **Step 4: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode
git add src-tauri/src/session_store.rs src-tauri/src/recorder.rs
git commit -m "feat(recorder): finalize 依模式分流 — 現場單軌出 meeting.md、線上維持雙檔"
```

---

## Task 6: RecordTab 模式切換 + 依模式顯示軌

**Files:**
- Modify: `src/tabs/RecordTab.tsx`

> recorder 前端無單元測試框架 → 本 task 以 `npm run build`(tsc)+ 手測把關。沿用 RecordTab 既有 class / `var(--…)` token。

- [ ] **Step 1: 加 mode state + 讀 config**

在 `const [participants, setParticipants] = useState("");`(`:40`)之後加:
```tsx
  const [mode, setMode] = useState<"online" | "in_person">("online");
  useEffect(() => {
    invoke<{ recording_mode?: string }>("get_config")
      .then((c) => { if (c?.recording_mode === "in_person") setMode("in_person"); })
      .catch(() => {});
  }, []);
  const changeMode = async (m: "online" | "in_person") => {
    if (recState !== "idle" || m === mode) return; // 錄音中鎖住
    setMode(m);
    try {
      const cfg = await invoke<Record<string, unknown>>("get_config");
      await invoke("set_config", { cfg: { ...cfg, recording_mode: m } });
    } catch (e) { console.error(e); }
  };
```

- [ ] **Step 2: 加模式 segmented 切換(callout 之後、record-control-bar 之前)**

把 `<div className="callout">⚠ {t("record.warning")}</div>`(`:137`)之後緊接著插入:
```tsx
      <div className="mode-switch" role="group" style={{ display: "flex", gap: 8, margin: "10px 0" }}>
        <button
          className={`mmr-btn${mode === "online" ? " primary" : ""}`}
          onClick={() => changeMode("online")}
          disabled={recState !== "idle"}
        >{t("record.mode_online")}</button>
        <button
          className={`mmr-btn${mode === "in_person" ? " primary" : ""}`}
          onClick={() => changeMode("in_person")}
          disabled={recState !== "idle"}
        >{t("record.mode_in_person")}</button>
      </div>
```

- [ ] **Step 3: 依模式顯示軌 pill**

把現有兩個 `<TrackPanel>`(`:158-171`)整段換成:
```tsx
      {mode === "in_person" ? (
        <TrackPanel
          kind="mic"
          label={t("record.room_pill")}
          sourceName={t("record.source_room")}
          level={levels?.mic ?? null}
          progress={{ done: status?.mic_done ?? 0, pending: status?.mic_pending ?? 0 }}
        />
      ) : (
        <>
          <TrackPanel
            kind="sys"
            label={t("capsule.system_pill")}
            sourceName={t("record.source_sys")}
            level={levels?.sys ?? null}
            progress={{ done: status?.sys_done ?? 0, pending: status?.sys_pending ?? 0 }}
          />
          <TrackPanel
            kind="mic"
            label={t("capsule.mic_pill")}
            sourceName={t("record.source_mic")}
            level={levels?.mic ?? null}
            progress={{ done: status?.mic_done ?? 0, pending: status?.mic_pending ?? 0 }}
          />
        </>
      )}
```

- [ ] **Step 4: 型別 + build 確認**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode && npm run build 2>&1 | tail -12`
Expected: tsc 無錯、vite build 成功。

- [ ] **Step 5: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode
git add src/tabs/RecordTab.tsx
git commit -m "feat(recorder): RecordTab 線上/現場 切換 + 現場單一房間軌 pill"
```

---

## Task 7: i18n 字串(en + zh-TW)

**Files:**
- Modify: `src/i18n/locales/en.json`(`record` 物件)
- Modify: `src/i18n/locales/zh-TW.json`(`record` 物件)

- [ ] **Step 1: en.json — record 物件加鍵**

在 `src/i18n/locales/en.json` 的 `record` 物件內(任一既有鍵之後,維持 JSON 合法)加:
```json
    "mode_online": "Online meeting",
    "mode_in_person": "In-person meeting",
    "room_pill": "Room",
    "source_room": "Room mic (default input)",
```

- [ ] **Step 2: zh-TW.json — record 物件加對稱鍵**

在 `src/i18n/locales/zh-TW.json` 的 `record` 物件內加:
```json
    "mode_online": "線上會議",
    "mode_in_person": "現場會議",
    "room_pill": "現場",
    "source_room": "房間麥克風(預設輸入)",
```

> 注意:加在既有鍵之間或之後都行,但**前一個鍵尾要有逗號、最後一個鍵後不要多逗號**。建議插在 `record` 物件第一個鍵之後以簡化逗號處理。

- [ ] **Step 3: 驗 JSON 合法**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode && node -e "JSON.parse(require('fs').readFileSync('src/i18n/locales/en.json','utf8'));JSON.parse(require('fs').readFileSync('src/i18n/locales/zh-TW.json','utf8'));console.log('json ok')"`
Expected: `json ok`。

- [ ] **Step 4: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode
git add src/i18n/locales/en.json src/i18n/locales/zh-TW.json
git commit -m "feat(recorder): i18n 線上/現場模式字串 (en + zh-TW)"
```

---

## Task 8: manifest description

**Files:**
- Modify: `src-tauri/src/manifest.rs`(description `:12`)

- [ ] **Step 1: 改 description**

把 `src-tauri/src/manifest.rs:12` 的 description 字串(現為 `"Dual-track meeting recorder (system + mic) with visibility-based export. Obs…"`)改成同句尾的版本,語意改為:雙軌(線上)+ 單軌(現場)。例如把開頭 `Dual-track meeting recorder (system + mic)` 換成:
```
Meeting recorder — online (dual-track system + mic, visibility-based export) or in-person (single room mic → meeting.md)
```
(保留原字串其餘部分;只擴寫模式描述。先 `Read` 該行取得完整原文再精準替換。)

- [ ] **Step 2: 編譯 + manifest 測試**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode/src-tauri && cargo test manifest 2>&1 | tail -12`
Expected: 通過(`manifest.rs` 既有測試只驗 kind / interfaces 長度,不驗 description 內文)。

- [ ] **Step 3: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode
git add src-tauri/src/manifest.rs
git commit -m "docs(recorder): manifest description 補現場單軌模式"
```

---

## Task 9: 全量驗證 + 真機手測 + PR

**Files:** 無(驗證)

- [ ] **Step 1: verify.sh 全綠(先 build 再驗)**

Run:
```bash
cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode
npm run build && bash scripts/verify.sh 2>&1 | tail -25
```
Expected: cargo test 全 PASS(含新測試 + 線上回歸)、npm build 成功、cargo check 乾淨。

- [ ] **Step 2: 真機手測(`npm run tauri dev`,動 Rust 要重啟 dev)**

手測清單:
  - Record 分頁出現「線上會議 / 現場會議」切換;預設線上、雙 pill(SYS+MIC)。
  - 切「現場會議」→ 只剩**一顆「現場」pill**;切回線上 → 恢復雙 pill。
  - 切現場 → Start → 對房間/麥講話 → 現場 pill 有訊號 + 即時字幕(走 mic lane)→ Stop。
  - 驗 `ls ~/.mori/meetings/<id>/` → 有 `meeting.md`(內含逐字稿)、**無** `meeting.public.md`/`meeting.internal.md`;`audio/room.wav` 存在;`timeline.json` 的 `recording_mode` = `"in_person"`、tracks 只有 room。
  - **回歸**:切回線上錄一小段 → 仍出 `meeting.public.md` + `meeting.internal.md`(雙軌行為不變)。
  - 錄音中模式切換鈕為 disabled。
  - (已知 follow-up)現場 session 按「分人」目前不分(safe no-op)—— 不在本 PR。

- [ ] **Step 3: push + PR(auto-merge)**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-in-person-mode
git push -u origin feat/in-person-meeting-mode
gh pr create --fill --base main --head feat/in-person-meeting-mode
gh pr merge --auto --squash
```
（recorder 無 CI;auto-merge 多半立即合。完成後回報 yazelin,並提醒 GUI 手測 + diarize-room follow-up。）

- [ ] **Step 4: worktree 清理(手測通過 + merge 後)**

```bash
cd /home/ct/mori-universe/mori-meeting-recorder
git worktree remove /home/ct/mori-universe/.worktrees/recorder-in-person-mode
```

---

## Self-Review

**Spec coverage**(對 `2026-06-03-in-person-meeting-mode-design.md`):
- 範圍#1 兩模式 → Task 1 config + Task 4 sources_for_mode。✅
- 範圍#2 現場只收麥/預設裝置 → Task 2 MeetingRoom→預設輸入(linux Ok(None) / windows default_input)。✅
- 範圍#3 單一 meeting.md → Task 3 export_single + Task 5 finalize in_person 分支。✅
- 範圍#4 新 SourceKind::MeetingRoom → Task 2。✅
- 範圍#5 RecordTab 快速切換 + 錄音中鎖 → Task 6(`disabled={recState !== "idle"}`)。✅
- 範圍#6 config 記住預設 → Task 1 + Task 6(讀 get_config)。✅
- 範圍#7 手動分人沿用 → 不改 diarize 演算法;**但現場軌的分人取檔=follow-up**(File Structure 已標,本 plan 不做)。⚠ 部分覆蓋(刻意)。
- 範圍#8 內部補充 flag 現場不適用 → export_single 不渲染補充區塊。✅
- 架構各點(mod/linux/windows/recorder/exporter/config/RecordTab/i18n/manifest)→ Task 2-8 對應。✅
- 回歸(線上零變動)→ Task 5 線上分支保留原邏輯 + Task 9 Step 2 回歸驗。✅

**Placeholder scan:** 無 TBD/TODO;每個 code step 有完整 code。Task 8 description 要求先 Read 原行再替換(因原字串被截斷未知全文)—— 已明示動作,非佔位。✅

**Type consistency:**
- `SourceKind::MeetingRoom`(Task 2 定義)= Task 4 `sources_for_mode` / Task 5 `segments_path(MeetingRoom)` 引用一致。✅
- `SessionMeta.recording_mode: String`(Task 3)= Task 5 finalize 兩處建構都填(in_person / online)。✅ 缺欄會編譯失敗 → Task 5 Step 3 全量 build 把關。
- `export_single() -> Result<(String, String), String>`(Task 3)= Task 5 `let (meeting_md, timeline) = export_single(...)` 解構一致。✅
- `meeting_md_path()`(Task 5 Step 1)= 同 task Step 2 `store.meeting_md_path()` 一致。✅
- `ActiveSession.recording_mode`(Task 4 Step 4)= start_session 填(Step 5e)、finalize 讀(Task 5)一致。✅
- i18n 鍵 `record.mode_online/mode_in_person/room_pill/source_room`(Task 7)= Task 6 引用一致。✅
- RecordTab `mode` 型別 `"online"|"in_person"` 與 config 字串一致。✅

**Task ordering note(每個 task 都留綠樹):** Task 3 加 `SessionMeta.recording_mode` 會讓 `recorder.rs:311` 建構缺欄 → Task 3 Step 6 **明確在同 task 暫填** `recording_mode: "online".into()`(故 Task 3 結束即綠)。Task 5 整段改寫 finalize、把暫填換成依 `session.recording_mode` 的雙分支。Task 2/4 各自只增 variant / 欄位 + 唯一建構處同步填,皆自足留綠。subagent-driven 逐 task 跑、每 task 末 `cargo test` 應全綠。
