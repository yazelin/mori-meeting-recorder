//! mori-summarize-serve — recorder 雙摘要 pipeline 的 **headless HTTP sidecar**。
//!
//! recorder 是使用者手動開/關的 GUI app(按 ✕ 真正 `app.exit(0)`),不是 always-on daemon。
//! 一旦 descriptor 指向某 port,AgentOS 在 recorder 關閉時 dispatch 會打到死 port —— 所以把摘要
//! 能力抽成這支**獨立、隨需啟動、閒置自關的 detached sidecar**:GUI 與 AgentOS 都當 client。
//!   - GUI 內按摘要鈕仍直接 in-process `summarize_session`(standalone-first,完全不依賴本服務)。
//!   - AgentOS 經 http-service `mode: json` dispatch → 平台 forward `/summarize` → 本服務跑同一條
//!     pipeline、寫 `.md` + audit、回 `SummaryResult` metadata。
//!
//! 端點(descriptor `~/.mori/mori-recorder-server.json` 廣告 host=127.0.0.1 / port / `/summarize`):
//!   GET  /  ·  /health   → 200「mori-recorder ok」(ready gate,client 驗活用)
//!   POST /summarize      → JSON `{session_id, force_local?}` → 跑摘要 → 200 `SummaryResult` JSON
//!                          /  壞輸入 400 · 查無 session 404 · pipeline 失敗 500(body 純文字)
//!
//! 對齊既有 `mori-whisper-serve.rs`:**`#[path]` 直接 include tauri-free 子圖**(不連 tauri/webview),
//! 閒置 TTL 用共用常數 `whisper_discovery::DEFAULT_IDLE_SECS`(別各寫各的 600)。

// tauri-free 子圖(全鏈零 tauri,已逐檔查證)。本 bin 不會用到每個 pub fn → allow dead_code。
#[allow(dead_code)]
#[path = "../audio/mod.rs"]
mod audio;
#[allow(dead_code)]
#[path = "../config.rs"]
mod config;
#[allow(dead_code)]
#[path = "../diarize.rs"]
mod diarize;
#[allow(dead_code)]
#[path = "../exporter.rs"]
mod exporter;
#[allow(dead_code)]
#[path = "../transcribe.rs"]
mod transcribe;
#[allow(dead_code)]
#[path = "../voiceprint.rs"]
mod voiceprint;
#[allow(dead_code)]
#[path = "../postprocess.rs"]
mod postprocess;
#[allow(dead_code)]
#[path = "../session_store.rs"]
mod session_store;
#[allow(dead_code)]
#[path = "../summarize.rs"]
mod summarize;
#[allow(dead_code)]
#[path = "../whisper_discovery.rs"]
mod whisper_discovery;
#[allow(dead_code)]
#[path = "../summarize_service.rs"]
mod summarize_service;

use std::time::{Duration, Instant};
use tiny_http::{Method, Request, Response, Server};

const DEFAULT_IDLE_SECS: u64 = whisper_discovery::DEFAULT_IDLE_SECS; // 共用單一事實來源(10 分鐘)
const REAP_CHECK_SECS: u64 = 15; // 每 15s 醒一次檢查閒置

struct Args {
    idle_secs: u64,
    port: Option<u16>,
    stop: bool,
    ensure: bool,
    help: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        idle_secs: DEFAULT_IDLE_SECS,
        port: None,
        stop: false,
        ensure: false,
        help: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--idle-secs" => a.idle_secs = it.next().and_then(|s| s.parse().ok()).unwrap_or(a.idle_secs),
            "--port" => a.port = it.next().and_then(|s| s.parse().ok()),
            "--stop" => a.stop = true,
            "--ensure" => a.ensure = true,
            "-h" | "--help" => a.help = true,
            other => eprintln!("[summarize-serve] ignoring unknown arg: {other}"),
        }
    }
    a
}

// 故意**不提供 --host**:平台 client 只連 loopback,descriptor 也只廣告 127.0.0.1。一律 bind loopback,
// 避免「bind 公開介面但 descriptor 謊稱 loopback」的 split-brain(review L1 defense-in-depth)。
const BIND_HOST: &str = "127.0.0.1";

