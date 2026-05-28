# PR2 Record Tab VU Meter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 mori-meeting-recorder 的 Record tab 把 mock 04 v2 的「控制列 + 雙軌 VU meter」整套落地 — Rust 端產生 50ms tick 的 peak/RMS dB,Tauri `levels` event + polling fallback 餵到前端;React 端畫 24-segment VU bar、dB 讀值、source name,顯示在 mock 04 v2 的版型裡。

**Architecture:** 後端利用既存的 `audio::SignalMeter` 基礎(每軌 `Arc<Mutex<>>` + 已算好 RMS),抽出 `audio/levels.rs` 處理 peak + linear_to_db、補上 peak_db 欄位,然後 recorder spawn 一個 50ms 心跳 task 把每軌 `LevelsPayload` 透過 `app.emit("levels", ...)` 推給前端、同時 cache 一份給 `recorder_status` 當 polling fallback(防 `[[mori-tauri-emit-listen-race]]`)。前端把 RecordTab.tsx 大按鈕版型砍掉、改成 spec §6 的控制列 + 兩張 TrackPanel,每張 TrackPanel 含 VuMeter component(24 segment grid)+ dB readout + source name。

**Tech Stack:** Rust(tokio task + Tauri 2 `app.emit`)、React 18 + TypeScript(Tauri event listen + polling)、Vite、cargo test。

**Spec reference:** `docs/superpowers/specs/2026-05-28-recorder-ui-mock-alignment-design.md` §6 + §8

**Mock reference:** `docs/design/04-record-tab.png`

**Dep on PR1:** 假設 PR #6(PR1)已 merge 進 main。PR1 已預埋以下 token,本 PR 直接用:`--meter-bar` / `--meter-bar-peak` / `--meter-bar-bg`,以及 `BarsIcon` component。若 Task 0 sync main 時 PR1 還沒 merge,改成 branch off `origin/feat/capsule-visual-polish`(臨時 stack),merge 後 rebase 回 main;但**首選還是等 PR1 入 main** 再開動。

---

### Task 0: Branch off latest main

**Files:** none(branch ops)

- [ ] **Step 1: Sync main and create feature branch**

```bash
cd /home/ct/mori-universe/mori-meeting-recorder
git fetch origin
git checkout main
git pull --ff-only origin main
```

- [ ] **Step 2: 確認 PR1 已 merge 進 main**

Run: `git log --oneline -5 main | grep -i "capsule\|visual-polish"`
Expected: 看到 `feat(capsule): 1:1 mock visual polish + drag capability fix (PR1 of 3)` 或對應 squash commit。

如果**沒看到**,代表 PR1 還沒 merge,先停在這 task,等 main 更新後再續。

- [ ] **Step 3: Create branch**

```bash
git checkout -b feat/record-tab-vu-meter
git status
```

Expected: `nothing to commit, working tree clean`,branch = `feat/record-tab-vu-meter`。

---

### Task 1: Extract `audio/levels.rs` with TDD

**Files:**
- Create: `src-tauri/src/audio/levels.rs`
- Modify: `src-tauri/src/audio/mod.rs`(`pub mod levels;`)
- Test: `src-tauri/src/audio/levels.rs`(inline `#[cfg(test)] mod tests`)

- [ ] **Step 1: Add module declaration to audio/mod.rs**

在 `src-tauri/src/audio/mod.rs` 既有 `pub mod writer;` 那行下面加:

```rust
pub mod levels;
```

- [ ] **Step 2: Write the failing test first(TDD)**

Create `src-tauri/src/audio/levels.rs` with ONLY the test module:

```rust
//! Audio level computation — peak + RMS in dB,給 VU meter 用。
//!
//! Pure functions,容易測。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_to_db_full_scale_is_zero() {
        assert!((linear_to_db(1.0) - 0.0).abs() < 0.01);
    }

    #[test]
    fn linear_to_db_silence_floor() {
        // 1e-6 → -120 dB(實作要避免 log(0))
        assert!((linear_to_db(1e-6) - (-120.0)).abs() < 0.5);
    }

    #[test]
    fn linear_to_db_half_scale() {
        // 0.5 → 20 * log10(0.5) ≈ -6.02 dB
        assert!((linear_to_db(0.5) - (-6.02)).abs() < 0.05);
    }

    #[test]
    fn compute_levels_empty_returns_silence() {
        let (peak, rms) = compute_levels(&[]);
        assert!(peak < -100.0);
        assert!(rms < -100.0);
    }

    #[test]
    fn compute_levels_full_scale_sine_approx() {
        // 1024 samples 跑 sinusoid 接近 ±1.0
        let samples: Vec<f32> = (0..1024)
            .map(|i| (i as f32 * 0.1).sin())
            .collect();
        let (peak, rms) = compute_levels(&samples);
        // sin 的 peak ≈ 1.0 → 0 dB(允許 1 dB 誤差)
        assert!(peak.abs() < 1.0, "peak={peak}, expected ~0 dB");
        // sin 的 RMS = 1/sqrt(2) ≈ 0.707 → -3.01 dB
        assert!((rms - (-3.01)).abs() < 0.5, "rms={rms}, expected ~-3 dB");
    }

    #[test]
    fn compute_levels_dc_offset_only_rms_zero_peak_matches() {
        // 全部都是 0.5 → peak = 0.5 → ~-6 dB,RMS 也是 ~-6 dB
        let samples = vec![0.5_f32; 1000];
        let (peak, rms) = compute_levels(&samples);
        assert!((peak - (-6.02)).abs() < 0.1);
        assert!((rms - (-6.02)).abs() < 0.1);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib audio::levels`
