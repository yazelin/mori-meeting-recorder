# PR3 Sessions Tab Cards Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 mori-meeting-recorder 的 Sessions tab 從純 id 列表升級成 mock 05 的卡片版型 — 每張卡顯示 meeting-id / 日期時間 / duration / public+internal segs 標籤 / first-line preview / 開啟按鈕。Rust 端讀 `timeline.json` + `meeting.public.md` + `*.segments.jsonl` 做 summary,前端用 MeetingCard 渲染。

**Architecture:** 在 `session_store.rs` 加 `SessionSummary` struct + `read_session_summary(id, base)` 函式,讀 `timeline.json`(取 started_at / duration_secs)、scan `transcript/*.segments.jsonl`(count visibility)、讀 `meeting.public.md` 第一行非空 body(preview)。新 Tauri command `list_sessions_detailed` 回 `Vec<SessionSummary>`,by started_at desc。前端 SessionsTab 改 invoke 它、map 成 `<MeetingCard>` 元件(grid layout: id+date | seg pills | open),壞掉 session 顯示「資料損毀」灰底版。

**Tech Stack:** Rust(serde_json, chrono),React 18 + TypeScript,Vite。

**Spec reference:** `docs/superpowers/specs/2026-05-28-recorder-ui-mock-alignment-design.md` §7 + §8

**Mock reference:** `docs/design/05-sessions-tab.png`

**Dep on PR1 + PR2:** 兩條都已 merge 進 main。PR1 預埋的 `--seg-pill-public-*` / `--seg-pill-internal-*` token 本 PR 直接用。

---

### Task 0: Branch off main(done)

Branch `feat/sessions-tab-cards` 已建好(off latest main 含 PR1+PR2)。
Plan 檔加 commit 後即進 Task 1。

---

### Task 1: SessionSummary struct + read_session_summary (TDD)

**Files:**
- Modify: `src-tauri/src/session_store.rs`(加 struct + 函式 + inline tests)

- [ ] **Step 1: Add SessionSummary struct + function signature**

在 `src-tauri/src/session_store.rs` 結尾(`#[cfg(test)]` 之前如果有 test mod,否則檔尾)加:

```rust
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SessionSummary {
    pub id: String,
    pub started_at: String,      // ISO 8601 + tz,從 timeline.json
    pub duration_secs: u64,      // 從 timeline.json
    pub public_segs: u32,        // count of visibility == "public" segments
    pub internal_segs: u32,      // count of visibility == "internal" segments
    pub preview: Option<String>, // meeting.public.md 第一行非空 body 文字,<=120 chars
    pub corrupt: bool,           // timeline.json 缺 / parse fail 時為 true
}

pub fn read_session_summary(id: &str, base: &std::path::Path) -> SessionSummary {
    let store = SessionStore { id: id.to_string(), root: base.join(id) };
    let timeline_path = store.timeline_path();
    let timeline_str = match std::fs::read_to_string(&timeline_path) {
        Ok(s) => s,
        Err(_) => {
            return SessionSummary {
                id: id.to_string(),
                started_at: String::new(),
                duration_secs: 0,
                public_segs: 0,
                internal_segs: 0,
                preview: None,
                corrupt: true,
            };
        }
    };
    let v: serde_json::Value = match serde_json::from_str(&timeline_str) {
        Ok(v) => v,
        Err(_) => {
            return SessionSummary {
                id: id.to_string(),
                started_at: String::new(),
                duration_secs: 0,
                public_segs: 0,
                internal_segs: 0,
                preview: None,
                corrupt: true,
            };
        }
    };
    let started_at = v.get("started_at").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let duration_secs = v.get("duration_secs").and_then(|x| x.as_u64()).unwrap_or(0);

    let (public_segs, internal_segs) = count_segments_by_visibility(&store.root);
    let preview = read_public_md_preview(&store.public_md_path());

    SessionSummary {
        id: id.to_string(),
        started_at,
        duration_secs,
        public_segs,
        internal_segs,
        preview,
        corrupt: false,
    }
}

fn count_segments_by_visibility(session_dir: &std::path::Path) -> (u32, u32) {
    let transcript_dir = session_dir.join("transcript");
    let entries = match std::fs::read_dir(&transcript_dir) {
        Ok(e) => e,
        Err(_) => return (0, 0),
    };
    let mut pub_count = 0_u32;
    let mut int_count = 0_u32;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") { continue; }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for line in content.lines() {
            if line.trim().is_empty() { continue; }
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            match v.get("visibility").and_then(|x| x.as_str()) {
                Some("public")   => pub_count += 1,
                Some("internal") => int_count += 1,
                _ => {}
            }
        }
    }
    (pub_count, int_count)
}

fn read_public_md_preview(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    // 跳過 markdown header(以 `#` 開頭 / blockquote `>` / 空行),拿第一行非空 body
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        if trimmed.starts_with('#') { continue; }
        if trimmed.starts_with('>') { continue; }
        if trimmed.starts_with("_(") { return None; }  // "_(no segments)_" placeholder
        let mut s = trimmed.to_string();
        if s.chars().count() > 120 {
            s = s.chars().take(120).collect::<String>() + "…";
        }
        return Some(s);
    }
    None
}
```

- [ ] **Step 2: Add inline tests at end of session_store.rs**

`src-tauri/src/session_store.rs` 結尾既有 `#[cfg(test)] mod tests` 內加(或新加 test mod 若沒既有):

