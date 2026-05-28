//! BI-1 manifest writer — 啟動時 overwrite `~/.mori/body-parts/mori.meeting-recorder/manifest.json`。

use std::path::{Path, PathBuf};

/// 產生 manifest JSON 字串(`entrypoints.app` 帶進來,讓測試可控)。
pub fn manifest_json(binary_path: &Path) -> String {
    serde_json::json!({
        "schema_version": 1,
        "id": "mori.meeting-recorder",
        "name": "Mori Meeting Recorder",
        "kind": "standalone_app",
        "description": "Dual-track meeting recorder (system + mic) with visibility-based export. Observer Mode MVP.",
        "capabilities": [
            "audio.capture.system",
            "audio.capture.mic",
            "transcribe.local"
        ],
        "entrypoints": {
            "app": binary_path.to_string_lossy()
        },
        "interfaces": [],
        "permissions": [],
        "data_policy": {
            "owns_raw_data": true,
            "default_ingestion": "off"
        }
    })
    .to_string()
}

/// 寫 manifest 到 `~/.mori/body-parts/mori.meeting-recorder/manifest.json`(overwrite)。
pub fn write_manifest_to(dir: &Path, binary_path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let json = manifest_json(binary_path);
    std::fs::write(dir.join("manifest.json"), json).map_err(|e| format!("write: {e}"))
}

/// 啟動時呼叫 — 解析 std::env::current_exe() + 算出真實 `~/.mori/body-parts/...` 路徑。
pub fn body_part_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("body-parts").join("mori.meeting-recorder"))
        .unwrap_or_else(|| PathBuf::from(".mori/body-parts/mori.meeting-recorder"))
}

pub fn write_on_startup() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    write_manifest_to(&body_part_dir(), &exe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn manifest_json_has_required_fields() {
        let path = PathBuf::from("/usr/local/bin/mori-meeting-recorder");
        let j = manifest_json(&path);
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["id"], "mori.meeting-recorder");
        assert_eq!(v["kind"], "standalone_app");
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["entrypoints"]["app"], "/usr/local/bin/mori-meeting-recorder");
        assert_eq!(v["interfaces"].as_array().unwrap().len(), 0);
        let caps = v["capabilities"].as_array().unwrap();
        assert!(caps.iter().any(|c| c == "audio.capture.system"));
        assert!(caps.iter().any(|c| c == "audio.capture.mic"));
        assert_eq!(v["data_policy"]["owns_raw_data"], true);
        assert_eq!(v["data_policy"]["default_ingestion"], "off");
    }

    #[test]
    fn write_manifest_to_creates_dir_and_file() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("body-parts").join("mori.meeting-recorder");
        let exe = PathBuf::from("/some/path/mori-meeting-recorder");
        write_manifest_to(&dir, &exe).unwrap();
        let manifest = dir.join("manifest.json");
        assert!(manifest.exists());
        let content = std::fs::read_to_string(&manifest).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["id"], "mori.meeting-recorder");
    }

    #[test]
    fn write_manifest_to_overwrites_existing() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("d");
        write_manifest_to(&dir, &PathBuf::from("/old/path")).unwrap();
        write_manifest_to(&dir, &PathBuf::from("/new/path")).unwrap();
        let content = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
        assert!(content.contains("/new/path"));
        assert!(!content.contains("/old/path"));
    }
}
