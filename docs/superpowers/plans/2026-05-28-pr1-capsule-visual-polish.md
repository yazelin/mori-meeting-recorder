# PR1 Capsule Visual Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 mori-meeting-recorder 膠囊(`CapsuleView.tsx`)的視覺從 ASCII 字符 icon + 沒邊光暈,升級到對齊 mock 01/02/03 的 SVG icon + 內光暈 + theme token,純前端,無後端動。

**Architecture:** 在 `src/components/` 下新建 5 個 SVG icon 元件 + `SignalPill` + `RecordButton`;`src/theme.css` 加 recording / transcribing 狀態的 token + inner-glow CSS rule;`CapsuleView.tsx` 用 `data-state` attribute 切狀態,renderTree 改用新元件。X11 限制下,所有色暈用 `box-shadow: inset` 而非 outer shadow。

**Tech Stack:** React 18 / TypeScript / Vite / Tauri 2(無動);SVG inline component;CSS custom properties。

**Spec reference:** `docs/superpowers/specs/2026-05-28-recorder-ui-mock-alignment-design.md` §5

**Mock reference:** `docs/design/01-recording-capsule.png` / `02-idle-capsule.png` / `03-transcribing-capsule.png`

**No frontend test framework** in this repo — TDD 形式弱化:每個元件用 `npm run build`(tsc 嚴格模式)當編譯閘,最後一階段是手動跑 `npm run tauri dev` 對 mock 視覺驗收。

---

### Task 0: Branch off main

**Files:** none(branch ops)

- [ ] **Step 1: Sync main and create feature branch**

```bash
cd /home/ct/mori-universe/mori-meeting-recorder
git fetch origin
git checkout main
git pull --ff-only origin main
git checkout -b feat/capsule-visual-polish
```

- [ ] **Step 2: Confirm clean working tree**

Run: `git status`
Expected: `nothing to commit, working tree clean`

---

### Task 1: Add theme.css tokens for recording / transcribing states

**Files:**
- Modify: `src/theme.css`(append to `:root` block + add new state rules)

- [ ] **Step 1: Add new CSS tokens to `:root` block**

Open `src/theme.css`,在現有 `:root { ... }` 區塊**結尾**(`--scale: 1;` 那行下面、`}` 上面)加:

```css
  /* recording state(對應 mock 01 / 04 v2) */
  --rec-accent:       rgb(255, 138, 80);
  --rec-glow-inset:   rgba(255, 138, 80, 0.35);
  --rec-border:       rgba(255, 138, 80, 0.40);

  /* transcribing state(對應 mock 03) */
  --trans-accent:     var(--waiting-color);
  --trans-glow-inset: rgba(255, 179, 64, 0.30);
  --trans-border:     rgba(255, 179, 64, 0.40);

  /* VU meter token(留給 PR2,提前一起加避免後續 conflict) */
  --meter-bar:        var(--found-color);
  --meter-bar-peak:   var(--rec-accent);
  --meter-bar-bg:     rgba(255,255,255,0.06);

  /* Seg pill token(留給 PR3) */
  --seg-pill-public-bg:   rgba(77, 242, 153, 0.18);
  --seg-pill-public-fg:   var(--found-color);
  --seg-pill-internal-bg: rgba(255, 179, 64, 0.18);
  --seg-pill-internal-fg: var(--waiting-color);
```

- [ ] **Step 2: Add capsule state CSS rules**

在 `theme.css` 找到既有 `.capsule { ... }` 區塊**之後**插入:

```css
.capsule[data-state="recording"] {
  box-shadow: inset 0 0 12px var(--rec-glow-inset);
  border-color: var(--rec-border);
}
.capsule[data-state="transcribing"] {
  box-shadow: inset 0 0 12px var(--trans-glow-inset);
  border-color: var(--trans-border);
}
.capsule[data-state="idle"] {
  box-shadow: none;
}
.capsule-status.recording {
  color: var(--rec-accent);
}
```

