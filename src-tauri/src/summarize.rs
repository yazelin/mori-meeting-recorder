//! 會議摘要 pipeline — 把停止錄音後的雙軌逐字稿整理成結構化繁中會議記錄。
//!
//! 全 **sync**(沿用 recorder 既有 ureq、不引入 reqwest / async-trait;整條鏈在
//! spawn_blocking 的 worker thread 內跑,不建任何 tokio runtime — 見 spec §4.5)。
//!
//! cloud-first:Groq `openai/gpt-oss-120b`(primary)→ 本機 Ollama
//! `qwen3:4b-instruct-2507-q4_K_M`(fallback)。public 與 internal 兩遍各自獨立走
//! fallback chain,各自回成功的 backend(SummaryResult 分兩欄)。
//!
//! 鏡射 mori-core 的 minimal slice(**不 import** mori-core),精確出處標在各 fn:
//! - key 解析:GroqProvider::discover_api_key(groq.rs:243-255)的 2 段行為
//! - is_placeholder(groq.rs:457-459)/ read_json_pointer(groq.rs:462-470)
//! - redact:redact_secrets / REDACTION_MARKER(redact.rs:63 / 36)
//! - token 估算:estimate_tokens 雙路徑(tokenize.rs:36-48)
//! - retry 常數:BACKOFF_SECS / MAX_ATTEMPTS / MAX_AUTOMATIC_RETRY_SECS(groq.rs:34/32/169)

use crate::transcribe::Segment;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;

// ── retry 常數(鏡射 groq.rs)──────────────────────────────────────────────
const MAX_ATTEMPTS: usize = 5; // groq.rs:32
const BACKOFF_SECS: [u64; 5] = [1, 2, 4, 8, 16]; // groq.rs:34
const MAX_AUTOMATIC_RETRY_SECS: u64 = 60; // groq.rs:169

const GROQ_DEFAULT_BASE_URL: &str = "https://api.groq.com/openai/v1"; // groq.rs:56
const REDACTION_MARKER: &str = "<REDACTED:probable-secret>"; // redact.rs:36

// ── 核心型別 ───────────────────────────────────────────────────────────────

/// 摘要任務的兩個版本。決定 prompt 與餵哪些 segment。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryKind {
    Public,   // 系統軌 only,只結論
    Internal, // 全部軌,結論 + 決議依據
}

/// 一次摘要請求的最小訊息結構(鏡射 OpenAI-compat chat message,只留 system/user)。
#[derive(Debug, Clone)]
pub struct SumMessage {
    pub role: &'static str, // "system" / "user"
    pub content: String,
}

#[derive(Debug)]
pub enum SumError {
    /// 網路 / server / timeout 類(觸發 fallback)。
    Backend(String),
    /// 缺 key / 缺模型 / 解析不出 content 等(也觸發 fallback,但訊息不同)。
    Config(String),
}

impl std::fmt::Display for SumError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SumError::Backend(s) => write!(f, "{s}"),
            SumError::Config(s) => write!(f, "{s}"),
        }
    }
}

/// HTTP 抽象(sync)。production 用 ureq、測試用 FakeTransport 注入 response。
pub trait HttpTransport: Send + Sync {
    /// 回 (status, body)。傳輸層失敗(斷網 / DNS / TLS)→ Err。
    fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<(u16, String), SumError>;
}

/// backend 抽象(sync)。藏在 trait 後 → 將來可換成 AgentOS HTTP service client。
pub trait Summarizer: Send + Sync {
    fn name(&self) -> &'static str; // "groq" / "ollama"
    /// 單回合純文字摘要;失敗回 Err(網路 / server / 解析)。
    fn complete(&self, messages: &[SumMessage]) -> Result<String, SumError>;
}

#[derive(serde::Serialize)]
pub struct SummaryResult {
    pub public_backend: String,   // "groq" / "ollama"(public 那遍實際用的)
    pub internal_backend: String, // "groq" / "ollama"(internal 那遍實際用的)
    pub public_chars: usize,
    pub internal_chars: usize,
    pub redaction_count: usize, // 兩遍合計、送 LLM 前共遮蔽幾處 secret
}

// ── production HTTP transport(ureq)─────────────────────────────────────────

/// ureq blocking transport。timeout 由 caller 經 agent 設定。
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    pub fn new(timeout: Duration) -> Self {
        let agent = ureq::AgentBuilder::new().timeout(timeout).build();
        Self { agent }
    }
}

impl HttpTransport for UreqTransport {
    fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<(u16, String), SumError> {
        let mut req = self.agent.post(url).set("Content-Type", "application/json");
        for (k, v) in headers {
            req = req.set(k, v);
        }
        match req.send_string(body) {
            Ok(resp) => {
                let status = resp.status();
                let text = resp
                    .into_string()
                    .map_err(|e| SumError::Backend(format!("read body: {e}")))?;
                Ok((status, text))
            }
            // ureq 把 4xx/5xx 當 Err(Status);其餘是 transport error。
            Err(ureq::Error::Status(code, resp)) => {
                let text = resp.into_string().unwrap_or_default();
                Ok((code, text))
            }
            Err(ureq::Error::Transport(t)) => {
                Err(SumError::Backend(format!("transport: {t}")))
            }
        }
    }
}

// ── Groq backend(/chat/completions,鏡射 groq.rs retry)────────────────────

pub struct GroqSummarizer {
    api_key: String,
    model: String,
    base_url: String,
    transport: Box<dyn HttpTransport>,
}

impl GroqSummarizer {
    pub fn new(api_key: String, model: String, transport: Box<dyn HttpTransport>) -> Self {
        Self {
            api_key,
            model,
            base_url: GROQ_DEFAULT_BASE_URL.to_string(),
            transport,
        }
    }

    /// production 建構:90s timeout ureq transport。
    pub fn production(api_key: String, model: String) -> Self {
        Self::new(
            api_key,
            model,
            Box::new(UreqTransport::new(Duration::from_secs(90))),
        )
    }
}

