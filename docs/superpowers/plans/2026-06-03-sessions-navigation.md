# Sessions 列表導覽優化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sessions 分頁可搜尋(主題 + 逐字稿內文)、依整理狀態篩選、卡片顯示主題 + 整理徽章、手動標記整理完成。

**Architecture:** `SessionSummary` 加 `topic`(讀 meeting-info.json)+ `organized`(讀獨立 session-state.json)。新增可測核心函式 `write_organized` / `search_fulltext`(session_store.rs,吃 base path)+ 薄 Tauri 命令(main.rs)。前端做即時 metadata 篩選(主題/狀態),逐字稿全文走後端命令。

**Tech Stack:** Rust(serde_json / std::fs)、Tauri v2 command、React + TS。

**Spec:** `docs/superpowers/specs/2026-06-03-sessions-navigation-design.md`

**Worktree / branch:** `/home/ct/mori-universe/.worktrees/recorder-sessions-nav` @ `feat/sessions-nav`(off origin/main `84da7a2`)。

⚠ cargo 在 `src-tauri/` 內跑;先 `npm run build`(generate_context 需 dist)再 cargo。手測 `npm run tauri dev`(動 Rust 要重啟)。

---

## File Structure

| 檔案 | 動作 | 責任 |
|---|---|---|
| `src-tauri/src/session_store.rs` | Modify | `SessionSummary` +topic/organized;read_session_summary 讀兩檔;`write_organized` / `search_fulltext` 可測核心 + 測試 |
| `src-tauri/src/main.rs` | Modify | `set_session_organized` / `search_sessions_fulltext` 命令 + 註冊 |
| `src/components/MeetingCard.tsx` | Modify | 顯示主題 + 整理徽章 + 標記完成鈕 |
| `src/tabs/SessionsTab.tsx` | Modify | 搜尋框 + 全文 checkbox + 狀態 chips + 篩選 + organized 切換 |

---

## Task 1: `SessionSummary` 加 topic/organized + read_session_summary 讀兩檔

**Files:** Modify `src-tauri/src/session_store.rs`

- [ ] **Step 1: 寫失敗測試**

在 `session_store.rs` 的 `#[cfg(test)] mod tests`(檔尾)內加:
```rust
    #[test]
    fn read_session_summary_includes_topic_and_organized() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let root = base.join("meeting-x");
        std::fs::create_dir_all(root.join("transcript")).unwrap();
        std::fs::write(root.join("timeline.json"),
            r#"{"schema_version":1,"session_id":"meeting-x","started_at":"2026-06-03T10:00:00+08:00","stopped_at":"t","duration_secs":60,"tracks":[],"exports":{"public":"","internal":""}}"#).unwrap();
        std::fs::write(root.join("meeting-info.json"), r#"{"topic":"季度檢討","participants":"甲,乙"}"#).unwrap();
        std::fs::write(root.join("session-state.json"), r#"{"organized":true}"#).unwrap();

        let s = read_session_summary("meeting-x", base);
        assert_eq!(s.topic, "季度檢討");
        assert!(s.organized);
        assert!(!s.corrupt);
    }

    #[test]
    fn read_session_summary_defaults_when_info_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let root = base.join("meeting-y");
        std::fs::create_dir_all(root.join("transcript")).unwrap();
        std::fs::write(root.join("timeline.json"),
            r#"{"schema_version":1,"session_id":"meeting-y","started_at":"t","stopped_at":"t","duration_secs":1,"tracks":[],"exports":{"public":"","internal":""}}"#).unwrap();
        // 無 meeting-info.json / session-state.json
        let s = read_session_summary("meeting-y", base);
        assert_eq!(s.topic, "");
        assert!(!s.organized);
    }
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-sessions-nav/src-tauri && cargo test read_session_summary_includes_topic 2>&1 | tail -12`
Expected: 編譯失敗(SessionSummary 無 topic/organized 欄位)。

- [ ] **Step 3: SessionSummary 加欄位**

`session_store.rs` 的 `SessionSummary`(`:65-74`)在 `corrupt: bool,` 之前加:
```rust
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub organized: bool,
```

- [ ] **Step 4: 三處 SessionSummary 建構補欄位**