- [ ] **Step 3: Verify build**

Run: `npm run build`
Expected: 完成,無 TS 錯,無 vite 警告。

- [ ] **Step 4: Commit**

```bash
git add src/theme.css
git commit -m "feat(theme): add recording/transcribing state tokens + inner-glow rules"
```

---

### Task 2: Create TriangleIcon SVG component

**Files:**
- Create: `src/components/icons/TriangleIcon.tsx`

- [ ] **Step 1: Create file with filled triangle SVG**

```tsx
// src/components/icons/TriangleIcon.tsx
//
// Filled play triangle, used by RecordButton in idle state.
// Sized 16×16 viewbox; consumer controls dimension via CSS.

export default function TriangleIcon({ size = 14 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M4 3 L13 8 L4 13 Z" />
    </svg>
  );
}
```

- [ ] **Step 2: Verify build**

Run: `npm run build`
Expected: 完成,無錯。

- [ ] **Step 3: Commit**

```bash
git add src/components/icons/TriangleIcon.tsx
git commit -m "feat(icons): add TriangleIcon (play)"
```

---

### Task 3: Create SquareIcon SVG component

**Files:**
- Create: `src/components/icons/SquareIcon.tsx`

- [ ] **Step 1: Create file with filled square SVG**

```tsx
// src/components/icons/SquareIcon.tsx
//
// Filled stop square, used by RecordButton in recording state.

export default function SquareIcon({ size = 12 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect x="3" y="3" width="10" height="10" rx="1.5" />
    </svg>
  );
}
```

- [ ] **Step 2: Verify build**

Run: `npm run build`
Expected: 完成,無錯。

- [ ] **Step 3: Commit**

```bash
git add src/components/icons/SquareIcon.tsx
git commit -m "feat(icons): add SquareIcon (stop)"
```

---

### Task 4: Create SpinnerIcon SVG component(含 rotation animation)

**Files:**
- Create: `src/components/icons/SpinnerIcon.tsx`
- Modify: `src/theme.css`(加 spinner 旋轉 keyframes)

- [ ] **Step 1: Add spinner keyframes to theme.css**

在 `theme.css` 結尾加:

```css
@keyframes spinnerRotate {
  from { transform: rotate(0deg); }
  to   { transform: rotate(360deg); }
}
.spinner-rotate {
  animation: spinnerRotate 1s linear infinite;
  display: inline-flex;
}
```

- [ ] **Step 2: Create SpinnerIcon component**

```tsx
// src/components/icons/SpinnerIcon.tsx
//
// Rotating arc used by RecordButton in transcribing state.
// Wrap in <span class="spinner-rotate"> for animation(animate the wrapper,
// not the <svg> directly, to keep stroke vector clean).

export default function SpinnerIcon({ size = 14 }: { size?: number }) {
  return (
    <span className="spinner-rotate" aria-label="transcribing">
      <svg
        width={size}
        height={size}
        viewBox="0 0 16 16"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        xmlns="http://www.w3.org/2000/svg"
      >
        <path d="M8 2 A6 6 0 0 1 14 8" />
      </svg>
    </span>
  );
}
```

- [ ] **Step 3: Verify build**

Run: `npm run build`
Expected: 完成,無錯。

- [ ] **Step 4: Commit**

```bash
git add src/components/icons/SpinnerIcon.tsx src/theme.css
git commit -m "feat(icons): add SpinnerIcon with rotation keyframes"
```

---

### Task 5: Create BarsIcon SVG component(SYS/MIC pill 用)

**Files:**
- Create: `src/components/icons/BarsIcon.tsx`

- [ ] **Step 1: Create file**

