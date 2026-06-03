//! 偵測 mori-desktop(hub）是否「正在執行」—— 讀它啟動時寫的 presence marker
//! `~/.mori/body-parts/mori.desktop/manifest.json`（含 `pid`）。
//!
//! 用途（BI-5 follow-up，雙向偵測 + 自適應 UI）:recorder 啟動時若偵測到 desktop 在跑
//! （或被帶 `--no-tray` 啟動），就不長自己的 tray icon —— 由 desktop 的 tray 代表本部件,
//! 避免「兩個 tray 圖示」。
//!
//! 關鍵:只看「**正在執行**(PID 還活著)」而非「**已安裝**」。否則 desktop 裝了沒開、
//! 或 crash 殘留 marker,會讓 recorder 永遠藏 tray。marker 不在 / PID 死掉 / 解析失敗
//! → 一律當作 desktop 沒在跑,recorder 行為與今日 standalone 完全相同。

use std::path::PathBuf;

/// desktop 寫的 presence marker 路徑(對齊 manifest.rs / session_store.rs 的 home_dir 慣例）。
pub fn desktop_marker_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| {
            h.join(".mori")
                .join("body-parts")
                .join("mori.desktop")
                .join("manifest.json")
        })
        .unwrap_or_else(|| PathBuf::from(".mori/body-parts/mori.desktop/manifest.json"))
}

/// marker 內我們唯一在意的欄位(其餘 BodyManifest 欄位 serde 預設忽略）。
#[derive(serde::Deserialize)]
struct DesktopMarker {
    #[serde(default)]
    pid: Option<u32>,
}

/// PID 是否還活著。跨平台 best-effort,對齊 `whisper_discovery::pid_alive`:
/// Linux 看 `/proc/<pid>`;其他平台沒有便宜的查法 → 保守回 true(寧可「以為它在」,
/// 因為 no_tray 時主視窗仍在工作列可達,最壞情況只是少一個 tray 圖示,不會變孤兒)。
#[cfg(target_os = "linux")]
fn pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}
#[cfg(not(target_os = "linux"))]
fn pid_alive(_pid: u32) -> bool {
    true
}

/// 解析 marker 內容 → 是否視為「desktop 正在執行」。抽成純函式方便單元測試。
fn running_from_contents(json: &str) -> bool {
    match serde_json::from_str::<DesktopMarker>(json) {
        Ok(m) => match m.pid {
            Some(pid) => pid_alive(pid),
            None => false, // 沒 pid 欄位 → 無法確認在跑 → 當作沒跑
        },
        Err(_) => false, // 壞 JSON → 當作沒跑
    }
}

/// recorder 啟動時呼叫:mori-desktop(hub)現在是否在執行?
/// marker 不存在 / 讀不到 / PID 死掉 → false(standalone 行為不變）。
pub fn is_desktop_running() -> bool {
    match std::fs::read_to_string(desktop_marker_path()) {
        Ok(contents) => running_from_contents(&contents),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_pid_is_not_running() {
        assert!(!running_from_contents(r#"{"id":"mori.desktop"}"#));
    }

    #[test]
    fn bad_json_is_not_running() {
        assert!(!running_from_contents("not json{{"));
    }

    #[test]
    fn empty_is_not_running() {
        assert!(!running_from_contents(""));
    }

    #[test]
    fn marker_path_shape() {
        let s = desktop_marker_path().to_string_lossy().replace('\\', "/");
        assert!(s.contains("body-parts/mori.desktop"));
        assert!(s.ends_with("manifest.json"));
    }

    // pid_alive 在非 Linux 保守回 true,所以「活 PID」測試只在 Linux 有意義。
    #[cfg(target_os = "linux")]
    #[test]
    fn live_pid_is_running_linux() {
        let pid = std::process::id();
        assert!(running_from_contents(&format!(r#"{{"pid":{pid}}}"#)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dead_pid_is_not_running_linux() {
        // 極不可能存在的大 PID
        assert!(!running_from_contents(r#"{"pid":4294967294}"#));
    }
}
