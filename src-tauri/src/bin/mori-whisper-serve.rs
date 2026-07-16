//! mori-whisper-serve — 共享 whisper-server 的 supervisor / idle-reaper。
//!
//! whisper.cpp 的 whisper-server 本身沒有 idle-exit(已驗 `--help`),所以包一層 supervisor:
//!   1. 搶單例 lock(`~/.mori/whisper-server.lock`,O_EXCL;stale lock 偵測 pid 死就接管)
//!   2. 選一個空閒 ephemeral port
//!   3. 起 whisper-server(child),Linux 設 PR_SET_PDEATHSIG → supervisor 被殺連帶收掉 child
//!   4. 等 `GET / → 200`(模型載完的 ready 訊號)
//!   5. 原子寫 descriptor(`~/.mori/whisper-server.json`,pid = whisper-server child)
//!   6. 看 child stdout/stderr 的 `processing '` 行(每筆 /inference 都會印)當活動訊號
//!   7. 閒置(無 /inference)超過 --idle-secs 就 SIGTERM child、刪 descriptor、放 lock、退出
//!
//! recorder 會把它 detached spawn(於是 supervisor 比 recorder 活得久,沒人用才自己關);
//! 也可手動當 serve-script 跑。對齊 agentos-notebook/05-mori-migration/whisper-server-contract.md。

// 直接 include 共享模組(不連 tauri)。整模組共用,本 bin 不會用到每個 pub fn → allow dead_code。
#[allow(dead_code)]
#[path = "../whisper_discovery.rs"]
mod whisper_discovery;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use whisper_discovery::WhisperServerDescriptor;

const DEFAULT_IDLE_SECS: u64 = whisper_discovery::DEFAULT_IDLE_SECS; // 共用單一事實來源(10 分鐘),可 --idle-secs 調
const READY_TIMEOUT_SECS: u64 = 90; // large-v3-turbo 載入較久,給足
const REAP_CHECK_SECS: u64 = 15; // 每 15s 檢查一次閒置 / child 是否還在

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

struct Args {
    model: String,
    idle_secs: u64,
    host: String,
    port: Option<u16>,
    threads: u32,
    prompt_file: Option<String>,
    stop: bool,
    ensure: bool,
    help: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        model: "small".to_string(),
        idle_secs: DEFAULT_IDLE_SECS,
        host: "127.0.0.1".to_string(),
        port: None,
        threads: 4,
        prompt_file: None,
        stop: false,
        ensure: false,
        help: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--model" => a.model = it.next().unwrap_or(a.model),
            "--idle-secs" => {
                a.idle_secs = it
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(a.idle_secs)
            }
            "--host" => a.host = it.next().unwrap_or(a.host),
            "--port" => a.port = it.next().and_then(|s| s.parse().ok()),
            "--threads" => a.threads = it.next().and_then(|s| s.parse().ok()).unwrap_or(a.threads),
            "--prompt-file" => a.prompt_file = it.next(),
            "--stop" => a.stop = true,
            "--ensure" => a.ensure = true,
            "-h" | "--help" => a.help = true,
            other => eprintln!("[whisper-serve] ignoring unknown arg: {other}"),
        }
    }
    a
}

fn expand_tilde(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        return dirs::home_dir().unwrap_or_default().join(rest);
    }
    std::path::PathBuf::from(path)
}

fn read_prompt_file(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn initial_prompt(args: &Args) -> Option<String> {
    args.prompt_file
        .as_deref()
        .and_then(|p| read_prompt_file(&expand_tilde(p)))
        .or_else(|| whisper_discovery::stt_initial_prompt(None))
}

fn home_join(parts: &[&str]) -> std::path::PathBuf {
    let mut p = dirs::home_dir().unwrap_or_default();
    for part in parts {
        p = p.join(part);
    }
    p
}

fn model_path(model: &str) -> std::path::PathBuf {
    home_join(&[".mori", "models", &format!("ggml-{model}.bin")])
}

fn whisper_server_bin() -> std::path::PathBuf {
    #[cfg(windows)]
    let name = "whisper-server.exe";
    #[cfg(not(windows))]
    let name = "whisper-server";
    home_join(&[".mori", "bin", name])
}

/// 選一個空閒 ephemeral port:bind :0 取得後立刻放掉(localhost 短暫 TOCTOU 可接受)。
fn free_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
}

