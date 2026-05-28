# Recorder UI — Mock Alignment Design

**Date**: 2026-05-28
**Status**: Spec draft pending user review
**Scope**: mori-meeting-recorder — 把 `docs/design/` 中 6 張 mock 落地成可運行 UI

## 0. 為什麼有這份 spec

BI-5 phase 1 ship 時前端 scaffold(CapsuleView / ExpandedView / RecordTab / SessionsTab / DepsTab)已接好基本功能,但**視覺與 mock 有落差**:
- Record tab 中央只有一顆大 Start/Stop,沒 VU meter
- Sessions tab 只有 `📁 meeting-id` 純文字列,沒卡片版型 / 元資料 / segs 標籤
- 膠囊 icon 是字符 `▶`/`■`,mock 已升級為 SVG 形狀;recording 狀態的色溫感缺

這次目標:**把 6 張 mock 完整落地**,並把每個視覺元素抽成 theme.css token 沿用既有慣例。

Mocks 是 source of truth:`docs/design/01-recording-capsule.png` … `06-deps-tab.png`。

## 1. 範圍紀律

### Build now

- PR1:膠囊 1:1 視覺收(SVG icon、內光暈、theme token、SYS/MIC pill 圖示)
- PR2:Record tab — 控制列 layout + 雙軌 VU meter(Rust audio level 計算 + Tauri event + React component)
- PR3:Sessions tab — 卡片版型 + 元資料(`list_sessions_detailed` command 讀 `timeline.json` + `meeting.public.md`)

### 刻意 defer(phase 2+)

- Mute MIC 副按鈕(本 phase 純照 mock,不加新動作)
- Pause / Resume(會引入錄音分段拼接後端複雜度)
- Mark moment(timeline.json 沒這欄位)
- Live captions
- mix-preview / mori-desktop RecorderTab / handoff
- macOS 支援

### 不做(non-goals)

- Visual regression CI(playwright + screenshot diff)— 太重,YAGNI
- Frontend lint(eslint / prettier)— 不在此範圍
- Sessions tab 內 inline 預覽 transcript 全文 — 卡片只顯示 first-line,點開資料夾看全文
- Device picker UI — Deps tab 已有「找不到 device」提示,phase 2 才加切換

## 2. 平台限制(影響設計)

**X11 不能視窗外半透明**:Tauri 2 + GNOME Wayland 預設 OK,X11 session 下視窗外的 `box-shadow` / `filter: drop-shadow` 會破。

**結論**:所有「強調色暈」一律用 `box-shadow: inset` 或 `border` 處理,**禁用** outer shadow。Mock 中的「橘暈延伸到膠囊外」純粹是設計意圖視覺;實作要轉成 inner glow,色溫感留在膠囊內。

## 3. 鎖定的設計決策

| Decision | 選項 | 拍板 |
|---|---|---|
| Start/Stop 位置 | 同一顆狀態切換 vs 分位置 | **同一顆**(膠囊 + Record tab control bar 各有一個);transcribing 變 spinner disabled |
| Record tab 中央版型 | 大孤按鈕 vs 控制列 vs 純監視器 | **控制列**(`● REC 00:12:34 ───── [Stop]`)|
| VU meter data 路徑 | Tauri event vs polling vs fake | **event 50ms emit + polling fallback**(`[[mori-tauri-emit-listen-race]]`)|
| 視覺切片策略 | 大 PR vs 垂直切片 vs 依層切 | **垂直切片三 PR**(`[[feedback_trunk_based_auto_merge]]`)|
| 視覺嚴格度 | 1:1 mock vs 抽象對齊 | **1:1 mock**,但用 theme.css token,**不**inline rgba |
| 邊光暈方向 | outer vs inner | **inner**(X11 限制)|

## 4. 三 PR 拆分