read_session_summary 有三處建構,都要補 `topic` / `organized`:
(a) timeline 讀檔失敗的 corrupt 回傳(`:82-90` 區,`corrupt: true` 那個)→ 在 `preview: None,` 後加:
```rust
                topic: String::new(),
                organized: false,
```
(b) timeline parse 失敗的 corrupt 回傳(`:96-104` 區,另一個 `corrupt: true`)→ 同樣在 `preview: None,` 後加:
```rust
                topic: String::new(),
                organized: false,
```
(c) 正常回傳(`:119-127`):先在 `SessionSummary {` 之前(`:118` 空行後)加讀取:
```rust
    let topic = std::fs::read_to_string(store.root.join("meeting-info.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("topic").and_then(|x| x.as_str()).map(|s| s.to_string()))
        .unwrap_or_default();
    let organized = std::fs::read_to_string(store.root.join("session-state.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("organized").and_then(|x| x.as_bool()))
        .unwrap_or(false);
```
再於正常 `SessionSummary { … corrupt: false, }` 內 `corrupt: false,` 之前加:
```rust
        topic,
        organized,
```

- [ ] **Step 5: 跑測試確認通過 + commit**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-sessions-nav/src-tauri && cargo test read_session_summary 2>&1 | tail -12`
Expected: 兩個新測試 PASS;既有 session_store 測試仍綠。
```bash
cd /home/ct/mori-universe/.worktrees/recorder-sessions-nav
git add src-tauri/src/session_store.rs
git commit -m "feat(recorder): SessionSummary +topic/organized(讀 meeting-info + session-state.json)"
```

---

## Task 2: `write_organized` + `search_fulltext` 可測核心

**Files:** Modify `src-tauri/src/session_store.rs`

- [ ] **Step 1: 寫失敗測試**

在 `session_store.rs` tests 內加:
```rust
    #[test]
    fn write_organized_roundtrips_and_keeps_topic() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let root = base.join("meeting-z");
        std::fs::create_dir_all(root.join("transcript")).unwrap();
        std::fs::write(root.join("timeline.json"),
            r#"{"schema_version":1,"session_id":"meeting-z","started_at":"t","stopped_at":"t","duration_secs":1,"tracks":[],"exports":{"public":"","internal":""}}"#).unwrap();
        std::fs::write(root.join("meeting-info.json"), r#"{"topic":"專案A","participants":""}"#).unwrap();

        write_organized(&root, true).unwrap();
        let s = read_session_summary("meeting-z", base);
        assert!(s.organized);
        assert_eq!(s.topic, "專案A"); // 分檔,topic 不受影響

        write_organized(&root, false).unwrap();
        assert!(!read_session_summary("meeting-z", base).organized);
    }

    #[test]
    fn search_fulltext_matches_md_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        for (id, md, text) in [
            ("meeting-a", "meeting.public.md", "討論 Roadmap 與排程"),
            ("meeting-b", "meeting.md", "現場閒聊"),
        ] {
            let root = base.join(id);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join(md), text).unwrap();
        }
        let hits = search_fulltext(base, "roadmap"); // 小寫 query 命中大寫 Roadmap
        assert_eq!(hits, vec!["meeting-a".to_string()]);
        assert!(search_fulltext(base, "不存在的字").is_empty());
        assert!(search_fulltext(base, "   ").is_empty()); // 空白 query
    }
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-sessions-nav/src-tauri && cargo test -- write_organized search_fulltext 2>&1 | tail -12`
Expected: 編譯失敗(函式不存在)。

- [ ] **Step 3: 實作兩個核心函式**

在 `session_store.rs`(`read_session_summary` 之後)加:
```rust
/// 寫該場 session-state.json 的 organized 旗標(獨立檔,不碰 meeting-info.json)。
pub fn write_organized(session_root: &std::path::Path, organized: bool) -> Result<(), String> {
    let body = serde_json::to_string_pretty(&serde_json::json!({ "organized": organized }))
        .map_err(|e| e.to_string())?;
    std::fs::write(session_root.join("session-state.json"), body)
        .map_err(|e| format!("write session-state.json: {e}"))
}

