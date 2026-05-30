---
title: mori-meeting-recorder — 會議摘要 Pipeline 設計 Spec
date: 2026-05-30
status: draft
---

> Standalone Tauri 2 + Rust + React 會議錄音工具(Mori universe)。本 spec 描述「停止錄音後,把雙軌逐字稿整理成結構化繁體中文會議記錄」的摘要功能。設計決策已與 user 拍板,本文件做細化、不推翻;經三輪對抗式 review 修正,所有 grounding 已對照本機 repo 實際程式碼逐行驗證(行號標在引用處)。

---

## 1. Overview / 動機

recorder 目前的流程到「停止錄音 → whisper 轉錄 → visibility-based 匯出 `meeting.public.md` / `meeting.internal.md`」為止。匯出的是**逐字稿**(誰在什麼時間說了什麼),不是**會議記錄**(達成了什麼)。本功能補上最後一哩:把逐字稿整理成結構化的繁體中文會議記錄。

整個設計的語意核心是 user 親自定義的雙軌語意:

- **系統軌(`source_kind="meeting_system"` / `track="system"` / `visibility="public"`)**:會議軟體裡所有人(含客戶)的發言。這是**真正的會議記錄 = 達成的協定/結論**(客戶需求、我方最終承諾)。是「結果」。
- **麥克風軌(`source_kind="mic_internal"` / `track="mic-internal"` / `visibility="internal"`)**:只收我方部分在場同事、不一定送進會議的「私聊」。這是**內部思考/評估過程**(我們評估了什麼、為什麼這樣回應客戶)= 「決議的依據 / why」。

一句話:**系統錄的是「決議」,麥克風錄的是「決議的依據」。** 摘要功能就是把這兩種素材分別整理成兩份產物 —— 一份只講結果(可給客戶),一份結果加上 why(內部用)。

相比 mori-desktop 的 clipboard 快照(event-driven、單次),這裡的雲端接觸面是**持續且主動**的:一整場 1 小時會議的逐字稿會被送上 Groq。因此 cloud-first 的同時必須提供逐字稿級別的資料主權逃生口(per-meeting「強制本地」),並落一筆本機 audit(§9.5)當知情同意的可驗證證據。

---

## 2. Goals / Non-goals

### Goals(v1)

- 在 recorder 內新增 `src-tauri/src/summarize.rs`,把停止錄音後的雙軌逐字稿整理成結構化繁中會議記錄。
- 後端 cloud-first:Groq `openai/gpt-oss-120b`(primary)→ 本機 Ollama `qwen3:4b-instruct-2507-q4_K_M`(fallback),任何 primary 失敗自動退本機。
- 產出兩份 Markdown:`meeting.summary.public.md`(系統軌 only,結論/協定)+ `meeting.summary.internal.md`(全部軌,結論 + 內部評估與決議依據)。
- per-meeting「強制本地」toggle:勾了就純本地、不碰網路。
- 送 LLM 前 redact 長壽憑證(API key 等),回 `redaction_count`,並落本機 audit(§9.5)。
- 可重跑(像現有的 `reexport_session`):改完逐字稿 / 改完 supplement 標記後重新整理。
- UI 在 `SessionWorkspace` 新增「會議紀錄」分區:摘要顯示 + 重新整理按鈕 + 後端狀態徽章(兩份各自標)+ 強制本地 toggle + 內部補充即時預覽。
- 本機 fallback 的 `num_ctx` 依逐字稿長度自動放大(避免 Ollama 預設 4096 默默截斷),且走 Ollama 原生 `/api/chat` 端點(OpenAI-compat 不認 `options`,§5.5)。

### Non-goals(v1 明確不做)

- **不做 tool-calling / agent loop**:摘要是單回合任務,只取回傳純文字。
- **不 import mori-core crate**:守 standalone-first + bundle-in-repo。鏡射(re-implement minimal slice)而非 import。
- **客戶版摘要不做進階功能**:public 版 v1 只做「整理達成的結論與協定」純結果,不做行動項分派、客戶情緒分析、自動寄送等。
- **不做 correction-audit / STT 錯字修正字典**:那套設計與摘要安全正交,defer v2。
- **不做 streaming 顯示**:摘要一次生成完整文本再寫檔、再前端讀檔顯示(沿用 reexport 的「跑完再 reload」模式)。
- **不做多 provider(Gemini / claude-cli / …)**:v1 只 Groq + Ollama 兩個 backend。trait 留下擴充點,但 v1 不接。
- **不做 PII / 語意脫敏**:v1 redaction 只擋 long-lived credential(API key);PII 由「強制本地」toggle 交給 user 自己判斷。
- **不做超長逐字稿(>2hr)的截斷 / 分段策略**:極罕見,記為已知限制(§13)。
- **不引入 async HTTP / reqwest / async-trait**:沿用 recorder 既有的 `ureq`(sync、blocking),整條摘要鏈做成 sync,在 `spawn_blocking` 內跑(§4.5)。

---

## 3. 架構

### 3.1 總覽

一個 `Summarizer` trait(**sync**),兩個 backend 實作:

- `GroqSummarizer`:`ureq` 同步 HTTPS 到 Groq OpenAI-compat `POST /chat/completions`。
- `OllamaSummarizer`:`ureq` 同步 HTTP 到本機 Ollama **原生 `POST /api/chat`**(不是 OpenAI-compat `/v1/chat/completions`),請求 body 帶 `options.num_ctx`(§5.5)。

fallback 邏輯鏡射 mori-core `chat_with_fallback` 語意(primary 任何失敗就退下一個,回傳「哪個 backend 成功」)。摘要任務跑兩遍(public prompt 一遍、internal prompt 一遍),**各自獨立**走 fallback chain,各自回成功的 backend(§4.4 SummaryResult 分兩欄)。

key 解析鏡射 mori-core **`GroqProvider::discover_api_key`**(`groq.rs:243-255`)的精確 2 段行為(§5.4),讀**同一份 `~/.mori/config.json`**。

trait 後面藏 backend,將來整碗可換成 AgentOS HTTP service client(同 whisper-server 模式,§12)。

### 3.2 Data-flow(ASCII)

