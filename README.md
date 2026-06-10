# mori-meeting-recorder

Standalone dual-track meeting recorder for the Mori universe.

**Observer Mode MVP** — 雙軌錄音(`meeting_system` 系統輸出 + `mic_internal` 本機麥克風)
→ 停止後 whisper.cpp 雙軌平行轉錄 → visibility-based `meeting.public.md` / `meeting.internal.md` 匯出。

## GitHub Pages 使用說明

完整的產品介紹、安裝教學與操作手冊已整理在 GitHub Pages 靜態網站：

- 本 repo 的 Pages 入口：[`docs/index.html`](docs/index.html)
- 部署 workflow：`.github/workflows/pages.yml`，push 到 `main` 後自動發布 `docs/` 內容。

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
- **STT initial prompt**:`~/.mori/stt/initial-prompt.md` 會在共享 `whisper-server`
  啟動時以 `--prompt` 帶入;也可手動
  `~/.mori/bin/mori-whisper-serve --prompt-file <path>` 覆寫。會議錄音自己的 server/CLI
  轉錄會先讀 `~/.mori/mori-meeting-recorder/stt-initial-prompt.md`,沒有才退回全域檔。
  這是 Whisper decoder context,適合放專有名詞 / 繁中 / 台灣用語提示,不是摘要 LLM 的
  system prompt。
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
  (雙摘要能力;`kind: http-service` + `mode: json` —— 平台把 dispatch forward 到本地 headless
  摘要 sidecar,見下)。安裝:`agentos install <path-to-agentos-manifest.json>`,之後 `agentos apps`
  看得到、受 broker→audit 治理。
- BI-1 manifest —— **mori-desktop** body registry,啟動時 self-register
  `~/.mori/body-parts/mori.meeting-recorder/manifest.json`(`schema_version: 1`、`kind: standalone_app`)。
  不同 schema、不同 consumer。

### meeting.summarize dispatch — headless 摘要 sidecar

recorder 是使用者手動開/關的 GUI app(按 ✕ 真正退出),不是 always-on daemon。所以摘要能力
被抽成一支**獨立、隨需啟動、閒置自關的 detached HTTP sidecar** `mori-summarize-serve`:GUI 與
AgentOS 都當 client。

- AgentOS 走 http-service `mode: json`:平台讀 descriptor `~/.mori/mori-recorder-server.json`(host=
  127.0.0.1)、驗活、把 `{session_id, force_local?}` 當 `application/json` POST 給 sidecar 的 `/summarize`;
  sidecar 跑 recorder 既有的雙摘要 pipeline(Groq → 本機 Ollama fallback)、寫 `meeting.summary.public.md` /
  `meeting.summary.internal.md` + `summary.audit.jsonl`、回 `SummaryResult` metadata。
- **安裝(部署/打包後)**:`bash scripts/install-supervisor.sh`(或 `scripts/install-supervisor.ps1`)
  —— build + 把 `mori-summarize-serve`(及 `mori-whisper-serve`)裝進 `~/.mori/bin/`。GUI 第一次跑也會
  best-effort 種一份,但**只在 dev 成立**(sidecar bin 在 app 旁邊時);packaged bundle 沒把 sidecar 列為
  Tauri `externalBin`,所以**打包/部署後請用本腳本鋪**(否則 AgentOS dispatch 找不到 sidecar)。
- 隨需喚醒:`~/.mori/bin/mori-summarize-serve --ensure`(冪等、自我背景化)。Groq key 由 sidecar
  內部自讀共享 `~/.mori/config.json`(**不接受 caller 帶 key**)。
- **`agentos run` 用 meeting.summarize 的兩個前提**:① manifest 已宣告 `input_schema`(`session_id`
  required / `force_local`),腦才知道要帶 `session_id`(agentos#21);② **agentos 腦讀 `GROQ_API_KEY`
  環境變數**(不是 `~/.mori/config.json` —— 那是 summarize pipeline 自己的 key),跑前需 `export GROQ_API_KEY=…`。
- **注意**:`agentos run` 是一個 LLM turn,final_text 會被腦改寫(甚至幻覺檔名)—— dispatch 成功 ≠ stdout
  是 pipeline 原文。要驗「真 pipeline 有跑」看**檔案 side-effect + audit**,不要拿 CLI stdout 斷言摘要原文。

**Standalone-first 不變**:agentos-manifest.json 是純宣告的 sidecar,不改 recorder 任何執行行為;
GUI 內按摘要鈕仍直接走 in-process `summarize_session`,完全不依賴 sidecar 起來。沒裝 AgentOS,
recorder 照常獨立跑。整場會議「optional-consumer」治理(逐段仍 local)是後續 Phase 2。

## Design

- 契約:[meeting-recorder.md](https://github.com/yazelin/mori-desktop/blob/main/docs/meeting-recorder.md)
- 本 repo 是 Body Interface 軌的 BI-5。設計 spec + 實作 plan 在 mori-desktop repo `docs/superpowers/`。
- BI-1 manifest:啟動時 self-register `~/.mori/body-parts/mori.meeting-recorder/manifest.json`
- AgentOS body-part 整合設計:`agentos-notebook/05-mori-migration/e3-real-organs-design.md` §2.2(E3-2)
- 本地 whisper 服務隨需啟動設計:`docs/superpowers/specs/2026-05-30-whisper-on-demand-activation-design.md`

## License

MIT