/// 全文搜尋:掃 meetings_dir 下每場匯出的逐字稿 md(meeting.md / public / internal),
/// query(去空白、小寫)子字串命中 → 收 session id。空 query → 空清單。讀檔失敗該場跳過。
pub fn search_fulltext(meetings_dir: &std::path::Path, query: &str) -> Vec<String> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    let Ok(rd) = std::fs::read_dir(meetings_dir) else { return hits };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("meeting-") || !entry.path().is_dir() {
            continue;
        }
        let root = entry.path();
        for md in ["meeting.md", "meeting.public.md", "meeting.internal.md"] {
            if let Ok(text) = std::fs::read_to_string(root.join(md)) {
                if text.to_lowercase().contains(&q) {
                    hits.push(name.clone());
                    break;
                }
            }
        }
    }
    hits.sort();
    hits
}
```

- [ ] **Step 4: 跑測試確認通過 + commit**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-sessions-nav/src-tauri && cargo test -- write_organized search_fulltext 2>&1 | tail -12`
Expected: PASS。
```bash
cd /home/ct/mori-universe/.worktrees/recorder-sessions-nav
git add src-tauri/src/session_store.rs
git commit -m "feat(recorder): write_organized + search_fulltext 可測核心(session_store)"
```

---

## Task 3: Tauri 命令 `set_session_organized` + `search_sessions_fulltext`

**Files:** Modify `src-tauri/src/main.rs`（命令加在 `list_sessions_detailed` 附近;註冊在 `generate_handler!` `:913`）

- [ ] **Step 1: 加兩個命令**

在 `main.rs` 的 `list_sessions_detailed`(`:382`)函式之後加:
```rust
/// 手動標記某場「整理完成」狀態 → 寫該場獨立 session-state.json。
#[tauri::command]
fn set_session_organized(session_id: String, organized: bool) -> Result<(), String> {
    let root = session_store::default_meetings_dir().join(&session_id);
    session_store::write_organized(&root, organized)
}

/// 逐字稿內文全文搜尋,回命中的 session id 清單。
#[tauri::command]
fn search_sessions_fulltext(query: String) -> Vec<String> {
    session_store::search_fulltext(&session_store::default_meetings_dir(), &query)
}
```

- [ ] **Step 2: 註冊進 generate_handler!**

在 `main.rs` 的 `generate_handler!`(`:913` 起)清單,`list_sessions_detailed,`(`:934`)之後加:
```rust
            set_session_organized,
            search_sessions_fulltext,
```

- [ ] **Step 3: 編譯 + commit**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-sessions-nav/src-tauri && cargo check 2>&1 | tail -6`
Expected: 通過。
```bash
cd /home/ct/mori-universe/.worktrees/recorder-sessions-nav
git add src-tauri/src/main.rs
git commit -m "feat(recorder): set_session_organized + search_sessions_fulltext 命令 + 註冊"
```

---

## Task 4: `MeetingCard` 顯示主題 + 整理徽章 + 標記鈕

**Files:** Modify `src/components/MeetingCard.tsx`

> 沿用既有 class / token,不寫死色值。

- [ ] **Step 1: 介面加欄位 + prop**

`MeetingCard.tsx` 的 `interface SessionSummary`(`:8-16`)在 `corrupt: boolean;` 之前加:
```tsx
  topic: string;
  organized: boolean;
```
`interface Props`(`:18-22`)在 `onWorkspace?` 之後加:
```tsx
  onToggleOrganized?: (id: string, next: boolean) => void;
```

- [ ] **Step 2: 顯示主題(mc-body 內,id 之後)**

在 `<span className="mc-id">{summary.id}</span>`(`:68`)之後加:
```tsx
        {summary.topic && <span className="mc-topic" style={{ fontWeight: 600 }}>{summary.topic}</span>}
```

- [ ] **Step 3: 徽章 + 標記鈕(右側按鈕欄,整理鈕之後)**

在右側欄(`:82-96`)的「整理」按鈕區塊之後、`mc-open`(↗)之前加:
```tsx
        <span
          className={`mmr-pill ${summary.organized ? "on" : ""}`}
          style={{ fontSize: 10, padding: "2px 6px", color: summary.organized ? "var(--found-color)" : "var(--text-dim)" }}
        >{summary.organized ? "已整理 ✓" : "未整理"}</span>
        {onToggleOrganized && (
          <button
            className="mmr-btn"
            style={{ fontSize: 10.5, padding: "2px 6px" }}
            onClick={(e) => { e.stopPropagation(); onToggleOrganized(summary.id, !summary.organized); }}
            title={summary.organized ? "取消整理完成標記" : "標記整理完成"}
          >{summary.organized ? "取消" : "標記完成"}</button>
        )}
