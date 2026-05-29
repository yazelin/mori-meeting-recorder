# Diarization 修正工具 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** 在會後處理工作區手動修正分人結果:合併講者、逐段改講者、重跑前警告。

**Architecture:** 三個純資料轉換 helper(`relabel_merge` / `relabel_one` / `drop_speakers`,放 `postprocess.rs`,可單測)+ 兩個薄 Tauri command(`merge_speakers` / `set_segment_speaker`,每軌 `read_segments_jsonl → 純函式 → write_segments_jsonl` 原子寫 + `speakers.json` drop)+ 工作區 UI(講者多選合併、逐段下拉、重跑確認)。無 schema 變更、無新 deps。

**Tech Stack:** Rust / Tauri 2 / React;沿用既有 `Segment.speaker` / `speakers.json` / `write_segments_jsonl` / `read_speakers` / `write_speakers` / custom `Select`。

守 `bash scripts/verify.sh` 綠;短命 branch→PR→squash;commit 尾 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。

---

## File Structure
- `src-tauri/src/postprocess.rs` — 加 `relabel_merge` / `relabel_one` / `drop_speakers`(純函式 + 測試)。
- `src-tauri/src/main.rs` — 加 `merge_speakers` / `set_segment_speaker` command + 註冊 handler。
- `src/tabs/SessionWorkspace.tsx` — 講者清單多選+合併鈕、逐字稿每段講者下拉、重跑分人確認。

---

## Task 1: 純 relabel helper(TDD 核心)

**Files:** Modify `src-tauri/src/postprocess.rs`(加 fn + tests)

- [ ] **Step 1: 寫失敗測試**

加到 postprocess.rs 的 `#[cfg(test)] mod tests`(若還沒有 use,補 `use crate::diarize::SpeakerInfo;` / `use crate::transcribe::Segment;`,並沿用該檔既有的 seg helper;若無就用下面的 inline 建構):
```rust
    fn seg_with(id: &str, track: &str, speaker: Option<&str>) -> Segment {
        Segment {
            id: id.into(), session_id: "m".into(), track: track.into(),
            source_kind: if track == "system" { "meeting_system".into() } else { "mic_internal".into() },
            visibility: "public".into(), start_ms: 0, end_ms: 1000, text: "x".into(),
            is_final: true, confidence: None,
            speaker: speaker.map(|s| s.to_string()), speaker_mixed: false,
        }
    }

    #[test]
    fn relabel_merge_reassigns_merge_ids_to_keep() {
        let segs = vec![
            seg_with("a", "system", Some("S1")),
            seg_with("b", "system", Some("S3")),
            seg_with("c", "system", Some("S2")),
            seg_with("d", "system", None),
        ];
        let out = relabel_merge(segs, "S1", &["S3".to_string()]);
        assert_eq!(out[0].speaker.as_deref(), Some("S1")); // 本來就是 keep,不動
        assert_eq!(out[1].speaker.as_deref(), Some("S1")); // S3 → S1
        assert_eq!(out[2].speaker.as_deref(), Some("S2")); // 不在 merge 清單,不動
        assert_eq!(out[3].speaker, None);                   // None 不動
    }

    #[test]
    fn relabel_one_only_changes_matching_seg_id() {
        let segs = vec![seg_with("a", "system", Some("S1")), seg_with("b", "system", Some("S1"))];
        let out = relabel_one(segs, "b", "S2");
        assert_eq!(out[0].speaker.as_deref(), Some("S1"));
        assert_eq!(out[1].speaker.as_deref(), Some("S2"));
    }

    #[test]
    fn drop_speakers_filters_listed_ids() {
        let list = vec![
            SpeakerInfo { id: "S1".into(), display: "甲".into(), track: "system".into() },
            SpeakerInfo { id: "S3".into(), display: "乙".into(), track: "system".into() },
        ];
        let out = drop_speakers(list, &["S3".to_string()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "S1");
    }
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd src-tauri && cargo test --release relabel_merge relabel_one drop_speakers`
Expected: 編譯失敗(三個 fn 未定義)。

- [ ] **Step 3: 實作三個純函式**