impl Summarizer for GroqSummarizer {
    fn name(&self) -> &'static str {
        "groq"
    }

    fn complete(&self, messages: &[SumMessage]) -> Result<String, SumError> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "messages": messages.iter().map(|m| serde_json::json!({
                "role": m.role,
                "content": m.content,
            })).collect::<Vec<_>>(),
        })
        .to_string();
        let bearer = format!("Bearer {}", self.api_key);

        // Retry loop:429 / 5xx 鏡射 groq.rs。429 等待 > 60s → 直接退(回 Err)。
        for attempt in 1..=MAX_ATTEMPTS {
            let headers = [("Authorization", bearer.as_str())];
            let (status, resp_body) = self.transport.post_json(&url, &headers, &body)?;
            if (200..300).contains(&status) {
                return parse_openai_content(&resp_body);
            }
            let retriable = status == 429 || (500..600).contains(&status);
            if !retriable || attempt == MAX_ATTEMPTS {
                return Err(SumError::Backend(format!(
                    "groq chat: HTTP {status}: {resp_body}"
                )));
            }
            // 429:等待 > 60s 不自動重試,直接退本機。
            if status == 429 {
                let wait = parse_retry_after_body(&resp_body).unwrap_or(BACKOFF_SECS[attempt - 1]);
                if wait > MAX_AUTOMATIC_RETRY_SECS {
                    return Err(SumError::Backend(format!(
                        "groq chat: rate limited, retry in {wait}s (> {MAX_AUTOMATIC_RETRY_SECS}s) — fall back local"
                    )));
                }
                // ≤ 60s:在 spawn_blocking thread 內 sleep backoff 再重試。
                std::thread::sleep(Duration::from_secs((wait + 1).clamp(1, MAX_AUTOMATIC_RETRY_SECS)));
                continue;
            }
            // 5xx:backoff 重試。
            std::thread::sleep(Duration::from_secs(BACKOFF_SECS[attempt - 1]));
        }
        unreachable!("loop returns on last attempt")
    }
}

/// 從 OpenAI-compat 回應抽 choices[0].message.content。解析不出 → Err(視為失敗,退本機)。
fn parse_openai_content(body: &str) -> Result<String, SumError> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| SumError::Backend(format!("parse json: {e}")))?;
    v.pointer("/choices/0/message/content")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| SumError::Backend("no choices[0].message.content".to_string()))
}

/// 從 Groq 429 body 抽「try again in Xs」秒數(精簡版,只取純秒/分秒,鏡射 groq.rs 概念)。
fn parse_retry_after_body(body: &str) -> Option<u64> {
    let lower = body.to_lowercase();
    let i = lower.find("try again in ")?;
    let rest = &body[i + "try again in ".len()..];
    let mut total = 0.0_f64;
    let mut current = String::new();
    let mut saw_unit = false;
    for ch in rest.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            current.push(ch);
        } else if matches!(ch, 'h' | 'm' | 's') {
            let n: f64 = current.parse().unwrap_or(0.0);
            let factor = match ch {
                'h' => 3600.0,
                'm' => 60.0,
                's' => 1.0,
                _ => unreachable!(),
            };
            total += n * factor;
            current.clear();
            saw_unit = true;
            if ch == 's' {
                break;
            }
        } else if ch.is_whitespace() {
            continue;
        } else {
            break;
        }
    }
    if !saw_unit {
        let n: f64 = current.parse().ok()?;
        if !(n.is_finite() && n >= 0.0) {
            return None;
        }
        return Some(n.ceil() as u64);
    }
    if !(total.is_finite() && total >= 0.0) {
        return None;
    }
    Some(total.ceil() as u64)
}

// ── Ollama backend(原生 /api/chat,帶 options.num_ctx)──────────────────────

pub struct OllamaSummarizer {
    base_url: String,
    model: String,
    num_ctx: u32,
    transport: Box<dyn HttpTransport>,
}

impl OllamaSummarizer {
    pub fn new(
        base_url: String,
        model: String,
        num_ctx: u32,
        transport: Box<dyn HttpTransport>,
    ) -> Self {
        Self {
            base_url,
            model,
            num_ctx,
            transport,
        }
    }

    /// production 建構:300s timeout(冷載容忍)。
    pub fn production(base_url: String, model: String, num_ctx: u32) -> Self {
        Self::new(
            base_url,
            model,
            num_ctx,
            Box::new(UreqTransport::new(Duration::from_secs(300))),
        )
    }
}

impl Summarizer for OllamaSummarizer {
    fn name(&self) -> &'static str {
        "ollama"
    }

    fn complete(&self, messages: &[SumMessage]) -> Result<String, SumError> {
        // 打 Ollama **原生** /api/chat(OpenAI-compat /v1/chat/completions 不認 options.num_ctx)。
        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.model,
            "messages": messages.iter().map(|m| serde_json::json!({
                "role": m.role,
                "content": m.content,
            })).collect::<Vec<_>>(),
            "stream": false,
            "options": { "num_ctx": self.num_ctx },
        })
        .to_string();

        let (status, resp_body) = self.transport.post_json(&url, &[], &body)?;
        if !(200..300).contains(&status) {
            return Err(SumError::Backend(format!(
                "ollama chat: HTTP {status}: {resp_body}"
            )));
        }
        // 原生 /api/chat non-stream 回應形狀:{"message":{"role":..,"content":..}, ...}
        let v: serde_json::Value = serde_json::from_str(&resp_body)
            .map_err(|e| SumError::Backend(format!("parse json: {e}")))?;
        v.pointer("/message/content")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| SumError::Backend("ollama: no message.content".to_string()))
    }
}

// ── fallback(鏡射 chat_with_fallback 語意)──────────────────────────────────