```rust
    use super::*;
    use tempfile::TempDir;

    fn write_file(path: &std::path::Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn read_session_summary_missing_timeline_returns_corrupt() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("meeting-x")).unwrap();
        let s = read_session_summary("meeting-x", tmp.path());
        assert!(s.corrupt);
        assert_eq!(s.id, "meeting-x");
        assert_eq!(s.public_segs, 0);
    }

    #[test]
    fn read_session_summary_happy_path() {
        let tmp = TempDir::new().unwrap();
        let session_dir = tmp.path().join("meeting-x");
        write_file(
            &session_dir.join("timeline.json"),
            r#"{"schema_version":1,"session_id":"meeting-x","started_at":"2026-05-28T14:30:00+08:00","stopped_at":"2026-05-28T15:00:00+08:00","duration_secs":1800,"tracks":[],"exports":{"public":"","internal":"","timeline":""}}"#,
        );
        write_file(
            &session_dir.join("transcript").join("system.segments.jsonl"),
            r#"{"id":"s1","session_id":"meeting-x","track":"system","source_kind":"meeting_system","visibility":"public","start_ms":0,"end_ms":1000,"text":"客戶要求三週後上線","is_final":true}
{"id":"s2","session_id":"meeting-x","track":"system","source_kind":"meeting_system","visibility":"public","start_ms":1000,"end_ms":2000,"text":"再說","is_final":true}
"#,
        );
        write_file(
            &session_dir.join("transcript").join("mic-internal.segments.jsonl"),
            r#"{"id":"m1","session_id":"meeting-x","track":"mic-internal","source_kind":"mic_internal","visibility":"internal","start_ms":500,"end_ms":1500,"text":"內部想法","is_final":true}
"#,
        );
        write_file(
            &session_dir.join("meeting.public.md"),
            "# Meeting Notes — 2026-05-28 14:30\n\n> Source: meeting_system.\n\n客戶要求三週後上線\n再說\n",
        );

        let s = read_session_summary("meeting-x", tmp.path());
        assert!(!s.corrupt);
        assert_eq!(s.id, "meeting-x");
        assert_eq!(s.started_at, "2026-05-28T14:30:00+08:00");
        assert_eq!(s.duration_secs, 1800);
        assert_eq!(s.public_segs, 2);
        assert_eq!(s.internal_segs, 1);
        assert_eq!(s.preview.as_deref(), Some("客戶要求三週後上線"));
    }

    #[test]
    fn read_session_summary_empty_public_md_yields_none_preview() {
        let tmp = TempDir::new().unwrap();
        let session_dir = tmp.path().join("m");
        write_file(
            &session_dir.join("timeline.json"),
            r#"{"schema_version":1,"session_id":"m","started_at":"2026-05-28T14:30:00+08:00","stopped_at":"2026-05-28T14:30:01+08:00","duration_secs":1,"tracks":[],"exports":{"public":"","internal":"","timeline":""}}"#,
        );
        write_file(
            &session_dir.join("meeting.public.md"),
            "# Meeting Notes — empty\n\n> Source: meeting_system.\n\n_(no segments)_\n",
        );
        let s = read_session_summary("m", tmp.path());
        assert!(!s.corrupt);
        assert_eq!(s.preview, None);
    }
```

- [ ] **Step 3: Add tempfile dev-dependency if missing**

Run: `grep -E "tempfile" src-tauri/Cargo.toml`

如果**沒有**(其他 test 應該有,但確認),在 `[dev-dependencies]` block 加:

```toml
tempfile = "3"
```

- [ ] **Step 4: Run tests to verify**

```bash
cd src-tauri && cargo test session_store 2>&1 | tail -10
```