加到 postprocess.rs(非 test 區,檔案上方 helper 區):
```rust
use crate::diarize::SpeakerInfo;
use crate::transcribe::Segment;

/// 把 speaker ∈ merge_ids 的段改成 keep_id(合併講者),其餘不動。
pub fn relabel_merge(mut segs: Vec<Segment>, keep_id: &str, merge_ids: &[String]) -> Vec<Segment> {
    for s in &mut segs {
        if let Some(spk) = &s.speaker {
            if merge_ids.iter().any(|m| m == spk) {
                s.speaker = Some(keep_id.to_string());
            }
        }
    }
    segs
}

/// 把 id == seg_id 的段 speaker 設成 speaker_id(逐段改講者)。
pub fn relabel_one(mut segs: Vec<Segment>, seg_id: &str, speaker_id: &str) -> Vec<Segment> {
    for s in &mut segs {
        if s.id == seg_id {
            s.speaker = Some(speaker_id.to_string());
        }
    }
    segs
}

/// 從 speakers 清單濾掉 drop_ids(合併後移除被併走的講者列)。
pub fn drop_speakers(list: Vec<SpeakerInfo>, drop_ids: &[String]) -> Vec<SpeakerInfo> {
    list.into_iter().filter(|s| !drop_ids.iter().any(|d| d == &s.id)).collect()
}
```
(若 `use crate::...Segment/SpeakerInfo` 已存在於檔案,不要重複 import — 檢查後沿用。)

- [ ] **Step 4: 跑測試確認通過**

Run: `cd src-tauri && cargo test --release relabel`
Expected: PASS(3 個測試)。

- [ ] **Step 5: Commit**
```bash
git add src-tauri/src/postprocess.rs
git commit -m "feat(diar-fix): pure relabel helpers — merge / set-one / drop-speakers (TDD)"
```

---

## Task 2: `merge_speakers` command

**Files:** Modify `src-tauri/src/main.rs`(command + handler 註冊)

- [ ] **Step 1: 加 command**

在 main.rs(其他 command 附近)加:
```rust
/// 合併講者:把 merge_ids 的段全改成 keep_id(兩軌)+ 從 speakers.json 移除 merge_ids。
#[tauri::command]
fn merge_speakers(session_id: String, keep_id: String, merge_ids: Vec<String>) -> Result<(), String> {
    let root = crate::session_store::default_meetings_dir().join(&session_id);
    for jsonl_rel in ["transcript/system.segments.jsonl", "transcript/mic-internal.segments.jsonl"] {
        let path = root.join(jsonl_rel);
        let segs = crate::transcribe::read_segments_jsonl(&path);
        if segs.is_empty() {
            continue;
        }
        let relabeled = crate::postprocess::relabel_merge(segs, &keep_id, &merge_ids);
        crate::transcribe::write_segments_jsonl(&path, &relabeled)?;
    }
    let sp_path = root.join("transcript").join("speakers.json");
    let kept = crate::postprocess::drop_speakers(crate::diarize::read_speakers(&sp_path), &merge_ids);
    crate::diarize::write_speakers(&sp_path, &kept)
}
```

- [ ] **Step 2: 註冊 handler**

在 `tauri::generate_handler![...]` 陣列加入 `merge_speakers`。

- [ ] **Step 3: 編譯 + verify**

Run: `cd .. && bash scripts/verify.sh`
Expected: `✓ verify ok`(編得過、測試綠)。

- [ ] **Step 4: Commit**
```bash
git add src-tauri/src/main.rs
git commit -m "feat(diar-fix): merge_speakers command (reassign both tracks + drop speakers.json rows)"
```

---

## Task 3: `set_segment_speaker` command

**Files:** Modify `src-tauri/src/main.rs`(command + handler 註冊)

- [ ] **Step 1: 加 command**
```rust
/// 逐段改講者:把指定 track 的 seg_id 那段 speaker 設成 speaker_id。
#[tauri::command]
fn set_segment_speaker(session_id: String, track: String, seg_id: String, speaker_id: String) -> Result<(), String> {
    let root = crate::session_store::default_meetings_dir().join(&session_id);
    let jsonl_rel = match track.as_str() {
        "system" => "transcript/system.segments.jsonl",
        "mic-internal" => "transcript/mic-internal.segments.jsonl",
        other => return Err(format!("unknown track: {other}")),
    };
    let path = root.join(jsonl_rel);
    let segs = crate::transcribe::read_segments_jsonl(&path);
    let relabeled = crate::postprocess::relabel_one(segs, &seg_id, &speaker_id);
    crate::transcribe::write_segments_jsonl(&path, &relabeled)
}
```