```

- [ ] **Step 4: build 確認 + commit**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-sessions-nav && npm run build 2>&1 | tail -5`
Expected: tsc/vite 無錯。
```bash
cd /home/ct/mori-universe/.worktrees/recorder-sessions-nav
git add src/components/MeetingCard.tsx
git commit -m "feat(recorder): MeetingCard 顯示主題 + 整理徽章 + 標記完成鈕"
```

---

## Task 5: `SessionsTab` 搜尋 / 篩選 / 狀態 + organized 切換

**Files:** Modify `src/tabs/SessionsTab.tsx`

- [ ] **Step 1: 介面加欄位**

`SessionsTab.tsx` 的 `interface SessionSummary`(`:7-15`)在 `corrupt: boolean;` 之前加:
```tsx
  topic: string;
  organized: boolean;
```

- [ ] **Step 2: 加搜尋/篩選 state + debounce 全文 + organized 切換**

把元件內 `const [openId, setOpenId] = useState<string | null>(null);` 之後加:
```tsx
  const [query, setQuery] = useState("");
  const [fulltext, setFulltext] = useState(false);
  const [statusFilter, setStatusFilter] = useState<"all" | "organized" | "unorganized">("all");
  const [fulltextIds, setFulltextIds] = useState<Set<string> | null>(null);

  // 全文搜尋:開啟且有 query 時 debounce 呼後端;否則清掉(只走主題)。
  useEffect(() => {
    if (!fulltext || query.trim() === "") { setFulltextIds(null); return; }
    const id = setTimeout(async () => {
      try {
        const ids = await invoke<string[]>("search_sessions_fulltext", { query });
        setFulltextIds(new Set(ids));
      } catch { setFulltextIds(new Set()); }
    }, 300);
    return () => clearTimeout(id);
  }, [query, fulltext]);

  const toggleOrganized = async (id: string, next: boolean) => {
    try {
      await invoke("set_session_organized", { sessionId: id, organized: next });
      setSummaries((prev) => prev?.map((s) => (s.id === id ? { ...s, organized: next } : s)) ?? prev);
    } catch (e) { console.error(e); }
  };

  const visible = (summaries ?? []).filter((s) => {
    const q = query.trim().toLowerCase();
    const textOk = q === ""
      ? true
      : s.topic.toLowerCase().includes(q) || (fulltext && (fulltextIds?.has(s.id) ?? false));
    const statusOk = statusFilter === "all"
      ? true
      : statusFilter === "organized" ? s.organized : !s.organized;
    return textOk && statusOk;
  });
```

- [ ] **Step 3: 加搜尋/篩選列 UI(標題 hint 之後、清單之前)**

在 `<p ...>{t("sessions.hint")}</p>` 之後加:
```tsx
      <div style={{ display: "flex", flexWrap: "wrap", gap: 8, alignItems: "center", marginBottom: 10 }}>
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜尋主題…"
          style={{ flex: 1, minWidth: 140, fontSize: 12, padding: "4px 8px" }}
        />
        <label style={{ fontSize: 11, color: "var(--text-secondary)", display: "flex", gap: 4, alignItems: "center" }}>
          <input type="checkbox" checked={fulltext} onChange={(e) => setFulltext(e.target.checked)} />
          含逐字稿內文
        </label>
        {(["all", "organized", "unorganized"] as const).map((v) => (
          <button
            key={v}
            className={`mmr-btn${statusFilter === v ? " primary" : ""}`}
            style={{ fontSize: 11, padding: "3px 8px" }}
            onClick={() => setStatusFilter(v)}
          >{v === "all" ? "全部" : v === "organized" ? "已整理" : "未整理"}</button>
        ))}
      </div>
```

- [ ] **Step 4: render 改用 `visible` + 傳 onToggleOrganized**

把清單 render(`summaries.map(...)`,`:46-48`)改成用 `visible` 並傳新 prop:
```tsx
        <div style={{ display: "flex", flexDirection: "column" }}>
          {visible.map((s) => (
            <MeetingCard key={s.id} summary={s} onOpen={onOpen} onWorkspace={s.corrupt ? undefined : setOpenId} onToggleOrganized={s.corrupt ? undefined : toggleOrganized} />
          ))}
        </div>
```
(空清單判斷 `summaries.length === 0` 維持;另可選:`visible.length === 0 && summaries.length > 0` 時顯示「無符合的會議」,非必要。)

