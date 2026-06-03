# 收音裝置選擇 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Record 分頁下拉選收音裝置(現場:麥;線上:系統音訊來源 + 麥),不用改 OS 預設。

**Architecture:** 兩個 config 欄位(`input_device` / `system_source`,空=預設)。新 `audio/devices.rs` 列舉裝置 + `list_audio_devices` 命令。純函式 `resolve_device(source, cfg)` 算出每軌要開的裝置;`open_capture` 加 `device: Option<String>` 參數(`Some` 用該裝置、`None` 維持現狀),`pick_source`/`pick_device` 不變。RecordTab 模式感知下拉。

**Tech Stack:** Rust(`pactl` 解析 / cpal / serde)、Tauri v2 command、React + TS、react-i18next。

**Spec:** `docs/superpowers/specs/2026-06-03-input-device-selection-design.md`

**Worktree / branch:** `/home/ct/mori-universe/.worktrees/recorder-device-selection` @ `feat/input-device-selection`(off origin/main `8746607`)。

⚠ cargo 在 `src-tauri/` 內跑(無 root Cargo.toml);generate_context! 需 `dist/` → 先 `npm run build` 再 cargo。手測 `npm run tauri dev`(動 Rust 要重啟)。Windows 分支在 Linux 上 `cfg` 掉,靠 cargo check 文法把關。

---

## File Structure

| 檔案 | 動作 | 責任 |
|---|---|---|
| `src-tauri/src/config.rs` | Modify | 加 `input_device` + `system_source` 欄位 |
| `src-tauri/src/audio/devices.rs` | Create | 列舉輸入/系統源(pactl 解析 / cpal)+ 純解析函式 |
| `src-tauri/src/audio/mod.rs` | Modify | `mod devices;` + `resolve_device()` + `open_capture` 加 `device` 參數(dispatch ×3) |
| `src-tauri/src/audio/linux.rs` | Modify | `open_capture` 用 `device`(Some→source name / None→pick_source) |
| `src-tauri/src/audio/windows.rs` | Modify | `open_capture` 用 `device`(Some→by-name 找,miss 退預設 / None→pick_device) |
| `src-tauri/src/recorder.rs` | Modify | start_session / voice / enroll 三處傳 `resolve_device(...)` |
| `src-tauri/src/main.rs` | Modify | `list_audio_devices` 命令 + 註冊 |
| `src/tabs/RecordTab.tsx` | Modify | 模式感知裝置下拉 |
| `src/i18n/locales/{en,zh-TW}.json` | Modify | 裝置字串 |

---

## Task 1: config 加 `input_device` + `system_source`

**Files:** Modify `src-tauri/src/config.rs`

- [ ] **Step 1: 加 default fns**

在 `default_recording_mode`(現有,#82 加的)之後加:
```rust
fn default_input_device() -> String {
    String::new() // "" = 系統預設輸入
}
fn default_system_source() -> String {
    String::new() // "" = 自動挑 default sink 的 .monitor
}
```

- [ ] **Step 2: 加 struct 欄位**

在 `RecorderConfig` 的 `recording_mode` 欄位之後加:
```rust
    #[serde(default = "default_input_device")]
    pub input_device: String,
    #[serde(default = "default_system_source")]
    pub system_source: String,
```

- [ ] **Step 3: 加進 Default impl**

在 `impl Default for RecorderConfig` 的 `recording_mode: default_recording_mode(),` 之後加:
```rust
            input_device: default_input_device(),
            system_source: default_system_source(),
```

- [ ] **Step 4: 編譯 + commit**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-device-selection/src-tauri && cargo check 2>&1 | tail -6`
Expected: 通過。
```bash
cd /home/ct/mori-universe/.worktrees/recorder-device-selection
git add src-tauri/src/config.rs
git commit -m "feat(recorder): config input_device + system_source (空=預設)"
```

---

## Task 2: `audio/devices.rs` 列舉 + `list_audio_devices` 命令

**Files:**
- Create: `src-tauri/src/audio/devices.rs`
- Modify: `src-tauri/src/audio/mod.rs`(加 `pub mod devices;`)
- Modify: `src-tauri/src/main.rs`(命令 + 註冊)

- [ ] **Step 1: 建 devices.rs(含 Linux 純解析 + 失敗測試)**

Create `src-tauri/src/audio/devices.rs`:
```rust
//! 列舉收音裝置:輸入(麥)+ 系統源(monitor)。Linux 走 pactl、Windows 走 cpal。
//! 解析與平台呼叫分離 → 純解析函式可單測。

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DeviceInfo {
    pub id: String,    // 開裝置用的技術名(pulse source name / cpal device name)
    pub label: String, // 友善顯示名
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AudioDevices {
    pub inputs: Vec<DeviceInfo>,
    pub system_sources: Vec<DeviceInfo>,
}

/// 解析 `pactl list sources`(verbose)→ name→Description map。
#[cfg(any(target_os = "linux", test))]
fn parse_descriptions(verbose: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let mut cur_name: Option<String> = None;
    for line in verbose.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Name: ") {
            cur_name = Some(rest.trim().to_string());
        } else if let Some(rest) = t.strip_prefix("Description: ") {
            if let Some(n) = &cur_name {
                map.insert(n.clone(), rest.trim().to_string());
            }
        }
    }
    map
}