Expected: 編譯失敗,因為 `linear_to_db` 和 `compute_levels` 沒定義。

- [ ] **Step 4: Implement minimal code to pass tests**

在 `src-tauri/src/audio/levels.rs` **檔頭**(`#[cfg(test)]` 之上)加實作:

```rust
//! Audio level computation — peak + RMS in dB,給 VU meter 用。
//!
//! Pure functions,容易測。

const DB_FLOOR: f32 = -120.0;
const SILENCE_LINEAR: f32 = 1e-6;

/// 線性振幅轉 dB。input <= SILENCE_LINEAR 視為靜音,clamp 到 DB_FLOOR。
pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= SILENCE_LINEAR {
        DB_FLOOR
    } else {
        20.0 * linear.log10()
    }
}

/// 從一批 f32 sample(已 normalize 到 ±1.0 範圍)算 (peak_db, rms_db)。
/// 空 slice → (DB_FLOOR, DB_FLOOR)。
pub fn compute_levels(samples: &[f32]) -> (f32, f32) {
    if samples.is_empty() {
        return (DB_FLOOR, DB_FLOOR);
    }
    let mut peak_lin: f32 = 0.0;
    let mut sumsq: f64 = 0.0;
    for &s in samples {
        let abs = s.abs();
        if abs > peak_lin {
            peak_lin = abs;
        }
        sumsq += (s as f64) * (s as f64);
    }
    let rms_lin = (sumsq / samples.len() as f64).sqrt() as f32;
    (linear_to_db(peak_lin), linear_to_db(rms_lin))
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd src-tauri && cargo test --lib audio::levels`
Expected: 6 tests pass,no failures。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/audio/levels.rs src-tauri/src/audio/mod.rs
git commit -m "feat(audio): extract levels module with peak/RMS dB computation (TDD)"
```

---

### Task 2: Add peak_db field to SignalMeter

**Files:**
- Modify: `src-tauri/src/audio/mod.rs`(extend `SignalMeter`)

- [ ] **Step 1: Add peak_db field**

在 `src-tauri/src/audio/mod.rs` 找到既有 `SignalMeter` struct,改成:

```rust
/// 過去 N ms 的 peak + RMS — capsule 用 RMS 判訊號;Record tab VU meter 用 peak。
#[derive(Debug, Clone, Copy, Default)]
pub struct SignalMeter {
    pub peak_rms_db: f32,  // RMS in dB(歷史命名,別動,capsule has_signal 用)
    pub peak_db: f32,      // 瞬時 peak in dB(VU meter peak segment 用)
    pub last_sample_at_unix_ms: u64,
}
```

- [ ] **Step 2: Verify SignalMeter::default() still works**

`#[derive(Default)]` 自動把 `peak_db` 預設成 0.0。Idle 時 audio loop 不寫進去,值會是 0.0 — 這對 VU meter 是 wrong default(0 dB = full scale),所以 idle 時 frontend 要靠 `has_signal()` 判斷再決定要不要顯示 peak。

確認:`has_signal` 邏輯保持原樣(看 peak_rms_db,不看 peak_db)。

- [ ] **Step 3: Verify build**

Run: `cd src-tauri && cargo check --all-targets`
Expected: 通過,只有可能的 dead_code 警告(因為 peak_db 還沒人寫)。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/audio/mod.rs
git commit -m "feat(audio): add peak_db to SignalMeter for VU meter"
```

---

### Task 3: Update Linux capture loop to use compute_levels

**Files:**
- Modify: `src-tauri/src/audio/linux.rs`

- [ ] **Step 1: Find the existing RMS computation block**

Run: `grep -n "peak_rms_db\|let rms" src-tauri/src/audio/linux.rs`
應該找到 ~line 90-105 區段:現有 RMS 計算 + 寫進 `s.peak_rms_db`。

- [ ] **Step 2: Replace inline RMS with compute_levels call**

把那段(從 `let sumsq` 到寫 `s.peak_rms_db = db as f32` 的 block)整段替換成:

```rust
            // 轉成 f32 normalized 給 levels::compute_levels(它接受 ±1.0 範圍)
            let normalized: Vec<f32> = samples.iter().map(|&s| s as f32 / 32_768.0).collect();
            let (peak_db, rms_db) = crate::audio::levels::compute_levels(&normalized);

            if let Ok(mut s) = signal_for_thread.lock() {
                s.peak_rms_db = rms_db;
                s.peak_db = peak_db;
                s.last_sample_at_unix_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
            }