/// 依序試 chain,第一個成功就回 (摘要, backend_name)。每次退 fallback 觸發 callback。
/// 全失敗回最後一個 error。
pub fn summarize_with_fallback(
    chain: &[Box<dyn Summarizer>],
    messages: &[SumMessage],
    mut on_fallback: impl FnMut(&str, Option<&str>, &SumError),
) -> Result<(String, &'static str), SumError> {
    if chain.is_empty() {
        return Err(SumError::Config("no summarizer in chain".to_string()));
    }
    let mut last_err: Option<SumError> = None;
    for (i, backend) in chain.iter().enumerate() {
        match backend.complete(messages) {
            Ok(text) => return Ok((text, backend.name())),
            Err(e) => {
                let next = chain.get(i + 1).map(|b| b.name());
                on_fallback(backend.name(), next, &e);
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| SumError::Backend("all backends failed".to_string())))
}

// ── 輔助(鏡射 mori-core,純 re-implement)─────────────────────────────────

/// 鏡射 groq.rs:457-459。`starts_with("REPLACE")` || `contains("YOUR_GROQ")` || `== "TODO"`。
/// 注意:TODO 是「全大寫完全相等」,不是 contains。
fn is_placeholder(s: &str) -> bool {
    let upper = s.to_uppercase();
    upper.starts_with("REPLACE") || upper.contains("YOUR_GROQ") || upper == "TODO"
}

/// 鏡射 groq.rs:462-470。讀 JSON pointer,空字串 / placeholder → None。
fn read_json_pointer(path: &Path, pointer: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let key = json.pointer(pointer)?.as_str()?;
    if key.is_empty() || is_placeholder(key) {
        return None;
    }
    Some(key.to_string())
}

/// ~/.mori/config.json(共享 config,不是 recorder 自己的 meeting-recorder/config.json)。
/// 鏡射 groq.rs:250-253 的 home.join(".mori").join("config.json")。
fn mori_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".mori").join("config.json"))
}

/// 鏡射 GroqProvider::discover_api_key(groq.rs:243-255)的精確 2 段行為。
/// ① env GROQ_API_KEY(非空 && 非 placeholder)② config /providers/groq/api_key。
pub fn resolve_groq_api_key(config_path: &Path) -> Option<String> {
    resolve_groq_api_key_at(config_path, |k| std::env::var(k).ok())
}

/// 可注入版(供測試):env_getter 取代 std::env::var。
fn resolve_groq_api_key_at(
    config_path: &Path,
    env_getter: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    // ① env(env 端有 placeholder 檢查,groq.rs:244-247)
    if let Some(key) = env_getter("GROQ_API_KEY") {
        if !key.is_empty() && !is_placeholder(&key) {
            return Some(key);
        }
    }
    // ② config /providers/groq/api_key(read_json_pointer 內含 is_empty/is_placeholder 過濾)
    read_json_pointer(config_path, "/providers/groq/api_key")
}

/// 鏡射 redact.rs:38-56 的 5 個 pattern(精準 → 寬鬆),redact.rs:63 的 replace_all 邏輯。
fn redact_patterns() -> &'static [Regex] {
    static CELL: OnceLock<Vec<Regex>> = OnceLock::new();
    CELL.get_or_init(|| {
        vec![
            Regex::new(r"gsk_[A-Za-z0-9]{40,}").unwrap(),
            Regex::new(r"sk-[A-Za-z0-9_\-]{40,}").unwrap(),
            Regex::new(r"AIzaSy[A-Za-z0-9_\-]{30,}").unwrap(),
            Regex::new(r"(?i)Bearer\s+[A-Za-z0-9._\-]{20,}").unwrap(),
            Regex::new(r"[A-Za-z0-9_\-]{40,}").unwrap(),
        ]
    })
}

/// 鏡射 redact.rs:63。回 (redacted_text, hit_count)。
fn redact_secrets(input: &str) -> (String, usize) {
    let mut result = input.to_string();
    let mut total = 0usize;
    for re in redact_patterns() {
        let count = re.find_iter(&result).count();
        if count > 0 {
            result = re.replace_all(&result, REDACTION_MARKER).into_owned();
            total += count;
        }
    }
    (result, total)
}

/// 鏡射 tokenize.rs:36-48 的雙路徑估算(gpt-oss path:cjk/1.50 + non_cjk/3.8)。
fn estimate_gpt_oss_tokens(text: &str) -> usize {
    let cjk_count = text.chars().filter(is_cjk).count();
    let non_cjk_count = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .count()
        .saturating_sub(cjk_count);
    (cjk_count as f64 / 1.50 + non_cjk_count as f64 / 3.8).round() as usize
}

/// 鏡射 tokenize.rs:65-70。
fn is_cjk(c: &char) -> bool {
    let code = *c as u32;
    (0x4E00..=0x9FFF).contains(&code)
        || (0x3400..=0x4DBF).contains(&code)
        || (0xF900..=0xFAFF).contains(&code)
}

/// num_ctx 依逐字稿長度自動放大(§5.5)。estimated_tokens 已含 prompt overhead ~2K。
/// ≤ ~14K → 16384(保底,不用 4096);否則 → 32768(上限)。
fn pick_num_ctx(estimated_tokens: usize) -> u32 {
    if estimated_tokens <= 14_000 {
        16384
    } else {
        32768
    }
}

// ── 共用收緊規則 + prompt builder(§7)──────────────────────────────────────

const TIGHTENING_RULES: &str = "\
規則(務必遵守):
- 只根據提供的逐字稿內容整理,不得加入逐字稿沒有的資訊。
- 不要編造「記錄時間」「記錄人員」「與會人員名單」「會議地點」等逐字稿未明確提到的欄位。
- 待辦事項只列逐字稿中明確說「要做 / 會處理 / 下次提供」的項目;沒有就寫「無」。
- 若整場沒有達成任何決議或協定,「決議 / 協定」一節就寫「無」,絕對不准編造或推測一個決議。
- 全程使用繁體中文。
- 直接輸出會議記錄本體,不要輸出你的思考過程、不要重複這些規則、不要加開場白或結語。";

/// public system prompt(系統軌 only,只結果,§7.2)。
fn public_system_prompt() -> String {
    format!(
        "你是一位專業的會議記錄整理者。以下是一場會議「系統軌」的逐字稿 ——\n\
這代表會議軟體裡所有與會者(含客戶)的發言,也就是這場會議真正達成的結果。\n\n\
請把它整理成一份可以提供給客戶的繁體中文會議記錄,只記錄「結果」,結構如下:\n\n\
## 會議主題\n(一句話概括這場會議在談什麼)\n\n\
## 客戶需求 / 重點\n(客戶提出的需求、關切、問題,逐點列出)\n\n\
## 雙方協定 / 決議\n(雙方明確達成的協定、結論;若無,寫「無」)\n\n\
## 我方承諾事項\n(我方在會議中明確承諾要做、要提供、要處理的事項;若無,寫「無」)\n\n\
{TIGHTENING_RULES}"
    )
}