const HELP: &str = "mori-summarize-serve — recorder 雙摘要 pipeline 的 headless HTTP sidecar
  --idle-secs <N>   閒置幾秒沒請求就自關(預設 600)
  --port <PORT>     指定埠(預設自動選 ephemeral;一律 bind 127.0.0.1)
  --ensure          確保 sidecar 在跑(沒有就背景拉起),冪等 + 馬上返回。
                    AgentOS dispatch 前喚醒本地摘要服務用這個。
  --stop            停掉目前在跑的 sidecar
  -h, --help        這個說明";

fn main() {
    let args = parse_args();
    if args.help {
        println!("{HELP}");
        return;
    }
    if args.stop {
        do_stop();
        return;
    }
    if args.ensure {
        do_ensure(&args);
        return;
    }

    // idempotent:已有「驗活過」的 sidecar → 不重起(serve 重複跑安全,避免雙寫 descriptor)。
    if let Some(d) = summarize_service::reachable_server() {
        eprintln!("[summarize-serve] already running at {}; nothing to do", d.base_url());
        return;
    }

    serve(&args);
}

/// `--ensure`:語言無關的「確保 server 在跑」入口(冪等 + 非阻塞)。有活的 → 立刻回;
/// 沒有 → 把自己以裸 serve 模式 detached 重啟、馬上返回。連打安全(下一個 serve 會先驗活讓位)。
fn do_ensure(args: &Args) {
    if let Some(d) = summarize_service::reachable_server() {
        eprintln!("[summarize-serve] already running at {}; nothing to do", d.base_url());
        return;
    }
    summarize_service::install_shared_sidecar();
    let bin = match summarize_service::locate_sidecar().or_else(|| std::env::current_exe().ok()) {
        Some(p) => p,
        None => {
            eprintln!("[summarize-serve] --ensure: 找不到 sidecar binary(locate + current_exe 都失敗)");
            std::process::exit(1);
        }
    };
    match summarize_service::spawn_sidecar_detached(&bin, args.idle_secs) {
        Ok(()) => eprintln!("[summarize-serve] ensured: detached sidecar spawned (idle-secs={}, bin={})", args.idle_secs, bin.display()),
        Err(e) => {
            eprintln!("[summarize-serve] --ensure spawn failed: {e}");
            std::process::exit(1);
        }
    }
}

/// `--stop`:讀 descriptor 殺 sidecar、刪 descriptor。最佳努力。
fn do_stop() {
    if let Some(d) = summarize_service::read_descriptor() {
        kill_pid(d.pid);
        eprintln!("[summarize-serve] sent stop to pid {}", d.pid);
    }
    summarize_service::remove_descriptor();
}

fn kill_pid(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
}

/// 搶單例 lock(`~/.mori/mori-recorder-server.lock`)—— **pin 成 advisory flock**:Unix =
/// `flock(LOCK_EX|LOCK_NB)`、Windows = `share_mode(0)` 獨占開檔;持有的 File 留到 process 死才釋放
/// (crash-safe,核心自動放鎖)。**lockfile 永不刪**(刪了換 inode 會讓兩 starter 各鎖各的 → 雙開)。
/// 回 held File;Err = 別人持鎖(我們不是 owner,別 spawn / 寫 descriptor)。對齊 mori-whisper-serve。
fn acquire_lock() -> Result<std::fs::File, String> {
    let path = summarize_service::lock_path();
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false) // lock 內容只是診斷用 pid,不截斷(行為同原本無 truncate 的預設)
            .share_mode(0) // 獨占:別的 starter 同樣 share_mode(0) 開會失敗 = 鎖
            .open(&path)
            .map_err(|_| "another mori-summarize-serve holds the lock (windows exclusive)".to_string())?
    };
    #[cfg(not(windows))]
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false) // 同上:lock 內容不截斷(預設行為,顯式標明以過 clippy)
        .open(&path)
        .map_err(|e| format!("open lock: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            return Err("another mori-summarize-serve holds the flock".into());
        }
    }
    use std::io::Write;
    let _ = (&file).write_all(std::process::id().to_string().as_bytes());
    Ok(file)
}