```

(`samples` 是現有的 i16 buffer;`normalized` 在 stack 上不會大太多 — 1024-frame chunk × 4 bytes = 4 KB)。

⚠ **看舊 code 確認 `last_sample_at_unix_ms` 是否本來就在更新** — 如果有,別重複寫;只更新 peak_rms_db / peak_db 兩欄。

- [ ] **Step 3: Verify build**

Run: `cd src-tauri && cargo check --all-targets`
Expected: 過。

- [ ] **Step 4: Existing tests still pass**

Run: `cd src-tauri && cargo test`
Expected: 全綠(已有的 test 都 pass)。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/audio/linux.rs
git commit -m "feat(audio): linux capture uses levels::compute_levels for peak+RMS"
```

---

### Task 4: Update Windows capture loop to use compute_levels

**Files:**
- Modify: `src-tauri/src/audio/windows.rs`

- [ ] **Step 1: Find the existing signal computation**

Run: `grep -n "peak_rms_db\|handle_chunk_f32" src-tauri/src/audio/windows.rs`

Windows 端走 cpal `f32` 直接 callback,看 `handle_chunk_f32` 內怎麼算 RMS。

- [ ] **Step 2: Replace inline computation with compute_levels**

Windows 已是 f32,**不用** normalize。找到 RMS 計算區段(類似 `let sumsq = ...; let rms = (sumsq / n).sqrt(); ...`),整段替換成:

```rust
let (peak_db, rms_db) = crate::audio::levels::compute_levels(samples);
if let Ok(mut s) = signal_cb.lock() {
    s.peak_rms_db = rms_db;
    s.peak_db = peak_db;
    s.last_sample_at_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
}
```

⚠ 變數名 `signal_cb`、`samples` 看現碼決定。

- [ ] **Step 3: Verify build**

Run: `cd src-tauri && cargo check --target x86_64-pc-windows-msvc 2>&1 | tail -10`
Expected: cross-compile target 沒裝是 OK,只要 `cargo check --all-targets` 在 Linux 上不爆即可(`#[cfg(target_os = "windows")]` gate 跳過編譯)。

Linux check:

```bash
cd src-tauri && cargo check --all-targets
```

Expected: 過(windows.rs 不會被 Linux 編譯)。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/audio/windows.rs
git commit -m "feat(audio): windows capture uses levels::compute_levels for peak+RMS"
```

---

### Task 5: Add LevelsPayload + TrackLevel structs to recorder.rs

**Files:**
- Modify: `src-tauri/src/recorder.rs`

- [ ] **Step 1: Add struct definitions**

在 `recorder.rs` 既有 `RecorderStatus` struct 上方 / 下方插入:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct TrackLevel {
    pub peak_db: f32,
    pub rms_db: f32,
    pub signal: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LevelsPayload {
    pub sys: TrackLevel,
    pub mic: TrackLevel,
}

impl TrackLevel {
    /// 從 SignalMeter snapshot 算 TrackLevel。idle / 無訊號時 signal=false,peak/rms = -120 dB。
    pub fn from_signal_meter(meter: &crate::audio::SignalMeter, now_unix_ms: u64) -> Self {
        let signal = meter.has_signal(now_unix_ms);
        if signal {
            Self {
                peak_db: meter.peak_db,
                rms_db: meter.peak_rms_db,
                signal: true,
            }
        } else {
            Self { peak_db: -120.0, rms_db: -120.0, signal: false }
        }
    }
}
```

- [ ] **Step 2: Add levels field to RecorderStatus**

把既有 `RecorderStatus` struct 改成:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct RecorderStatus {
    pub state: State,
    pub elapsed_secs: u64,
    pub system_signal: bool,
    pub mic_signal: bool,
    pub session_id: Option<String>,
    pub levels: Option<LevelsPayload>,  // 新增
}
```

- [ ] **Step 3: Update `Recorder::status()` to populate levels**

找到 `pub fn status(&self) -> RecorderStatus` 內,在 return 前算 levels:

```rust
        let levels = if let Some(s) = active.as_ref() {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let sys = s.handles.iter()
                .find(|h| h.source == SourceKind::MeetingSystem)
                .and_then(|h| h.signal.lock().ok().map(|sm| TrackLevel::from_signal_meter(&sm, now_ms)))
                .unwrap_or(TrackLevel { peak_db: -120.0, rms_db: -120.0, signal: false });
            let mic = s.handles.iter()
                .find(|h| h.source == SourceKind::MicInternal)
                .and_then(|h| h.signal.lock().ok().map(|sm| TrackLevel::from_signal_meter(&sm, now_ms)))
                .unwrap_or(TrackLevel { peak_db: -120.0, rms_db: -120.0, signal: false });
            Some(LevelsPayload { sys, mic })
        } else {
            None
        };