- [ ] **Step 2: 註冊 handler**

在 `generate_handler!` 加入 `set_segment_speaker`。

- [ ] **Step 3: verify + Commit**
```bash
cd .. && bash scripts/verify.sh
git add src-tauri/src/main.rs
git commit -m "feat(diar-fix): set_segment_speaker command (per-segment reassign)"
```

---

## Task 4: 工作區 UI — 合併 / 逐段改 / 重跑確認

**Files:** Modify `src/tabs/SessionWorkspace.tsx`

> 全程沿用既有 `.mori-*` / `var(--*)` token 與 custom `Select`(別寫死顏色、別用原生 `<select>`)。invoke 參數 camelCase。reload = 既有「跑完重載 speakers + transcript」那條。

- [ ] **Step 1: 講者清單多選 + 合併**

講者清單每列加一個勾選框(`useState<Set<string>>` 存被選的 speaker id)。清單上方/下方加「合併所選」按鈕,選 ≥2 個才 enable。按下:`keepId` = 被選中的第一個(以清單順序),`mergeIds` = 其餘:
```tsx
await invoke("merge_speakers", { sessionId, keepId, mergeIds });
// 重載 speakers + transcript,清空選取
```
合併後清單少幾列、逐字稿那些段改顯示 keep 的名字。

- [ ] **Step 2: 逐字稿每段講者下拉**

逐字稿每段前面加一個 custom `Select`(options = 現有講者的 `{value: id, label: display}`,value = `seg.speaker`)。onChange:
```tsx
await invoke("set_segment_speaker", { sessionId, track: seg.track, segId: seg.id, speakerId });
// 重載 transcript
```
(`seg.track` 是 jsonl 內的 `track` 欄,值為 `"system"` / `"mic-internal"`,與 command 的 match 一致。)

- [ ] **Step 3: 重跑分人前確認**

工作區的「分人」按鈕:若這場**已經有 speakers**(已分過,`speakers.length > 0`),按下時先 `window.confirm`(或既有的 inline 確認 UI)顯示「會重新分群,已改的講者名字會重置」,確認才呼 `diarize_session`。沒分過則照舊直接跑。

- [ ] **Step 4: build + verify**

Run: `npm run build`(repo root,TS 要過)然後 `bash scripts/verify.sh`
Expected: build 乾淨、`✓ verify ok`。

- [ ] **Step 5: Commit**
```bash
git add src/tabs/SessionWorkspace.tsx
git commit -m "feat(diar-fix): workspace UI — merge speakers, per-segment reassign, re-run confirm"
```

---

## Self-Review

- **Spec coverage**:合併(Task 1 `relabel_merge` + Task 2 command + Task 4 UI)✓;逐段改(Task 1 `relabel_one` + Task 3 command + Task 4 UI)✓;重跑警告(Task 4 Step 3)✓;`drop_speakers`(Task 1 + Task 2 用)✓。名字轉移(A)/ enrollment 不在範圍(spec §6)✓。
- **Placeholder scan**:無 TODO/TBD;每步有完整程式碼或具體 UI 行為;Task 4 是前端,沿用既有 Select/token 為 codebase pattern(skill 允許)。
- **Type consistency**:`relabel_merge(Vec<Segment>, &str, &[String])` / `relabel_one(Vec<Segment>, &str, &str)` / `drop_speakers(Vec<SpeakerInfo>, &[String])` 三處定義與 command 呼叫一致;command 參數(`session_id/keep_id/merge_ids` 、`session_id/track/seg_id/speaker_id`)對應前端 camelCase(`sessionId/keepId/mergeIds`、`sessionId/track/segId/speakerId`);`Segment.speaker: Option<String>` / `SpeakerInfo{id,display,track}` 既有。
- **原子寫**:沿用 `write_segments_jsonl`(tmp+rename)+ `write_speakers`(tmp+rename),不留資料遺失窗。