```
PR1 capsule-visual-polish (front-only)
  └─ src/theme.css   (+token: --rec-accent, --rec-glow-inset, --rec-border,
                       --trans-accent, --trans-glow-inset, --trans-border,
                       --meter-bar, --meter-bar-peak, --meter-bar-bg,
                       --seg-pill-public-*, --seg-pill-internal-*)
  └─ src/CapsuleView.tsx  (data-state attr, swap to SVG icons)
  └─ src/components/icons/{Triangle,Square,Spinner,Bars,ChevronDown}Icon.tsx
  └─ src/components/{SignalPill,RecordButton}.tsx

PR2 record-tab-vu-meter (full stack)
  └─ src-tauri/src/audio/levels.rs           (new: LevelMeter, linear_to_db)
  └─ src-tauri/src/recorder.rs               (50ms tick emit "levels")
  └─ src-tauri/src/recorder_status.rs        (cache last LevelsPayload)
  └─ src/components/{VuMeter,TrackPanel}.tsx
  └─ src/tabs/RecordTab.tsx                  (control bar + 2× TrackPanel)

PR3 sessions-tab-cards (full stack)
  └─ src-tauri/src/session_store.rs          (read_session_summary)
  └─ src-tauri/src/commands.rs               (list_sessions_detailed)
  └─ src/components/{MeetingCard,SegPill}.tsx
  └─ src/tabs/SessionsTab.tsx                (use MeetingCard)
```

無 hard dep,但建議順序 PR1 → PR2 → PR3(PR1 加的 theme token 是公共底,先合避免 rebase token 衝突)。

## 5. PR1 — 膠囊視覺收

### 視覺差距(mock vs 現碼)

| 元素 | 現碼 | mock | 改 |
|---|---|---|---|
| 「REC」字色 | `--found-color`(綠) | 橘紅 | 新 token `--rec-accent`,recording 時用它 |
| 膠囊邊光 | 無 | recording=橘暈 / transcribing=黃暈 / idle=無 | inner glow:`box-shadow: inset 0 0 12px var(--rec-glow-inset)`,state class 切換 |
| Start/Stop button | 字符 `▶`/`■` 配 `.icon-btn` | 填色 SVG glyph | 改 SVG icon component + ghost button(透明底 + 上色 glyph) |
| SYS/MIC pill 圖示 | `●` 圓點 | 3-vertical-bars mini chart | 改 `<BarsIcon>` SVG;active 時上色,inactive 時灰 |
| ▾ 收合鍵 | 字符 `▾` | chevron 形 | `<ChevronDownIcon>` SVG |

### 元件結構

```
src/CapsuleView.tsx
└─ <div.capsule data-state={state}>          // state = idle | recording | transcribing
   ├─ <span.capsule-dot />
   ├─ <span.capsule-title>Recorder
   ├─ <span.capsule-status>{REC | idle | transcribing...}
   ├─ <span.capsule-time>{fmt(elapsed)}
   ├─ <SignalPill kind="sys" active={...} />
   ├─ <SignalPill kind="mic" active={...} />
   ├─ <RecordButton state={state} onClick={onStartStop} />
   └─ <button.icon-btn onClick={onExpand}><ChevronDownIcon /></button>
```

### theme.css 新增

```css
:root {
  /* recording state */
  --rec-accent:       rgb(255, 138, 80);
  --rec-glow-inset:   rgba(255, 138, 80, 0.35);
  --rec-border:       rgba(255, 138, 80, 0.40);

  /* transcribing state */
  --trans-accent:     var(--waiting-color);
  --trans-glow-inset: rgba(255, 179, 64, 0.30);
  --trans-border:     rgba(255, 179, 64, 0.40);
}

.capsule[data-state="recording"]    { box-shadow: inset 0 0 12px var(--rec-glow-inset);   border-color: var(--rec-border); }
.capsule[data-state="transcribing"] { box-shadow: inset 0 0 12px var(--trans-glow-inset); border-color: var(--trans-border); }
.capsule[data-state="idle"]         { box-shadow: none; }

.capsule-status.recording { color: var(--rec-accent); }
```

`RecordButton` 規則(filled style,對齊 mock 01 / 02 / 03):