/// 搶單例 lock → bind loopback → 寫 descriptor → serve loop(`recv_timeout` 每 REAP_CHECK 醒一次檢查
/// 閒置 TTL)→ 收尾刪 descriptor。**只有 lock owner 才往下 bind+寫 descriptor**(兩個 `--ensure`
/// 同時拉起時不會雙開 listener / 雙寫 descriptor)。
fn serve(args: &Args) {
    // 持到 serve 結束才釋放(flock / 獨占 handle)。搶不到 = 別人正在起/在跑 → 安靜退(exit 0,非錯誤)。
    let _lock = match acquire_lock() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[summarize-serve] not the owner ({e}); another instance is starting/running — exiting");
            std::process::exit(0);
        }
    };

    let bind = format!("{BIND_HOST}:{}", args.port.unwrap_or(0));
    let server = match Server::http(&bind) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[summarize-serve] bind {bind} failed: {e}");
            std::process::exit(1);
        }
    };
    let bound_port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(0);
    if bound_port == 0 {
        eprintln!("[summarize-serve] no bound port");
        std::process::exit(1);
    }

    let desc = summarize_service::RecorderServerDescriptor {
        contract_version: 1,
        // descriptor 一律廣告 loopback(平台 client 只連 127.0.0.1/::1;即使 --host 給別的也不對外廣告)。
        host: "127.0.0.1".to_string(),
        port: bound_port,
        model: "mori-meeting-recorder/summarize".to_string(),
        pid: std::process::id(),
        started_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        inference_path: summarize_service::INFERENCE_PATH.to_string(),
    };
    if let Err(e) = summarize_service::write_descriptor(&desc) {
        eprintln!("[summarize-serve] write descriptor: {e}");
        std::process::exit(1);
    }
    eprintln!(
        "[summarize-serve] ready at http://127.0.0.1:{bound_port}{} (pid={}, idle-secs={})",
        summarize_service::INFERENCE_PATH,
        desc.pid,
        args.idle_secs
    );

    // meetings_dir 是 home-relative 常數(serve 期間不變)→ 算一次。force_local 預設改為**每請求**
    // 讀 config(review L4:serve 長命時 config 編輯應即時生效,不要 snapshot 整個生命週期)。
    let meetings_dir = session_store::default_meetings_dir();

    let mut last_activity = Instant::now();
    loop {
        match server.recv_timeout(Duration::from_secs(REAP_CHECK_SECS)) {
            Ok(Some(req)) => {
                last_activity = Instant::now();
                handle(req, &meetings_dir);
            }
            Ok(None) => {
                if last_activity.elapsed() >= Duration::from_secs(args.idle_secs) {
                    eprintln!("[summarize-serve] idle {}s — reaping", args.idle_secs);
                    break;
                }
            }
            Err(e) => {
                eprintln!("[summarize-serve] recv error: {e}");
                break;
            }
        }
    }

    summarize_service::remove_descriptor();
    eprintln!("[summarize-serve] stopped, removed descriptor");
}

fn handle(req: Request, meetings_dir: &std::path::Path) {
    let method = req.method().clone();
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or(&url).to_string();
    match (&method, path.as_str()) {
        (Method::Get, "/") | (Method::Get, "/health") => {
            let _ = req.respond(Response::from_string("mori-recorder ok"));
        }
        (Method::Post, p) if p == summarize_service::INFERENCE_PATH => {
            respond_summarize(req, meetings_dir);
        }
        _ => {
            let _ = req.respond(Response::from_string("not found").with_status_code(404));
        }
    }
}

/// 讀 body → 純 handler(真 pipeline = `summarize_session_inner`)→ 單一回應點。
/// force_local 預設每請求讀 config(L4);pipeline panic 用 catch_unwind 包住,回 500、**不拖垮整個
/// serve loop**(L3:一個壞請求不該讓 sidecar 整個死掉、留下指向死 port 的 stale descriptor)。
fn respond_summarize(mut req: Request, meetings_dir: &std::path::Path) {
    let mut body = String::new();
    if req.as_reader().read_to_string(&mut body).is_err() {
        let _ = req.respond(Response::from_string("read body failed").with_status_code(400));
        return;
    }
    let default_force_local = config::read_config().summary_force_local_default;
    let (status, out) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        summarize_service::handle_summarize_request(
            &body,
            meetings_dir,
            default_force_local,
            summarize::summarize_session_inner,
        )
    }))
    .unwrap_or_else(|_| (500, "summarize pipeline panicked".to_string()));
    let resp = if status == 200 {
        Response::from_string(out)
            .with_status_code(200)
            .with_header(json_header())
    } else {
        Response::from_string(out).with_status_code(status)
    };
    let _ = req.respond(resp);
}

fn json_header() -> tiny_http::Header {
    tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("static header always valid")
}