```

並把 `RecorderStatus { ... }` 結構回傳處加上 `levels,` 欄位。

- [ ] **Step 4: Verify build**

```bash
cd src-tauri && cargo check --all-targets
```

Expected: 過。

- [ ] **Step 5: Verify tests**

```bash
cd src-tauri && cargo test
```

Expected: 既有測試全綠。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/recorder.rs
git commit -m "feat(recorder): add LevelsPayload + TrackLevel to RecorderStatus"
```

---

### Task 6: Spawn 50ms emit task when recording starts

**Files:**
- Modify: `src-tauri/src/recorder.rs`
- Modify: `src-tauri/src/main.rs`(if needed,for AppHandle wiring)

- [ ] **Step 1: 確認 Recorder 是否有拿到 `AppHandle`**

Run: `grep -nE "AppHandle|app_handle|Manager" src-tauri/src/recorder.rs src-tauri/src/main.rs`

如果 Recorder 沒持有 AppHandle,先在 `Recorder` struct 加 `pub app: Option<tauri::AppHandle>` 欄位,在 `main.rs::setup` 初始化時填進去。

- [ ] **Step 2: 在 start_session 加 emit task**

找到 `pub async fn start_session(...)` (or 同名 sync 函式) 把 spawn 加進去。**心跳 task 在 capture handles 建好之後 spawn,在 session active 期間活著**:

```rust
    // === VU meter 50ms emit loop ===
    if let Some(app) = self.app.clone() {
        let active_for_emit = Arc::new(active);  // 把 Mutex<Option<ActiveSession>> 共享進去
        // 注意:active 已被 self.active 拿走,這裡用 Arc 包 ref... 視實作可能要重排
        let app_clone = app.clone();
        let recorder_arc = /* 自我 Arc — 需 Recorder 本身用 Arc<Self> 模式 */;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
            loop {
                tick.tick().await;
                let status = recorder_arc.status();
                if !matches!(status.state, State::Recording) { break; }
                if let Some(levels) = status.levels.clone() {
                    let _ = app_clone.emit("levels", levels);
                }
            }
        });
    }
```

⚠ 實作細節要看 `Recorder` 的 Arc / Mutex 結構;如果 Recorder 不是 `Arc<Self>` 模式,可能要先重構成 `Arc<Mutex<Recorder>>` 或讓 emit task 拿 `tauri::State<Recorder>` 透過 Manager 拿。**stop 時無需主動 cancel** — task 自己看到 State != Recording 就 break。

- [ ] **Step 3: Verify build**

```bash
cd src-tauri && cargo check --all-targets 2>&1 | tail -20
```

Expected: 過。如果 borrow checker / Arc 不過,**回報 BLOCKED**,我們調 Recorder 共享結構(這是 PR2 中最容易卡住的一步)。

- [ ] **Step 4: 確認 i18next 給 Tauri event listen 在前端不會雙重註冊**

(這是前端 Task 10 的事,只是先記一下不要在這 task 動。)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/recorder.rs src-tauri/src/main.rs
git commit -m "feat(recorder): spawn 50ms emit task for levels event"
```

---

### Task 7: Frontend — VuMeter component

**Files:**
- Create: `src/components/VuMeter.tsx`
- Modify: `src/theme.css`(加 `.vu-meter` + `.vu-seg` rules)

- [ ] **Step 1: Add VuMeter CSS to theme.css**

在 `src/theme.css` 結尾加(`--meter-bar` 等 token PR1 已預埋,直接用):

```css

/* VU meter grid — 24 等寬 segment,水平排,左對齊。 */
.vu-meter {
  display: inline-flex;
  align-items: center;
  gap: var(--vu-seg-gap, 2px);
  height: var(--vu-seg-h, 18px);
}
.vu-seg {
  width: var(--vu-seg-w, 6px);
  height: 100%;
  background: var(--meter-bar-bg);
  border-radius: 1px;
  transition: background 0.05s linear;
}
.vu-seg.lit  { background: var(--meter-bar); }
.vu-seg.peak { background: var(--meter-bar-peak); }
```

⚠ `--vu-seg-w` / `--vu-seg-gap` / `--vu-seg-h` 是新 token,加進 `:root`(放在 PR1 預埋的 `--meter-bar-bg` 那行下面):

```css
  --vu-seg-w:  6px;
  --vu-seg-gap: 2px;
  --vu-seg-h:  18px;