| state | glyph | bg | glyph color | 備註 |
|---|---|---|---|---|
| idle | `<TriangleIcon />` | `var(--rec-accent)` | white | mock 02:橘三角填色 |
| recording | `<SquareIcon />` | `var(--danger-color)` | white | mock 01:紅方塊填色 |
| transcribing | `<SpinnerIcon />` | transparent | `var(--trans-accent)` | mock 03:無底色,disabled,glyph 自轉 |

button size 跟現有 `.icon-btn`(28×28)一致,維持膠囊比例;非膠囊中孤立按鈕,inner glow 已分擔色溫。

## 6. PR2 — Record tab VU meter

### 視覺契約(mock 04)

- 控制列:`● REC 00:12:34 ──────── [Stop]`,Stop 醒目但非中心孤立
- 控制列 Stop button(mock 04 v2 中 ~36px rounded square,填 `var(--danger-color)` + 白方 glyph)— 比膠囊 RecordButton 大一號,**只在 recording 時顯示**;idle 顯示 `<TriangleIcon>` 填 `var(--rec-accent)`;transcribing 顯示 spinner,disabled
- VU 列:每軌一張 micro-card,包含 `dot + bars icon + label`(左)/ VU bar grid(中)/ dB readout(右)/ source name(下)
- VU bar:24 segments 等寬,左對齊;0–85% 綠,最右側 lit segment 橘(peak);無訊號全暗

### Rust 後端

```rust
// src-tauri/src/audio/levels.rs
pub fn linear_to_db(x: f32) -> f32 {
    20.0 * f32::log10(x.max(1e-6))
}

pub struct LevelMeter { window: VecDeque<f32>, capacity: usize }
impl LevelMeter {
    pub fn new(capacity: usize) -> Self { ... }
    pub fn push(&mut self, samples: &[f32]) -> (f32, f32) { /* peak_db, rms_db */ }
}

// src-tauri/src/recorder.rs (audio loop 加 50ms tick)
#[derive(Serialize, Clone)]
pub struct LevelsPayload { pub sys: TrackLevel, pub mic: TrackLevel }

#[derive(Serialize, Clone)]
pub struct TrackLevel { pub peak_db: f32, pub rms_db: f32, pub signal: bool }

// emit
app.emit("levels", payload.clone())?;
// also cache for polling fallback
state.last_levels.lock().unwrap().replace(payload);
```

`recorder_status` response 加 `levels: Option<LevelsPayload>` 欄位。

### React 前端

```
src/components/VuMeter.tsx
  Props: { peakDb, rmsDb, signal, segments = 24 }
  // -60 dB → 0 seg, 0 dB → 24 seg
  const segs = Math.max(0, Math.min(segments, Math.round(((rmsDb + 60) / 60) * segments)));
  // peak idx = bin where peakDb falls (similar mapping, 1 seg highlight)

src/components/TrackPanel.tsx
  Props: { kind: "sys" | "mic", label: string, sourceName: string, level: TrackLevel | null }

src/tabs/RecordTab.tsx (改)
  useEffect:
    const unlisten = await listen("levels", e => setLevels(e.payload));
    const id = setInterval(async () => {
      // polling fallback
      const s = await invoke("recorder_status");
      if (s.levels) setLevels(s.levels);
    }, 500);
    return () => { unlisten(); clearInterval(id); };
```

### theme.css 新增

```css
:root {
  --meter-bar:      rgb(77, 242, 153);
  --meter-bar-peak: var(--rec-accent);
  --meter-bar-bg:   rgba(255,255,255,0.06);
  --vu-seg-w:       6px;
  --vu-seg-gap:     2px;
  --vu-seg-h:       18px;
}
```

## 7. PR3 — Sessions tab 卡片

### 視覺契約(mock 05)

