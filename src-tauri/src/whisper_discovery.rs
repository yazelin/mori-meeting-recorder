//! Whisper-Server 共享發現契約 v1。
//! 對齊 agentos-notebook/05-mori-migration/whisper-server-contract.md(跨 repo 單一事實來源)。
//!
//! consumer 讀 `~/.mori/whisper-server.json` 找本地共享 server,**先驗活**(pid + `GET /` → 200)
//! 才信(§3.1)。`GET /` 是 whisper.cpp 的 ready 訊號:模型載完才回 200,不只是 listening。
//! descriptor 的 `model` 是「哪個模型在跑」的唯一來源(server 無查詢端點)。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 本實作支援到的契約版本上限(§6 / §8)。讀到 `contract_version` 比這大的 descriptor
/// MUST 視為 UNUSABLE(當作沒有 server,fall through 到 spawn / cli),**不可當舊版硬吃**。
pub const SUPPORTED_CONTRACT_VERSION: u32 = 1;

/// 共用閒置 TTL:supervisor 閒置(無 `/inference`)超過這秒數就自關(契約 §11 Activation)。
/// 單一事實來源 —— supervisor、`ensure_server`、`--ensure` 全引這個,別各寫各的 600。
pub const DEFAULT_IDLE_SECS: u64 = 600; // 10 分鐘

fn default_contract_version() -> u32 { 1 }
fn default_inference_path() -> String { "/inference".to_string() }

/// 發現檔 schema(v1)。前向相容:未知欄位容忍 + 缺欄回預設(§6)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperServerDescriptor {
    #[serde(default = "default_contract_version")]
    pub contract_version: u32,
    pub host: String,
    pub port: u16,
    pub model: String, // 短名 "small" / "large-v3-turbo"(對齊 config.rs);唯一「哪個模型在跑」來源
    pub pid: u32,
    pub started_at: String, // ISO8601 UTC
    #[serde(default = "default_inference_path")]
    pub inference_path: String,
}

impl WhisperServerDescriptor {
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
    pub fn inference_url(&self) -> String {
        format!("{}{}", self.base_url(), self.inference_path)
    }
}

/// 固定路徑 `~/.mori/whisper-server.json`(home 用 dirs;Windows = %USERPROFILE%)。
pub fn descriptor_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("whisper-server.json"))
        .unwrap_or_else(|| PathBuf::from("whisper-server.json"))
}

/// single-instance lockfile 路徑(§3.2)。
pub fn lock_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("whisper-server.lock"))
        .unwrap_or_else(|| PathBuf::from("whisper-server.lock"))
}

/// 解析 descriptor 字串 + 版本天花板(§8 HIGH)。壞 json / 太新版本 → None。
/// 抽成純函式好單測;read_descriptor 與測試共用同一條判斷。
fn parse_descriptor_str(s: &str) -> Option<WhisperServerDescriptor> {
    let desc: WhisperServerDescriptor = serde_json::from_str(s).ok()?;
    if desc.contract_version > SUPPORTED_CONTRACT_VERSION {
        eprintln!(
            "[whisper] descriptor contract_version {} > supported {} — treating as unusable",
            desc.contract_version, SUPPORTED_CONTRACT_VERSION
        );
        return None;
    }
    Some(desc)
}

/// 讀 descriptor。缺檔 / 壞檔 / 版本太新 → None(視為無 server)。
pub fn read_descriptor() -> Option<WhisperServerDescriptor> {
    let s = std::fs::read_to_string(descriptor_path()).ok()?;
    parse_descriptor_str(&s)
}

/// 原子寫 descriptor:先寫 `.tmp` 再 rename(§1,避免讀到寫一半)。
pub fn write_descriptor(desc: &WhisperServerDescriptor) -> Result<(), String> {
    let path = descriptor_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(desc).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(&tmp, body).map_err(|e| format!("write tmp: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))
}

/// 最佳努力刪 descriptor(server 正常退出時)。consumer 不可假設一定被刪(crash 不刪)。
pub fn remove_descriptor() {
    let _ = std::fs::remove_file(descriptor_path());
}

/// pid 是否還活著。Linux 看 `/proc/<pid>`;其他平台保守回 true,靠 health GET 把關。
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        true
    }
}