Expected: 3 new tests + 既有 session_store tests 全綠。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/session_store.rs src-tauri/Cargo.toml
git commit -m "feat(session-store): SessionSummary + read_session_summary (TDD)"
```

---

### Task 2: list_sessions_detailed Tauri command

**Files:**
- Modify: `src-tauri/src/main.rs`(加 command + register in invoke_handler)

- [ ] **Step 1: Add command**

在 `src-tauri/src/main.rs` 找到既有 `fn list_sessions() -> Vec<String>` 上方或下方,加:

```rust
#[tauri::command]
fn list_sessions_detailed() -> Vec<session_store::SessionSummary> {
    let dir = session_store::default_meetings_dir();
    let mut summaries: Vec<session_store::SessionSummary> = std::fs::read_dir(&dir)
        .ok()
        .map(|it| {
            it.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if !name.starts_with("meeting-") { return None; }
                    if !e.path().is_dir() { return None; }
                    Some(session_store::read_session_summary(&name, &dir))
                })
                .collect()
        })
        .unwrap_or_default();
    // Sort newest first by started_at desc;corrupt 排到尾巴。
    summaries.sort_by(|a, b| {
        match (a.corrupt, b.corrupt) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => b.started_at.cmp(&a.started_at),
        }
    });
    summaries
}
```

- [ ] **Step 2: Register command in invoke_handler**

找到 `tauri::generate_handler![...]`(約 line 163),把 `list_sessions_detailed,` 加進去(放在 `list_sessions,` 旁邊):

```rust
            list_sessions,
            list_sessions_detailed,
```

- [ ] **Step 3: Verify**

```bash
cd src-tauri && cargo check --all-targets
cd src-tauri && cargo test 2>&1 | tail -5
```

兩個都過。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "feat(commands): list_sessions_detailed Tauri command"
```

---

### Task 3: SegPill component + CSS

**Files:**
- Create: `src/components/SegPill.tsx`
- Modify: `src/theme.css`(加 `.seg-pill` rules)

- [ ] **Step 1: Add SegPill CSS to theme.css**

Append to END of `src/theme.css`:

```css

/* Sessions tab 卡片的 public/internal segs 標籤。 */
.seg-pill {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 500;
  white-space: nowrap;
}
.seg-pill[data-tone="public"]   { background: var(--seg-pill-public-bg);   color: var(--seg-pill-public-fg); }
.seg-pill[data-tone="internal"] { background: var(--seg-pill-internal-bg); color: var(--seg-pill-internal-fg); }
```

- [ ] **Step 2: Create SegPill component**

```tsx
// src/components/SegPill.tsx
//
// Sessions tab 卡片右上角的 segs 標籤 — public/internal 各一,顯示「public: 142 segs」。

type Tone = "public" | "internal";

interface Props {
  tone: Tone;
  count: number;
}

const LABEL: Record<Tone, string> = { public: "public", internal: "internal" };

export default function SegPill({ tone, count }: Props) {
  return (
    <span className="seg-pill" data-tone={tone}>
      {LABEL[tone]}: {count} segs
    </span>
  );
}
```

- [ ] **Step 3: Verify + commit**

```bash
npm run build
git add src/components/SegPill.tsx src/theme.css
git commit -m "feat(sessions-tab): SegPill component for public/internal segs"
```

---

### Task 4: MeetingCard component + CSS

**Files:**
- Create: `src/components/MeetingCard.tsx`
- Modify: `src/theme.css`(加 `.meeting-card` rules)

- [ ] **Step 1: Append meeting-card CSS**

```css

/* Sessions tab 一張會議卡。Grid:身體 | seg-pills | open-icon。 */
.meeting-card {
  display: grid;
  grid-template-columns: 1fr auto auto;
  gap: 10px 14px;
  align-items: center;
  padding: 12px 14px;
  border-radius: 12px;
  border: 0.5px solid var(--border);
  background: rgba(255,255,255,0.02);
  cursor: pointer;
  margin-bottom: 6px;
  transition: background 0.15s;
}
.meeting-card:hover { background: var(--hover); }
.meeting-card.corrupt { background: rgba(255,99,99,0.06); cursor: default; }
.meeting-card.corrupt:hover { background: rgba(255,99,99,0.08); }

.meeting-card .mc-body {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}
.meeting-card .mc-id {
  font-family: ui-monospace, "SF Mono", "Cascadia Code", "Consolas", monospace;
  font-size: 11px;
  color: var(--text);
  font-weight: 500;
}
.meeting-card .mc-subtitle {
  font-size: 11px;
  color: var(--text-secondary);
}
.meeting-card .mc-preview {
  font-size: 11px;
  color: var(--text-secondary);
  margin-top: 2px;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.meeting-card .mc-corrupt-tag {
  font-size: 10px;
  color: var(--danger-color);
  font-weight: 500;
}
.meeting-card .mc-pills {
  display: flex;
  flex-direction: column;
  gap: 4px;
  align-items: flex-end;
}
.meeting-card .mc-open {
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--icon-color);
  background: transparent;
  border: none;
  border-radius: 8px;
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
}
.meeting-card .mc-open:hover { background: var(--btn-bg-hover); color: var(--icon-hover); }
```