```
┌──────────────────────────────────────────────────────────┐
│ 📁  meeting-20260528-143000           [public: 142 segs] │
│     2026-05-28 14:30 · 45m 23s        [internal: 67 segs]│
│     客戶要求三週後上線                              [↗]   │
└──────────────────────────────────────────────────────────┘
```

### Rust 後端

```rust
// src-tauri/src/session_store.rs
//
// 對齊既有 schema(`exporter::SessionMeta`):
//   - `started_at` 是 ISO 8601 帶 timezone 的字串(例 "2026-05-28T14:30:00+08:00"),前端 new Date() 直接 parse
//   - `duration_secs` 是 u64 秒
//   - public/internal seg 數,**按 `Segment.visibility`**(不是 source_kind):segments 中 visibility == "public" / "internal" 各自 count
#[derive(Serialize, Clone)]
pub struct SessionSummary {
    pub id: String,
    pub started_at: String,        // ISO 8601 + tz
    pub duration_secs: u64,
    pub public_segs: u32,          // count where visibility == "public"
    pub internal_segs: u32,        // count where visibility == "internal"
    pub preview: Option<String>,   // 第一行非空 body 文字,≤120 chars
    pub corrupt: bool,
}

pub fn read_session_summary(id: &str, base: &Path) -> Result<SessionSummary> { ... }

// src-tauri/src/commands.rs
#[tauri::command]
pub async fn list_sessions_detailed() -> Vec<SessionSummary> {
    // 讀 ~/.mori/meetings/*/,呼 read_session_summary,by started_at desc
    // corrupt 也回(灰底卡片 + 「資料損毀」標示)
}
```

### Edge cases

| 情況 | 卡片顯示 |
|---|---|
| `timeline.json` 缺 / parse fail | `corrupt: true`,灰底 + 「資料損毀」標籤,只顯示 id |
| `meeting.public.md` 空 | `preview: None`,卡片顯示「(無公開內容)」灰字 |
| `duration_secs == 0` | duration 字段顯示「(0s)」 |
| Session dir 完全空(沒任何檔) | filter 掉,不列 |

### React 前端

```
src/components/MeetingCard.tsx
  Props: { summary: SessionSummary }
  fmtStartedAt(s: string) → new Date(s).toLocaleString("zh-TW", { dateStyle: "short", timeStyle: "short" })
  fmtDuration(secs: number) → "45m 23s" / "1h 12m" / "0s"
  click → invoke("open_session_dir", { sessionId: summary.id })

src/components/SegPill.tsx
  Props: { tone: "public" | "internal", count: number }
```

### theme.css 新增

```css
:root {
  --seg-pill-public-bg:   rgba(77, 242, 153, 0.18);
  --seg-pill-public-fg:   var(--found-color);
  --seg-pill-internal-bg: rgba(255, 179, 64, 0.18);
  --seg-pill-internal-fg: var(--waiting-color);
}

.meeting-card {
  display: grid;
  grid-template-columns: auto 1fr auto;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 12px;
  border: 0.5px solid var(--border);
  background: rgba(255,255,255,0.02);
  cursor: pointer;
  margin-bottom: 6px;
}
.meeting-card:hover { background: var(--hover); }
.meeting-card.corrupt { background: rgba(255,99,99,0.06); }
.meeting-card .preview {
  font-size: 11px;
  color: var(--text-secondary);
  margin-top: 4px;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.seg-pill {
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 500;
}
.seg-pill[data-tone="public"]   { background: var(--seg-pill-public-bg);   color: var(--seg-pill-public-fg); }
.seg-pill[data-tone="internal"] { background: var(--seg-pill-internal-bg); color: var(--seg-pill-internal-fg); }
```

## 8. Tauri command 對外契約異動

| Command | 改動 | PR |
|---|---|---|
| `recorder_status` | response 加 `levels: LevelsPayload \| null` | PR2 |
| `list_sessions_detailed` | **新增**,return `Vec<SessionSummary>` | PR3 |
| `recorder_start` / `recorder_stop` / `set_window_mode` / `open_session_dir` / `list_sessions` / `deps_check` | 不變 | — |

