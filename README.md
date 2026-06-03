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

## 共享本地 whisper 服務(隨需啟動、閒置自關)

一台機器**一個共享的本地 whisper-server**(模型只載一次進 VRAM,省資源),由 supervisor
`mori-whisper-serve` 管理。**任何 app 都能隨需喚醒它**,沒人用滿 10 分鐘就自己關。

- **服務發現**:consumer 讀 `~/.mori/whisper-server.json`(host/port/model/pid),先驗活
  (pid 還在 + `GET /` 回 200)才信;POST 音訊到 `/inference`。
- **隨需喚醒(任何 app)** —— 沒在跑就背景拉起,冪等、馬上返回:

  ```bash
  ~/.mori/bin/mori-whisper-serve --ensure
  ```

  - **Rust consumer**(recorder / mori-desktop / AgentOS):呼叫共用函式
    `whisper_discovery::ensure_server(model)`,行為相同。
  - **非 Rust**(python mori-ear / shell / 資料 app):跑上面那行指令即可。
  - **`--ensure` 不等 ready**(馬上返回,server 在背景載模型)。用之前要自己 poll:讀
    `~/.mori/whisper-server.json` + `GET host:port/` 直到 200(~90s 預算),逾時就 fallback 本地
    `whisper-cli`。
- **閒置自關**:supervisor 盯 `/inference` 活動,閒置(無轉錄)超過 10 分鐘
  (`whisper_discovery::DEFAULT_IDLE_SECS = 600`)就 SIGTERM 收掉 whisper-server、刪發現檔、退出。
- **手動停止**:`~/.mori/bin/mori-whisper-serve --stop`。
- **安裝 supervisor 到 `~/.mori/bin`**(讓別的 app 找得到):**權威鋪法**是
  `bash scripts/install-supervisor.sh`(或 `scripts/install-supervisor.ps1`)—— build + 裝進
  `~/.mori/bin`。dev 跑 recorder(`tauri dev`)啟動時也會 best-effort 自種一份。**注意**:正式
  packaged bundle 目前沒把 supervisor 列為 Tauri sidecar,所以打包後請用 install-supervisor 腳本鋪。
- **standalone-first**:supervisor 缺 / 起不來時,consumer 退回本地 `whisper-cli`(per-call),不會壞。
- **單一共享、單一 model**:沿用「正在跑的那台」;冷啟動者選的 model 說了算。某 app 真要特定
  model 又不符,讀 `descriptor.model` 自行決定要不要 fallback cli。
- **跨 repo 契約**:`agentos-notebook/05-mori-migration/whisper-server-contract.md`(§11 Activation)。

## AgentOS 整合(body-part)

recorder 可被 [AgentOS](https://github.com/yazelin/agentos) 當 **body-part** 安裝治理。
**兩份 manifest、兩套系統,別混為一談**:

- `agentos-manifest.json`(repo root)—— **AgentOS** `AppManifest` v2。`kind: body-part`、
  `data_policy.owns_raw_data: true`(會議原始音訊歸 user,不對外送)、`consumes: transcribe.local`
  (standalone-first:本機 whisper 為主,偵測到共享服務才委派)、`provides: meeting.summarize`
  (雙摘要能力;`kind: external` —— 由 app process 提供,真正跨 app dispatch 待 Phase 7 ACP)。
  安裝:`agentos install <path-to-agentos-manifest.json>`,之後 `agentos apps` 看得到、受 broker→audit 治理。
- BI-1 manifest —— **mori-desktop** body registry,啟動時 self-register
  `~/.mori/body-parts/mori.meeting-recorder/manifest.json`(`schema_version: 1`、`kind: standalone_app`)。
  不同 schema、不同 consumer。

**Standalone-first 不變**:agentos-manifest.json 是純宣告的 sidecar,不改 recorder 任何執行行為;
沒裝 AgentOS,recorder 照常獨立跑。整場會議「optional-consumer」治理(偵測到 AgentOS 才把整場轉錄/匯出
走治理、逐段仍 local)是後續 Phase 2,待 AgentOS cross-app proxy 落地。

## Design

- 契約:[meeting-recorder.md](https://github.com/yazelin/mori-desktop/blob/main/docs/meeting-recorder.md)
- 本 repo 是 Body Interface 軌的 BI-5。設計 spec + 實作 plan 在 mori-desktop repo `docs/superpowers/`。
- BI-1 manifest:啟動時 self-register `~/.mori/body-parts/mori.meeting-recorder/manifest.json`
- AgentOS body-part 整合設計:`agentos-notebook/05-mori-migration/e3-real-organs-design.md` §2.2(E3-2)
- 本地 whisper 服務隨需啟動設計:`docs/superpowers/specs/2026-05-30-whisper-on-demand-activation-design.md`

## License

MIT
