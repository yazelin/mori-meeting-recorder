# 本地 whisper 服務:隨需啟動(任何 app)+ 閒置自關 — 設計

**日期**:2026-05-30
**狀態**:已實作(branch `feat/whisper-on-demand-activation`)
**契約**:`agentos-notebook/05-mori-migration/whisper-server-contract.md` §11 Activation

## 1. 問題

本地 whisper-server 是「一台機器一個共享資源」(模型載一次進 VRAM)。需求:

> 任何 app(**不一定是 recorder**,也可能 mori-desktop / AgentOS / python mori-ear / shell)
> 需要本地 whisper 服務但它沒在跑時,要能把它啟動;直到 **10 分鐘都沒人用**才停。

## 2. 現況盤點(改之前)

- supervisor `mori-whisper-serve` 已做好:搶 flock 單例、選埠、起 whisper-server、寫
  `~/.mori/whisper-server.json`、盯 `/inference` 活動、**閒置 600 秒(10 分鐘)自關**、`--stop`。
  → **「10 分鐘自關」這半邊本來就有。**
- **缺口在「喚醒」**:啟動邏輯 `autostart_whisper_server` 埋在 `recorder.rs`、綁在「開始錄音」、
  且只在 **recorder 執行檔旁邊**找 supervisor。別的 app 只能用「已經在跑的」server,**沒有
  共用的、固定位置的方式去冷啟動它**。

## 3. 設計(角色不變,契約 §3.2 / §8)

supervisor 仍是唯一 **Starter+Owner**;consumer 仍是 **Adopter**,只是多了「踢一下這個冪等
supervisor」這個被允許的動作。喚醒入口抽成共用、語言無關。

### 3.1 共用啟動函式 `whisper_discovery::ensure_server(model)`

- 有**驗活過**的 server → return(沿用正在跑的,**不管它載哪個 model**)。
- 否則:`install_shared_supervisor()`(best-effort 把 sibling supervisor 種進 `~/.mori/bin`)
  → `locate_supervisor()`(先 `~/.mori/bin`、再 sibling)→ `spawn_supervisor_detached()`
  (`setsid`/`DETACHED_PROCESS`,fire-and-forget,不等 ready、不卡)。
- 找不到 / spawn 失敗 → 安靜略過(consumer 之後 fallback cli,**standalone-first 不破**)。

### 3.2 supervisor 共用安裝點 `~/.mori/bin/mori-whisper-serve`

跟 `whisper-cli` / `whisper-server` 同窩。**任何 app 從這個固定路徑找/喚醒它**。種法:
- **權威**:`scripts/install-supervisor.{sh,ps1}`(`cargo build --release --bin mori-whisper-serve`
  → 裝進 `~/.mori/bin`)。
- **dev 便利**:recorder 啟動時(`.setup()` 背景執行緒)+ `ensure_server` 冷啟動路徑都 best-effort
  自種(`install_shared_supervisor()`)。在 `tauri dev` 下兩 bin 同在 `target/<profile>/` → sibling 找得到。
- **caveat**:packaged Tauri bundle 目前**沒**把 supervisor 列為 `externalBin` sidecar → 正式包裝下
  sibling 會 None,要靠 install-supervisor 腳本鋪。bundle 自帶是 packaging follow-up。

安裝用「寫 per-pid `.tmp-install.<pid>` 再 `rename` 覆蓋」—— rename 蓋過正在被 exec 的舊 binary 在
Linux 安全(避免 `ETXTBSY`);per-pid tmp 讓並發種子不互相寫半截檔。freshness 看 `need_seed`
(沒種過 / 大小不同 / sibling 較新)。

### 3.3 語言無關喚醒指令 `mori-whisper-serve --ensure`

非 Rust 的 app 一行喚醒:有活的 server → 立刻回;沒有 → 把**自己**以**裸 supervise 模式**
(無 `--ensure`,避免無限自我 re-ensure)背景化重啟、馬上 exit 0。連打安全(裸 supervisor 靠
flock 單例)。

### 3.4 閒置自關(stop 條件,維持)

`DEFAULT_IDLE_SECS = 600` 提成 `whisper_discovery` 的共用常數(supervisor / `ensure_server` /
`--ensure` 全引它,不各寫各的)。「使用」= `/inference` 流量。

## 4. Model 策略(對齊契約 §3.4)

`ensure_server` 只保證「有一台」server;冷啟動者的 model 說了算。某 app 真需要特定 model 又不符
→ 讀 `descriptor.model` 自行決定用它或 fallback 自家 cli。**不做 mismatch 重啟**(避免 thrash)。

## 5. 已知取捨(誠實列出)

- **會議中長靜音被收**:>10 分鐘無 `/inference` → supervisor 收掉,下次講話冷啟動(多載入延遲,
  非錯誤)。符合「10 分鐘沒人用就停」的語義,**不修**。
- **沒有 lease/keep-alive**:「使用」只認 `/inference`(YAGNI)。
- **首次種子**:`~/.mori/bin` 還沒種過時,非 Rust 的 `--ensure` 會找不到 binary → 先跑
  `install-supervisor.sh`(權威),或 dev 跑一次 recorder(startup 背景自種)。
- **`--ensure` 不等 ready**:呼叫者用 server 前要自己 poll descriptor + `GET / 200`(~90s),逾時
  fallback cli(README / 契約 §11 有 recipe)。
- **FD 繼承**:detached spawn **必須** close fd `3..RLIMIT_NOFILE`(`mori-spawn-close-fds-linux`)—— 否則
  長命 supervisor 繼承 recorder 的 single-instance socket,父死後卡住下次啟動。兩個 spawn site
  (`spawn_supervisor_detached` / supervisor→whisper-server)都套了。

## 6. 改動清單

| 檔案 | 改動 |
|---|---|
| `src-tauri/src/whisper_discovery.rs` | +`DEFAULT_IDLE_SECS` / `supervisor_bin_name` / `shared_supervisor_path` / `install_shared_supervisor` / `locate_supervisor` / `spawn_supervisor_detached` / `ensure_server` + 單元測試 |
| `src-tauri/src/bin/mori-whisper-serve.rs` | +`--ensure` 模式(`do_ensure`);idle 常數引共用;裸呼叫 = supervise(back-compat) |
| `src-tauri/src/recorder.rs` | 呼叫點改 `ensure_server`;刪掉 `whisper_serve_bin` + `autostart_whisper_server`(搬進共用模組) |
| `scripts/install-supervisor.sh` / `.ps1` | 新增:build + 安裝 supervisor 進 `~/.mori/bin` |
| `scripts/install-whisper-linux.sh` | 末尾指向 install-supervisor |
| `README.md` / `CLAUDE.md` | 記錄共享服務 + 喚醒方式 |
| `agentos-notebook/.../whisper-server-contract.md` | +§11 Activation(co-sign,`contract_version` 不變) |

## 7. 測試

- 單元:`shared_supervisor_path` 落 `~/.mori/bin`、`supervisor_bin_name` 對平台、
  `DEFAULT_IDLE_SECS == 600`;既有 descriptor / parse 測試全綠。
- 手動 e2e(真機):`--ensure` 冷啟動 → descriptor 出現 + `GET /` 200;再打一次 = 不雙開;
  閒置 600 收;recorder 開場照舊接上 server。
