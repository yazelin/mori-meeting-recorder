//! 摘要服務發現契約 + headless sidecar 的「純請求處理 / 隨需啟動」核心。
//!
//! recorder 既有的雙摘要 pipeline(`summarize::summarize_session_inner`)是純 sync、不依賴 tauri。
//! 本模組把它包成一個**受治理、可被 AgentOS / 其他 client 消費的 HTTP 端點**的可重用核心:
//!   - descriptor `~/.mori/mori-recorder-server.json`(與 whisper-server.json / mori-ear-server.json
//!     並存不衝突)讓 client 發現 sidecar(host=127.0.0.1、port、`/summarize`)。
//!   - `handle_summarize_request`:純函式(可注入 summarize_fn → deterministic 測試,不碰真 LLM),
//!     POST JSON `{session_id, force_local?}` → 讀 `~/.mori/meetings/<id>/` → 跑 pipeline → 寫
//!     `.md` + audit → 回 `SummaryResult` metadata。
//!   - `ensure_server`:隨需把 sidecar 以 **detached 程序**拉起(GUI / `--ensure` 共用)。
//!
//! AgentOS 端走 http-service `mode: json`(平台把 args 整包當 application/json 轉發、回應原樣帶回,
//! 見 agentos `whisper_client::forward_json`)。對應的 sidecar bin 在 `bin/mori-summarize-serve.rs`。
//!
//! 紅線:
//!   - **standalone-first**(硬規矩 #4):沒裝 AgentOS / sidecar 沒起,GUI 內按摘要鈕仍直接走
//!     in-process `summarize_session`,完全不依賴本服務。
//!   - **HTTP listener 必須是獨立 detached 程序**(GUI setup 只能背景 `ensure_server`,絕不把
//!     listener 綁進 GUI 進程,否則 app 按 ✕ 退出後服務同死)。
//!   - **不接受 caller 帶 key**(硬規矩 #2):Groq key 由 pipeline 內部自讀共享 `~/.mori/config.json`,
//!     sidecar 與 recorder 同機共用 home。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 本實作支援到的 descriptor 契約版本上限。讀到更高 → 視為 unusable(對齊 whisper / ear 契約 §8)。
pub const SUPPORTED_CONTRACT_VERSION: u32 = 1;
/// sidecar 對外服務的 inference 路徑(descriptor 也寫這個;AgentOS 用 descriptor.inference_path 組 URL)。
pub const INFERENCE_PATH: &str = "/summarize";

fn default_contract_version() -> u32 {
    1
}
fn default_inference_path() -> String {
    INFERENCE_PATH.to_string()
}

/// sidecar 發現檔 schema(v1)。形狀沿用 whisper-server 契約(host/port 必填、其餘預設),
/// 讓 AgentOS `WhisperDescriptor` 解析器直接讀(它只認 host/port/inference_path/contract_version)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecorderServerDescriptor {
    #[serde(default = "default_contract_version")]
    pub contract_version: u32,
    pub host: String,
    pub port: u16,
    /// informational —— 「哪台 sidecar」而非單一模型(摘要是動態 backend:Groq / Ollama)。
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub pid: u32,
    #[serde(default)]
    pub started_at: String,
    #[serde(default = "default_inference_path")]
    pub inference_path: String,
}

impl RecorderServerDescriptor {
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

/// `~/.mori/mori-recorder-server.json`(與 whisper-server.json / mori-ear-server.json 並存不衝突)。
pub fn descriptor_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("mori-recorder-server.json"))
        .unwrap_or_else(|| PathBuf::from("mori-recorder-server.json"))
}

/// single-instance lockfile 路徑(`~/.mori/mori-recorder-server.lock`)。sidecar 用它擋雙開
/// (兩個 `--ensure` 同時拉起 → 雙 listener / 雙寫 descriptor)。對齊 whisper-serve 的 flock 單例。
pub fn lock_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("mori-recorder-server.lock"))
        .unwrap_or_else(|| PathBuf::from("mori-recorder-server.lock"))
}