- [ ] **Step 5: build 確認 + commit**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-sessions-nav && npm run build 2>&1 | tail -5`
Expected: tsc/vite 無錯。
```bash
cd /home/ct/mori-universe/.worktrees/recorder-sessions-nav
git add src/tabs/SessionsTab.tsx
git commit -m "feat(recorder): SessionsTab 搜尋(主題+全文)/狀態篩選/標記整理"
```

---

## Task 6: 全量驗證 + 真機手測 + PR

- [ ] **Step 1: verify.sh 全綠**

Run:
```bash
cd /home/ct/mori-universe/.worktrees/recorder-sessions-nav
npm run build && bash scripts/verify.sh 2>&1 | tail -20
```
Expected: cargo test 全 PASS(含 read_session_summary topic/organized、write_organized、search_fulltext 新測 + 既有回歸)、npm build、cargo check 乾淨。

- [ ] **Step 2: 真機手測(`npm run tauri dev`)**

- Sessions 分頁頂部出現搜尋框 + 「含逐字稿內文」+ 狀態 chips。
- 卡片顯示**會議主題**(有填過 topic 的場次)+ 「已整理/未整理」徽章 + 「標記完成」鈕。
- 打主題關鍵字 → 清單即時縮。
- 勾「含逐字稿內文」+ 打逐字稿裡的字 → 命中該場(主題沒提到也搜得到)。
- 狀態 chips 切「已整理 / 未整理」→ 正確篩。
- 按「標記完成」→ 徽章變已整理、`~/.mori/meetings/<id>/session-state.json` 出現 `{"organized":true}`;重開分頁仍記得。
- meeting-info.json 的 topic/participants 不受標記影響(分檔)。

- [ ] **Step 3: push + PR(auto-merge)**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-sessions-nav
git push -u origin feat/sessions-nav
gh pr create --fill --base main --head feat/sessions-nav
gh pr merge --auto --squash
```

- [ ] **Step 4: worktree 清理(merge 後)**

```bash
cd /home/ct/mori-universe/mori-meeting-recorder
git worktree remove /home/ct/mori-universe/.worktrees/recorder-sessions-nav
```

---

## Self-Review

**Spec coverage**(對 `2026-06-03-sessions-navigation-design.md`):
- 範圍#1 手動整理狀態 → Task 2 write_organized + Task 3 命令 + Task 4 標記鈕 + Task 5 切換。✅
- 範圍#2 搜尋主題+日期+狀態+全文 → Task 5(主題即時 / 全文 checkbox→後端 / 狀態 chips / 日期沿用排序)+ Task 2 search_fulltext。✅
- 範圍#3 做法 A(前端 metadata + 後端全文)→ Task 5 前端篩選 + Task 2/3 後端全文。✅
- 範圍#4 卡片主題+徽章+鈕 → Task 4。✅
- 範圍#5 維持點卡進 workspace → 未動 onWorkspace 流程。✅
- 架構(SessionSummary +topic/organized 讀兩檔 / write_organized / search_fulltext / 命令 / UI)→ Task 1-5。✅
- organized 分檔(session-state.json,不碰 meeting-info writers)→ Task 1 讀 / Task 2 write_organized 寫,皆獨立檔。✅
- 回歸(既有 summary 欄位/排序/corrupt 不變)→ Task 1 只加欄位 + corrupt 路徑補預設;既有測試應仍綠。✅

**Placeholder scan:** 無 TBD/TODO;每 code step 有完整 code。✅

**Type consistency:**
- `SessionSummary` +`topic: String`/`organized: bool`(Task 1 Rust)= 前端 interface(Task 4 MeetingCard / Task 5 SessionsTab,`topic: string`/`organized: boolean`)一致;serde 預設讓舊資料 graceful。✅
- `write_organized(&Path, bool)` / `search_fulltext(&Path, &str)->Vec<String>`(Task 2)= main.rs 命令呼叫(Task 3)一致。✅
- 命令 `set_session_organized(session_id, organized)` / `search_sessions_fulltext(query)`(Task 3)= 前端 `invoke("set_session_organized",{sessionId,organized})` / `invoke("search_sessions_fulltext",{query})`(Task 5)一致(Tauri v2 auto-camelCase:`session_id`→`sessionId`)。✅
- `onToggleOrganized?(id, next)`(Task 4 prop)= Task 5 `toggleOrganized(id, next)` 簽名一致。✅