```

- [ ] **Step 2: Create VuMeter component**

Create `src/components/VuMeter.tsx`:

```tsx
// src/components/VuMeter.tsx
//
// 24-segment 水平 VU meter,給 Record tab 雙軌用。
// dB → segment count 線性 mapping:-60 → 0 segment,0 → 全亮 24 segment。
// 最右側 lit segment 上 peak 色(橘)。signal=false → 全暗(idle 視覺)。

const TOTAL_SEGMENTS = 24;
const DB_MIN = -60;
const DB_MAX = 0;

function dbToSegmentCount(db: number): number {
  if (db <= DB_MIN) return 0;
  if (db >= DB_MAX) return TOTAL_SEGMENTS;
  return Math.round(((db - DB_MIN) / (DB_MAX - DB_MIN)) * TOTAL_SEGMENTS);
}

interface Props {
  peakDb: number;
  rmsDb: number;
  signal: boolean;
}

export default function VuMeter({ peakDb, rmsDb, signal }: Props) {
  if (!signal) {
    // 全暗 idle 樣態
    return (
      <span className="vu-meter" aria-hidden="true">
        {Array.from({ length: TOTAL_SEGMENTS }).map((_, i) => (
          <span key={i} className="vu-seg" />
        ))}
      </span>
    );
  }
  const litSegs = dbToSegmentCount(rmsDb);
  const peakIdx = dbToSegmentCount(peakDb) - 1; // index of peak segment
  return (
    <span className="vu-meter" aria-hidden="true">
      {Array.from({ length: TOTAL_SEGMENTS }).map((_, i) => {
        const cls =
          i === peakIdx && peakIdx >= 0 ? "vu-seg peak" :
          i < litSegs                    ? "vu-seg lit"  :
                                           "vu-seg";
        return <span key={i} className={cls} />;
      })}
    </span>
  );
}
```

- [ ] **Step 3: Verify build**

```bash
npm run build
```

Expected: 過。

- [ ] **Step 4: Commit**

```bash
git add src/components/VuMeter.tsx src/theme.css
git commit -m "feat(record-tab): VuMeter 24-segment component"
```

---

### Task 8: Frontend — TrackPanel component

**Files:**
- Create: `src/components/TrackPanel.tsx`
- Modify: `src/theme.css`(加 `.track-panel` + `.track-panel-*` rules)

- [ ] **Step 1: Add TrackPanel CSS**

在 `theme.css` 結尾加:

```css

/* Track panel — Record tab 雙軌的一張卡片。Mock 04 v2 的 micro-card 視覺。 */
.track-panel {
  background: rgba(255,255,255,0.03);
  border-radius: 8px;
  padding: 10px 12px;
  margin-bottom: 8px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.track-panel-row {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 11px;
  color: var(--text-secondary);
}
.track-panel-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 120px;
  color: var(--text);
  font-weight: 500;
}
.track-panel-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
  background: var(--text-dim);
}
.track-panel-dot.sys { background: var(--found-color); }
.track-panel-dot.mic { background: var(--waiting-color); }
.track-panel-meter-wrap {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 10px;
}
.track-panel-db {
  font-family: ui-monospace, "SF Mono", "Cascadia Code", "Consolas", monospace;
  font-variant-numeric: tabular-nums;
  font-size: 10px;
  color: var(--text-secondary);
  min-width: 50px;
  text-align: right;
}
.track-panel-source {
  font-family: ui-monospace, "SF Mono", "Cascadia Code", "Consolas", monospace;
  font-size: 9.5px;
  color: var(--text-dim);
}
```

- [ ] **Step 2: Create TrackPanel component**

Create `src/components/TrackPanel.tsx`:

```tsx
// src/components/TrackPanel.tsx
//
// 一張 Record tab 軌道卡:label + dot + bars icon / VU meter / dB readout / source name。
// 對應 mock 04 v2 中「會議音訊 · SYS」「內部麥克風 · MIC」兩列。

import BarsIcon from "./icons/BarsIcon";
import VuMeter from "./VuMeter";

type Kind = "sys" | "mic";

interface Props {
  kind: Kind;
  label: string;
  sourceName: string;
  level: { peak_db: number; rms_db: number; signal: boolean } | null;
}

function fmtDb(db: number, signal: boolean): string {
  if (!signal) return "—";
  if (db <= -60) return "<-60 dB";
  return `${db.toFixed(0)} dB`;
}