/// 解析 descriptor 字串 + 版本天花板(抽純函式好單測)。
fn parse_descriptor_str(s: &str) -> Option<RecorderServerDescriptor> {
    let d: RecorderServerDescriptor = serde_json::from_str(s).ok()?;
    if d.contract_version > SUPPORTED_CONTRACT_VERSION {
        return None;
    }
    Some(d)
}

/// 讀 descriptor。缺檔 / 壞檔 / 版本太新 → None。
pub fn read_descriptor() -> Option<RecorderServerDescriptor> {
    parse_descriptor_str(&std::fs::read_to_string(descriptor_path()).ok()?)
}

/// 原子寫 descriptor(先寫 `.tmp` 再 rename,避免 client 讀到寫一半)。
pub fn write_descriptor(desc: &RecorderServerDescriptor) -> Result<(), String> {
    let path = descriptor_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(desc).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(&tmp, body).map_err(|e| format!("write tmp: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))
}

/// 最佳努力刪 descriptor(sidecar 正常退出時)。client 不可假設一定被刪(crash 不刪)。
pub fn remove_descriptor() {
    let _ = std::fs::remove_file(descriptor_path());
}

/// 已有「在線的」sidecar?(讀 descriptor → loopback pin → `GET /` 200)。
/// `--serve` 用它讓位(已在線就 exit,避免雙寫 descriptor);`ensure_server` 也先驗活再決定拉不拉。
pub fn reachable_server() -> Option<RecorderServerDescriptor> {
    let d = read_descriptor()?;
    if !(d.host == "127.0.0.1" || d.host == "::1") || d.port == 0 {
        return None;
    }
    match ureq::get(&d.base_url())
        .timeout(Duration::from_millis(800))
        .call()
    {
        Ok(r) if r.status() == 200 => Some(d),
        _ => None,
    }
}

// ── 純請求處理(可注入 summarize_fn → deterministic 測試)─────────────────────

/// `POST /summarize` 的 body schema。`force_local` 省略 → 用 server 端預設(config 的
/// `summary_force_local_default`)。**不收任何 key 欄位**(硬規矩 #2)。
#[derive(Debug, Deserialize)]
pub struct SummarizeRequest {
    pub session_id: String,
    #[serde(default)]
    pub force_local: Option<bool>,
}

/// `session_id` 必須是「乾淨的單段目錄名」:非空、不含路徑分隔 / `..` / NUL,且只有單一 Normal component。
/// 防 path traversal —— caller 帶 `../../etc` 之類解析到 `meetings_dir` 之外。
pub fn is_safe_session_id(id: &str) -> bool {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") || id.contains('\0') {
        return false;
    }
    let mut comps = Path::new(id).components();
    matches!(comps.next(), Some(std::path::Component::Normal(_))) && comps.next().is_none()
}

/// 純請求處理:解析 body → 驗 session_id → 解析 force_local → 呼 `summarize_fn` → 回 (status, body)。
/// 成功 200 + `SummaryResult` JSON;壞 JSON / 不安全 session_id = 400;查無 session 目錄 = 404;
/// pipeline 失敗 = 500。**真 LLM 在 `summarize_fn`(production = `summarize_session_inner`),測試可注入 fake**。
pub fn handle_summarize_request<F>(
    body: &str,
    meetings_dir: &Path,
    default_force_local: bool,
    summarize_fn: F,
) -> (u16, String)
where
    F: FnOnce(&Path, bool) -> Result<crate::summarize::SummaryResult, String>,
{
    let req: SummarizeRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return (400, format!("bad request JSON: {e}")),
    };
    if !is_safe_session_id(&req.session_id) {
        return (400, format!("invalid session_id: {:?}", req.session_id));
    }
    let session_root = meetings_dir.join(&req.session_id);
    if !session_root.is_dir() {
        return (404, format!("session not found: {}", req.session_id));
    }
    let force_local = req.force_local.unwrap_or(default_force_local);
    match summarize_fn(&session_root, force_local) {
        Ok(result) => match serde_json::to_string(&result) {
            Ok(s) => (200, s),
            Err(e) => (500, format!("serialize result: {e}")),
        },
        Err(e) => (500, format!("summarize failed: {e}")),
    }
}