```tsx
// src/components/icons/BarsIcon.tsx
//
// Tiny 3-vertical-bars equalizer icon for SignalPill.
// Color = currentColor — caller decides active/inactive.

export default function BarsIcon({ size = 10 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 10 10"
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect x="1" y="5" width="1.6" height="4" rx="0.4" />
      <rect x="4.2" y="2" width="1.6" height="7" rx="0.4" />
      <rect x="7.4" y="4" width="1.6" height="5" rx="0.4" />
    </svg>
  );
}
```

- [ ] **Step 2: Verify build**

Run: `npm run build`
Expected: 完成,無錯。

- [ ] **Step 3: Commit**

```bash
git add src/components/icons/BarsIcon.tsx
git commit -m "feat(icons): add BarsIcon (signal pill mini equalizer)"
```

---

### Task 6: Create ChevronDownIcon SVG component(收合鍵)

**Files:**
- Create: `src/components/icons/ChevronDownIcon.tsx`

- [ ] **Step 1: Create file**

```tsx
// src/components/icons/ChevronDownIcon.tsx
//
// 收合 / expand 鍵的箭頭。需要往上指時 caller 自己加 CSS transform: rotate(180deg)
// 或用 ChevronUpIcon(此 PR 不需,ExpandedView 那顆暫不動)。

export default function ChevronDownIcon({ size = 12 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M3 4.5 L6 7.5 L9 4.5" />
    </svg>
  );
}
```

- [ ] **Step 2: Verify build**

Run: `npm run build`
Expected: 完成,無錯。

- [ ] **Step 3: Commit**

```bash
git add src/components/icons/ChevronDownIcon.tsx
git commit -m "feat(icons): add ChevronDownIcon (collapse arrow)"
```

---

### Task 7: Create SignalPill component

**Files:**
- Create: `src/components/SignalPill.tsx`
- Modify: `src/theme.css`(`.signal-pill` 既有 rule 加 BarsIcon 對應 padding 微調 — 視需要)

- [ ] **Step 1: Inspect existing `.signal-pill` CSS in theme.css**

Run: `grep -n "signal-pill" src/theme.css`
確認既有 rule 是用 `signal-pill-dot` 圓點;這個 task 不刪舊 rule,只是 SignalPill component 改用 BarsIcon。

- [ ] **Step 2: Create SignalPill component**

```tsx
// src/components/SignalPill.tsx
//
// 取代 CapsuleView 中既有的 inline `<span className="signal-pill">...<span className="signal-pill-dot" />SYS</span>`。
// 圖示從圓點換成 BarsIcon,符合 mock 01/02/04 v2 的視覺。
// Active 時 .on 套既有 theme.css rule(綠底綠字);err 時 .err 套(紅底紅字)。

import { useTranslation } from "react-i18next";
import BarsIcon from "./icons/BarsIcon";

type Kind = "sys" | "mic";

interface Props {
  kind: Kind;
  active: boolean;
}

const LABEL: Record<Kind, string> = { sys: "SYS", mic: "MIC" };
const TITLE_KEY: Record<Kind, string> = {
  sys: "capsule.system_pill",
  mic: "capsule.mic_pill",
};

export default function SignalPill({ kind, active }: Props) {
  const { t } = useTranslation();
  return (
    <span className={`signal-pill${active ? " on" : ""}`} title={t(TITLE_KEY[kind])}>
      <BarsIcon size={10} />
      {LABEL[kind]}
    </span>
  );
}
```

- [ ] **Step 3: Verify build**

Run: `npm run build`
Expected: 完成,無錯。

- [ ] **Step 4: Commit**

```bash
git add src/components/SignalPill.tsx
git commit -m "feat(capsule): SignalPill component using BarsIcon"
```

---

### Task 8: Create RecordButton component

**Files:**
- Create: `src/components/RecordButton.tsx`
- Modify: `src/theme.css`(加 `.record-btn` filled-style class)

- [ ] **Step 1: Add `.record-btn` CSS**

在 `theme.css` 結尾加(`.icon-btn` 區塊之後):