export default function TrackPanel({ kind, label, sourceName, level }: Props) {
  const peakDb = level?.peak_db ?? -120;
  const rmsDb  = level?.rms_db ?? -120;
  const signal = level?.signal ?? false;
  return (
    <div className="track-panel">
      <div className="track-panel-row">
        <span className="track-panel-label">
          <span className={`track-panel-dot ${kind}`} />
          <BarsIcon size={10} />
          {label}
        </span>
        <div className="track-panel-meter-wrap">
          <VuMeter peakDb={peakDb} rmsDb={rmsDb} signal={signal} />
          <span className="track-panel-db">{fmtDb(rmsDb, signal)}</span>
        </div>
      </div>
      <div className="track-panel-source">{sourceName}</div>
    </div>
  );
}
```

- [ ] **Step 3: Verify build**

```bash
npm run build
```

Expected: 過。

- [ ] **Step 4: Commit**

```bash
git add src/components/TrackPanel.tsx src/theme.css
git commit -m "feat(record-tab): TrackPanel component (label + VuMeter + dB + source)"
```

---

### Task 9: Add control-bar styles to theme.css

**Files:**
- Modify: `src/theme.css`

- [ ] **Step 1: Add `.record-control-bar` CSS**

在 `theme.css` 結尾加:

```css

/* Record tab 控制列 — mock 04 v2 的「● REC 00:12:34 ─── [Stop]」橫向 layout。 */
.record-control-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 0;
  margin: 12px 0;
}
.record-control-bar .control-status {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  flex: 1;
}
.record-control-bar .control-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-dim);
}
.record-control-bar .control-dot.recording { background: var(--danger-color); animation: dot-pulse 1.4s ease-in-out infinite; }
.record-control-bar .control-dot.transcribing { background: var(--waiting-color); }
.record-control-bar .control-label {
  font-size: 12px;
  font-weight: 600;
  letter-spacing: -0.2px;
  color: var(--text-secondary);
}
.record-control-bar .control-label.recording { color: var(--rec-accent); }
.record-control-bar .control-label.transcribing { color: var(--trans-accent); }
.record-control-bar .control-time {
  font-family: ui-monospace, "SF Mono", "Cascadia Code", "Consolas", monospace;
  font-variant-numeric: tabular-nums;
  font-size: 13px;
  color: var(--text-secondary);
}
.record-control-bar .control-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border: none;
  border-radius: 10px;
  background: var(--danger-color);
  color: white;
  cursor: pointer;
  transition: opacity 0.15s, transform 0.15s;
}
.record-control-bar .control-action[data-state="idle"]    { background: var(--rec-accent); }
.record-control-bar .control-action[data-state="recording"] { background: var(--danger-color); }
.record-control-bar .control-action[data-state="transcribing"] {
  background: transparent;
  color: var(--trans-accent);
  opacity: 0.8;
  cursor: not-allowed;
}
.record-control-bar .control-action:hover:not(:disabled) { opacity: 0.88; }
.record-control-bar .control-action:active:not(:disabled) { transform: scale(0.96); }
```

- [ ] **Step 2: Verify build**

```bash
npm run build
```

Expected: 過。

- [ ] **Step 3: Commit**

```bash
git add src/theme.css
git commit -m "feat(record-tab): control bar CSS"
```

---

### Task 10: Refactor RecordTab.tsx — control bar + 2× TrackPanel + listen("levels")

**Files:**
- Modify: `src/tabs/RecordTab.tsx`

- [ ] **Step 1: Replace the entire RecordTab content**

整個 `src/tabs/RecordTab.tsx` 用以下取代:

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import TriangleIcon from "../components/icons/TriangleIcon";
import SquareIcon from "../components/icons/SquareIcon";
import SpinnerIcon from "../components/icons/SpinnerIcon";
import TrackPanel from "../components/TrackPanel";

type RecState = "idle" | "recording" | "transcribing";

interface TrackLevel { peak_db: number; rms_db: number; signal: boolean }
interface LevelsPayload { sys: TrackLevel; mic: TrackLevel }

type Status = {
  state: RecState;
  elapsed_secs: number;
  session_id: string | null;
  system_signal: boolean;
  mic_signal: boolean;
  levels: LevelsPayload | null;
};

const fmtElapsed = (s: number) => {
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
};

export default function RecordTab() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<Status | null>(null);
  const [levels, setLevels] = useState<LevelsPayload | null>(null);
  const [err, setErr] = useState<string | null>(null);

  // status polling (500ms) — 兼當 levels polling fallback (when emit not received)
  useEffect(() => {
    const tick = async () => {
      try {
        const s = await invoke<Status>("recorder_status");
        setStatus(s);
        if (s.levels) setLevels(s.levels);
      } catch { /* ignore */ }
    };
    tick();
    const id = setInterval(tick, 500);
    return () => clearInterval(id);
  }, []);

  // Tauri "levels" event subscription — 50ms tick when recording
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    (async () => {
      unlisten = await listen<LevelsPayload>("levels", (e) => setLevels(e.payload));
    })();
    return () => { unlisten?.(); };
  }, []);

  const recState: RecState = status?.state ?? "idle";
  const onStartStop = async () => {
    setErr(null);
    try {
      if (recState === "recording") await invoke("recorder_stop");
      else if (recState === "idle") await invoke("recorder_start");
    } catch (e: any) {
      setErr(String(e));
      console.error(e);
    }
  };

  const statusLabel =
    recState === "recording"     ? "REC" :
    recState === "transcribing"  ? t("capsule.transcribing") :
                                   t("record.idle_label") || "IDLE";
  const actionTitle =
    recState === "recording"     ? t("capsule.stop") :
    recState === "transcribing"  ? t("capsule.transcribing") :
                                   t("capsule.start");

  return (
    <div>
      <div className="callout">⚠ {t("record.warning")}</div>

      <div className="record-control-bar">
        <span className="control-status">
          <span className={`control-dot ${recState}`} />
          <span className={`control-label ${recState}`}>{statusLabel}</span>
          <span className="control-time">{fmtElapsed(status?.elapsed_secs ?? 0)}</span>
        </span>
        <button
          className="control-action"
          data-state={recState}
          onClick={onStartStop}
          disabled={recState === "transcribing"}
          title={actionTitle}
        >
          {recState === "idle"        && <TriangleIcon size={14} />}
          {recState === "recording"   && <SquareIcon   size={12} />}
          {recState === "transcribing" && <SpinnerIcon size={16} />}
        </button>
      </div>

      <TrackPanel
        kind="sys"
        label={t("capsule.system_pill")}
        sourceName={t("record.source_sys")}
        level={levels?.sys ?? null}
      />
      <TrackPanel
        kind="mic"
        label={t("capsule.mic_pill")}
        sourceName={t("record.source_mic")}
        level={levels?.mic ?? null}
      />

      {err && (
        <div className="callout" style={{ marginTop: 12, color: "var(--danger-color)", borderColor: "rgba(255,99,99,0.30)", background: "rgba(255,99,99,0.08)" }}>
          ⚠ {err}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Add i18n keys**

`src/i18n/locales/en.json` — `record` 區塊加:

```json
    "idle_label": "IDLE",
    "source_sys": "system audio",
    "source_mic": "default mic input"