/// 搶單例 lock —— **pin 成 advisory flock**(契約 §8 HIGH):Unix = `flock(LOCK_EX|LOCK_NB)`、
/// Windows = `share_mode(0)` 獨占開檔;**持有的 File 留到 process 死才釋放**(crash-safe,核心自動放鎖,
/// 不留 stale lockfile)。**lockfile 永不刪**(刪了換 inode 會讓兩 starter 各鎖各的 → 雙開爆 VRAM)。
/// 回傳 held File;Err = 別人持鎖(我們不是 owner,別 spawn)。
fn acquire_lock() -> Result<std::fs::File, String> {
    let path = whisper_discovery::lock_path();
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
            .map_err(|_| {
                "another mori-whisper-serve holds the lock (windows exclusive)".to_string()
            })?
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
            return Err("another mori-whisper-serve holds the flock".into());
        }
    }

    // pid 寫進去純診斷(鎖本身靠 flock / share_mode,不靠檔案內容)。&File 實作 Write。
    let _ = (&file).write_all(std::process::id().to_string().as_bytes());
    Ok(file)
}

/// 等 `GET base/` 回 200(模型載完的 ready 訊號)。child 中途自己掛了(如 GPU OOM、埠被搶)
/// 就提早 bail,別傻等滿 timeout。
fn wait_ready(child: &mut Child, base_url: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(Some(status)) = child.try_wait() {
            eprintln!("[whisper-serve] whisper-server exited during startup: {status}");
            return false;
        }
        let ok = ureq::get(base_url)
            .timeout(Duration::from_millis(800))
            .call()
            .map(|r| r.status() == 200)
            .unwrap_or(false);
        if ok {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

/// 起 whisper-server child。Linux 設 PR_SET_PDEATHSIG(supervisor 死 → child 收 SIGTERM)。
fn spawn_server(args: &Args, port: u16) -> Result<Child, String> {
    let bin = whisper_server_bin();
    let model = model_path(&args.model);
    if !bin.exists() {
        return Err(format!("whisper-server not found: {}", bin.display()));
    }
    if !model.exists() {
        return Err(format!("model not found: {}", model.display()));
    }
    let mut cmd = Command::new(&bin);
    let initial_prompt = initial_prompt(args);
    cmd.args([
        "-m",
        &model.to_string_lossy(),
        "--host",
        &args.host,
        "--port",
        &port.to_string(),
        "--inference-path",
        "/inference",
        "-t",
        &args.threads.to_string(),
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    if let Some(prompt) = initial_prompt.as_deref() {
        cmd.arg("--prompt").arg(prompt);
        eprintln!(
            "[whisper-serve] loaded STT initial prompt ({} chars)",
            prompt.chars().count()
        );
    }

    #[cfg(target_os = "linux")]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            // child 在 supervisor 先死時收到 SIGTERM,避免變孤兒繼續佔 VRAM。
            libc::prctl(
                libc::PR_SET_PDEATHSIG,
                libc::SIGTERM as libc::c_ulong,
                0,
                0,
                0,
            );
            // 關掉繼承來的 fd(尤其 supervisor 持的 flock)→ whisper-server 不該守住單例鎖
            // (memory: mori-spawn-close-fds-linux)。stdout/stderr 已 dup2 到 1/2,關 ≥3 不影響活動監看。
            whisper_discovery::close_inherited_fds();
            Ok(())
        });
    }

    whisper_discovery::hide_console(&mut cmd); // Windows: 不要閃 console 黑窗
    cmd.spawn()
        .map_err(|e| format!("spawn whisper-server: {e}"))
}

/// 一行 whisper-server log 是否代表「有一筆 /inference 進來」(活動訊號)。
/// whisper-server 每筆推論都印 `operator(): processing '<file>' (N samples, ...)`(已對活的 server 驗過格式)。
/// 模型載入 / backend init 那些行不含 `processing '`,所以不會誤判成活動。
fn is_activity_line(line: &str) -> bool {
    line.contains("processing '")
}

/// 看一個 stream(stdout/stderr),每行掃活動訊號 → 更新 last_activity;同時轉印(方便看 log)。
fn watch_stream<R: std::io::Read + Send + 'static>(
    stream: R,
    tag: &'static str,
    last_activity: Arc<AtomicU64>,
) {
    std::thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines().map_while(Result::ok) {
            if is_activity_line(&line) {
                last_activity.store(now_unix(), Ordering::Relaxed);
            }
            eprintln!("[whisper-server/{tag}] {line}");
        }
    });
}

