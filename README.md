# mori-meeting-recorder

Standalone dual-track meeting recorder for the Mori universe.

**Observer Mode MVP** — 雙軌錄音(`meeting_system` 系統輸出 + `mic_internal` 本機麥克風)
→ 停止後 whisper.cpp 雙軌平行轉錄 → visibility-based `meeting.public.md` / `meeting.internal.md` 匯出。

## Quick start

```bash
git clone https://github.com/yazelin/mori-meeting-recorder
cd mori-meeting-recorder
npm install
bash scripts/install-whisper-linux.sh   # 或 .ps1 on Windows
npm run tauri dev
```

## Design

- 契約:[meeting-recorder.md](https://github.com/yazelin/mori-desktop/blob/main/docs/meeting-recorder.md)
- 本 repo 是 Body Interface 軌的 BI-5。設計 spec + 實作 plan 在 mori-desktop repo `docs/superpowers/`。
- BI-1 manifest:啟動時 self-register `~/.mori/body-parts/mori.meeting-recorder/manifest.json`

## License

MIT