```css
/* Filled-style action button used inside the capsule;對應 mock 01/02/03 中央按鈕。
 * 設計上跟 .icon-btn 並列;不用 var(--btn-bg),而是 state-specific filled bg。 */
.record-btn {
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 8px;
  cursor: pointer;
  flex-shrink: 0;
  color: white;
  transition: opacity 0.15s, transform 0.15s;
}
.record-btn:hover  { opacity: 0.88; }
.record-btn:active { transform: scale(0.96); }
.record-btn:disabled { cursor: not-allowed; }

.record-btn[data-state="idle"]         { background: var(--rec-accent); }
.record-btn[data-state="recording"]    { background: var(--danger-color); }
.record-btn[data-state="transcribing"] {
  background: transparent;
  color: var(--trans-accent);
  opacity: 0.8;
}
```

- [ ] **Step 2: Create RecordButton component**

```tsx
// src/components/RecordButton.tsx
//
// Filled-style action button — 三個 state 對應 mock 01(stop)/ 02(start)/ 03(transcribing)。
// 跟 CapsuleView 既有 `.icon-btn` 並列,不替換它。

import { useTranslation } from "react-i18next";
import TriangleIcon from "./icons/TriangleIcon";
import SquareIcon from "./icons/SquareIcon";
import SpinnerIcon from "./icons/SpinnerIcon";

type State = "idle" | "recording" | "transcribing";

interface Props {
  state: State;
  onClick: () => void;
}

export default function RecordButton({ state, onClick }: Props) {
  const { t } = useTranslation();
  const disabled = state === "transcribing";
  const title =
    state === "recording"   ? t("capsule.stop")  :
    state === "transcribing" ? t("capsule.transcribing") :
                               t("capsule.start");

  return (
    <button
      type="button"
      className="record-btn"
      data-state={state}
      onClick={onClick}
      disabled={disabled}
      title={title}
    >
      {state === "idle"        && <TriangleIcon size={12} />}
      {state === "recording"   && <SquareIcon   size={10} />}
      {state === "transcribing" && <SpinnerIcon size={14} />}
    </button>
  );
}
```

- [ ] **Step 3: Verify build**

Run: `npm run build`
Expected: 完成,無錯。

- [ ] **Step 4: Commit**

```bash
git add src/theme.css src/components/RecordButton.tsx
git commit -m "feat(capsule): RecordButton with filled state-specific styles"
```

---

### Task 9: Refactor CapsuleView.tsx to use data-state + new components

**Files:**
- Modify: `src/CapsuleView.tsx`

- [ ] **Step 1: Replace CapsuleView return JSX**

整段替換 `CapsuleView` 元件的 `return ( ... )`(留 useEffect / startDragOnMouseDown / onStartStop 不動)。新 return:

```tsx
  return (
    <div className="capsule" data-state={recState} onMouseDown={startDragOnMouseDown}>
      <span className={dotClass} />
      <span className="capsule-title">Recorder</span>
      <span className={statusClass}>{statusLabel}</span>
      <span className="capsule-spacer" />
      <span className="capsule-time">{fmt(status?.elapsed_secs ?? 0)}</span>
      <span className="signal-pills">
        <SignalPill kind="sys" active={!!status?.system_signal} />
        <SignalPill kind="mic" active={!!status?.mic_signal} />
        {err && (
          <span className="signal-pill err" title={err}>⚠</span>
        )}
      </span>
      <RecordButton state={recState} onClick={onStartStop} />
      <button className="icon-btn" onClick={onExpand} title={t("capsule.expand")}>
        <ChevronDownIcon size={12} />
      </button>
    </div>
  );
```

- [ ] **Step 2: Add imports at top of file**

把這四行加進 `src/CapsuleView.tsx` 既有 import 區塊(在 `useTranslation` import 之後):

```tsx
import SignalPill from "./components/SignalPill";
import RecordButton from "./components/RecordButton";
import ChevronDownIcon from "./components/icons/ChevronDownIcon";
```