/// internal system prompt(全部軌,結論 + 決議依據,§7.3)。
fn internal_system_prompt() -> String {
    format!(
        "你是一位專業的會議記錄整理者。以下是一場會議的完整逐字稿,分兩種來源:\n\
- 【系統軌】會議軟體裡所有人(含客戶)的發言 = 達成的結果與協定。\n\
- 【麥克風軌】我方部分在場同事不一定送進會議的私下討論 = 內部的思考與評估過程,\n\
  也就是「我們為什麼這樣回應客戶」的依據。\n\n\
請整理成一份「內部版」繁體中文會議記錄,既要記錄結果,也要補上背後的內部評估,結構如下:\n\n\
## 會議主題\n\n## 客戶需求 / 重點\n\n\
## 雙方協定 / 決議\n(系統軌中雙方明確達成的協定;若無,寫「無」)\n\n\
## 我方承諾事項\n(我方明確承諾的事項;若無,寫「無」)\n\n\
## 內部評估與決議依據\n\
(根據麥克風軌的私下討論,說明每個決議 / 對客戶的回應背後,我方評估了什麼、\n\
  考量了哪些因素、為什麼這樣決定。逐字稿中被特別標記為「決議依據」的段落要重點納入。\n\
  這一節只能根據麥克風軌與系統軌實際內容推導,不得憑空編造動機。)\n\n\
{TIGHTENING_RULES}"
    )
}

/// public 逐字稿文本:只餵 visibility=="public" 的段(鏡射 exporter.rs:90)。已 redact。
/// 回 (文本, redaction_count)。
fn build_public_transcript(segments: &[Segment]) -> (String, usize) {
    let mut filtered: Vec<&Segment> =
        segments.iter().filter(|s| s.visibility == "public").collect();
    filtered.sort_by_key(|s| s.start_ms);
    let mut out = String::new();
    let mut redactions = 0usize;
    for s in filtered {
        let (red, n) = redact_secrets(&s.text);
        redactions += n;
        out.push_str(&red);
        out.push('\n');
    }
    (out, redactions)
}

/// internal 逐字稿文本:全部段(不做 visibility 過濾,§6 修正),每段前綴來源軌、
/// supplement 段加 [決議依據]。已 redact。回 (文本, redaction_count)。
fn build_internal_transcript(segments: &[Segment]) -> (String, usize) {
    let mut sorted: Vec<&Segment> = segments.iter().collect();
    sorted.sort_by_key(|s| s.start_ms);
    let mut out = String::new();
    let mut redactions = 0usize;
    for s in sorted {
        let source_prefix = match s.source_kind.as_str() {
            "meeting_system" => "[系統]",
            "mic_internal" => "[麥克風]",
            _ => "[未知]",
        };
        let supp = if s.supplement { "[決議依據]" } else { "" };
        let (red, n) = redact_secrets(&s.text);
        redactions += n;
        out.push_str(&format!("{source_prefix}{supp} {red}\n"));
    }
    (out, redactions)
}

/// 組 public messages(system + user)。回 (messages, redaction_count)。
pub fn build_public_prompt(segments: &[Segment]) -> (Vec<SumMessage>, usize) {
    let (transcript, redactions) = build_public_transcript(segments);
    let user = format!("逐字稿:\n{transcript}");
    (
        vec![
            SumMessage {
                role: "system",
                content: public_system_prompt(),
            },
            SumMessage {
                role: "user",
                content: user,
            },
        ],
        redactions,
    )
}

/// 組 internal messages(system + user)。回 (messages, redaction_count)。
pub fn build_internal_prompt(segments: &[Segment]) -> (Vec<SumMessage>, usize) {
    let (transcript, redactions) = build_internal_transcript(segments);
    let user = format!(
        "逐字稿(已標明來源軌與決議依據標記、已 redact 疑似密鑰):\n{transcript}"
    );
    (
        vec![
            SumMessage {
                role: "system",
                content: internal_system_prompt(),
            },
            SumMessage {
                role: "user",
                content: user,
            },
        ],
        redactions,
    )
}

// ── chain 組裝 ───────────────────────────────────────────────────────────────

/// 為一遍摘要組 fallback chain。force_local → 只 [ollama](連 Groq 都不建構)。
/// 非 force_local 且有 Groq key → [groq, ollama];無 key → [ollama]。
fn build_chain(
    force_local: bool,
    groq_key: Option<&str>,
    groq_model: &str,
    ollama_base_url: &str,
    ollama_model: &str,
    num_ctx: u32,
) -> Vec<Box<dyn Summarizer>> {
    let mut chain: Vec<Box<dyn Summarizer>> = Vec::new();
    if !force_local {
        if let Some(key) = groq_key {
            chain.push(Box::new(GroqSummarizer::production(
                key.to_string(),
                groq_model.to_string(),
            )));
        }
    }
    chain.push(Box::new(OllamaSummarizer::production(
        ollama_base_url.to_string(),
        ollama_model.to_string(),
        num_ctx,
    )));
    chain
}

// ── 主流程(鏡射 postprocess::reexport_session 的讀→處理→原子寫檔)────────────