/// §3.1 先驗活:pid 活 **且** `GET host:port/` → 200(ready 訊號)。任一不過 → 視為無 server。
pub fn verify_alive(desc: &WhisperServerDescriptor) -> bool {
    if !pid_alive(desc.pid) {
        return false;
    }
    match ureq::get(&desc.base_url())
        .timeout(Duration::from_millis(800))
        .call()
    {
        Ok(resp) => resp.status() == 200,
        Err(_) => false,
    }
}

/// 找一個「可用的」共享 server:讀 descriptor → 驗活 → Some(desc) 才回。否則 None(走 cli / 當 starter)。
pub fn reachable_server() -> Option<WhisperServerDescriptor> {
    let desc = read_descriptor()?;
    if verify_alive(&desc) {
        Some(desc)
    } else {
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// §11 Activation —— 隨需啟動(任何 app 都能喚醒共享 server)
//
// 角色不變(契約 §3.2 / §8):supervisor(`mori-whisper-serve`)是唯一 Starter+Owner
// (搶 flock、起 whisper-server、寫/刪 descriptor、閒置自關);這裡提供的是「任何 consumer
// 都能踢一下這個冪等 supervisor」的共用入口 —— Rust app 直接呼 `ensure_server`;非 Rust
// (python mori-ear / shell / 資料 app)走 `mori-whisper-serve --ensure`(同一支 binary)。
// ─────────────────────────────────────────────────────────────────────────────

/// supervisor 執行檔名(平台)。
pub fn supervisor_bin_name() -> &'static str {
    #[cfg(windows)]
    {
        "mori-whisper-serve.exe"
    }
    #[cfg(not(windows))]
    {
        "mori-whisper-serve"
    }
}

/// 共用安裝點 `~/.mori/bin/mori-whisper-serve`(跟 whisper-cli / whisper-server 同窩)。
/// **任何 app 都從這個固定路徑找/喚醒 supervisor** —— 含非 Rust 的 `--ensure` 呼叫。
pub fn shared_supervisor_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("bin").join(supervisor_bin_name()))
        .unwrap_or_else(|| PathBuf::from(supervisor_bin_name()))
}

/// current_exe 旁邊的 supervisor。`cargo`/`tauri dev` 下兩支 bin 同在 `target/<profile>/`,
/// 所以 dev 跑 recorder 能就近找到 supervisor 當「種子」來源。
/// **注意**:packaged Tauri bundle **目前沒**把 supervisor 列為 sidecar(tauri.conf.json 無
/// `externalBin`),所以正式包裝下 sibling 會是 None → 共用安裝走 `scripts/install-supervisor.{sh,ps1}`
/// 為**權威**鋪法。要讓 bundle 自帶 supervisor 是之後 packaging 的 follow-up。
fn sibling_supervisor_path() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let p = dir.join(supervisor_bin_name());
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// `(len, mtime)` 摘要,給 `need_seed` 判新舊用(抽成純資料好單測)。
type FileStamp = (u64, Option<std::time::SystemTime>);

fn file_stamp(meta: &std::fs::Metadata) -> FileStamp {
    (meta.len(), meta.modified().ok())
}

/// 要不要把 sibling 重種進 shared:**shared 不存在**、**大小不同**、或 **sibling 比 shared 新**
/// → true。純函式(不碰 fs)→ 單測各 arm。`shared=None` 代表共用點還沒種過。
///
/// 加 mtime 是因為:單看大小,兩個不同 build 偶爾 size 相同就不會更新(stale supervisor 卡住,
/// 見 review finding)。sibling mtime > shared mtime 就視為「有新 build」要重種。
fn need_seed(shared: Option<FileStamp>, sib: FileStamp) -> bool {
    match shared {
        None => true,
        Some((slen, smt)) => {
            slen != sib.0
                || match (sib.1, smt) {
                    (Some(s), Some(d)) => s > d, // sibling 比 shared 新
                    _ => false,
                }
        }
    }
}