/// --stop:讀 descriptor 殺 whisper-server、讀 lock 殺 supervisor,刪兩個檔。最佳努力。
fn do_stop() {
    if let Some(desc) = whisper_discovery::read_descriptor() {
        kill_pid(desc.pid);
        eprintln!(
            "[whisper-serve] sent stop to whisper-server pid {}",
            desc.pid
        );
    }
    if let Ok(s) = std::fs::read_to_string(whisper_discovery::lock_path()) {
        if let Ok(pid) = s.trim().parse::<u32>() {
            kill_pid(pid);
        }
    }
    whisper_discovery::remove_descriptor();
    // 不刪 lockfile:被殺的 supervisor 行程死亡時 flock/handle 自動釋放,lockfile 留著當下次的鎖對象。
}

/// `--ensure`:語言無關的「確保 server 在跑」入口(契約 §11 Activation,冪等 + 非阻塞)。
/// 有活的 server → 立刻回;沒有 → 把**自己**以背景化 supervise 模式重啟、馬上 exit 0。
/// 於是 python(mori-ear)/ shell / 資料 app 只要一行 `mori-whisper-serve --ensure`,
/// 不用自己處理「丟背景」就能喚醒共享 server。連打安全(supervise 子程序靠 flock 單例,不會雙開)。
fn do_ensure(args: &Args) {
    if let Some(desc) = whisper_discovery::reachable_server() {
        eprintln!(
            "[whisper-serve] already running at {} (model={}); nothing to do",
            desc.base_url(),
            desc.model
        );
        return;
    }
    // 對齊 ensure_server:best-effort 把自己種進 ~/.mori/bin,再從**共用點**起(讓往後任何 app 都找得到)。
    // 若 current_exe 就是共用點則 install no-op;找不到共用點就退回 current_exe。
    whisper_discovery::install_shared_supervisor();
    let bin = match whisper_discovery::locate_supervisor().or_else(|| std::env::current_exe().ok())
    {
        Some(p) => p,
        None => {
            eprintln!(
                "[whisper-serve] --ensure: 找不到 supervisor binary(locate + current_exe 都失敗)"
            );
            std::process::exit(1);
        }
    };
    // 背景化 spawn 的是「裸 supervise 模式」(無 --ensure)→ 不會無限自我 re-ensure。
    match whisper_discovery::spawn_supervisor_detached(
        &bin,
        &args.model,
        args.idle_secs,
        args.prompt_file.as_deref(),
    ) {
        Ok(()) => eprintln!(
            "[whisper-serve] ensured: detached supervisor spawned (model={}, idle-secs={}, bin={})",
            args.model,
            args.idle_secs,
            bin.display()
        ),
        Err(e) => {
            eprintln!("[whisper-serve] --ensure spawn failed: {e}");
            std::process::exit(1);
        }
    }
}

fn kill_pid(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
}

/// 收尾:殺 child + 刪 descriptor。**不刪 lockfile** —— flock/獨占 handle 在本 process 結束時
/// (held File drop / 行程死)自動釋放;lockfile 留著當下次 flock 的對象。
fn cleanup(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
    whisper_discovery::remove_descriptor();
}