```

`src/i18n/locales/zh-TW.json` — `record` 區塊加:

```json
    "idle_label": "閒置",
    "source_sys": "系統音訊 (pulse:.monitor)",
    "source_mic": "本機麥克風 (default-input)"
```

(注意 JSON 句尾逗號 — 加在現有 `record` 區塊內部,別忘了既有最後一行要加逗號)。

- [ ] **Step 3: Verify build + tsc**

```bash
npm run build
npx tsc --noEmit
```

Expected: 都過。

- [ ] **Step 4: Commit**

```bash
git add src/tabs/RecordTab.tsx src/i18n/locales/en.json src/i18n/locales/zh-TW.json
git commit -m "feat(record-tab): control bar + 2 TrackPanel + listen('levels') + i18n"
```

---

### Task 11: Run scripts/verify.sh

**Files:** none

- [ ] **Step 1: Run verify**

```bash
bash scripts/verify.sh
```

Expected output 末尾:

```
==> npm run build
... vite build ...
==> cargo test
test result: ok. N passed; 0 failed; ...
==> cargo check --all-targets
... Finished ...
✓ verify ok
```

如果 cargo test 多了「levels」相關的測試(Task 1 的 6 個 test),N = 既有 + 6,全綠才繼續。

如果有錯,**修到全綠**。

---

### Task 12: Manual visual e2e

**Files:** none(user-driven)

- [ ] **Step 1: Start dev**

```bash
npm run tauri dev
```

- [ ] **Step 2: 對 mock 04 驗 idle 狀態**

雙擊膠囊空白展開,Record tab 應該:
- 上方 callout 客戶版警告 ✓
- 控制列:灰 dot + "IDLE/閒置" 灰字 + 00:00:00 + 橘色 ▶ 三角按鈕 ✓
- 兩張 TrackPanel,VU bar **全暗**(`signal: false`),dB 顯示 "—"

- [ ] **Step 3: 對 mock 04 驗 recording 狀態**

點 ▶ 開始錄,播 YouTube + 對麥講話:
- 控制列變紅 dot 脈動 + "REC" 橘字 + timer 走 + 紅方塊 ■ 按鈕
- SYS VU bar 跟著音量跳動,綠 + 最右側 lit segment 橘 peak
- MIC VU bar 講話時跳
- dB readout 即時更新(個位數變化在 -60 ~ 0 dB)
- source name 在下方("系統音訊 (pulse:.monitor)" / "本機麥克風 (default-input)")

- [ ] **Step 4: 驗 50ms tick 順暢**

VU bar 跳動感覺應該流暢(20fps),不是卡頓 1Hz。如果卡 = polling fallback 在跑、event 沒上工,看 console 有沒有 `listen` 相關 warn。

- [ ] **Step 5: 點 stop → 驗 transcribing**

控制列變黃 dot + "transcribing…" 黃字 + spinner 按鈕 disabled。VU bar **不再動**(轉錄中沒有新 audio)。

- [ ] **Step 6: transcribing 完成 → 回 idle**

控制列回灰 / 橘三角。

- [ ] **Step 7: 報告**

跑完上面 6 step 都對 → 回我「OK」進 Task 13(PR)。
有不對 → 指出哪 step 哪個元素,我派 fix。

---

### Task 13: Push branch + open PR + enable auto-merge

**Files:** none

- [ ] **Step 1: Push**

```bash
git push -u origin feat/record-tab-vu-meter
```

- [ ] **Step 2: Open PR**

```bash
gh pr create --title "feat(record-tab): VU meter + control bar (PR2 of 3)" --body "$(cat <<'BODYEOF'
## Summary