- [ ] **Step 2: Create MeetingCard component**

```tsx
// src/components/MeetingCard.tsx
//
// Sessions tab 一張會議卡。對應 mock 05 的版型。
// 點卡身或 ↗ 都會 open session folder;corrupt session 只顯示 id + 警告,不能 open。

import SegPill from "./SegPill";

interface SessionSummary {
  id: string;
  started_at: string;
  duration_secs: number;
  public_segs: number;
  internal_segs: number;
  preview: string | null;
  corrupt: boolean;
}

interface Props {
  summary: SessionSummary;
  onOpen: (id: string) => void;
}

function fmtStartedAt(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const yyyy = d.getFullYear();
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  const hh = String(d.getHours()).padStart(2, "0");
  const mi = String(d.getMinutes()).padStart(2, "0");
  return `${yyyy}-${mm}-${dd} ${hh}:${mi}`;
}

function fmtDuration(secs: number): string {
  if (secs === 0) return "0s";
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

export default function MeetingCard({ summary, onOpen }: Props) {
  if (summary.corrupt) {
    return (
      <div className="meeting-card corrupt" onClick={(e) => e.stopPropagation()}>
        <div className="mc-body">
          <span className="mc-id">{summary.id}</span>
          <span className="mc-corrupt-tag">⚠ 資料損毀(無 timeline.json)</span>
        </div>
        <div className="mc-pills" />
        <button className="mc-open" disabled title="無法開啟損毀的 session">↗</button>
      </div>
    );
  }

  const open = () => onOpen(summary.id);
  return (
    <div className="meeting-card" onClick={open}>
      <div className="mc-body">
        <span className="mc-id">{summary.id}</span>
        <span className="mc-subtitle">
          {fmtStartedAt(summary.started_at)} · {fmtDuration(summary.duration_secs)}
        </span>
        {summary.preview ? (
          <span className="mc-preview">{summary.preview}</span>
        ) : (
          <span className="mc-preview" style={{ fontStyle: "italic", color: "var(--text-dim)" }}>(無公開內容)</span>
        )}
      </div>
      <div className="mc-pills">
        <SegPill tone="public"   count={summary.public_segs} />
        <SegPill tone="internal" count={summary.internal_segs} />
      </div>
      <button
        className="mc-open"
        onClick={(e) => { e.stopPropagation(); open(); }}
        title="開啟資料夾"
      >↗</button>
    </div>
  );
}
```

- [ ] **Step 3: Verify + commit**

```bash
npm run build
git add src/components/MeetingCard.tsx src/theme.css
git commit -m "feat(sessions-tab): MeetingCard component"
```

---

### Task 5: SessionsTab refactor

**Files:**
- Modify: `src/tabs/SessionsTab.tsx`

- [ ] **Step 1: Overwrite SessionsTab.tsx**

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import MeetingCard from "../components/MeetingCard";

interface SessionSummary {
  id: string;
  started_at: string;
  duration_secs: number;
  public_segs: number;
  internal_segs: number;
  preview: string | null;
  corrupt: boolean;
}

