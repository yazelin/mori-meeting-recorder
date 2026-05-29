//! Whisper-Server 共享發現契約 v1。
//! 對齊 agentos-notebook/05-mori-migration/whisper-server-contract.md(跨 repo 單一事實來源)。
//!
//! consumer 讀 `~/.mori/whisper-server.json` 找本地共享 server,**先驗活**(pid + `GET /` → 200)
//! 才信(§3.1)。`GET /` 是 whisper.cpp 的 ready 訊號:模型載完才回 200,不只是 listening。
//! descriptor 的 `model` 是「哪個模型在跑」的唯一來源(server 無查詢端點)。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

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

/// 讀 descriptor。缺檔 / 壞檔 → None(視為無 server)。
pub fn read_descriptor() -> Option<WhisperServerDescriptor> {
    let s = std::fs::read_to_string(descriptor_path()).ok()?;
    serde_json::from_str(&s).ok()
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
fn pid_alive(pid: u32) -> bool {
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
}