`list_sessions` 保留(回傳 `Vec<String>`),不過前端 SessionsTab 不再用,改用 `list_sessions_detailed`。其他 caller(沒有)不影響。

## 9. 測試策略

### Unit(`cargo test`)

| File | Cases |
|---|---|
| `src-tauri/src/audio/levels.rs` | `linear_to_db(1.0)` ≈ 0;`linear_to_db(1e-6)` ≈ -120;`LevelMeter::push` peak / RMS 正確;空 sample 不 panic |
| `src-tauri/src/session_store.rs::read_session_summary` | 完整 session;`timeline.json` 缺 → corrupt;`public.md` 空 → preview None;`duration_secs=0` |

### 手動 e2e

跑 `npm run tauri dev`:

1. **膠囊 3 狀態**(PR1)— idle / recording / transcribing 各對 mock 01/02/03;Ubuntu 24.04 預設 Wayland session,X11 session 是 fallback(GNOME 登入畫面選「在 Xorg 上」),**至少 Wayland 必驗,X11 best-effort 看狀況**
2. **Record tab**(PR2)— 開 YouTube + 對麥講話,看 SYS / MIC 兩條 VU 動;靜音環境驗 idle bar 全暗;Stop → transcribing 中 VU 不動
3. **Sessions tab**(PR3)— 看 `~/.mori/meetings/` 既有 session 卡片;點卡片開資料夾;手動造一個壞 `timeline.json` 驗「資料損毀」標示

### 通用 gate

每 PR push 前:

```bash
bash scripts/verify.sh   # cargo test + npm run build + cargo check
```

全綠才開 PR + auto-merge。

## 10. Mori voice safety

照 `[[feedback_no_restart_mori_during_voice]]`:這次動的全在 `mori-meeting-recorder` 自己 repo,**不會碰 mori-desktop**。Recorder dev rebuild 不影響 mori-desktop session。

## 11. Branch / PR 命名

照 `[[mori-branch-naming]]`:

- `feat/capsule-visual-polish`
- `feat/record-tab-vu-meter`
- `feat/sessions-tab-cards`

**不**用 `codex/...`。

## 12. 風險與限制

| 風險 | 影響 | 緩解 |
|---|---|---|
| Tauri emit "levels" 在 StrictMode / HMR 不可靠 | VU meter 卡住 | polling fallback 500ms 從 `recorder_status.levels` 補 |
| X11 + GNOME 下 `box-shadow: inset` 也偶有 compositor bug | 邊光暈看不到 | 退路:改用 `border` 加粗 + 上色(視覺降一級但保證可見) |
| `timeline.json` schema 之後改動 | sessions 卡 parse fail | `corrupt: true` 路徑保證 UI 不爆;schema 改動屬 phase 2 |
| Windows 下 cpal WASAPI loopback 跟 50ms tick 時序未驗 | VU meter Windows 端表現未知 | Spec 明寫「未驗 Windows」;phase 2 親測 |
| Mock 中橘色精確值與 theme token 估值差異 | 視覺感受微差 | 容許,人工 e2e 校正一次即可 |

## 13. 下一步

spec 通過後:

1. 呼 `superpowers:writing-plans`,把這份 spec 拆成 step-by-step implementation plan(三 PR 各自的 step 序列 + subagent 派工)
2. 跑 `superpowers:executing-plans` 或 `subagent-driven-development` 落地
3. Ship,留 docs/design/ 的 6 張 mock 當 visual regression 對照

---

**Mocks**: `docs/design/01..06-*.png`
**Related memories**: `[[feedback_trunk_based_auto_merge]]` `[[feedback_ui_theme_tokens]]` `[[mori-branch-naming]]` `[[mori-tauri-emit-listen-race]]` `[[feedback_no_restart_mori_during_voice]]` `[[project_mori_body_interface]]`
**BI-5 parent spec**: `mori-desktop/docs/superpowers/specs/2026-05-28-bi-5-meeting-recorder-design.md`