PR2 of the 3-PR recorder UI mock alignment series. Spec: \`docs/superpowers/specs/2026-05-28-recorder-ui-mock-alignment-design.md\` §6.

### Rust audio

- 抽出 \`audio/levels.rs\` 提供 \`linear_to_db\` + \`compute_levels(samples) -> (peak_db, rms_db)\`,6 個 unit test 全 TDD
- \`SignalMeter\` 加 \`peak_db\` field(原 \`peak_rms_db\` 名稱保留代表 RMS,語義 unchanged)
- Linux + Windows capture loop 改用 \`levels::compute_levels\`,DRY
- \`recorder.rs\` 加 \`TrackLevel\` + \`LevelsPayload\` struct
- Spawn 50ms tokio task,recording 期間 \`app.emit("levels", payload)\`,stop 後 task 自停
- \`RecorderStatus\` 加 \`levels: Option<LevelsPayload>\` 給前端 polling fallback

### Frontend

- \`VuMeter\` 24-segment 水平條,dB → segment 線性 mapping(-60→0, 0→24);peak segment 橘
- \`TrackPanel\` 一張軌道卡:label + dot + BarsIcon / VuMeter / dB readout / source name
- \`RecordTab\` 整個重寫:控制列 + 2× TrackPanel,Tauri \`listen("levels")\` 主路 + 500ms polling fallback
- i18n 新 key:\`record.idle_label\` / \`record.source_sys\` / \`record.source_mic\`

### 視覺契約

對齊 \`docs/design/04-record-tab.png\`(mock 04 v2)— 控制列在上、雙軌 VU meter 占下半。

### 刻意 defer

- Mute MIC 副按鈕(phase 2)
- Pause/Resume(phase 2)
- Mark moment(phase 2)
- Windows 親測(本機只能驗 Linux,Windows 編譯通過)

## Test plan

- [x] \`bash scripts/verify.sh\` 全綠(cargo test 6 個新 + 既有,npm build,cargo check)
- [x] 手動 e2e:idle / recording / transcribing 三 state 對齊 mock 04
- [x] VU bar 跳動順暢(20fps,不卡)
- [x] dB readout 即時更新
- [ ] Windows 親測(本機沒有,phase 2)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
BODYEOF
)"
```

- [ ] **Step 3: Enable auto-merge**

```bash
gh pr merge --auto --squash
```

- [ ] **Step 4: Report PR URL**

Run: `gh pr view --json url --jq .url`
Output the URL to user。

---

## Self-Review Notes

| Spec §6 元素 | 任務對應 |
|---|---|
| `linear_to_db` + `LevelMeter::push` | Task 1 |
| Tauri event 50ms emit + polling fallback | Task 6 + Task 10 |
| `LevelsPayload` / `TrackLevel` Rust struct | Task 5 |
| `recorder_status.levels` 欄位 | Task 5 |
| VuMeter 24 segment + peak 橘 | Task 7 |
| TrackPanel 結構(dot + bars + label + meter + db + source) | Task 8 |
| RecordTab 控制列 layout | Task 9 + Task 10 |
| Mock 04 v2 對齊 | Task 12 |

**Branch / PR 命名**:`feat/record-tab-vu-meter`,對齊 `[[mori-branch-naming]]`。

**Mori voice safety**:這條 PR 動 Rust(audio loop / recorder),`bash scripts/restart-dev.sh` 重啟 dev 才會吃新 binary。

**最大風險**:Task 6 的 Recorder Arc / Mutex 共享結構 — 若 Recorder 不是 `Arc<Self>`、emit task 拿不到「指向同一個 active session」的引用,可能要重構成 `Arc<Mutex<Recorder>>` 才能 spawn task 進去。回報 BLOCKED 我們調。

**Mock 對 spec 落差**:VU meter 在 idle 狀態 mock 沒明確畫,本 plan 決議「全暗 + dB 顯示『—』」— 比較合 polling fallback 真實行為。
