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
- **UI css token 自己一套**:不沿 mori-desktop var(--c-*),`src/theme.css` 自己定義
- **單視窗切 size**:collapsed 360×60(膠囊),expanded 720×480(3-tab),`window.setSize` 在前後端切
- **Tauri v2 auto-camelCase**:`event_id: String` Rust 對應 JS `eventId`
- **共用驗證入口** — `bash scripts/verify.sh`:`cargo test` + `npm run build` + `cargo check`