// ── sidecar 隨需啟動(headless、detached;GUI 絕不綁 listener)──────────────────

/// sidecar 執行檔名(平台)。
pub fn sidecar_bin_name() -> &'static str {
    #[cfg(windows)]
    {
        "mori-summarize-serve.exe"
    }
    #[cfg(not(windows))]
    {
        "mori-summarize-serve"
    }
}

/// 共用安裝點 `~/.mori/bin/mori-summarize-serve`(跟 whisper-serve 同窩),`--ensure` 從這找。
pub fn shared_sidecar_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("bin").join(sidecar_bin_name()))
        .unwrap_or_else(|| PathBuf::from(sidecar_bin_name()))
}

/// current_exe 旁邊的 sidecar(dev:`cargo`/`tauri dev` 下兩支 bin 同在 `target/<profile>/`)。
/// packaged bundle 目前沒把 sidecar 列為 externalBin → 正式包裝下為 None(同 whisper-serve 的已知限制)。
fn sibling_sidecar_path() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let p = dir.join(sidecar_bin_name());
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// 純複製邏輯(吃明確 src/dst,好單測且明確「只碰 fs、不開 socket」)。
/// per-pid `.tmp` + rename(ETXTBSY-safe,對齊 whisper `install_shared_supervisor`);只在沒種過 /
/// 大小不同 / src 較新時才複製。回 true = 有複製。失敗一律 false(不致命)。
fn seed_sidecar(src: &Path, dst: &Path) -> bool {
    let src_meta = match std::fs::metadata(src) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let need = match std::fs::metadata(dst) {
        Ok(dm) => dm.len() != src_meta.len() || src_meta.modified().ok() > dm.modified().ok(),
        Err(_) => true, // 還沒種過
    };
    if !need {
        return false;
    }
    let Some(parent) = dst.parent() else {
        return false;
    };
    let _ = std::fs::create_dir_all(parent);
    let tmp = dst.with_extension(format!("tmp-install.{}", std::process::id()));
    if std::fs::copy(src, &tmp).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
        }
        if std::fs::rename(&tmp, dst).is_ok() {
            return true;
        }
        let _ = std::fs::remove_file(&tmp);
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
    false
}

/// best-effort 把 sibling sidecar 種進 `~/.mori/bin`,讓 `--ensure` 從任何 context 找得到。
/// **純 fs 操作,絕不開 socket / 綁 listener**(GUI setup 背景呼這個是安全的)。失敗不致命。
pub fn install_shared_sidecar() {
    let Some(sib) = sibling_sidecar_path() else {
        return;
    };
    let _ = seed_sidecar(&sib, &shared_sidecar_path());
}

/// 找可執行的 sidecar:先共用安裝點 `~/.mori/bin`,再退回 current_exe 旁邊(dev)。
pub fn locate_sidecar() -> Option<PathBuf> {
    let shared = shared_sidecar_path();
    if shared.exists() {
        return Some(shared);
    }
    sibling_sidecar_path()
}

/// detached spawn sidecar(裸 serve 模式)。**Linux**:`setsid`(呼叫者關掉不連帶收它)+ close
/// 繼承 fd(防 single-instance socket 洩漏,memory: mori-spawn-close-fds-linux)。**Windows**:
/// `DETACHED_PROCESS`。stdio 全 null、fire-and-forget。
pub fn spawn_sidecar_detached(bin: &Path, idle_secs: u64) -> Result<(), String> {
    let mut cmd = std::process::Command::new(bin);
    cmd.args(["--idle-secs", &idle_secs.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(target_os = "linux")]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setsid();
            crate::whisper_discovery::close_inherited_fds();
            Ok(())
        });
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("spawn {}: {e}", bin.display()))
}