/// 主流程(sync)。讀 segments → visibility 切兩路 → redact(在 prompt builder 內)
/// → fallback chain → 原子寫兩份 .md → append audit。
pub fn summarize_session_inner(
    session_root: &Path,
    force_local: bool,
) -> Result<SummaryResult, String> {
    let segments = crate::postprocess::read_session_segments(session_root);
    let cfg = crate::config::read_config();
    let groq_key_owned = if force_local {
        None
    } else {
        mori_config_path().and_then(|p| resolve_groq_api_key(&p))
    };

    // 逐字稿空 → 不呼叫 LLM,寫佔位內容(§9 錯誤表)。
    if segments.is_empty() {
        let placeholder = "(無逐字稿內容)\n";
        atomic_write(&summary_public_path(session_root), placeholder)?;
        atomic_write(&summary_internal_path(session_root), placeholder)?;
        append_audit(session_root, force_local, "none", "none", 0, placeholder.len(), placeholder.len());
        return Ok(SummaryResult {
            public_backend: "none".to_string(),
            internal_backend: "none".to_string(),
            public_chars: placeholder.len(),
            internal_chars: placeholder.len(),
            redaction_count: 0,
        });
    }

    // num_ctx 依「全段」估算(internal 餵最多 → 用它定 num_ctx,public 同 ctx 安全)。
    let (internal_transcript_for_est, _) = build_internal_transcript(&segments);
    let est = estimate_gpt_oss_tokens(&internal_transcript_for_est) + 2_000; // prompt overhead
    let num_ctx = pick_num_ctx(est);

    let groq_key_ref = groq_key_owned.as_deref();

    // ── public 那遍 ──
    let (public_msgs, public_redactions) = build_public_prompt(&segments);
    let public_chain = build_chain(
        force_local,
        groq_key_ref,
        &cfg.summary_groq_model,
        &cfg.summary_ollama_base_url,
        &cfg.summary_ollama_model,
        num_ctx,
    );
    let public_res = summarize_with_fallback(&public_chain, &public_msgs, |failed, next, err| {
        eprintln!(
            "summarize public: backend '{failed}' failed ({err}); next = {}",
            next.unwrap_or("(none)")
        );
    });

    // ── internal 那遍 ──
    let (internal_msgs, internal_redactions) = build_internal_prompt(&segments);
    let internal_chain = build_chain(
        force_local,
        groq_key_ref,
        &cfg.summary_groq_model,
        &cfg.summary_ollama_base_url,
        &cfg.summary_ollama_model,
        num_ctx,
    );
    let internal_res =
        summarize_with_fallback(&internal_chain, &internal_msgs, |failed, next, err| {
            eprintln!(
                "summarize internal: backend '{failed}' failed ({err}); next = {}",
                next.unwrap_or("(none)")
            );
        });

    let redaction_count = public_redactions + internal_redactions;

    // 各遍成功才覆寫自己的 .md(原子寫,沿用 reexport)。失敗那遍不覆寫舊檔。
    let public_outcome = match &public_res {
        Ok((text, backend)) => {
            atomic_write(&summary_public_path(session_root), text)?;
            Some((backend.to_string(), text.chars().count()))
        }
        Err(_) => None,
    };
    let internal_outcome = match &internal_res {
        Ok((text, backend)) => {
            atomic_write(&summary_internal_path(session_root), text)?;
            Some((backend.to_string(), text.chars().count()))
        }
        Err(_) => None,
    };

    // audit:成功 / 部分成功都記一筆(§9.5)。
    let (pub_b, pub_c) = public_outcome
        .clone()
        .unwrap_or_else(|| ("(failed)".to_string(), 0));
    let (int_b, int_c) = internal_outcome
        .clone()
        .unwrap_or_else(|| ("(failed)".to_string(), 0));
    append_audit(
        session_root,
        force_local,
        &pub_b,
        &int_b,
        redaction_count,
        pub_c,
        int_c,
    );

    // 兩遍都成功 → Ok;任一遍失敗 → Err(已寫的檔保留)。
    match (public_res, internal_res) {
        (Ok((_, pb)), Ok((_, ib))) => Ok(SummaryResult {
            public_backend: pb.to_string(),
            internal_backend: ib.to_string(),
            public_chars: pub_c,
            internal_chars: int_c,
            redaction_count,
        }),
        (Err(pe), Err(ie)) => Err(format!(
            "摘要失敗:雲端與本機都無法處理 —— public: {pe};internal: {ie}"
        )),
        (Err(pe), Ok(_)) => Err(format!("客戶版摘要失敗:{pe}(內部版已更新)")),
        (Ok(_), Err(ie)) => Err(format!("內部版摘要失敗:{ie}(客戶版已更新)")),
    }
}

fn summary_public_path(root: &Path) -> PathBuf {
    root.join("meeting.summary.public.md")
}
fn summary_internal_path(root: &Path) -> PathBuf {
    root.join("meeting.summary.internal.md")
}
fn summary_audit_path(root: &Path) -> PathBuf {
    root.join("summary.audit.jsonl")
}

/// 原子寫檔(沿用 reexport 的 write;tmp + rename 保證 reader 不讀到半截)。
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("write tmp: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename: {e}"))
}