```
停止錄音後(或 user 按「重新整理」)
        │
        ▼
summarize_session(session_id, force_local)             [Tauri command, async wrapper → spawn_blocking]
        │
        ▼
summarize_session_inner(session_root, force_local)     [sync,內部全 ureq blocking,不碰 async runtime]
        │
        ▼
read_session_segments(<root>)                          [既有 postprocess fn:兩軌 jsonl 合併、依 start_ms 排序]
   Vec<Segment>  (含 track / source_kind / visibility / supplement)
        │
        ├──────────────────────────────────────┐
        │                                       │
   visibility=="public"                  全部 segments(不做 visibility 過濾)
   (系統軌 only;鏡射 exporter.rs:90)     (系統軌 + 麥克風軌;internal 不套 exporter 過濾)
        │                                       │
        ▼                                       ▼
   redact_secrets(text)                  redact_secrets(text)        [送 LLM 前遮蔽 API key,§5.6]
        │                                       │
        ▼                                       ▼
   build_public_prompt()                 build_internal_prompt()     [兩個不同 system prompt]
        │                                       │
        ▼                                       ▼
   ┌──────── fallback chain(force_local 時只剩 [ollama],連 Groq 都不建構) ─────────┐
   │  ① GroqSummarizer  (openai/gpt-oss-120b, cloud HTTPS)                          │
   │        │ 任何失敗(斷網/5xx/timeout/429>60s)→ on_fallback callback             │
   │        ▼                                                                       │
   │  ② OllamaSummarizer (qwen3:4b, local /api/chat, num_ctx 依長度 16384/32768)    │
   └────────────────────────────────────────────────────────────────────────────┘
        │                                       │
   (summary_public, public_backend)      (summary_internal, internal_backend)
        │                                       │
        ▼                                       ▼
   write meeting.summary.public.md       write meeting.summary.internal.md   [原子寫檔,同 reexport]
        │                                       │
        └──────────────────┬────────────────────┘
                           ▼
        append summary.audit.jsonl 一筆(§9.5:timestamp / 兩 backend / force_local / redaction_count)
                           ▼
        回傳 SummaryResult { public_backend, internal_backend, public_chars, internal_chars, redaction_count }
                           │
                           ▼
   前端 reload 兩個 .md 檔顯示 + 兩個徽章各自標(☁ Groq / ⚡ Ollama)
```