/// best-effort:把 current_exe 旁邊的 supervisor 種進 `~/.mori/bin`,讓**其他 app**
/// (含非 Rust 的 `--ensure`)之後都能在固定路徑找到它。失敗一律不致命(只是少了共用捷徑)。
///
/// 寫 per-pid `.tmp-install.<pid>` 再 `rename` 覆蓋 —— (a) rename 蓋過「正在被 exec 的舊 binary」
/// 在 Linux 安全(舊 inode 留給仍在跑的 process,檔名指向新 inode);直接 `copy` truncate 會 ETXTBSY。
/// (b) per-pid tmp → 兩個 consumer 同時種**不會撞同一個 tmp** 寫出半截檔(rename 本身原子,各 rename 各的)。
/// 只在 `need_seed`(沒種過 / 大小不同 / sibling 較新)時才複製,避免每次無謂 IO。
pub fn install_shared_supervisor() {
    let sib = match sibling_supervisor_path() {
        Some(p) => p,
        None => return, // 這個 app 沒附帶 supervisor(例:純 adopter)→ 沒東西可種
    };
    let shared = shared_supervisor_path();
    let sib_meta = match std::fs::metadata(&sib) {
        Ok(m) => m,
        Err(_) => return, // sibling 讀不到 → 沒東西可種
    };
    let shared_stamp = std::fs::metadata(&shared).ok().map(|m| file_stamp(&m));
    if !need_seed(shared_stamp, file_stamp(&sib_meta)) {
        return;
    }
    let parent = match shared.parent() {
        Some(p) => p,
        None => return,
    };
    let _ = std::fs::create_dir_all(parent);
    // 唯一 tmp 名:pid 防**跨 process** 撞(兩個 app 同時種);seq 防**同 process** 多執行緒撞
    // (startup .setup 自種 + 開場冷啟動 ensure_server 可能同時跑)。rename 本身原子,各 rename 各的。
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let uniq = format!(
        "{}.{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let tmp = shared.with_extension(format!("tmp-install.{uniq}"));
    if std::fs::copy(&sib, &tmp).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
        }
        if std::fs::rename(&tmp, &shared).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// 在「共用點 / sibling」兩個候選間挑可執行的 supervisor:**先 shared、再 sibling**。
/// 抽成吃參數的純路徑邏輯(只碰 `Path::exists`)好用 tempdir 單測順序。
fn locate_supervisor_in(shared: &Path, sibling: Option<PathBuf>) -> Option<PathBuf> {
    if shared.exists() {
        return Some(shared.to_path_buf());
    }
    sibling
}

/// 找可執行的 supervisor:**先共用安裝點 `~/.mori/bin`**,再退回 current_exe 旁邊(dev)。
pub fn locate_supervisor() -> Option<PathBuf> {
    locate_supervisor_in(&shared_supervisor_path(), sibling_supervisor_path())
}

/// async-signal-safe:fork 後、exec 前關掉繼承來的 fd `3..RLIMIT_NOFILE`。
/// **必要**(見 memory `mori-spawn-close-fds-linux`):`single-instance` crate 在 Linux 用 Unix socket
/// 做 lock 但**沒設 FD_CLOEXEC**,detached 的長命子程序會繼承父(recorder)的 single-instance socket,
/// 父死後仍守住 → 下次啟動誤判「已有實例在跑」。`getrlimit`/`close` 皆 async-signal-safe。
/// stdio(0/1/2)保留;`spawn_*` 的 stdio 在 pre_exec 前已 dup2 到 0/1/2,關 ≥3 不影響
/// (supervisor 靠 piped stdout/stderr 看活動,已驗 pre_exec 在 dup2 之後跑 → 不會關掉 pipe)。
#[cfg(target_os = "linux")]
pub fn close_inherited_fds() {
    unsafe {
        let mut rlim: libc::rlimit = std::mem::zeroed();
        let max_fd = if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) == 0 {
            (rlim.rlim_cur as libc::c_long).min(libc::c_int::MAX as libc::c_long) as libc::c_int
        } else {
            1024
        };
        for fd in 3..max_fd {
            libc::close(fd);
        }
    }
}

