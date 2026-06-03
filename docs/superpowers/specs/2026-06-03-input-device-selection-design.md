# 收音裝置選擇 — 在 app 內選麥克風 / 系統音訊來源(設計)

> **Goal**: 讓使用者在 Record 分頁**直接下拉選收音裝置**,不用每次跑去改作業系統預設輸入。
> 現場模式選「房間麥」、線上模式選「我方麥」+「系統音訊來源」。延續現場/線上雙模式(PR #82)。
>
> **Plan output**: 本 spec → `writing-plans` → `docs/superpowers/plans/2026-06-03-input-device-selection.md` → 實作。

## 背景

PR #82 後,recorder 有兩種模式:`online`(系統 monitor + 麥)/ `in_person`(單支房間麥)。但**收哪支裝置一律走作業系統預設**:
- Linux `audio/linux.rs::pick_source`(`:30`):Mic/Room → `Ok(None)`(預設輸入);System → `pick_system_monitor()`(自動挑 default sink 的 `.monitor`)。
- Windows `audio/windows.rs::pick_device`(`:16`):Mic/Room → `default_input_device()`;System → `default_output_device()`(loopback)。

要換麥只能去 OS 聲音設定。實機 `pactl list short sources` 已能看到使用者的 **fifine USB 麥**(`alsa_input.usb-...fifine_Microphone...`),正是現場會議想指定的裝置。本 spec 讓使用者在 app 內選。

## 範圍(yazelin 2026-06-03 拍板)

| # | 決議 | 值 |
|---|---|---|
| 1 | 可選範圍 | **麥 + 系統聲源都可選**。麥(`input_device`)用於 現場 room + 線上 mic_internal;系統源(`system_source`)用於 線上 meeting_system。 |
| 2 | UI 位置 | **Record 分頁、模式切換旁**。模式感知:現場 1 個下拉(麥克風);線上 2 個下拉(系統音訊來源 + 麥克風)。 |
| 3 | 「預設」選項 | 每個下拉含「預設 / 自動」(對應 config 空字串)= 維持現狀(麥=OS 預設輸入、系統=auto-monitor)。 |
| 4 | 裝置消失 | 選定裝置不在了(拔線/改名)→ **退回預設**,不讓 session 開不起來。 |
| 5 | 顯示名稱 | 友善名稱(Linux 取 `pactl list sources` Description;Windows 取 `device.name()`);抓不到退技術名。 |
| 6 | 錄音中 | 下拉 **disabled**(不能中途換裝置)。 |
| 7 | 做法 | A:兩個 config 欄位 + `resolve_device` 純函式 + `open_capture` 收 `device` 參數(保持 `pick_source`/`pick_device` 純、既有測試不動)。 |

## 不混淆:跟模式 / loopback 的邊界

- 裝置選擇**不改錄音模式語意**(現場仍單軌 room→meeting.md、線上仍雙軌→public/internal)。只改「每軌實際開哪個裝置」。
- 系統源仍是 loopback / monitor(輸出的鏡像);`system_source` 只是讓使用者在「多個 sink monitor」中指定一個,空=維持現有自動挑選。

## 架構

### 後端 `src-tauri/src/config.rs`(兩個欄位)

- 加 `#[serde(default)] input_device: String`(預設 `""`)+ `#[serde(default)] system_source: String`(預設 `""`)。空 = 預設/自動。沿用既有 serde-default + Default impl pattern。

### 後端 `src-tauri/src/audio/devices.rs`(新模組:列舉)

```rust
pub struct DeviceInfo { pub id: String, pub label: String }   // Serialize
pub struct AudioDevices { pub inputs: Vec<DeviceInfo>, pub system_sources: Vec<DeviceInfo> }
pub fn list_devices() -> AudioDevices;   // cfg(linux) / cfg(windows) / 其他回空
```
- **Linux**:`pactl list short sources` → 第 2 欄 name;`name.ends_with(".monitor")` → system_sources,否則 → inputs。友善 label:解析 `pactl list sources`(verbose)建 Name→Description map,查不到用 name。純解析函式(餵字串)可單測。
- **Windows**:cpal `host.input_devices()` → inputs(`id=label=device.name()`);`host.output_devices()` → system_sources。
- **其他平台**:回空兩清單(MVP 已限 linux+windows)。

### 後端 Tauri command(`main.rs` 註冊)

- `list_audio_devices() -> AudioDevices`:包 `devices::list_devices()`。前端開分頁時呼叫。

### 後端 `resolve_device` + `open_capture` 接線

- 新純函式(放 `audio/mod.rs`):
```rust
/// 依 source + config 算出要開的裝置名;None = 用平台預設(麥=系統預設輸入、系統=auto-monitor)。
pub fn resolve_device(source: SourceKind, cfg: &crate::config::RecorderConfig) -> Option<String> {
    let pick = match source {
        SourceKind::MicInternal | SourceKind::MeetingRoom => &cfg.input_device,
        SourceKind::MeetingSystem => &cfg.system_source,
    };
    if pick.trim().is_empty() { None } else { Some(pick.clone()) }
}
```
- `open_capture(source, out_path, vad_cfg, pending)` **加參數 `device: Option<String>`** → `open_capture(source, device, out_path, vad_cfg, pending)`。平台 impl:
  - `device` 是 `Some` → 直接用該裝置(Linux 當 pulse source name;Windows 依 name 在對應 device 清單找,**找不到 → 退回平台預設**(範圍#4))。
  - `device` 是 `None` → 維持現狀(Linux 呼既有 `pick_source(source)`;Windows 呼既有 `pick_device(source)`)。
- `pick_source` / `pick_device` **簽名與行為不變**(只在 `device==None` 時被呼叫)→ 既有純測試保留。
- recorder.rs `start_session` 開軌迴圈:`let device = crate::audio::resolve_device(kind, &cfg);` 傳進 `open_capture`。`voice_input` / `enroll` 的麥 capture 也傳 `resolve_device(MicInternal, &cfg)`(語音輸入/聲紋登錄用同一支選定麥)。

### 前端 `src/tabs/RecordTab.tsx`(模式感知下拉)

- 開分頁時 `invoke("list_audio_devices")` 取兩清單;存 state。
- 模式切換旁渲染下拉(沿用 RecordTab 既有 inline style + recorder `var(--…)` token):
  - **現場**:1 個「麥克風」下拉(綁 `input_device`)。
  - **線上**:2 個「系統音訊來源」(綁 `system_source`)+「麥克風」(綁 `input_device`)。
  - 每個下拉第一項 = 「預設」(value="");選了就 `get_config`→`set_config` 寫回(同模式切換的 persist-first 模式)。
  - **錄音中 disabled**(`recState !== "idle"`)。
- i18n:`record.device_mic` / `record.device_system` / `record.device_default` 等鍵(en + zh-TW)。

## 資料流

```
開 Record 分頁 → list_audio_devices() → { inputs[], system_sources[] } → 下拉
使用者選裝置 → get_config + set_config({...cfg, input_device / system_source}) → config.json
開始錄音 → start_session 每軌 resolve_device(kind,cfg) → open_capture(kind, device, …)
  device Some → 用該裝置(找不到退預設);None → 平台預設/auto-monitor
```

## 錯誤處理

- `list_audio_devices` 失敗(pactl 缺 / cpal error)→ 回空清單,前端下拉只剩「預設」(仍可錄,走 OS 預設)。
- 選定裝置不在了 → Windows find-by-name 落空 → 退平台預設;Linux 給了不存在的 source name → libpulse 開失敗,該軌 open_capture Err(現場無備援軌會讓 session 失敗,線上則該軌略過)。**為避免現場整場開不起來,Linux Mic/Room 在 `Some(device)` 開失敗時 retry 一次 `None`(預設輸入)**(範圍#4 的落地)。
- 模式切換 / 裝置下拉錄音中 disabled。
- 線上模式行為:未選裝置時與 PR #82 完全一致(回歸點)。

## 測試(TDD)

- `resolve_device`:Mic/Room→input_device(空→None / 有值→Some)、System→system_source(空→None / 有值→Some)。純函式單測。
- Linux 來源行分類純函式(餵 `pactl list short sources` 樣本字串)→ 正確分 inputs / system_sources、Description map 套用 / fallback。
- `open_capture` 的 `device=None` 路徑 = 既有行為(既有 capture 測試 / 手測涵蓋);`Some` 找不到退預設(Windows 單測或手測)。
- **回歸**:未選裝置時線上雙軌、現場單軌行為不變。
- 前端:沿用 recorder 既有前端慣例(手測為主)。
- `bash scripts/verify.sh` 全綠。
- 真機手測:Record 分頁下拉出現你的 fifine USB 麥 → 選它 → 現場錄 → 確認收的是該麥;拔掉該麥再錄 → 退回預設不報死;線上模式 2 個下拉、系統源可選不同 monitor。

## 非目標 / Follow-up

- 裝置**熱插拔即時刷新**清單(目前開分頁時抓一次;要刷新可重開分頁)。
- per-device 增益 / 取樣率 / 聲道設定。
- 藍牙延遲補償、裝置別名自訂。
- macOS 裝置列舉(沿用「其他平台回空」)。

## 驗證

- `bash scripts/verify.sh` 全綠。
- 真機手測清單(見上),逐項通過(動了 Rust → 重啟 tauri dev)。