關鍵不變式:**public 版只看到 `visibility=="public"` 的段,麥克風軌的文本在過濾階段就被排除,根本不會進到 public 的 prompt 裡**(hard rule #3 在資料流最早一站就守住,不依賴 prompt 自律;有 §10.1 單元測試守門)。

> ⚠️ rule-#3 vs 雲端暴露的分界(review 釐清):rule-#3 守的是「mic 不混進**客戶版** public.md」,這條 §6 守得死。它**不等於**「mic 不上雲」。internal 那遍(含麥克風私聊)在非 force_local 時**一樣會送 Groq**。force_local toggle 的語意是「兩份都純本地」(見 §5.3 / §8.1 文案)。

---

## 4. 元件與介面

### 4.1 `src-tauri/src/summarize.rs`(新檔)

> 設計定調:**全 sync**。recorder 既有 HTTP 走 `ureq`(`transcribe.rs:275` 已驗證 `ureq::post(url).set(...).timeout(...).send_bytes(...)`),沒有 reqwest / async-trait(`Cargo.toml` 已驗證 deps 只有 tokio/ureq/...)。摘要鏈本來就在 `spawn_blocking` 的 sync 世界(§4.5),用 sync trait + `ureq` 零新增 async stack、天然契合,也避開「blocking thread 內 nest tokio runtime」的 panic 坑(reference_tauri2_gotchas)。

**核心型別(草案簽名,sync)**:

```rust
/// 摘要任務的兩個版本。決定 prompt 與餵哪些 segment。
pub enum SummaryKind {
    Public,    // 系統軌 only,只結論
    Internal,  // 全部軌,結論 + 決議依據
}

/// 一次摘要請求的最小訊息結構(鏡射 OpenAI-compat chat message,只留 system/user)。
pub struct SumMessage {
    pub role: &'static str,   // "system" / "user"
    pub content: String,
}

#[derive(Debug)]
pub enum SumError {
    /// 網路 / server / timeout 類(觸發 fallback)。
    Backend(String),
    /// 缺 key / 缺模型 / 解析不出 content 等(也觸發 fallback,但訊息不同)。
    Config(String),
}

/// backend 抽象(sync)。藏在 trait 後 → 將來可換成 AgentOS HTTP service client。
pub trait Summarizer: Send + Sync {
    fn name(&self) -> &'static str;          // "groq" / "ollama"
    /// 單回合純文字摘要;失敗回 Err(網路 / server / 解析)。
    fn complete(&self, messages: &[SumMessage]) -> Result<String, SumError>;
}

pub struct GroqSummarizer {
    api_key: String,
    model: String,        // 預設 openai/gpt-oss-120b
    base_url: String,     // 預設 https://api.groq.com/openai/v1
    // ureq agent(90s timeout)+ 鏡射 groq.rs 的 429/5xx retry。
}

pub struct OllamaSummarizer {
    base_url: String,     // 預設 http://localhost:11434
    model: String,        // 預設 qwen3:4b-instruct-2507-q4_K_M
    num_ctx: u32,         // 依逐字稿長度算出(見 §5.5)
    // ureq agent(300s timeout,冷載容忍)。打 /api/chat(原生端點才認 options)。
}

/// fallback 入口(鏡射 chat_with_fallback 語意,sync)。
pub fn summarize_with_fallback(
    chain: &[Box<dyn Summarizer>],
    messages: &[SumMessage],
    mut on_fallback: impl FnMut(&str, Option<&str>, &SumError),  // (failed, next_or_none, err)
) -> Result<(String, &'static str), SumError>;  // (摘要文本, 成功的 backend name)

/// 主流程(鏡射 postprocess::reexport_session 的讀→處理→寫檔模式,sync)。
pub fn summarize_session_inner(
    session_root: &std::path::Path,
    force_local: bool,
) -> Result<SummaryResult, String>;

#[derive(serde::Serialize)]
pub struct SummaryResult {
    pub public_backend: String,      // "groq" / "ollama"(public 那遍實際用的)
    pub internal_backend: String,    // "groq" / "ollama"(internal 那遍實際用的)
    pub public_chars: usize,
    pub internal_chars: usize,
    pub redaction_count: usize,      // 兩遍合計、送 LLM 前共遮蔽幾處 secret
}
```

> 兩 backend 分欄(`public_backend` / `internal_backend`)是拍板決議(review 採納,原 Open Q1):兩遍各走獨立 chain,可能 public 用雲、internal 退本機,單值欄位會誤導使用者(這正是資料主權知情同意點,不能含糊)。force_local 下兩欄恆為 `"ollama"`。

**輔助(鏡射 mori-core,標明精確出處,純 re-implement)**:

```rust
/// 鏡射 GroqProvider::discover_api_key(groq.rs:243-255)的精確 2 段行為。
/// 順序:① env GROQ_API_KEY(非空 && 非 placeholder)
///       ② config /providers/groq/api_key(經 read_json_pointer,內含空字串/placeholder 過濾)
/// 測試用可注入 path / env 的內部版 resolve_groq_api_key_at(config_path, env_getter)。
fn resolve_groq_api_key(config_path: &Path) -> Option<String>;

/// 鏡射 groq.rs:462-470。讀 JSON pointer,空字串 / placeholder → None。
fn read_json_pointer(path: &Path, pointer: &str) -> Option<String>;

/// 鏡射 groq.rs:457-459:upper.starts_with("REPLACE") || upper.contains("YOUR_GROQ") || upper == "TODO"
/// (注意:TODO 是「全大寫完全相等」,不是 contains;REPLACE 是 starts_with;YOUR_GROQ 是 contains)
fn is_placeholder(s: &str) -> bool;

/// ~/.mori/config.json(注意:不是 recorder 自己的 meeting-recorder/config.json)
/// 鏡射 groq.rs:250-253 的 home.join(".mori").join("config.json")。
fn mori_config_path() -> Option<PathBuf>;  // dirs::home_dir()?.join(".mori").join("config.json")

/// 鏡射 redact.rs:63 redact_secrets。回 (redacted_text, hit_count)。
/// marker 常數 = "<REDACTED:probable-secret>"(鏡射 redact.rs:36 REDACTION_MARKER)。
fn redact_secrets(text: &str) -> (String, usize);

/// 鏡射 tokenize.rs:36-48 的雙路徑估算 → 算 num_ctx(§5.5)。
fn estimate_gpt_oss_tokens(text: &str) -> usize;   // cjk/1.50 + non_cjk/3.8(tokenize.rs:44)
fn pick_num_ctx(estimated_tokens: usize) -> u32;
```

### 4.2 Tauri command(`main.rs`,插在 `reexport_session` / 既有 line 619-627 之後)

鏡射既有 `reexport_session`(已驗證 `tauri::async_runtime::spawn_blocking(move || …).await.map_err(…)?`):

```rust
/// 用目前 jsonl + ~/.mori/config.json provider 設定,生成兩份摘要 .md。
/// force_local=true → 跳過 Groq,純本機 Ollama。可重跑。
#[tauri::command]
async fn summarize_session(
    session_id: String,
    force_local: bool,           // Tauri v2 auto-camelCase → JS forceLocal
) -> Result<summarize::SummaryResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        summarize::summarize_session_inner(
            &session_store::default_meetings_dir().join(&session_id),
            force_local,
        )
    })
    .await
    .map_err(|e| format!("join summarize_session: {e}"))?
}
```

註冊:`generate_handler!`(main.rs:847)清單裡,於 `reexport_session,`(line 876)之後加一行 `summarize_session,`。

### 4.3 `session_store.rs` 新 path helper(SessionStore impl 內,line 29-30 之後)

```rust
pub fn summary_public_md_path(&self) -> PathBuf { self.root.join("meeting.summary.public.md") }
pub fn summary_internal_md_path(&self) -> PathBuf { self.root.join("meeting.summary.internal.md") }
pub fn summary_audit_path(&self) -> PathBuf { self.root.join("summary.audit.jsonl") }
```

> 命名沿用既有慣例:現有是 `meeting.public.md` / `meeting.internal.md`(session_store.rs:28-29 已驗證),摘要產物用 `meeting.summary.public.md` / `meeting.summary.internal.md`,跟逐字稿匯出檔並列、一眼可分。

### 4.4 `config.rs` 新欄位(`RecorderConfig`,line 62 之後,各帶 serde per-field default,並補進 `Default::default()` 與 §10.3 round-trip 測試)

```rust
#[serde(default = "default_summary_groq_model")]
pub summary_groq_model: String,      // "openai/gpt-oss-120b"
#[serde(default = "default_summary_ollama_model")]
pub summary_ollama_model: String,    // "qwen3:4b-instruct-2507-q4_K_M"
#[serde(default = "default_summary_ollama_base_url")]
pub summary_ollama_base_url: String, // "http://localhost:11434"
#[serde(default)]                    // false
pub summary_force_local_default: bool, // 全域預設(per-meeting toggle 可覆寫)
```

> 兩個 config 是不同檔(review 已驗證):recorder 自己的可調參數在 `~/.mori/meeting-recorder/config.json`(config.rs:84-89);**provider / api_key 讀的是共享的 `~/.mori/config.json`**(跟 mori-desktop 同一份)。摘要的「用哪個 model / base_url」這類本工具偏好放 recorder config;「金鑰」這種跨工具的只讀共享 config、recorder 不寫它。Groq base_url 不做成 config 欄位(固定 `https://api.groq.com/openai/v1`,鏡射 `GroqProvider::DEFAULT_BASE_URL`),避免使用者誤把 key 送到非 Groq 端點。

### 4.5 sync 執行模型(拍板,消解 review 的 sync/async 矛盾)

`summarize_session_inner` 是 **sync** 函式,內部所有 HTTP 用 `ureq` blocking client。它在 `spawn_blocking` 的 worker thread 內跑,**不建任何 tokio runtime、不呼叫 block_on**。原 draft「或建一個 local tokio runtime 跑 async」的選項刪除(那是 reference_tauri2_gotchas 記過的 panic 坑)。trait `Summarizer::complete` 對應改為 sync `fn`(非 `async fn`),不需 `async-trait`。

### 4.6 Cargo.toml 依賴變更(`src-tauri/Cargo.toml`,bundle-in-repo 合規動作)

新增 / 調整(review 漏列,本次補上):

- `ureq`:**現有** `default-features = false`(line 32)只給 localhost whisper-server 用,**不含 TLS**。Groq 是 HTTPS → 必須加 TLS feature。改為:
  ```toml
  ureq = { version = "2", default-features = false, features = ["tls"] }
  ```
  `tls` feature 走 rustls(純 Rust,無系統 OpenSSL 依賴,對齊 standalone-first / bundle-in-repo)。版本維持 2(lock 現為 2.12.1)。
- `regex`:redact 鏡射(redact.rs 用 `regex::Regex`)。新增 `regex = "1"`。
- `serde` / `serde_json`:**已有**(line 25-26),摘要型別 serialize 沿用。
- `dirs`:**已有**(line 28),`mori_config_path` 沿用。
- `chrono`:**已有**(line 27),audit timestamp 沿用。
- **不新增** reqwest / async-trait(§4.5 定調 sync)。
- dev-dependencies:**不新增** wiremock / httpmock。改用 `HttpTransport` trait 注入 fake response(§10.2),既有 `tempfile`(line 48)足夠,verify.sh `cargo test` 不多編 mock server stack。

---

## 5. 後端策略與 fallback

### 5.1 cloud-first chain

非 force_local 時 chain = `[GroqSummarizer(gpt-oss-120b), OllamaSummarizer(qwen3:4b)]`。Groq primary、Ollama fallback。沿用 mori-core `chat_with_fallback` 語意:依序試,第一個成功就回 `(摘要, backend_name)`,全失敗回最後一個 error。public 與 internal 兩遍**各自**跑這條 chain。

### 5.2 fallback 觸發條件

primary(Groq)**任何**失敗都退本機:

- 斷網 / DNS 失敗 / connection refused / TLS 握手失敗
- Groq 5xx
- timeout(90s)
- 限流 429,且 Retry-After / body 解析出的等待 > 60s(鏡射 `MAX_AUTOMATIC_RETRY_SECS = 60`,groq.rs:169)→ 不自動重試、直接退本機
- 429 且等待 ≤ 60s → 先在 Groq 內部按 `BACKOFF_SECS = [1,2,4,8,16]`(groq.rs:34)backoff,最多 `MAX_ATTEMPTS = 5` 次**嘗試**(含首發,即最多 4 次重試等待;groq.rs:32);全用完才退本機
- 缺 key / placeholder key → `GroqSummarizer` 根本不進 chain(chain 直接 = `[ollama]`)
- 回 200 但解析不出 `choices[0].message.content` → 視為失敗,退本機

每次退 fallback 觸發 `on_fallback(failed_name, next_name, err)` callback;recorder 用它記錄並在 UI 徽章顯示「這遍退了本機」。

### 5.3 per-meeting 強制本地

UI toggle `forceLocal=true` 時,**兩遍**的 chain 都直接組成 `[OllamaSummarizer]`(連 Groq 都不建構,不讀 key、不碰網路)。這是 cloud-first 下的資料主權逃生口 + 知情同意機制。

**文案明確(消解 review minor #4 的含糊)**:勾「強制本地」= 兩份(public + internal)都純本地;不勾 = 兩份都可能上雲,**含 internal 那遍的麥克風私聊**。v1 不提供「只 public 上雲 / internal 純本地」的更細粒度;若 user 之後要,記為 v2(§13)。

### 5.4 key 解析(精確 2 段,鏡射 `discover_api_key`)

> ⚠️ 這是三輪 review 共同抓出的 blocker:原 draft 把 mori-core 兩個不同函式(`mod.rs:resolve_api_key_at` 與 `groq.rs:discover_api_key`)縫成一條不存在的 3 段 chain(env → `/api_keys/GROQ_API_KEY` → `/providers/groq/api_key`),並號稱「精確鏡射」。實際 Groq 的 key 解析是 `GroqProvider::discover_api_key`(groq.rs:243-255),只有 **2 段、無 `/api_keys/` 那段**。`/api_keys/<NAME>` 只給 GEMINI 用(`resolve_api_key_at`,mod.rs)。本 spec 鎖定鏡射 `discover_api_key`,刪掉 `/api_keys/GROQ_API_KEY` 那一段與對應測試。

讀 `~/.mori/config.json`,鏡射 `discover_api_key`(groq.rs:243-255):

1. env `GROQ_API_KEY`:`!is_empty() && !is_placeholder()`(env 端**有** placeholder 檢查,groq.rs:244-247)→ 用它
2. config `/providers/groq/api_key`:經 `read_json_pointer`(groq.rs:466 內含 `is_empty() || is_placeholder()` → None)→ 用它
3. 都沒有 → `None` → Groq 不進 chain,自動純本機

**關鍵 gotcha**:空字串視為「未設」(不是只檢查欄位不存在);env 與 config 值都必須非空非 placeholder 才算數。`is_placeholder` 精確語意(groq.rs:457-459):`starts_with("REPLACE")` || `contains("YOUR_GROQ")` || `== "TODO"`(全大寫比較;TODO 是完全相等,別寫成 `contains("TODO")` 否則含 TODO 子字串的合法 key 會被誤判)。

### 5.5 Ollama num_ctx 自動放大 + 端點選擇

Ollama 預設 `num_ctx=4096` 會**默默截斷**逐字稿。`OllamaSummarizer` 必須帶 `options.num_ctx`。

> ⚠️ 端點修正(review minor #6):mori-core ollama.rs 走 OpenAI-compat `/v1/chat/completions`,該層**不認 `options` 欄位**,`num_ctx` 會被默默忽略 → 仍 4096 截斷(正是想避免的 bug)。因此 `OllamaSummarizer` **不鏡射 mori-core 的端點**,改打 Ollama **原生 `POST /api/chat`**(原生端點才解析 `options`)。request body:
> ```json
> {
>   "model": "qwen3:4b-instruct-2507-q4_K_M",
>   "messages": [{"role":"system","content":"..."},{"role":"user","content":"..."}],
>   "stream": false,
>   "options": { "num_ctx": 16384 }
> }
> ```
> response 取 `message.content`(原生 `/api/chat` 的 non-stream 回應形狀)。

num_ctx 由 `pick_num_ctx(estimated_tokens)` 決定。token 估算鏡射 tokenize.rs:44 的**雙路徑**公式 `cjk_count/1.50 + non_cjk_count/3.8`(逐字稿混大量非 CJK:時間戳、`[系統]`/`[麥克風]`/`[決議依據]` 標記、講者前綴、標點 → 單一 1.50 會偏估,故照抄雙路徑)。`pick_num_ctx` 以「估出的 token 數 + prompt overhead ~2K」取下一個 2 的冪、再 clamp:

```
estimated_tokens (含 overhead) ≤ ~14K   →  16384   (含極短會議:保底 16384,不用 4096,避免任何截斷風險)
~14K < tokens ≤ ~30K                     →  32768
tokens > ~30K                            →  32768   (上限;>2hr 超長記為已知限制 §13)
```

qwen3:4b 原生支援長 context,16K–32K 對逐字稿在 8GB VRAM 上實用。warm-up 不做(§13;300s timeout 容忍冷載)。

### 5.6 redaction(拍板納入 v1,鏡射 redact.rs)

送任一 backend 前,每段文本先過 `redact_secrets`(鏡射 redact.rs:63;5 個 pattern:`gsk_`/`sk-`/`AIzaSy`/`Bearer`/40+ 高熵 fallback,marker 固定 `<REDACTED:probable-secret>`)。回 `redaction_count` 累加進 `SummaryResult` 與 audit。

> 拍板理由(消解 review major + Open Q2):redaction 是 rule-#2 的可驗證證據、零成本、驗證「強制本地」toggle 真實性。不只擋雲端 —— 本機 fallback 寫檔也先 redact,對齊 redact.rs docstring 的「本機磁碟洩漏防護」。v1 不降為「只記不遮」。

---

## 6. 兩份摘要產物與 visibility

| 產物 | 餵哪些段 | 內容 | rule-#3 |
|---|---|---|---|
| `meeting.summary.public.md` | **`visibility=="public"` 的段(系統軌 only)** | 達成的協定 / 結論:主題、客戶需求、雙方協定、我方承諾 | safe,可給客戶 |
| `meeting.summary.internal.md` | **`read_session_segments` 回傳的全部段(不做 visibility 過濾)** + supplement 標記 | 上述結論 + 內部思考/評估過程;被勾 supplement 的段特別點出 | 內部用,不對外發 |

> ⚠️ internal 輸入來源修正(review blocker #2):原 draft 說 internal「鏡射 exporter 的 visibility 過濾」。但 exporter `render_md(segments, "internal", ...)`(exporter.rs:90)是 `filter(s.visibility == "internal")`,而系統軌 segment 的 `visibility` 是 `"public"`(audio default_visibility)→ 既有 `meeting.internal.md` 主體**只含麥克風軌**(系統軌的結論靠末尾 supplement 區塊補,不是全軌)。若 internal 摘要照字面套這個過濾,會丟掉所有系統軌的結論/協定,而 internal prompt(§7.3)明確要「雙方協定」「我方承諾」這些只能從系統軌得到的小節 → 直接矛盾。**修正:internal 摘要直接用 `read_session_segments` 的全部段,不套 exporter 過濾**;每段前綴 `[系統]` / `[麥克風]` 區分(§7.3)。

過濾規則(精確):

- **public**:`segments.iter().filter(|s| s.visibility == "public")` —— 鏡射 exporter.rs:90 的 public 分支。這是唯一進 public prompt 的文本。麥克風軌(visibility=internal)在這一步消失,**不靠 prompt 自律**。即使某 public 軌段被誤標 `supplement=true`,它仍是 public-visibility 段、其文本本就該進 public(supplement 標記在 public 端不帶任何 internal 文本進來),不違反 rule-#3。
- **internal**:**不做 visibility 過濾**,用全部段。`supplement==true` 的段在 internal prompt 裡額外加 `[決議依據]` 標記(對齊 exporter.rs:71-77 概念,但這裡是當「請特別說明這些段背後考量」的提示餵 LLM,不是 append 原文)。

可重跑:user 在工作區改完逐字稿 / 勾完 supplement → 按「重新整理」→ 重新讀 segments、重生兩份摘要、覆寫、補一筆 audit。

---

## 7. Prompts

兩個 system prompt,共用一套收緊規則。實測結論(user memory 已驗證):qwen3:0.6b 報廢、1.7b 編造決議,4b 誠實會寫「無」。下列硬規則直接寫進 prompt。

### 7.1 共用收緊規則(兩個 prompt 都含)

```
規則(務必遵守):
- 只根據提供的逐字稿內容整理,不得加入逐字稿沒有的資訊。
- 不要編造「記錄時間」「記錄人員」「與會人員名單」「會議地點」等逐字稿未明確提到的欄位。
- 待辦事項只列逐字稿中明確說「要做 / 會處理 / 下次提供」的項目;沒有就寫「無」。
- 若整場沒有達成任何決議或協定,「決議 / 協定」一節就寫「無」,絕對不准編造或推測一個決議。
- 全程使用繁體中文。
- 直接輸出會議記錄本體,不要輸出你的思考過程、不要重複這些規則、不要加開場白或結語。
```

### 7.2 public prompt(系統軌 only,只結果)

```
你是一位專業的會議記錄整理者。以下是一場會議「系統軌」的逐字稿 ——
這代表會議軟體裡所有與會者(含客戶)的發言,也就是這場會議真正達成的結果。

請把它整理成一份可以提供給客戶的繁體中文會議記錄,只記錄「結果」,結構如下:

## 會議主題
（一句話概括這場會議在談什麼）

## 客戶需求 / 重點
（客戶提出的需求、關切、問題,逐點列出）

## 雙方協定 / 決議
（雙方明確達成的協定、結論;若無,寫「無」）

## 我方承諾事項
（我方在會議中明確承諾要做、要提供、要處理的事項;若無,寫「無」）

<共用收緊規則>

逐字稿:
<系統軌逐字稿文本（已 redact 疑似密鑰）>
```

### 7.3 internal prompt(全部軌,結論 + 決議依據)

```
你是一位專業的會議記錄整理者。以下是一場會議的完整逐字稿,分兩種來源:
- 【系統軌】會議軟體裡所有人(含客戶)的發言 = 達成的結果與協定。
- 【麥克風軌】我方部分在場同事不一定送進會議的私下討論 = 內部的思考與評估過程,
  也就是「我們為什麼這樣回應客戶」的依據。

請整理成一份「內部版」繁體中文會議記錄,既要記錄結果,也要補上背後的內部評估,結構如下:

## 會議主題

## 客戶需求 / 重點

## 雙方協定 / 決議
（系統軌中雙方明確達成的協定;若無,寫「無」）

## 我方承諾事項
（我方明確承諾的事項;若無,寫「無」）

## 內部評估與決議依據
（根據麥克風軌的私下討論,說明每個決議 / 對客戶的回應背後,我方評估了什麼、
  考量了哪些因素、為什麼這樣決定。逐字稿中被特別標記為「決議依據」的段落要重點納入。
  這一節只能根據麥克風軌與系統軌實際內容推導,不得憑空編造動機。）

<共用收緊規則>

逐字稿（已標明來源軌與決議依據標記、已 redact 疑似密鑰）:
<全部段;每段前綴 [系統] / [麥克風];supplement=true 的段額外加 [決議依據] 標記>
```

> internal 餵的文本格式:每段帶來源前綴(`[系統]` / `[麥克風]`)讓模型分得清結果 vs 依據;supplement 段加 `[決議依據]` 提示模型重點納入第 5 節。前綴依 `source_kind`:`meeting_system` → `[系統]`,`mic_internal` → `[麥克風]`。

---

## 8. UI

### 8.1 「會議紀錄」分區(`SessionWorkspace.tsx`)

在現有 reexport section 之前插入。與「逐字稿」並列的分區結構:

```
會議紀錄
┌──────────────────────────────────────────────┐
│ 客戶版:[☁ Groq] / [⚡ Ollama]   內部版:[☁ Groq] / [⚡ Ollama]   兩個徽章各自標 │
│ ☐ 強制本地處理(兩份都純本地、不上雲)      forceLocal toggle(文案見下)│
│ [生成摘要 / 重新整理]  (生成中…)                │
├──────────────────────────────────────────────┤
│ 分頁:[客戶版] [內部版]                          │
│ ┌─ 客戶版 ──────────────────────────────────┐ │
│ │ (讀 meeting.summary.public.md 顯示)         │ │
│ └────────────────────────────────────────────┘ │
│ ┌─ 內部版 ──────────────────────────────────┐ │
│ │ (讀 meeting.summary.internal.md 顯示)       │ │
│ │ ── 內部補充即時預覽 ──                       │ │
│ │ 勾選的麥克風段(supplement=true)列表,        │ │
│ │ 不用先重匯出就能看(直接讀記憶體裡的 segments)│ │
│ └────────────────────────────────────────────┘ │
└──────────────────────────────────────────────┘
```

force_local toggle 文案(知情同意明確,§5.3):
- label:`強制本地處理(這場敏感)`
- hint(toggle 旁小字):`勾選 = 客戶版與內部版都只用本機 Ollama;不勾 = 兩份都可能送雲端 Groq(含麥克風私聊)`

### 8.2 state hooks(鏡射 reexport 的三件組,**獨立 state 不共用**)

```tsx
const [summarizing, setSummarizing] = useState(false);
const [summaryMsg, setSummaryMsg] = useState<string | null>(null);
const [summaryErr, setSummaryErr] = useState<string | null>(null);
const [forceLocal, setForceLocal] = useState(false);
const [summaryPublic, setSummaryPublic] = useState<string | null>(null);
const [summaryInternal, setSummaryInternal] = useState<string | null>(null);
const [publicBackend, setPublicBackend] = useState<"groq" | "ollama" | null>(null);
const [internalBackend, setInternalBackend] = useState<"groq" | "ollama" | null>(null);
```

> gotcha:summary state 跟 reexport state(reexporting/reexportMsg/reexportErr)**必須獨立**,否則同時觸發互相 stomp。

handler 鏡射 reexport():`invoke("summarize_session", { sessionId, forceLocal })` → 回 `SummaryResult` → `setPublicBackend(result.public_backend)` / `setInternalBackend(result.internal_backend)` → reload 兩個 .md 檔。

### 8.3 沿用既有樣式 / i18n

- recorder UI 用自己一套 css token(`src/theme.css`,**不沿 mori-desktop 的 `var(--c-*)`** —— recorder CLAUDE.md 明定「UI css token 自己一套」)。新增徽章 / 分區 token 在 theme.css 自己定義(如 `--backend-badge-groq` 雲端色、`--backend-badge-ollama` 本地色)。
- i18n key:**全部巢狀在既有 `workspace` namespace 底下**(對齊 zh-TW.json:76-110 既有 `workspace.reexport_title` 等結構,review minor 修正,別用裸名):`workspace.summary_title`(會議紀錄)、`workspace.summary_tab_public`(客戶版)、`workspace.summary_tab_internal`(內部版)、`workspace.summary_btn`(生成摘要)、`workspace.summary_reload`(重新整理)、`workspace.summarizing`(生成中…)、`workspace.summary_ok`(摘要已更新)、`workspace.force_local`(強制本地處理)、`workspace.force_local_hint`(上述 hint 文案)、`workspace.backend_groq`(Groq)、`workspace.backend_ollama`(本機 Ollama)、`workspace.supplement_hint`(內部補充)。zh-TW.json 與 en.json 都補齊。

---

## 9. 錯誤處理(每種的 user-facing 訊息)

| 情況 | 後端行為 | UI / user-facing 訊息 |
|---|---|---|
| Groq 無 key / placeholder key | 不建構 Groq,chain = `[ollama]`,直接本機 | 徽章「⚡ 本機 Ollama」+ 提示「未設定 Groq 金鑰,改用本機處理」 |
| 斷網 / Groq 5xx / timeout / TLS 失敗 | 退 Ollama;成功則正常出摘要 | 對應徽章「⚡ Ollama」+ 訊息「雲端連線失敗,已改用本機處理」 |
| Groq 429,等待 ≤ 60s | Groq 內部 backoff 自動重試(最多 5 次嘗試) | (透明,使用者只感覺慢一點;成功後徽章「☁ Groq」) |
| Groq 429,等待 > 60s(如 TPD 用完) | 不重試、退 Ollama | 「Groq 用量已達上限(需等 N 分鐘),已改用本機處理」 |
| force_local + 本機沒裝 Ollama / 連不上 11434 | 兩遍 chain 都只有 ollama → 兩遍都失敗 → **合併成單一 error**(別讓 public 成功 internal 失敗只寫一半檔) | error「找不到本機 Ollama 服務,請確認 Ollama 已啟動(localhost:11434)」 |
| 本機有 Ollama 但沒拉 qwen3:4b 模型 | Ollama 回 model not found | error「本機缺少模型 qwen3:4b-instruct-2507-q4_K_M,請先 `ollama pull …`」(把 Ollama 回傳的 model 名帶進訊息) |
| 雲端掛 + 本機也掛(非 force_local,兩遍各退到底都失敗) | 各遍回最後一個 error,合併 | error「摘要失敗:雲端與本機都無法處理 —— <最後一個 error>」 |
| public 成功、internal 失敗(或反之) | 已成功那遍寫檔;失敗那遍不覆寫舊檔、回 error | 成功檔正常顯示 + 失敗那份顯示 error「內部版摘要失敗:<err>(客戶版已更新)」 |
| 逐字稿空 / 無 segment | 不呼叫 LLM,寫「(無逐字稿內容)」 | 訊息「這場沒有可整理的逐字稿」 |
| 逐字稿含疑似 API key | redact 後才送,`redaction_count > 0` | (透明)摘要正常出;audit hint「已遮蔽 N 處疑似密鑰」 |

所有 error 字串 surface 到 `summaryErr` state、顯示在分區內,不打斷錄音 / 其他功能。

> 部分成功語意拍板(review minor):兩遍各自獨立寫檔。先跑 public、再跑 internal;各遍成功才覆寫自己的 .md(原子寫,沿用 reexport 的 `std::fs::write`)。任一遍失敗不影響另一遍已寫的檔。`SummaryResult` 只在**兩遍都成功**時回 Ok;有任一遍失敗 → 回 Err 但已寫的檔保留(error 訊息點明哪份成功哪份失敗)。

### 9.5 本機 audit(rule-#2 可驗證證據,拍板納入 v1)

> review major 修正:cloud-first 把整場逐字稿主動送雲,原 draft 唯一機制是「徽章 + toggle」這種非持久、一閃即逝的 UI 提示,不足以當知情同意證據。對齊 redact.rs docstring「caller 應寫 audit log」的設計意圖,v1 落一筆 append-only 本機 audit。

每次 `summarize_session_inner` 成功 / 部分成功後,append 一行 JSON 到 `<session_root>/summary.audit.jsonl`(`SessionStore::summary_audit_path`):

```json
{
  "ts": "2026-05-30T14:32:10+08:00",
  "force_local": false,
  "public_backend": "groq",
  "internal_backend": "ollama",
  "redaction_count": 2,
  "public_chars": 1843,
  "internal_chars": 3120
}
```

**不存逐字稿原文、不存被遮的字串內容**(沿用 redact.rs:25-28「不存原文」原則)。audit 留本機 session 目錄,不對外傳(rule-#2)。append-only:重跑累加,可回看「這場被送過幾次雲、哪次退本機」。寫 audit 失敗只 `eprintln!` warning,不讓它擋住摘要主流程。

---

## 10. 測試計畫

`bash scripts/verify.sh`(= `cargo test` + `npm run build` + `cargo check`)為入口。

### 10.1 純函式單元測試(無網路)

- **prompt builder**:給定固定 segments,`build_public_prompt` 輸出含「客戶需求 / 雙方協定」骨架、含共用收緊規則、**不含任何 `[麥克風]` 文本**;`build_internal_prompt` 含 `[系統]`/`[麥克風]` 前綴與 supplement 段的 `[決議依據]` 標記、含「內部評估與決議依據」小節。
- **visibility 過濾(rule-#3 守門)**:餵一組混合 segments(系統 public + 麥克風 internal + 一個 public 軌被誤標 supplement),斷言 public 輸入文本**完全不含**任何 internal/麥克風段文本;斷言 internal 輸入文本**含**系統軌與麥克風軌兩者(對齊 §6 修正:internal 用全段不過濾)。
- **key 解析(鏡射 discover_api_key 的精確 2 段)**:用可注入 env_getter + config path 的 `resolve_groq_api_key_at`,測:(a) env 設非空非 placeholder → 用 env;(b) env 設 placeholder(如 `"REPLACE_ME"`)→ 跳過,讀 config `/providers/groq/api_key`;(c) env 空 → 讀 config;(d) config 值是 `"YOUR_GROQ_KEY"` placeholder → None;(e) config 值空字串 → None;(f) 全缺 → None。**不測 `/api_keys/GROQ_API_KEY`**(那不是 Groq 的解析位,§5.4 已刪)。
- **is_placeholder 精確語意**:斷言 `"REPLACEME"` true、`"my-YOUR_GROQ-key"` true、`"TODO"` true、`"a-TODO-list-key"` **false**(TODO 是完全相等非 contains)、合法 `"gsk_..."` false。
- **fallback 選擇**:兩個 fake `Summarizer`(primary 永遠 Err、fallback 永遠 Ok),斷言 `summarize_with_fallback` 回 `(fallback 文本, "ollama")` 且 `on_fallback` 被呼叫一次帶正確 failed/next name。force_local 時 chain 只有 ollama,primary 不被建構。
- **num_ctx 估算**:`estimate_gpt_oss_tokens` 對純中文 / 中英混雜分別驗雙路徑公式;`pick_num_ctx` 對 ~30K token 回 32768、~10K token 回 16384(保底)、極短回 16384;邊界值測。
- **redact**:含 `gsk_…` / `sk-…` / `AIzaSy…` / `Bearer …` / 40+ 高熵字串的逐字稿 → 輸出 `contains(REDACTION_MARKER)`(斷言用常數 `<REDACTED:probable-secret>`,不寫成 `<REDACTED:…>` 樣式 — review minor 修正:marker 不分類別)、`count` 正確。

### 10.2 backend HTTP 解析測試(`HttpTransport` 注入,不起 mock server)

把 backend 的 HTTP 呼叫抽一層:

```rust
pub trait HttpTransport: Send + Sync {
    /// 回 (status, body)。
    fn post_json(&self, url: &str, headers: &[(&str, &str)], body: &str) -> Result<(u16, String), SumError>;
}
```

`GroqSummarizer` / `OllamaSummarizer` 各持一個 `Box<dyn HttpTransport>`;production transport 用 `ureq`,測試用 `FakeTransport`(預設好的 response,可記錄收到的 url + body)。好處:零新增 dev-dep、response 解析變純函式可測、不需起真 mock server。測試:

- Groq fake 回 200 + OpenAI-compat body → 斷言 `GroqSummarizer.complete` 取到 `choices[0].message.content`。
- Groq fake 回 503 → 斷言 `complete` 回 `Err`,`summarize_with_fallback` 觸發退 ollama;回 429 + `Retry-After: 120` → 斷言不自動重試、退 fallback。
- Ollama fake:斷言送出的 **url 是 `…/api/chat`(不是 `/v1/chat/completions`)** 且 body 含 `options.num_ctx`(這是 mori-core 原版沒送、本 spec 新加的關鍵點,review 確認正確);回 `/api/chat` non-stream body → 斷言取到 `message.content`。
- 端到端(fake transport):一場含 secret 的 transcript → 跑 `summarize_session_inner` → 斷言兩份 .md 寫出、`meeting.summary.public.md` 不含麥克風內容、`redaction_count > 0`、`summary.audit.jsonl` 多一行且不含逐字稿原文。

### 10.3 前端

- `npm run build` 通過(型別 / i18n key 齊全;所有 summary key 在 `workspace` namespace 下)。
- summary state 與 reexport state 不共用的型別檢查(避免 stomp);`SummaryResult` 前端型別含 `public_backend` / `internal_backend` 兩欄。

### 10.4 config round-trip

- `RecorderConfig` 新欄位 serde per-field default:空 JSON / 缺欄 → 回各自 default(對齊 config.rs:144-157 既有 `empty_json_all_defaults` / `missing_field_falls_back_to_default` 範式);`Default::default()` 補上新欄位。

---

## 11. 硬規矩合規逐條

1. **不公開比較其他專案** — spec 全程用 Mori 自己詞彙,不寫「比 X 好」。✅
2. **User-owned data** — `~/.mori/` 是 user 的;recorder 只**讀**共享 `~/.mori/config.json` 取 provider/key,**不寫、不對外傳 config**。摘要產物 + audit 寫回 `~/.mori/meetings/<id>/`,留本機。Groq 是 user 自己的 key、user 自己的帳,recorder 不經任何中繼。送雲端有**持久本機 audit**(§9.5)當知情同意證據 + redact 長壽憑證(§5.6)。✅
3. **mic 永不混進客戶版** — public 摘要輸入在資料流**最早一站** `filter(visibility=="public")`(鏡射 exporter.rs:90),麥克風軌文本根本不進 public prompt,不靠 prompt 自律;§10.1 守門測試。即使 public 軌段被誤標 supplement 也不帶 internal 文本進 public。⚠️ 已釐清:rule-#3 是「不混進**客戶版檔案**」,internal 那遍上不上雲由 force_local 控制(§5.3 文案明確)。✅
4. **Standalone-first** — 不 import mori-core,鏡射 minimal slice(key 解析 / retry 常數 / redact / token 估算),全標精確行號;沒 mori-desktop 也能跑;勾「強制本地」走 Ollama 完全離線可用,雲端不可用自動 fallback 本機。✅
5. **Bundle deps in repo** — `summarize.rs` 自帶 `ureq` HTTP(TLS=rustls)+ 鏡射邏輯,新 dep(ureq tls feature / regex)寫進本 repo `src-tauri/Cargo.toml`(§4.6),不從外部 setup repo 拉。✅
6. **trunk-based + auto-merge** — 短命 branch off 最新 main、PR 設 auto-merge。(實作階段遵守)✅

---

## 12. 未來:AgentOS service 遷移路徑

trait `Summarizer` 是刻意留的抽換點。當共享 LLM / 摘要能力收斂成 AgentOS HTTP service(如 whisper-server「一份能力進 service,各 app 當 client」),遷移路徑:

1. 新增第三個實作 `AgentOsSummarizer`,內部 `ureq` POST 到 AgentOS service endpoint(經 manifest 宣告的 capability,走 broker allow/deny + audit)。沿用同一 `HttpTransport` 抽象。
2. `summarize_session_inner` 的 chain 組裝改成:有 AgentOS service 在線 → chain 首位放 `AgentOsSummarizer`,後面仍接本機 Ollama 當離線 fallback。
3. **上層完全不動**:prompt builder、visibility 過濾、兩份產物、UI、command 簽名全部不變 —— 它們只依賴 `Summarizer` trait 與 `SumMessage` / 純文字回傳。
4. 與 recorder 既有 `transcribe_engine: "auto"`(config.rs:29-33,whisper-server vs cli)同精神:摘要可加 `summary_engine: "auto"`,有 service 用 service、沒有退 cloud/local chain。

「換的是水管,不是價值」:雙軌語意、兩份產物、rule-#3、資料主權逃生口都留在 recorder 這層。

---

## 13. 已知限制 / 留待 v2

- **超長逐字稿(>2hr)**:`gpt-oss-120b` context window(公開資訊 128K,程式碼未記為常數)對 1–2hr 逐字稿(~25–30K token)安全;>2hr 可能逼近本機 num_ctx 上限 32768。v1 不做截斷 / 分段(極罕見),`pick_num_ctx` 上限 clamp 32768,超長會被 Ollama num_ctx 截尾 —— 記為已知限制。
- **更細粒度雲端控制**:v1 force_local 是「兩份都本地 / 都可上雲」二選一。「只 public 上雲、internal 純本地」defer v2(需 per-kind chain 與更複雜 UI)。
- **warm-up**:Ollama 冷載慢(qwen3:4b 首呼可能數十秒)。v1 不做 fire-and-forget warm-up,靠 300s timeout 容忍。若體驗不佳,v2 可加非阻塞 `/api/generate` warm-up(空 prompt、keep_alive)。
- **PII / 語意脫敏**:v1 redaction 只擋長壽憑證(API key 樣式),不擋 PII;PII 由 force_local 交 user 判斷。

---

## 14. 審查取捨(未採納的 minor / 設計選擇理由)

三輪對抗式 review 共 20 條 findings。blocker(2)+ major(6)全數修進 spec(見各節 ⚠️ 標註)。minor 處理:

- **採納**:exporter internal 過濾矛盾(blocker)、key 解析 3 段→2 段(blocker)、缺 Cargo.toml/dep 清單(major)→ §4.6、缺 audit(major)→ §9.5、internal 雲端暴露語意含糊(minor)→ §5.3/§8.1 文案、redact marker 字串精確化(minor)→ §10.1、num_ctx 端點 `/api/chat`(minor)→ §5.5、sync/async 矛盾(major)→ §4.5 定調 sync+ureq、backoff 常數名(minor)→ §5.2、i18n namespace(minor)→ §8.3、backend_used 單值→分欄(minor)→ §4.1、local runtime 坑(minor)→ §4.5 刪選項、tokenize 雙路徑(minor)→ §5.5、HttpTransport 注入免 mock dep(minor)→ §10.2、force_local 錯誤路徑(minor)→ §9 表 + §9 部分成功語意。
- **設計選擇**:Groq base_url **不**做成 config 欄位(§4.4),固定鏡射 `DEFAULT_BASE_URL`,避免使用者誤把 key 送到非 Groq 端點;recorder 自定可調的只有 model / ollama base_url。`summary_provider` 欄位從原 draft **移除**(chain 順序是固定 cloud-first,provider 由「有沒有 Groq key」+「force_local」決定,不需額外 provider 欄位徒增狀態)。

---

相關檔案(絕對路徑):
- 新檔:`/home/ct/mori-universe/mori-meeting-recorder/src-tauri/src/summarize.rs`
- 改:
  - `/home/ct/mori-universe/mori-meeting-recorder/src-tauri/Cargo.toml`(ureq tls feature + regex,§4.6)
  - `/home/ct/mori-universe/mori-meeting-recorder/src-tauri/src/main.rs`(`summarize_session` command + generate_handler 註冊)
  - `/home/ct/mori-universe/mori-meeting-recorder/src-tauri/src/config.rs`(摘要 model/base_url/force_local default 欄位)
  - `/home/ct/mori-universe/mori-meeting-recorder/src-tauri/src/session_store.rs`(3 個 path helper)
- 前端:
  - `/home/ct/mori-universe/mori-meeting-recorder/src/tabs/SessionWorkspace.tsx`
  - `/home/ct/mori-universe/mori-meeting-recorder/src/theme.css`
  - `/home/ct/mori-universe/mori-meeting-recorder/src/i18n/locales/zh-TW.json`、`/home/ct/mori-universe/mori-meeting-recorder/src/i18n/locales/en.json`
- 鏡射來源(只讀、不 import,行號已驗證):
  - `/home/ct/mori-universe/mori-desktop/crates/mori-core/src/llm/groq.rs`(discover_api_key 243-255 / is_placeholder 457-459 / read_json_pointer 462-470 / BACKOFF_SECS 34 / MAX_ATTEMPTS 32 / MAX_AUTOMATIC_RETRY_SECS 169 / DEFAULT_BASE_URL 56)
  - `/home/ct/mori-universe/mori-desktop/crates/mori-core/src/redact.rs`(redact_secrets 63 / REDACTION_MARKER 36)
  - `/home/ct/mori-universe/mori-desktop/crates/mori-core/src/tokenize.rs`(estimate_tokens 36-48 雙路徑 / is_cjk 65-70)
