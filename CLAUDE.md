# Claude Code 指引 — mori-meeting-recorder

mori-meeting-recorder 是 Mori universe 的 standalone 會議錄音工具 — Tauri 2 + Rust + React,
Observer Mode MVP:雙軌(`meeting_system` + `mic_internal`)→ 停止後 whisper 轉錄 → visibility-based
`meeting.public.md` / `meeting.internal.md` 匯出。

## 設計來源

- Body Interface 軌契約:[mori-desktop/docs/meeting-recorder.md](https://github.com/yazelin/mori-desktop/blob/main/docs/meeting-recorder.md)
- BI-5 設計 spec:[mori-desktop/docs/superpowers/specs/2026-05-28-bi-5-meeting-recorder-design.md](https://github.com/yazelin/mori-desktop/blob/main/docs/superpowers/specs/2026-05-28-bi-5-meeting-recorder-design.md)
- 實作 plan:[mori-desktop/docs/superpowers/plans/2026-05-28-bi-5-meeting-recorder.md](https://github.com/yazelin/mori-desktop/blob/main/docs/superpowers/plans/2026-05-28-bi-5-meeting-recorder.md)

## 硬規矩

1. **不公開比較其他專案** — 用 Mori 自己的詞彙
2. **User-owned data** — `~/.mori/meetings/` 是 user 的;recorder 不對外傳
3. **mic 永不混進客戶版** — `meeting.public.md` filter visibility=public only
4. **Standalone-first** — 沒 mori-desktop 也要能跑;deps 自己 bundle scripts
5. **Bundle deps in repo** — 不從外部 setup repo 拉
6. **trunk-based + auto-merge** — 短命 branch off main,PR 設 auto-merge

## 工程注意

- **平台 audio lib 各家**:Linux 走 libpulse(對齊 OBS linux-pulseaudio plugin);Windows 走 cpal WASAPI loopback(對齊 mori-desktop)。Task 1 spike 已驗證 cpal Linux 看不到 PipeWire `.monitor` source。
- **共用 ~/.mori/ 路徑**:`~/.mori/bin/whisper-cli` 跟 `~/.mori/models/ggml-small.bin` 跟 mori-desktop 共享(filesystem 慣例,不 IPC)
- **共享 whisper 服務隨需啟動**:啟動邏輯住共用模組 `whisper_discovery::ensure_server(model)`(**不要**再寫在 recorder.rs)。supervisor `mori-whisper-serve` 裝在 `~/.mori/bin/`(跟 whisper-cli 同窩),任何 app 都能喚醒:Rust 呼 `ensure_server`、非 Rust 跑 `mori-whisper-serve --ensure`(冪等、自我背景化)。閒置 TTL 用共用常數 `whisper_discovery::DEFAULT_IDLE_SECS=600`(別各寫各的 600)。契約 = `agentos-notebook/05-mori-migration/whisper-server-contract.md` §11。
- **UI css token 自己一套**:不沿 mori-desktop var(--c-*),`src/theme.css` 自己定義
- **UI 控件一律走設計系統,禁用裸 native 控件** — 下拉用自製 `components/Select.tsx`(別用原生 `<select>`;Linux GTK 與 Windows WebView2 的原生下拉配色都鎖不住),input / checkbox / 捲軸靠 `theme.css` 全域 theme(`::-webkit-scrollbar`、`input:where(...)`、`input[type=checkbox]` appearance:none 自繪勾)。**改 UI 務必 Windows + Linux 都看過**才算完成 —— 兩個 webview 的 native 渲染差很多,只測 Linux 會漏掉 Windows 的白底框 / 粗捲軸。
- **單視窗切 size**:collapsed **480×44**(膠囊,寬度要夠放右側 ✕),expanded 預設 **900×620**(可拉、收合時記住;預設在 `config.rs::default_expanded_*`,由 `set_window_mode` 讀 config 套用)
- **Tauri v2 auto-camelCase**:`event_id: String` Rust 對應 JS `eventId`
- **共用驗證入口** — `bash scripts/verify.sh`:`cargo test` + `npm run build` + `cargo check`
- **兩份 manifest 別混**:`agentos-manifest.json`(repo root)= **AgentOS** AppManifest v2(`kind: body-part`,`agentos install` 用,受 broker→audit 治理);`src-tauri/src/manifest.rs` 寫的是 **mori-desktop** BI-1 BodyManifest(`schema_version:1`,啟動 self-register 到 `~/.mori/body-parts/`)。不同 schema、不同 consumer。改其一不要順手改另一。AgentOS manifest 是純宣告 sidecar,**不影響 standalone 行為**。