/// 純函式:把 `pactl list short sources` + verbose 解析成 AudioDevices。
/// short 每行第 2 欄=name;`.monitor` 結尾→system_sources、否則→inputs。label 取 Description,缺則用 name。
#[cfg(any(target_os = "linux", test))]
pub fn parse_pactl(short: &str, verbose: &str) -> AudioDevices {
    let desc = parse_descriptions(verbose);
    let mut inputs = Vec::new();
    let mut system_sources = Vec::new();
    for line in short.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 2 {
            continue;
        }
        let name = cols[1];
        let label = desc.get(name).cloned().unwrap_or_else(|| name.to_string());
        let info = DeviceInfo { id: name.to_string(), label };
        if name.ends_with(".monitor") {
            system_sources.push(info);
        } else {
            inputs.push(info);
        }
    }
    AudioDevices { inputs, system_sources }
}

#[cfg(target_os = "linux")]
pub fn list_devices() -> AudioDevices {
    use std::process::Command;
    let short = Command::new("pactl").args(["list", "short", "sources"]).output();
    let verbose = Command::new("pactl").args(["list", "sources"]).output();
    let short = short.ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let verbose = verbose.ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    parse_pactl(&short, &verbose)
}

#[cfg(target_os = "windows")]
pub fn list_devices() -> AudioDevices {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let to_info = |d: &cpal::Device| {
        let name = d.name().unwrap_or_else(|_| "(unknown)".into());
        DeviceInfo { id: name.clone(), label: name }
    };
    let inputs = host.input_devices().map(|it| it.map(|d| to_info(&d)).collect()).unwrap_or_default();
    let system_sources = host.output_devices().map(|it| it.map(|d| to_info(&d)).collect()).unwrap_or_default();
    AudioDevices { inputs, system_sources }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn list_devices() -> AudioDevices {
    AudioDevices { inputs: Vec::new(), system_sources: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pactl_splits_inputs_and_monitors_with_descriptions() {
        let short = "\
1062\talsa_input.pci-0000.analog-stereo\tPipeWire\ts16le\tSUSPENDED
1064\talsa_input.usb-fifine.mono-fallback\tPipeWire\ts24le\tSUSPENDED
1061\talsa_output.pci-0000.hdmi-stereo.monitor\tPipeWire\ts16le\tSUSPENDED";
        let verbose = "\
Source #1064
\tName: alsa_input.usb-fifine.mono-fallback
\tDescription: fifine Microphone Mono
Source #1061
\tName: alsa_output.pci-0000.hdmi-stereo.monitor
\tDescription: Monitor of HDMI";
        let d = parse_pactl(short, verbose);
        assert_eq!(d.inputs.len(), 2);
        assert_eq!(d.system_sources.len(), 1);
        // 友善名稱套用
        let fifine = d.inputs.iter().find(|x| x.id.contains("fifine")).unwrap();
        assert_eq!(fifine.label, "fifine Microphone Mono");
        // 無 Description 的退技術名
        let builtin = d.inputs.iter().find(|x| x.id.contains("analog-stereo")).unwrap();
        assert_eq!(builtin.label, builtin.id);
        // monitor 進 system_sources
        assert!(d.system_sources[0].id.ends_with(".monitor"));
    }

    #[test]
    fn parse_pactl_empty_input_yields_empty() {
        let d = parse_pactl("", "");
        assert!(d.inputs.is_empty() && d.system_sources.is_empty());
    }
}
```

- [ ] **Step 2: 跑測試確認失敗→實作已含→通過**

先在 `src-tauri/src/audio/mod.rs` 的 module 宣告區(`pub mod writer; pub mod levels; pub mod vad;` 附近)加:
```rust
pub mod devices;
```
Run: `cd /home/ct/mori-universe/.worktrees/recorder-device-selection/src-tauri && cargo test parse_pactl 2>&1 | tail -15`
Expected: 2 tests PASS。

- [ ] **Step 3: 加 `list_audio_devices` 命令 + 註冊**

`src-tauri/src/main.rs`:在 `file_transcribe_list_dir` 命令附近(或任一命令之後)加:
```rust
/// 列舉收音裝置(輸入麥 + 系統 monitor 源)給前端下拉。
#[tauri::command]
fn list_audio_devices() -> audio::devices::AudioDevices {
    audio::devices::list_devices()
}
```
在 `generate_handler!`(`:907` 起)清單內加一行:
```rust
            list_audio_devices,
```

- [ ] **Step 4: 編譯 + commit**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-device-selection/src-tauri && cargo check 2>&1 | tail -6`
Expected: 通過。
```bash
cd /home/ct/mori-universe/.worktrees/recorder-device-selection
git add src-tauri/src/audio/devices.rs src-tauri/src/audio/mod.rs src-tauri/src/main.rs
git commit -m "feat(recorder): audio/devices.rs 列舉 + list_audio_devices 命令"
```

---

## Task 3: `resolve_device()` 純函式

**Files:** Modify `src-tauri/src/audio/mod.rs`

- [ ] **Step 1: 寫失敗測試**

在 `src-tauri/src/audio/mod.rs` 的測試模組(Task #82 加的 `source_kind_tests`,或新 `#[cfg(test)] mod resolve_tests`)內加:
```rust
#[cfg(test)]
mod resolve_tests {
    use super::*;
    use crate::config::RecorderConfig;

    #[test]
    fn resolve_device_maps_source_to_config_field() {
        let mut cfg = RecorderConfig::default();
        // 預設(空)→ None
        assert_eq!(resolve_device(SourceKind::MicInternal, &cfg), None);
        assert_eq!(resolve_device(SourceKind::MeetingRoom, &cfg), None);
        assert_eq!(resolve_device(SourceKind::MeetingSystem, &cfg), None);
        // 有值 → Some
        cfg.input_device = "mic-x".into();
        cfg.system_source = "monitor-y".into();
        assert_eq!(resolve_device(SourceKind::MicInternal, &cfg), Some("mic-x".into()));
        assert_eq!(resolve_device(SourceKind::MeetingRoom, &cfg), Some("mic-x".into()));
        assert_eq!(resolve_device(SourceKind::MeetingSystem, &cfg), Some("monitor-y".into()));
        // 空白字串視為未選
        cfg.input_device = "   ".into();
        assert_eq!(resolve_device(SourceKind::MicInternal, &cfg), None);
    }
}
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-device-selection/src-tauri && cargo test resolve_device_maps 2>&1 | tail -12`
Expected: 編譯失敗 `cannot find function resolve_device`。

- [ ] **Step 3: 實作 `resolve_device`**

在 `src-tauri/src/audio/mod.rs` 的 `impl SourceKind` 之後(top-level fn)加:
```rust
/// 依 source + config 算出要開的裝置名;None = 用平台預設
/// (麥=系統預設輸入、系統=auto-monitor)。麥/room 看 input_device,系統看 system_source。
pub fn resolve_device(source: SourceKind, cfg: &crate::config::RecorderConfig) -> Option<String> {
    let pick = match source {
        SourceKind::MicInternal | SourceKind::MeetingRoom => &cfg.input_device,
        SourceKind::MeetingSystem => &cfg.system_source,
    };
    if pick.trim().is_empty() {
        None
    } else {
        Some(pick.clone())
    }
}
```

- [ ] **Step 4: 跑測試確認通過 + commit**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-device-selection/src-tauri && cargo test resolve_device_maps 2>&1 | tail -12`
Expected: PASS。
```bash
cd /home/ct/mori-universe/.worktrees/recorder-device-selection
git add src-tauri/src/audio/mod.rs
git commit -m "feat(recorder): resolve_device(source, cfg) 純函式"
```

---

## Task 4: `open_capture` 加 `device` 參數 + recorder 三處接線

> 簽名變更要一次到位(所有 call site 同時改才編得過)。

**Files:** Modify `src-tauri/src/audio/mod.rs`、`audio/linux.rs`、`audio/windows.rs`、`recorder.rs`

- [ ] **Step 1: mod.rs 三個 dispatch wrapper 加參數**

`src-tauri/src/audio/mod.rs` 把三個 `open_capture`(linux / windows / 其他 fallback)都加 `device: Option<String>`(放在 `source` 之後)並傳進去。例如 linux 版:
```rust
#[cfg(target_os = "linux")]
pub fn open_capture(
    source: SourceKind,
    device: Option<String>,
    out_path: std::path::PathBuf,
    vad_cfg: vad::VadConfig,
    pending: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) -> Result<CaptureResult, String> {
    linux::open_capture(source, device, out_path, vad_cfg, pending)
}
```
windows 版同樣加 `device` 並傳 `windows::open_capture(source, device, out_path, vad_cfg, pending)`;`#[cfg(not(any(...)))]` fallback 版加 `_device: Option<String>` 參數(不使用)。

- [ ] **Step 2: linux.rs open_capture 用 device**

`src-tauri/src/audio/linux.rs` 的 `open_capture`(`:105`)加參數 `device: Option<String>`(在 `source` 之後),並把 `:111` 的
```rust
    let source_name = pick_source(source)?;
```
改成
```rust
    // 指定裝置 → 確認還在才用;不在則麥/room 退預設輸入(範圍#4),系統源讓它後續開失敗。
    // 未指定 → 維持原行為(麥=預設輸入、系統=auto-monitor)。
    let source_name = match device {
        Some(d) if pulse_source_exists(&d) => Some(d),
        Some(_) if matches!(source, SourceKind::MicInternal | SourceKind::MeetingRoom) => None,
        Some(d) => Some(d),
        None => pick_source(source)?,
    };
```
並在 `pick_source`(`:30`)之後加 helper:
```rust
/// 指定的 pulse source name 目前是否存在(裝置被拔/改名後退預設用)。
fn pulse_source_exists(name: &str) -> bool {
    std::process::Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l.split_whitespace().nth(1) == Some(name))
        })
        .unwrap_or(false)
}
```

- [ ] **Step 3: windows.rs open_capture 用 device(指定 miss 退預設)**

`src-tauri/src/audio/windows.rs` 的 `open_capture`(`:31`)加參數 `device: Option<String>`(在 `source` 之後),把 `:37` 的
```rust
    let device = pick_device(source)?;
```
改成(注意原本區域變數也叫 `device`,改用 `cpal_device`,後續用到 `device` 的地方一併改成 `cpal_device`):
```rust
    let cpal_device = match device.as_deref() {
        Some(name) => find_device_by_name(source, name).unwrap_or(pick_device(source)?),
        None => pick_device(source)?,
    };
```
並在 `pick_device`(`:16`)之後加 helper:
```rust
/// 依名稱在對應清單(麥→input、系統→output)找裝置;找不到回 None(呼叫端退預設)。
fn find_device_by_name(source: SourceKind, name: &str) -> Option<Device> {
    let host = cpal::default_host();
    let iter = match source {
        SourceKind::MeetingSystem => host.output_devices().ok()?,
        _ => host.input_devices().ok()?,
    };
    for d in iter {
        if d.name().map(|n| n == name).unwrap_or(false) {
            return Some(d);
        }
    }
    None
}
```
後面 `default_config` 的 match 與 stream 建立把 `device` 改用 `cpal_device`(該函式內所有後續 `device.` 都改 `cpal_device.`)。

- [ ] **Step 4: recorder.rs 三處 call site 傳 device**

(a) `start_session` 開軌迴圈(`:175`):把
```rust
            match audio::open_capture(kind, out, vad_cfg.clone(), prog.pending.clone()) {
```
改成
```rust
            let device = audio::resolve_device(kind, &cfg);
            match audio::open_capture(kind, device, out, vad_cfg.clone(), prog.pending.clone()) {
```
(`cfg` 已於 `:133` `let cfg = crate::config::read_config();` 讀到,直接用。)

(b) voice_input(`:418`)把
```rust
            audio::open_capture(SourceKind::MicInternal, temp_path.clone(), vad_cfg, dummy_pending)?;
```
改成
```rust
            let device = audio::resolve_device(SourceKind::MicInternal, &crate::config::read_config());
            audio::open_capture(SourceKind::MicInternal, device, temp_path.clone(), vad_cfg, dummy_pending)?;
```

(c) enroll(`:477`)同 (b) 一模一樣的改法(該行也是 `open_capture(SourceKind::MicInternal, temp_path.clone(), vad_cfg, dummy_pending)?`):
```rust
            let device = audio::resolve_device(SourceKind::MicInternal, &crate::config::read_config());
            audio::open_capture(SourceKind::MicInternal, device, temp_path.clone(), vad_cfg, dummy_pending)?;
```

- [ ] **Step 5: 全量編譯 + 測試**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-device-selection/src-tauri && cargo test 2>&1 | tail -15`
Expected: 全綠(含 resolve_device / parse_pactl 新測 + 既有回歸)。

- [ ] **Step 6: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-device-selection
git add src-tauri/src/audio/mod.rs src-tauri/src/audio/linux.rs src-tauri/src/audio/windows.rs src-tauri/src/recorder.rs
git commit -m "feat(recorder): open_capture 收 device 參數 + start/voice/enroll 接 resolve_device"
```

---

## Task 5: RecordTab 模式感知裝置下拉 + i18n

**Files:** Modify `src/tabs/RecordTab.tsx`、`src/i18n/locales/{en,zh-TW}.json`

> 沿用 RecordTab 既有 inline style + recorder `var(--…)` token。

- [ ] **Step 1: i18n 加鍵(en.json,record 物件)**

```json
    "device_mic": "Microphone",
    "device_system": "System audio source",
    "device_default": "Default",
```

- [ ] **Step 2: i18n 加鍵(zh-TW.json,record 物件,對稱)**

```json
    "device_mic": "麥克風",
    "device_system": "系統音訊來源",
    "device_default": "預設",
```

- [ ] **Step 3: RecordTab 加裝置 state + 載入 + 寫回**

在 `changeMode`(`:51-58`)之後加:
```tsx
  type DeviceInfo = { id: string; label: string };
  const [inputs, setInputs] = useState<DeviceInfo[]>([]);
  const [systemSources, setSystemSources] = useState<DeviceInfo[]>([]);
  const [inputDevice, setInputDevice] = useState("");
  const [systemSource, setSystemSource] = useState("");
  useEffect(() => {
    invoke<{ inputs: DeviceInfo[]; system_sources: DeviceInfo[] }>("list_audio_devices")
      .then((d) => { setInputs(d.inputs ?? []); setSystemSources(d.system_sources ?? []); })
      .catch(() => {});
    invoke<{ input_device?: string; system_source?: string }>("get_config")
      .then((c) => { setInputDevice(c?.input_device ?? ""); setSystemSource(c?.system_source ?? ""); })
      .catch(() => {});
  }, []);
  const persistDevice = async (patch: { input_device?: string; system_source?: string }) => {
    try {
      const cfg = await invoke<Record<string, unknown>>("get_config");
      await invoke("set_config", { cfg: { ...cfg, ...patch } });
    } catch (e) { console.error(e); }
  };
  const onInputDevice = (v: string) => { setInputDevice(v); persistDevice({ input_device: v }); };
  const onSystemSource = (v: string) => { setSystemSource(v); persistDevice({ system_source: v }); };
```

- [ ] **Step 4: RecordTab 加下拉(模式切換 div 之後)**

在 mode-switch `<div>`(role="group" 那塊,`:155-167` 區)的**收尾 `</div>` 之後**插入:
```tsx
      <div style={{ display: "flex", flexDirection: "column", gap: 6, margin: "0 0 10px" }}>
        {mode === "online" && (
          <label style={{ fontSize: 11, color: "var(--text-secondary)" }}>
            {t("record.device_system")}
            <select
              value={systemSource}
              onChange={(e) => onSystemSource(e.target.value)}
              disabled={recState !== "idle"}
              style={{ width: "100%", marginTop: 2 }}
            >
              <option value="">{t("record.device_default")}</option>
              {systemSources.map((d) => <option key={d.id} value={d.id}>{d.label}</option>)}
            </select>
          </label>
        )}
        <label style={{ fontSize: 11, color: "var(--text-secondary)" }}>
          {t("record.device_mic")}
          <select
            value={inputDevice}
            onChange={(e) => onInputDevice(e.target.value)}
            disabled={recState !== "idle"}
            style={{ width: "100%", marginTop: 2 }}
          >
            <option value="">{t("record.device_default")}</option>
            {inputs.map((d) => <option key={d.id} value={d.id}>{d.label}</option>)}
          </select>
        </label>
      </div>
```
(現場模式只顯示「麥克風」下拉;線上模式多顯示「系統音訊來源」下拉。)

- [ ] **Step 5: JSON 合法 + build**

Run:
```bash
cd /home/ct/mori-universe/.worktrees/recorder-device-selection
node -e "JSON.parse(require('fs').readFileSync('src/i18n/locales/en.json','utf8'));JSON.parse(require('fs').readFileSync('src/i18n/locales/zh-TW.json','utf8'));console.log('json ok')"
npm run build 2>&1 | tail -6
```
Expected: `json ok` + tsc/vite 無錯。

- [ ] **Step 6: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-device-selection
git add src/tabs/RecordTab.tsx src/i18n/locales/en.json src/i18n/locales/zh-TW.json
git commit -m "feat(recorder): RecordTab 模式感知收音裝置下拉 + i18n"
```

---

## Task 6: 全量驗證 + 真機手測 + PR

- [ ] **Step 1: verify.sh 全綠**

Run:
```bash
cd /home/ct/mori-universe/.worktrees/recorder-device-selection
npm run build && bash scripts/verify.sh 2>&1 | tail -20
```
Expected: cargo test 全 PASS(含 parse_pactl / resolve_device 新測 + 既有回歸)、npm build、cargo check 乾淨。

- [ ] **Step 2: 真機手測(`npm run tauri dev`)**

- Record 分頁:現場模式只見「麥克風」下拉;切線上 → 多「系統音訊來源」下拉。兩者第一項都是「預設」。
- 麥下拉列得出 fifine USB 麥 → 選它 → 現場錄一段 → 確認收的是該麥(對它講話有訊號/字幕)。
- 拔掉選的麥再錄 → 不報死(退回預設)。
- 不選(預設)→ 行為跟 #82 一樣(線上雙軌 / 現場單軌)。
- 錄音中下拉 disabled。

- [ ] **Step 3: push + PR(auto-merge)**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-device-selection
git push -u origin feat/input-device-selection
gh pr create --fill --base main --head feat/input-device-selection
gh pr merge --auto --squash
```

- [ ] **Step 4: worktree 清理(merge 後)**

```bash
cd /home/ct/mori-universe/mori-meeting-recorder
git worktree remove /home/ct/mori-universe/.worktrees/recorder-device-selection
```

---

## Self-Review

**Spec coverage**(對 `2026-06-03-input-device-selection-design.md`):
- 範圍#1 麥+系統源都可選 → Task 1 兩欄位 + Task 3 resolve_device(麥/room→input_device、system→system_source)。✅
- 範圍#2 Record 分頁模式感知下拉 → Task 5 Step 4(現場 1 / 線上 2)。✅
- 範圍#3 「預設」選項(空字串)→ Task 5 `<option value="">`。✅
- 範圍#4 裝置消失退預設 → Task 4 windows `find_device_by_name(...).unwrap_or(pick_device)`;Linux `pulse_source_exists` 檢查,麥/room 不在則退預設(Task 4 Step 2)。✅
- 範圍#5 友善名稱 → Task 2 parse_descriptions / cpal name。✅
- 範圍#6 錄音中鎖 → Task 5 `disabled={recState !== "idle"}`。✅
- 範圍#7 做法 A(resolve_device + open_capture device 參數、pick_source/pick_device 不變)→ Task 3/4。✅
- 列舉(devices.rs + 命令)→ Task 2。✅ 接線 → Task 4。✅ UI → Task 5。✅
- 回歸(未選=#82 行為)→ Task 4 `None` 分支維持原狀 + Task 6 Step 2 驗。✅

**Placeholder scan:** 無 TBD/TODO;每 code step 有完整 code(windows 因 cfg 在 Linux 不編,靠 cargo check 文法 + review)。✅

**Type consistency:**
- `resolve_device(SourceKind, &RecorderConfig) -> Option<String>`(Task 3)= Task 4 三處 call site 一致。✅
- `open_capture(source, device: Option<String>, out_path, vad_cfg, pending)` 新簽名(Task 4 Step 1)= linux/windows impl(Step 2/3)+ recorder call(Step 4)一致。✅
- `AudioDevices { inputs, system_sources }` / `DeviceInfo { id, label }`(Task 2)= `list_audio_devices` 回傳 = RecordTab 前端型別(Task 5 Step 3,snake_case `system_sources`)一致。✅
- config `input_device` / `system_source`(Task 1)= resolve_device 讀(Task 3)+ 前端 persistDevice patch(Task 5)一致。✅
- i18n `record.device_mic/device_system/device_default`(Task 5 Step 1/2)= Step 4 引用一致。✅