/// 隨需確保 sidecar 在跑(GUI setup 背景呼 / `--ensure` 都走這)。已在線 → 不動。
/// 找不到 / spawn 失敗 → 安靜略過(**standalone-first 不破**:GUI 內按鈕仍直接 in-process summarize)。
pub fn ensure_server(idle_secs: u64) {
    if reachable_server().is_some() {
        return;
    }
    install_shared_sidecar();
    let Some(bin) = locate_sidecar() else {
        eprintln!("[summarize] mori-summarize-serve not found (~/.mori/bin or next to exe); skip autostart");
        return;
    };
    match spawn_sidecar_detached(&bin, idle_secs) {
        Ok(()) => eprintln!("[summarize] ensured sidecar (bin={})", bin.display()),
        Err(e) => eprintln!("[summarize] autostart sidecar failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarize::SummaryResult;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn descriptor_round_trips_and_rejects_newer_version() {
        let json = r#"{"host":"127.0.0.1","port":54321,"model":"mori-meeting-recorder/summarize","pid":42,"started_at":"2026-06-04T10:00:00Z","inference_path":"/summarize"}"#;
        let d = parse_descriptor_str(json).unwrap();
        assert_eq!(d.host, "127.0.0.1");
        assert_eq!(d.port, 54321);
        assert_eq!(d.inference_path, "/summarize");
        assert_eq!(d.base_url(), "http://127.0.0.1:54321");
        // 缺 contract_version / inference_path → 預設(前向相容)。
        let d2 = parse_descriptor_str(r#"{"host":"127.0.0.1","port":7}"#).unwrap();
        assert_eq!(d2.contract_version, 1);
        assert_eq!(d2.inference_path, "/summarize");
        // 比支援上限新 → unusable。
        assert!(parse_descriptor_str(r#"{"contract_version":2,"host":"127.0.0.1","port":7}"#).is_none());
        assert!(parse_descriptor_str("{ not json").is_none());
    }

    #[test]
    fn descriptor_serializes_with_keys_agentos_reads() {
        // 跨 repo interop(acceptance item 2):AgentOS WhisperDescriptor 讀 host/port/inference_path/
        // contract_version。serve() 一律廣告 loopback host、/summarize、contract_version 1 —— 鎖住
        // 這些序列化鍵,任一端漂移即抓(對應 agentos 端 whisper_descriptor_parses_recorder_sidecar_shape)。
        let d = RecorderServerDescriptor {
            contract_version: 1,
            host: "127.0.0.1".into(),
            port: 51234,
            model: "mori-meeting-recorder/summarize".into(),
            pid: 4242,
            started_at: "2026-06-04T10:00:00Z".into(),
            inference_path: INFERENCE_PATH.into(),
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["host"], "127.0.0.1");
        assert_eq!(v["port"], 51234);
        assert_eq!(v["inference_path"], "/summarize");
        assert_eq!(v["contract_version"], 1);
        // 也能 round-trip 回我們自己的 parser。
        assert_eq!(parse_descriptor_str(&serde_json::to_string(&d).unwrap()).unwrap(), d);
    }

    #[test]
    fn seed_sidecar_copies_when_absent_skips_when_unchanged() {
        // install 只碰 fs、不開 socket(對應「GUI setup 只 install、不綁 listener」紅線的可測面)。
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src-bin");
        std::fs::write(&src, b"FAKEBINARY").unwrap();
        let dst = tmp.path().join("bin").join("mori-summarize-serve");
        // dst 不存在 → 複製。
        assert!(seed_sidecar(&src, &dst));
        assert_eq!(std::fs::read(&dst).unwrap(), b"FAKEBINARY");
        // 同大小、src 不比 dst 新 → 不再複製。
        assert!(!seed_sidecar(&src, &dst));
        // src 內容變大 → 重新複製。
        std::fs::write(&src, b"FAKEBINARY-BIGGER").unwrap();
        assert!(seed_sidecar(&src, &dst));
        assert_eq!(std::fs::read(&dst).unwrap(), b"FAKEBINARY-BIGGER");
    }

    #[test]
    fn safe_session_id_accepts_meeting_ids_rejects_traversal() {
        assert!(is_safe_session_id("meeting-20260604-101010"));
        assert!(is_safe_session_id("meeting-x"));
        // path traversal / 分隔符 / 絕對路徑 / 空 → 一律擋。
        assert!(!is_safe_session_id("../etc/passwd"));
        assert!(!is_safe_session_id("a/b"));
        assert!(!is_safe_session_id("a\\b"));
        assert!(!is_safe_session_id(".."));
        assert!(!is_safe_session_id("."));
        assert!(!is_safe_session_id("/abs"));
        assert!(!is_safe_session_id(""));
        assert!(!is_safe_session_id("with\0nul"));
    }

    #[test]
    fn handle_rejects_bad_json_without_calling_pipeline() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let (status, _) = handle_summarize_request("{ not json", Path::new("/tmp"), false, |_, _| {
            c.fetch_add(1, Ordering::SeqCst);
            Err("should not run".into())
        });
        assert_eq!(status, 400);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "壞 JSON 不該呼到 pipeline");
    }

    #[test]
    fn handle_rejects_unsafe_session_id_without_calling_pipeline() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let tmp = tempfile::tempdir().unwrap();
        let (status, _) = handle_summarize_request(
            r#"{"session_id":"../../etc"}"#,
            tmp.path(),
            false,
            |_, _| {
                c.fetch_add(1, Ordering::SeqCst);
                Err("should not run".into())
            },
        );
        assert_eq!(status, 400);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "path traversal 不該呼到 pipeline");
    }

    #[test]
    fn handle_returns_404_when_session_dir_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let (status, _) = handle_summarize_request(
            r#"{"session_id":"meeting-nope"}"#,
            tmp.path(),
            false,
            |_, _| panic!("不該呼到 pipeline"),
        );
        assert_eq!(status, 404);
    }

    #[test]
    fn handle_happy_path_runs_pipeline_writes_files_and_returns_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let session_root = tmp.path().join("meeting-x");
        std::fs::create_dir_all(&session_root).unwrap();

        // fake pipeline:寫出真 pipeline 會寫的三個檔(.md ×2 + audit),回 metadata。
        // 證「dispatch → handler → 真 side-effect 落地」這條(對齊 issue 驗收的檔案 side-effect 斷言)。
        let (status, body) = handle_summarize_request(
            r#"{"session_id":"meeting-x","force_local":true}"#,
            tmp.path(),
            false, // server 預設 false,但 body 帶 true → 應以 body 為準
            |root, force_local| {
                assert_eq!(force_local, true, "body 的 force_local 應覆寫 server 預設");
                std::fs::write(root.join("meeting.summary.public.md"), "公開版").unwrap();
                std::fs::write(root.join("meeting.summary.internal.md"), "內部版").unwrap();
                std::fs::write(root.join("summary.audit.jsonl"), "{}\n").unwrap();
                Ok(SummaryResult {
                    public_backend: "groq".into(),
                    internal_backend: "ollama".into(),
                    public_chars: 3,
                    internal_chars: 3,
                    redaction_count: 0,
                })
            },
        );
        assert_eq!(status, 200);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["public_backend"], "groq");
        assert_eq!(v["internal_backend"], "ollama");
        // 三個檔案 side-effect 都落地。
        assert!(session_root.join("meeting.summary.public.md").is_file());
        assert!(session_root.join("meeting.summary.internal.md").is_file());
        assert!(session_root.join("summary.audit.jsonl").is_file());
    }

    #[test]
    fn handle_force_local_defaults_to_server_setting_when_omitted() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("meeting-x")).unwrap();
        let (status, _) = handle_summarize_request(
            r#"{"session_id":"meeting-x"}"#, // 不帶 force_local
            tmp.path(),
            true, // server 預設 true → 應套用
            |_, force_local| {
                assert_eq!(force_local, true, "省略時應套 server 預設");
                Ok(SummaryResult {
                    public_backend: "ollama".into(),
                    internal_backend: "ollama".into(),
                    public_chars: 0,
                    internal_chars: 0,
                    redaction_count: 0,
                })
            },
        );
        assert_eq!(status, 200);
    }
}