- [ ] **Step 3: Remove unused `isRecording` / `isTranscribing` const declarations**

既有 `CapsuleView.tsx` 在 `onStartStop` 之後有:

```tsx
const isRecording = recState === "recording";
const isTranscribing = recState === "transcribing";
```

新 RecordButton 內部處理 disabled / title / glyph,這兩個 const **沒有 caller** — 刪掉,避免 `tsc --noUnusedLocals` 擋。`recState` / `dotClass` / `statusLabel` / `statusClass` 留著(新 JSX 用)。

- [ ] **Step 4: Confirm old inline SignalPill / icon-btn markup gone**

`grep -n "signal-pill-dot\|capsule.system_pill\|isRecording\|isTranscribing" src/CapsuleView.tsx`
Expected: 0 hits(這些都隨新 JSX / Step 3 移除掉)。

- [ ] **Step 5: Verify build(包含 tsc -b typecheck)**

Run: `npm run build`
Expected: 完成,無 TS 錯,無 vite 警告。

- [ ] **Step 6: 額外 strict typecheck**

Run: `npx tsc --noEmit`
Expected: 完成,無錯。

- [ ] **Step 7: Commit**

```bash
git add src/CapsuleView.tsx
git commit -m "refactor(capsule): use data-state attr + SignalPill/RecordButton/ChevronDownIcon"
```

---

### Task 10: Visual e2e against mocks 01/02/03

**Files:** none(手動驗收)

- [ ] **Step 1: Start dev server**

Run(本機開另一個 terminal):
```bash
npm run tauri dev
```
等到 Tauri window 浮出膠囊。

- [ ] **Step 2: 驗 idle state(對 mock `docs/design/02-idle-capsule.png`)**

膠囊應顯示:
- 灰圓點(`.capsule-dot.idle`)
- "Recorder" 白字
- "idle" 灰字
- "00:00:00" timer
- SYS / MIC pill 暗灰(無 `.on` class)
- 中央按鈕:橘三角(TriangleIcon)在 `var(--rec-accent)` 橘底上 — 對應 mock 02
- ▾(ChevronDownIcon)
- **無邊光暈**(`box-shadow: none`)

如果哪項不對,回對應 Task 改 CSS / component。

- [ ] **Step 3: 驗 recording state(對 mock `docs/design/01-recording-capsule.png`)**

點中央 RecordButton 開始錄音。膠囊應變成:
- 紅圓點脈動(`.capsule-dot.recording`,既有 `dot-pulse` keyframes)
- "REC" 橘字(`var(--rec-accent)`)
- timer 開始走
- SYS / MIC pill 點亮(綠底)— 假設有音訊
- 中央按鈕:白方塊(SquareIcon)在 `var(--danger-color)` 紅底上
- 膠囊**內側橘暈**(inset box-shadow),邊框橘色
- **暈不外溢**(在 X11 session 下也驗一次,確認 inner glow 沒切到視窗外)

- [ ] **Step 4: 驗 transcribing state(對 mock `docs/design/03-transcribing-capsule.png`)**

點 stop。膠囊在 transcribing 階段應變成:
- 黃圓點(`.capsule-dot.transcribing`)
- "transcribing..." 黃字
- 中央按鈕:旋轉 spinner 在透明底 + 黃 glyph,disabled 不可按
- 膠囊**內側黃暈**,邊框黃色

等 transcribing 完成,膠囊應回 idle state。

- [ ] **Step 5: Drag 行為驗證**

按住膠囊空白區拖移 → 視窗跟著走;按住 RecordButton / ChevronDownIcon 拖 → 視窗不應移動(button.closest check 應擋住)。

- [ ] **Step 6: Wayland + X11 驗證(若可)**

預設 Wayland session 已驗。若 GNOME 登入畫面可選 X11 session,登 X11 跑一遍確認 inner glow 不破。若無 X11 session,在 commit 訊息 / PR body 註明「Wayland-only verified」。