export default function SessionsTab() {
  const { t } = useTranslation();
  const [summaries, setSummaries] = useState<SessionSummary[] | null>(null);

  useEffect(() => {
    invoke<SessionSummary[]>("list_sessions_detailed")
      .then(setSummaries)
      .catch(() => setSummaries([]));
  }, []);

  const onOpen = async (id: string) => {
    try { await invoke("open_session_dir", { sessionId: id }); } catch {}
  };

  return (
    <div>
      <h3 style={{ marginTop: 0 }}>{t("sessions.title")}</h3>
      <p style={{ fontSize: 11, color: "var(--text-dim)", marginBottom: 12 }}>{t("sessions.hint")}</p>
      {summaries === null ? (
        <div style={{ color: "var(--text-dim)" }}>讀取中…</div>
      ) : summaries.length === 0 ? (
        <div style={{ color: "var(--text-dim)" }}>{t("sessions.empty")}</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column" }}>
          {summaries.map((s) => (
            <MeetingCard key={s.id} summary={s} onOpen={onOpen} />
          ))}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Verify build + tsc**

```bash
npm run build
npx tsc --noEmit
```

兩個都過。

- [ ] **Step 3: Commit**

```bash
git add src/tabs/SessionsTab.tsx
git commit -m "feat(sessions-tab): use list_sessions_detailed + MeetingCard"
```

---

### Task 6: Run scripts/verify.sh

```bash
bash scripts/verify.sh
```

Expected 全綠 — cargo test 包括 Task 1 新加的 3 個 session_store tests。

---

### Task 7: Manual visual e2e

跑 `npm run tauri dev` → 雙擊膠囊展開 → 切到 Sessions tab → 對 mock 05 驗:

- 若 `~/.mori/meetings/` 有既有 session → 顯示卡片(id / 日期時間+duration / preview / public+internal segs / ↗)
- 若沒 session → 顯示 `sessions.empty` 字
- **手動造一個壞 session**(eg `mkdir -p ~/.mori/meetings/meeting-fake-broken` 不放 timeline.json)→ 應顯示「⚠ 資料損毀」灰底卡
- 點卡 / 點 ↗ → 開啟資料夾(走 `open_session_dir`)

OK / 哪個不對 → 回我。

---

### Task 8: Push + open PR + auto-merge

```bash
git push -u origin feat/sessions-tab-cards

gh pr create --title "feat(sessions-tab): MeetingCard layout (PR3 of 3)" --body "$(cat <<'BODYEOF'
PR3 of the 3-PR recorder UI mock alignment series — completes the 5-mock alignment.

## Summary

Spec: \`docs/superpowers/specs/2026-05-28-recorder-ui-mock-alignment-design.md\` §7.

### Rust
- \`session_store::SessionSummary\` struct + \`read_session_summary(id, base)\` function with 3 TDD unit tests(missing timeline / happy path / empty public.md)
- Reads \`timeline.json\` for started_at + duration_secs, scans \`transcript/*.segments.jsonl\` for public/internal seg counts, parses \`meeting.public.md\` for first-line preview
- New Tauri command \`list_sessions_detailed\` returns \`Vec<SessionSummary>\`, sorted newest-first, corrupt sessions to bottom
- \`list_sessions\` (returns Vec<String>) kept for backward compat, not used by frontend any more

### Frontend
- \`SegPill\` component using PR1's pre-emptive \`--seg-pill-*\` tokens (public 綠 / internal 黃)
- \`MeetingCard\` component:grid layout 身體 | seg pills | open icon,corrupt 灰底版,empty preview 顯示 "(無公開內容)" 灰字
- \`SessionsTab\` refactored:invoke \`list_sessions_detailed\` → map to MeetingCard;loading state + empty state + 既有 i18n key

### 視覺契約
對齊 \`docs/design/05-sessions-tab.png\`。

### 刻意 defer
- Sessions tab 內 inline preview transcript 全文 — 卡片只顯示 first-line,點開資料夾看
- 卡片刪除 / 重命名 — phase 2
- Pagination — 假設 session 數量 < 100,phase 2 才做

## Test plan
- [x] \`bash scripts/verify.sh\` 全綠(3 新 + 既有 cargo test)
- [x] Manual e2e:既有 session 卡片 / empty state / 損毀 session 灰底 / 點卡開資料夾

🤖 Generated with [Claude Code](https://claude.com/claude-code)
BODYEOF
)"

gh pr merge --auto --squash
gh pr view --json url --jq .url
```

Output PR URL。

---

## Self-Review Notes

| Spec §7 元素 | 任務對應 |
|---|---|
| `SessionSummary` Rust struct | Task 1 |
| `read_session_summary` 含 corrupt path | Task 1 |
| `list_sessions_detailed` command | Task 2 |
| SegPill 雙色 | Task 3 |
| MeetingCard grid layout | Task 4 |
| Empty preview / corrupt 視覺 | Task 4 |
| SessionsTab 用新 command | Task 5 |
| Mock 05 對齊 | Task 7 |

**Branch / PR 命名**:`feat/sessions-tab-cards` 對齊 `[[mori-branch-naming]]`。

**Mori voice safety**:動 Rust(session_store + main.rs),需 `bash scripts/restart-dev.sh`(實機開 dev 時),不影響 mori-desktop。

**未在此 PR 做的**:
- Sessions tab 內 inline 預覽 / 編輯(phase 2)
- 刪除 / 重命名 session(phase 2)
- Pagination(< 100 session 假設,phase 2)