/// append 一行 audit JSON(§9.5)。**不存逐字稿原文 / 被遮字串**。失敗只 warning。
#[allow(clippy::too_many_arguments)]
fn append_audit(
    session_root: &Path,
    force_local: bool,
    public_backend: &str,
    internal_backend: &str,
    redaction_count: usize,
    public_chars: usize,
    internal_chars: usize,
) {
    use std::io::Write;
    let entry = serde_json::json!({
        "ts": chrono::Local::now().to_rfc3339(),
        "force_local": force_local,
        "public_backend": public_backend,
        "internal_backend": internal_backend,
        "redaction_count": redaction_count,
        "public_chars": public_chars,
        "internal_chars": internal_chars,
    });
    let path = summary_audit_path(session_root);
    let line = match serde_json::to_string(&entry) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("summary audit: serialize failed: {e}");
            return;
        }
    };
    let res = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| writeln!(f, "{line}"));
    if let Err(e) = res {
        eprintln!("summary audit: append failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn seg(track: &str, source_kind: &str, visibility: &str, start_ms: u64, text: &str, supplement: bool) -> Segment {
        Segment {
            id: "x".into(),
            session_id: "m".into(),
            track: track.into(),
            source_kind: source_kind.into(),
            visibility: visibility.into(),
            start_ms,
            end_ms: start_ms + 1000,
            text: text.into(),
            is_final: true,
            confidence: None,
            speaker: None,
            speaker_mixed: false,
            supplement,
        }
    }

    fn mixed_segments() -> Vec<Segment> {
        vec![
            seg("system", "meeting_system", "public", 1000, "客戶要求三週後上線", false),
            seg("mic-internal", "mic_internal", "internal", 2000, "我們其實做不到三週這是麥克風私聊", false),
            // 一個 public 軌段被誤標 supplement → 仍是 public、不帶 internal 文本
            seg("system", "meeting_system", "public", 3000, "我方承諾兩週內給報價", true),
            // 一個 mic 段被標 supplement(決議依據)
            seg("mic-internal", "mic_internal", "internal", 4000, "報價要保守抓不然會虧", true),
        ]
    }

    // ── 10.1 prompt builder ──
    #[test]
    fn public_prompt_has_skeleton_and_rules_no_mic() {
        let (msgs, _) = build_public_prompt(&mixed_segments());
        let system = &msgs[0].content;
        let user = &msgs[1].content;
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[1].role, "user");
        assert!(system.contains("客戶需求"));
        assert!(system.contains("雙方協定"));
        assert!(system.contains("只根據提供的逐字稿內容整理")); // 共用收緊規則
        // public user 文本不含任何麥克風段文本,也不含 [麥克風] 前綴
        assert!(!user.contains("[麥克風]"), "public must not carry mic prefix");
        assert!(!user.contains("這是麥克風私聊"), "public leaked mic text:\n{user}");
        assert!(!user.contains("報價要保守抓不然會虧"));
        // public 仍含系統軌文本
        assert!(user.contains("客戶要求三週後上線"));
        assert!(user.contains("我方承諾兩週內給報價"));
    }

    #[test]
    fn internal_prompt_has_prefixes_and_supplement_marker() {
        let (msgs, _) = build_internal_prompt(&mixed_segments());
        let system = &msgs[0].content;
        let user = &msgs[1].content;
        assert!(system.contains("內部評估與決議依據"));
        assert!(system.contains("只根據提供的逐字稿內容整理"));
        // internal 含系統與麥克風兩種前綴
        assert!(user.contains("[系統]"));
        assert!(user.contains("[麥克風]"));
        // supplement 段加 [決議依據]
        assert!(user.contains("[決議依據]"), "internal missing supplement marker:\n{user}");
        // 含麥克風與系統兩軌文本
        assert!(user.contains("這是麥克風私聊"));
        assert!(user.contains("客戶要求三週後上線"));
    }

    // ── 10.1 visibility 過濾守門(rule-#3)──
    #[test]
    fn visibility_gate_public_excludes_internal_internal_includes_both() {
        let segs = mixed_segments();
        let (pub_text, _) = build_public_transcript(&segs);
        let (int_text, _) = build_internal_transcript(&segs);
        // public 完全不含任何 internal/麥克風段文本
        assert!(!pub_text.contains("這是麥克風私聊"));
        assert!(!pub_text.contains("報價要保守抓不然會虧"));
        // internal 含系統軌與麥克風軌兩者
        assert!(int_text.contains("客戶要求三週後上線"));
        assert!(int_text.contains("這是麥克風私聊"));
    }

    // ── 10.1 key 解析(精確 2 段)──
    #[test]
    fn resolve_key_env_non_placeholder_wins() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.json");
        std::fs::write(&cfg, r#"{"providers":{"groq":{"api_key":"gsk_fromconfig"}}}"#).unwrap();
        let env = |k: &str| if k == "GROQ_API_KEY" { Some("gsk_fromenv".to_string()) } else { None };
        assert_eq!(resolve_groq_api_key_at(&cfg, env).as_deref(), Some("gsk_fromenv"));
    }

    #[test]
    fn resolve_key_env_placeholder_falls_to_config() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.json");
        std::fs::write(&cfg, r#"{"providers":{"groq":{"api_key":"gsk_realconfigkey"}}}"#).unwrap();
        let env = |k: &str| if k == "GROQ_API_KEY" { Some("REPLACE_ME".to_string()) } else { None };
        assert_eq!(resolve_groq_api_key_at(&cfg, env).as_deref(), Some("gsk_realconfigkey"));
    }

    #[test]
    fn resolve_key_env_empty_falls_to_config() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.json");
        std::fs::write(&cfg, r#"{"providers":{"groq":{"api_key":"gsk_cfg"}}}"#).unwrap();
        let env = |k: &str| if k == "GROQ_API_KEY" { Some(String::new()) } else { None };
        assert_eq!(resolve_groq_api_key_at(&cfg, env).as_deref(), Some("gsk_cfg"));
    }

    #[test]
    fn resolve_key_config_placeholder_is_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.json");
        std::fs::write(&cfg, r#"{"providers":{"groq":{"api_key":"YOUR_GROQ_KEY"}}}"#).unwrap();
        let env = |_: &str| None;
        assert_eq!(resolve_groq_api_key_at(&cfg, env), None);
    }

    #[test]
    fn resolve_key_config_empty_is_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.json");
        std::fs::write(&cfg, r#"{"providers":{"groq":{"api_key":""}}}"#).unwrap();
        let env = |_: &str| None;
        assert_eq!(resolve_groq_api_key_at(&cfg, env), None);
    }

    #[test]
    fn resolve_key_all_missing_is_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = dir.path().join("config.json");
        std::fs::write(&cfg, r#"{"providers":{}}"#).unwrap();
        let env = |_: &str| None;
        assert_eq!(resolve_groq_api_key_at(&cfg, env), None);
    }

    // ── 10.1 is_placeholder 精確語意 ──
    #[test]
    fn is_placeholder_exact_semantics() {
        assert!(is_placeholder("REPLACEME"));
        assert!(is_placeholder("REPLACE_ME_WITH_YOUR_GROQ_API_KEY"));
        assert!(is_placeholder("my-YOUR_GROQ-key"));
        assert!(is_placeholder("TODO"));
        assert!(is_placeholder("todo")); // 全大寫比較
        assert!(!is_placeholder("a-TODO-list-key")); // TODO 是完全相等,非 contains
        assert!(!is_placeholder("gsk_legitkey123"));
    }

    // ── 10.1 num_ctx 估算 ──
    #[test]
    fn estimate_tokens_dual_path() {
        // 純中文 30 字 → ~30/1.5 = 20
        let zh = "今天天氣很好我們去公園散步順便買點咖啡跟麵包回家當晚餐喔啦啦啦";
        let est = estimate_gpt_oss_tokens(zh);
        let expect = (zh.chars().count() as f64 / 1.50).round() as usize;
        assert_eq!(est, expect);
        // 中英混雜:走雙路徑(cjk/1.50 + non_cjk/3.8)
        let mixed = "系統 update 完成 deploy v2";
        let cjk = mixed.chars().filter(is_cjk).count();
        let non_cjk = mixed.chars().filter(|c| !c.is_whitespace()).count() - cjk;
        let expect_mixed = (cjk as f64 / 1.50 + non_cjk as f64 / 3.8).round() as usize;
        assert_eq!(estimate_gpt_oss_tokens(mixed), expect_mixed);
    }

    #[test]
    fn pick_num_ctx_boundaries() {
        assert_eq!(pick_num_ctx(0), 16384); // 極短保底
        assert_eq!(pick_num_ctx(10_000), 16384);
        assert_eq!(pick_num_ctx(14_000), 16384); // 邊界含等於
        assert_eq!(pick_num_ctx(14_001), 32768);
        assert_eq!(pick_num_ctx(30_000), 32768);
        assert_eq!(pick_num_ctx(100_000), 32768); // 上限 clamp
    }

    // ── 10.1 redact ──
    #[test]
    fn redact_catches_all_pattern_types() {
        let s = "key gsk_TESTXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX 跟 \
                 sk-test-XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX 跟 \
                 AIzaSyTESTXXXXXXXXXXXXXXXXXXXXXXXXXXX 跟 \
                 Bearer fakeXXXXXXXXXXXXXXXXXXXX 跟 \
                 abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH 結束";
        let (out, n) = redact_secrets(s);
        assert!(out.contains("<REDACTED:probable-secret>"));
        assert!(!out.contains("gsk_TEST"));
        assert!(n >= 5, "expected >=5 redactions, got {n}");
    }

    #[test]
    fn redact_safe_text_unchanged() {
        let s = "今天天氣很好,翻譯成英文。";
        let (out, n) = redact_secrets(s);
        assert_eq!(out, s);
        assert_eq!(n, 0);
    }

    // ── 10.1 fallback 選擇 ──
    struct FakeBackend {
        name: &'static str,
        ok: bool,
        reply: String,
    }
    impl Summarizer for FakeBackend {
        fn name(&self) -> &'static str {
            self.name
        }
        fn complete(&self, _messages: &[SumMessage]) -> Result<String, SumError> {
            if self.ok {
                Ok(self.reply.clone())
            } else {
                Err(SumError::Backend("fake backend down".to_string()))
            }
        }
    }

    #[test]
    fn fallback_picks_second_when_primary_fails() {
        let chain: Vec<Box<dyn Summarizer>> = vec![
            Box::new(FakeBackend { name: "groq", ok: false, reply: String::new() }),
            Box::new(FakeBackend { name: "ollama", ok: true, reply: "本機摘要".into() }),
        ];
        let mut calls: Vec<(String, Option<String>)> = Vec::new();
        let res = summarize_with_fallback(
            &chain,
            &[SumMessage { role: "user", content: "x".into() }],
            |failed, next, _err| calls.push((failed.to_string(), next.map(|s| s.to_string()))),
        )
        .unwrap();
        assert_eq!(res.0, "本機摘要");
        assert_eq!(res.1, "ollama");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "groq");
        assert_eq!(calls[0].1.as_deref(), Some("ollama"));
    }

    #[test]
    fn force_local_chain_has_only_ollama() {
        // build_chain(force_local=true, key=Some) → 連 Groq 都不建構。
        let chain = build_chain(true, Some("gsk_x"), "m", "http://localhost:11434", "om", 16384);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].name(), "ollama");
    }

    #[test]
    fn no_key_chain_has_only_ollama() {
        let chain = build_chain(false, None, "m", "http://localhost:11434", "om", 16384);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].name(), "ollama");
    }

    #[test]
    fn with_key_chain_is_groq_then_ollama() {
        let chain = build_chain(false, Some("gsk_x"), "m", "http://localhost:11434", "om", 16384);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].name(), "groq");
        assert_eq!(chain[1].name(), "ollama");
    }

    // ── 10.2 HttpTransport 注入 ──
    struct FakeTransport {
        status: u16,
        body: String,
    }
    impl FakeTransport {
        fn new(status: u16, body: &str) -> Self {
            Self { status, body: body.to_string() }
        }
    }
    impl HttpTransport for FakeTransport {
        fn post_json(&self, _url: &str, _headers: &[(&str, &str)], _body: &str) -> Result<(u16, String), SumError> {
            Ok((self.status, self.body.clone()))
        }
    }

    #[test]
    fn groq_parses_openai_content() {
        let fake = FakeTransport::new(
            200,
            r#"{"choices":[{"message":{"role":"assistant","content":"整理好的會議記錄"}}]}"#,
        );
        let g = GroqSummarizer::new("gsk_x".into(), "openai/gpt-oss-120b".into(), Box::new(fake));
        let out = g.complete(&[SumMessage { role: "user", content: "x".into() }]).unwrap();
        assert_eq!(out, "整理好的會議記錄");
    }

    #[test]
    fn groq_429_long_wait_falls_back_no_retry() {
        // 429 + body 指示 120s > 60s → 立刻 Err,不 sleep retry。
        let fake = FakeTransport::new(
            429,
            r#"{"error":{"message":"Rate limit reached. Please try again in 120s."}}"#,
        );
        let g = GroqSummarizer::new("gsk_x".into(), "m".into(), Box::new(fake));
        let res = g.complete(&[SumMessage { role: "user", content: "x".into() }]);
        assert!(res.is_err(), "429 long-wait should be Err");
        // fallback chain 接住
        let g2 = GroqSummarizer::new(
            "gsk_x".into(),
            "m".into(),
            Box::new(FakeTransport::new(
                429,
                r#"{"error":{"message":"Please try again in 120s."}}"#,
            )),
        );
        let chain: Vec<Box<dyn Summarizer>> = vec![
            Box::new(g2),
            Box::new(FakeBackend { name: "ollama", ok: true, reply: "本機接手".into() }),
        ];
        let out = summarize_with_fallback(
            &chain,
            &[SumMessage { role: "user", content: "x".into() }],
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(out.1, "ollama");
        assert_eq!(out.0, "本機接手");
    }

    #[test]
    fn ollama_hits_api_chat_with_num_ctx_and_parses_message_content() {
        let fake = FakeTransport::new(
            200,
            r#"{"model":"qwen3","message":{"role":"assistant","content":"本機整理結果"},"done":true}"#,
        );
        let o = OllamaSummarizer::new(
            "http://localhost:11434".into(),
            "qwen3:4b-instruct-2507-q4_K_M".into(),
            32768,
            Box::new(fake),
        );
        let out = o.complete(&[SumMessage { role: "system", content: "s".into() }]).unwrap();
        assert_eq!(out, "本機整理結果");
    }

    // 用一個能 inspect 的 transport 驗 url + body
    struct CapturingTransport {
        captured: std::sync::Arc<Mutex<Vec<(String, String)>>>,
        status: u16,
        body: String,
    }
    impl HttpTransport for CapturingTransport {
        fn post_json(&self, url: &str, _headers: &[(&str, &str)], body: &str) -> Result<(u16, String), SumError> {
            self.captured.lock().unwrap().push((url.to_string(), body.to_string()));
            Ok((self.status, self.body.clone()))
        }
    }

    #[test]
    fn ollama_request_url_is_api_chat_and_body_has_options_num_ctx() {
        let captured = std::sync::Arc::new(Mutex::new(Vec::new()));
        let t = CapturingTransport {
            captured: captured.clone(),
            status: 200,
            body: r#"{"message":{"content":"ok"}}"#.to_string(),
        };
        let o = OllamaSummarizer::new(
            "http://localhost:11434".into(),
            "qwen3:4b".into(),
            16384,
            Box::new(t),
        );
        o.complete(&[SumMessage { role: "user", content: "x".into() }]).unwrap();
        let cap = captured.lock().unwrap();
        let (url, body) = &cap[0];
        assert!(url.ends_with("/api/chat"), "ollama must hit /api/chat, got {url}");
        assert!(!url.contains("/v1/chat/completions"), "must NOT be OpenAI-compat endpoint");
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(v.pointer("/options/num_ctx").and_then(|x| x.as_u64()), Some(16384));
        assert_eq!(v.pointer("/stream").and_then(|x| x.as_bool()), Some(false));
    }

    // ── 10.2 端到端(fake transport,經 chain 注入) ──
    // 端到端用真實 summarize_session_inner 會 hit production transport(連真網路)。
    // 為純測試,直接組裝 fake chain 跑 prompt builder + 寫檔 + audit 的等價流程。
    #[test]
    fn end_to_end_writes_two_md_public_no_mic_redaction_audit() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let transcript_dir = root.join("transcript");
        std::fs::create_dir_all(&transcript_dir).unwrap();

        // 一場含 secret 的 transcript(系統軌 + 麥克風軌)
        let sys = seg(
            "system",
            "meeting_system",
            "public",
            1000,
            "客戶說 API key 是 gsk_TESTXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX 要保密",
            false,
        );
        let mic = seg(
            "mic-internal",
            "mic_internal",
            "internal",
            2000,
            "私下講:這客戶很機車這是麥克風內容",
            true,
        );
        crate::transcribe::write_segments_jsonl(
            &transcript_dir.join("system.segments.jsonl"),
            &[sys],
        )
        .unwrap();
        crate::transcribe::write_segments_jsonl(
            &transcript_dir.join("mic-internal.segments.jsonl"),
            &[mic],
        )
        .unwrap();

        // 用 fake chain 跑「summarize_session_inner 的等價骨架」(避開 production transport)。
        let segments = crate::postprocess::read_session_segments(root);
        let (public_msgs, pub_red) = build_public_prompt(&segments);
        let (internal_msgs, int_red) = build_internal_prompt(&segments);
        let redaction_count = pub_red + int_red;
        assert!(redaction_count > 0, "secret should have been redacted");

        let fake_chain = |reply: &str| -> Vec<Box<dyn Summarizer>> {
            vec![Box::new(FakeBackend { name: "ollama", ok: true, reply: reply.to_string() })]
        };
        let (pub_text, pub_b) =
            summarize_with_fallback(&fake_chain("客戶版摘要本體"), &public_msgs, |_, _, _| {}).unwrap();
        let (int_text, int_b) =
            summarize_with_fallback(&fake_chain("內部版摘要本體"), &internal_msgs, |_, _, _| {}).unwrap();

        atomic_write(&summary_public_path(root), &pub_text).unwrap();
        atomic_write(&summary_internal_path(root), &int_text).unwrap();
        append_audit(root, true, pub_b, int_b, redaction_count, pub_text.chars().count(), int_text.chars().count());

        // 兩份 .md 寫出
        let pub_md = std::fs::read_to_string(summary_public_path(root)).unwrap();
        let int_md = std::fs::read_to_string(summary_internal_path(root)).unwrap();
        assert_eq!(pub_md, "客戶版摘要本體");
        assert_eq!(int_md, "內部版摘要本體");

        // public prompt 不含麥克風內容(守門)
        assert!(!public_msgs[1].content.contains("這是麥克風內容"));
        // 而且 secret 不會原文出現在 public prompt(已 redact)
        assert!(!public_msgs[1].content.contains("gsk_TEST"));

        // audit jsonl 一行 + 不含逐字稿原文 / 被遮字串
        let audit = std::fs::read_to_string(summary_audit_path(root)).unwrap();
        let lines: Vec<&str> = audit.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 1, "audit should have exactly 1 line");
        assert!(!audit.contains("gsk_TEST"), "audit must not contain redacted secret");
        assert!(!audit.contains("這是麥克風內容"), "audit must not contain transcript text");
        assert!(!audit.contains("客戶版摘要本體"), "audit must not contain summary body");
        let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v.get("redaction_count").and_then(|x| x.as_u64()), Some(redaction_count as u64));
        assert!(v.get("ts").is_some());
        assert_eq!(v.get("force_local").and_then(|x| x.as_bool()), Some(true));
    }

    // ── 主流程:逐字稿空 → 寫佔位、不呼叫 LLM ──
    #[test]
    fn empty_transcript_writes_placeholder_no_llm() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("transcript")).unwrap();
        // force_local=true 但 segments 空 → 不會 hit Ollama
        let res = summarize_session_inner(root, true).unwrap();
        assert_eq!(res.public_backend, "none");
        assert_eq!(res.internal_backend, "none");
        let pub_md = std::fs::read_to_string(summary_public_path(root)).unwrap();
        assert!(pub_md.contains("(無逐字稿內容)"));
    }
}