const HELP: &str = "mori-whisper-serve — 共享 whisper-server supervisor(idle 自關)
  --model <small|large-v3-turbo>   要載入的模型(預設 small)
  --idle-secs <N>                  閒置幾秒沒 /inference 就自關(預設 600)
  --host <HOST>                    預設 127.0.0.1
  --port <PORT>                    指定埠(預設自動選空閒埠)
  --threads <N>                    whisper-server 執行緒(預設 4)
  --prompt-file <PATH>             STT initial prompt 檔;省略則讀 ~/.mori/stt/initial-prompt.md
  --ensure                         確保共享 server 在跑(沒有就背景拉起),冪等 + 馬上返回。
                                   任何 app / 腳本喚醒本地 whisper 用這個。
  --stop                           停掉目前在跑的共享 server
  -h, --help                       這個說明";

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
        // 語言無關的隨需喚醒:確保有 server,馬上返回(背景化 supervise 由 do_ensure spawn)。
        do_ensure(&args);
        return;
    }

    // 已有「驗活過」的共享 server → idempotent:不重起,直接結束(serve-script 重複跑安全)。
    if let Some(desc) = whisper_discovery::reachable_server() {
        eprintln!(
            "[whisper-serve] already running at {} (model={}); nothing to do",
            desc.base_url(),
            desc.model
        );
        return;
    }

    // 持到 main 結束才釋放(flock / 獨占 handle)。搶不到 = 別人是 owner → 我們不 spawn,安靜退出
    // (exit 0:非錯誤;recorder 端會 adopt 既有 server 或 fallback cli)。**只有 owner 才往下 spawn+寫 descriptor**。
    let _lock = match acquire_lock() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[whisper-serve] not the owner ({e}); another instance is starting/running — exiting");
            std::process::exit(0);
        }
    };

    let port = args.port.or_else(free_port).unwrap_or(0);
    if port == 0 {
        eprintln!("[whisper-serve] no free port");
        std::process::exit(1); // _lock drop → flock 釋放
    }

    let mut child = match spawn_server(&args, port) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[whisper-serve] {e}");
            std::process::exit(1);
        }
    };

    let last_activity = Arc::new(AtomicU64::new(now_unix()));
    if let Some(out) = child.stdout.take() {
        watch_stream(out, "out", last_activity.clone());
    }
    if let Some(err) = child.stderr.take() {
        watch_stream(err, "err", last_activity.clone());
    }

    let base_url = format!("http://{}:{}", args.host, port);
    if !wait_ready(
        &mut child,
        &base_url,
        Duration::from_secs(READY_TIMEOUT_SECS),
    ) {
        eprintln!("[whisper-serve] server not ready within {READY_TIMEOUT_SECS}s; aborting");
        cleanup(&mut child);
        std::process::exit(1);
    }

    let desc = WhisperServerDescriptor {
        contract_version: 1,
        host: args.host.clone(),
        port,
        model: args.model.clone(),
        pid: child.id(),
        // RFC3339 UTC 尾綴 Z(契約 §1/§8):owner 推 staleness 才不會 mis-parse offset。
        started_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        inference_path: "/inference".to_string(),
    };
    if let Err(e) = whisper_discovery::write_descriptor(&desc) {
        eprintln!("[whisper-serve] write descriptor: {e}");
        cleanup(&mut child);
        std::process::exit(1);
    }
    eprintln!(
        "[whisper-serve] ready at {} (model={}, pid={}, idle-secs={})",
        base_url, args.model, desc.pid, args.idle_secs
    );

    // 主迴圈:child 自己掛了 → 退出;閒置超過 TTL → 收掉。
    loop {
        std::thread::sleep(Duration::from_secs(REAP_CHECK_SECS));
        match child.try_wait() {
            Ok(Some(status)) => {
                eprintln!("[whisper-serve] whisper-server exited on its own: {status}");
                break;
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("[whisper-serve] try_wait error: {e}");
                break;
            }
        }
        let idle = now_unix().saturating_sub(last_activity.load(Ordering::Relaxed));
        if idle >= args.idle_secs {
            eprintln!(
                "[whisper-serve] idle {idle}s >= {}s — reaping",
                args.idle_secs
            );
            break;
        }
    }

    cleanup(&mut child);
    eprintln!("[whisper-serve] stopped, cleaned up descriptor + lock");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_line_matches_real_inference_log_only() {
        // 真實 whisper-server 每筆 /inference 的 log(已對活 server 抓過格式)→ 算活動
        assert!(is_activity_line(
            "operator(): processing 'clip.wav' (16000 samples, 1.0 sec), 4 threads, 1 processors, lang = zh, task = transcribe"
        ));
        // 載入 / backend / 系統資訊那些行 → 不算活動(否則啟動瞬間就被當「有人用」永遠不閒置)
        assert!(!is_activity_line(
            "whisper_init_state: kv self size = 18.87 MB"
        ));
        assert!(!is_activity_line(
            "system_info: n_threads = 4 / 16 | WHISPER : COREML = 0 | CUDA : ARCHS = 890"
        ));
        assert!(!is_activity_line("ggml_cuda_init: found 1 CUDA devices"));
        assert!(!is_activity_line(""));
    }

    #[test]
    fn model_path_and_server_bin_under_mori() {
        let mp = model_path("large-v3-turbo");
        assert!(mp.ends_with("ggml-large-v3-turbo.bin"));
        assert!(mp.to_string_lossy().contains(".mori"));
        assert!(whisper_server_bin().to_string_lossy().contains(".mori"));
    }
}