---

### Task 11: Run scripts/verify.sh

**Files:** none

- [ ] **Step 1: Run verify**

```bash
bash scripts/verify.sh
```

Expected output 末尾:
```
==> npm run build (必須在 cargo check 之前 — generate_context! 需要 dist/)
... vite build output ...
==> cargo test
... test summary: N passed; 0 failed ...
==> cargo check --all-targets
... Finished ...
✓ verify ok
```

如有任何錯,**修到全綠**才繼續。

---

### Task 12: Push branch + open PR + enable auto-merge

**Files:** none

- [ ] **Step 1: Push branch**

```bash
git push -u origin feat/capsule-visual-polish
```

- [ ] **Step 2: Open PR**

```bash
gh pr create --title "feat(capsule): 1:1 mock visual polish (PR1 of 3)" --body "$(cat <<'BODYEOF'
## Summary

PR1 of the 3-PR recorder UI mock alignment series(spec: `docs/superpowers/specs/2026-05-28-recorder-ui-mock-alignment-design.md`,#5).

- 膠囊 SVG icon(Triangle / Square / Spinner / Bars / ChevronDown)
- `data-state` attribute 切 idle / recording / transcribing
- Inner glow box-shadow(X11 安全)+ 對應 state border color
- `SignalPill` 用 BarsIcon mini equalizer 取代圓點
- `RecordButton` filled style 三 state
- `theme.css` 加 recording / transcribing token + spinner rotate keyframes
- **預埋** PR2(VU meter)/ PR3(seg pill)的 token,避免後續 conflict

純前端,**無後端動**。

## Test plan

- [x] `bash scripts/verify.sh` 全綠(npm run build + cargo test + cargo check)
- [x] 手動 e2e:`npm run tauri dev` 開膠囊
- [x] idle state 對齊 mock 02
- [x] recording state 對齊 mock 01,內暈不外溢
- [x] transcribing state 對齊 mock 03
- [x] drag 行為:空白拖動 / button 內不拖
- [ ] X11 session 驗(若 Wayland-only 已驗,於 follow-up 補)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
BODYEOF
)"
```

- [ ] **Step 3: Enable auto-merge**

```bash
gh pr merge --auto --squash
```

- [ ] **Step 4: 確認 PR 已開且 auto-merge enabled**

Run: `gh pr view --json url,autoMergeRequest`
Expected: `url` 有值,`autoMergeRequest` 不是 null。

- [ ] **Step 5: 報告 PR URL 給 user**

Output the PR URL produced by Step 2 to the user。

---

## Self-Review Notes(自查跟 spec 對齊)

| Spec §5 元素 | 任務對應 |
|---|---|
| REC 字色 `--rec-accent` | Task 1 |
| 膠囊邊光 inner glow | Task 1 |
| Start/Stop SVG icon | Task 2/3 + Task 8 |
| SYS/MIC pill mini bars | Task 5 + Task 7 |
| ▾ ChevronDown SVG | Task 6 + Task 9 |
| `data-state` attribute | Task 9 |
| filled RecordButton 3-state table | Task 8 |
| theme.css token 不 inline rgba | Task 1 / 4 / 8(rgba 都在 :root token 內) |
| X11 inner-glow 限制 | Task 1 / Task 10 Step 6 |

**Branch / PR 命名**:`feat/capsule-visual-polish` 對齊 `[[mori-branch-naming]]`,**不**用 `codex/...`。

**Mori voice safety**:這條 PR 不動 Rust,Tauri dev server 不重啟 mori-tauri / mori-desktop。

**未在此 PR 做的(留 PR2/PR3)**:
- VU meter component + audio level Rust(PR2)
- Sessions card / SegPill / MeetingCard / list_sessions_detailed(PR3)
- Mute MIC 副按鈕 / Pause / Mark moment(phase 2)
- X11 fallback(GNOME 切 session 視 user 能不能切)