/// detached spawn supervisor —— **裸呼叫(無 `--ensure`)= supervise 主迴圈**(契約 §11)。
/// **Linux**:`setsid`(自成 session,呼叫者關掉也不連帶收掉它)+ close 繼承 fd(防 single-instance
/// socket 洩漏)。**Windows**:`DETACHED_PROCESS`。**其他平台(含 macOS)目前不 detach**,照常 spawn
/// (recorder 只支援 Linux + Windows,macOS 無音訊 backend)。stdio 全 null、fire-and-forget(不 wait)。
pub fn spawn_supervisor_detached(bin: &Path, model: &str, idle_secs: u64) -> Result<(), String> {
    let mut cmd = std::process::Command::new(bin);
    cmd.args(["--model", model, "--idle-secs", &idle_secs.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(target_os = "linux")]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setsid(); // 自成 session,呼叫者關掉不連帶收它
            close_inherited_fds(); // 別讓長命子程序守住父的 single-instance socket(memory: mori-spawn-close-fds-linux)
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

/// 隨需確保「有一台」共享 whisper-server 在跑(契約 §11 Activation 的 Rust 入口)。
///
/// - 已有**驗活過**的 server → 直接 return(**沿用正在跑的,不管它載哪個 model**;呼叫者若
///   真的需要特定 model,讀 `reachable_server()?.model` 自行決定要不要 fallback cli,見 §3.4)。
/// - 否則:best-effort 把 supervisor 種進 `~/.mori/bin`,再 detached 拉起它(fire-and-forget,
///   不等 ready、不卡)。找不到 supervisor / spawn 失敗 → 安靜略過(consumer 之後 fallback cli;
///   **standalone-first 不破**,契約 §3.3)。
///
/// `model`:**只有當這次是冷啟動者**才會用到(supervisor 載這個 model);已有 server 時忽略。
/// recorder / mori-desktop / AgentOS 等任何 Rust consumer 都可呼叫;非 Rust 走 `--ensure`。
pub fn ensure_server(model: &str) {
    if reachable_server().is_some() {
        return; // 已有活的共享 server(任何 model)→ 沿用,免動
    }
    install_shared_supervisor(); // 讓往後任何 app（含非 Rust）都能在 ~/.mori/bin 找到它
    let bin = match locate_supervisor() {
        Some(b) => b,
        None => {
            eprintln!(
                "[whisper] mori-whisper-serve not found (~/.mori/bin or next to exe); skip autostart — using cli"
            );
            return;
        }
    };
    match spawn_supervisor_detached(&bin, model, DEFAULT_IDLE_SECS) {
        Ok(()) => eprintln!(
            "[whisper] ensured shared whisper-server (model={model}, supervisor={})",
            bin.display()
        ),
        Err(e) => eprintln!("[whisper] autostart supervisor failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_roundtrips_and_tolerates_missing_optional_fields() {
        // 缺 contract_version / inference_path → 回預設(前向相容)
        let json = r#"{"host":"127.0.0.1","port":12345,"model":"large-v3-turbo","pid":67890,"started_at":"2026-05-29T12:34:56Z"}"#;
        let d: WhisperServerDescriptor = serde_json::from_str(json).unwrap();
        assert_eq!(d.contract_version, 1);
        assert_eq!(d.inference_path, "/inference");
        assert_eq!(d.port, 12345);
        assert_eq!(d.model, "large-v3-turbo");
        assert_eq!(d.inference_url(), "http://127.0.0.1:12345/inference");
    }

    #[test]
    fn unknown_fields_tolerated() {
        let json = r#"{"host":"h","port":1,"model":"small","pid":1,"started_at":"t","inference_path":"/inference","future_field":42}"#;
        let d: WhisperServerDescriptor = serde_json::from_str(json).unwrap();
        assert_eq!(d.host, "h");
    }

    #[test]
    fn contract_version_ceiling_rejects_newer_descriptor() {
        // v1 / 缺欄(預設 1)→ 收;比支援上限新(v2)→ 視為 unusable(None),不可當舊版吃(§8 HIGH)。
        assert!(parse_descriptor_str(
            r#"{"contract_version":1,"host":"127.0.0.1","port":1,"model":"small","pid":1,"started_at":"2026-05-29T00:00:00Z"}"#
        ).is_some());
        assert!(parse_descriptor_str(
            r#"{"host":"127.0.0.1","port":1,"model":"small","pid":1,"started_at":"2026-05-29T00:00:00Z"}"#
        ).is_some());
        assert!(parse_descriptor_str(
            r#"{"contract_version":2,"host":"127.0.0.1","port":1,"model":"small","pid":1,"started_at":"2026-05-29T00:00:00Z"}"#
        ).is_none());
        // 壞 json → None
        assert!(parse_descriptor_str("{ not json").is_none());
    }

    #[test]
    fn shared_supervisor_path_under_mori_bin() {
        let p = shared_supervisor_path();
        let s = p.to_string_lossy();
        assert!(s.contains(".mori"), "supervisor 應住 ~/.mori/bin: {s}");
        assert!(s.contains("bin"), "supervisor 應住 ~/.mori/bin: {s}");
        assert!(p.ends_with(supervisor_bin_name()), "檔名應為 {}", supervisor_bin_name());
    }

    #[test]
    fn supervisor_bin_name_matches_platform() {
        #[cfg(windows)]
        assert_eq!(supervisor_bin_name(), "mori-whisper-serve.exe");
        #[cfg(not(windows))]
        assert_eq!(supervisor_bin_name(), "mori-whisper-serve");
    }

    #[test]
    fn default_idle_is_ten_minutes() {
        // 契約 §11:閒置 10 分鐘自關。單一事實來源,別飄。
        assert_eq!(DEFAULT_IDLE_SECS, 600);
    }

    #[test]
    fn need_seed_covers_all_arms() {
        use std::time::{Duration, SystemTime};
        let t0 = SystemTime::UNIX_EPOCH;
        let t1 = t0 + Duration::from_secs(100);
        // shared 還沒種過 → 一定種
        assert!(need_seed(None, (10, Some(t1))));
        // 大小不同 → 種
        assert!(need_seed(Some((9, Some(t1))), (10, Some(t1))));
        // 同大小、sibling 較新 → 種(避免 stale supervisor 卡住)
        assert!(need_seed(Some((10, Some(t0))), (10, Some(t1))));
        // 同大小、sibling 不比較新 → 不種
        assert!(!need_seed(Some((10, Some(t1))), (10, Some(t0))));
        assert!(!need_seed(Some((10, Some(t1))), (10, Some(t1))));
        // mtime 讀不到時退回純大小判斷:同大小 → 不種
        assert!(!need_seed(Some((10, None)), (10, None)));
    }

    #[test]
    fn locate_supervisor_prefers_shared_then_sibling() {
        let tmp = tempfile::TempDir::new().unwrap();
        let shared = tmp.path().join("shared-bin");
        let sibling = tmp.path().join("sibling-bin");
        // 兩個都不存在 → 給的 sibling 候選為 None → None
        assert!(locate_supervisor_in(&shared, None).is_none());
        // shared 不存在、有 sibling 候選 → 回 sibling
        std::fs::write(&sibling, b"x").unwrap();
        assert_eq!(
            locate_supervisor_in(&shared, Some(sibling.clone())),
            Some(sibling.clone())
        );
        // shared 存在 → 一律先 shared(即使 sibling 也在)
        std::fs::write(&shared, b"y").unwrap();
        assert_eq!(
            locate_supervisor_in(&shared, Some(sibling.clone())),
            Some(shared.clone())
        );
    }
}
