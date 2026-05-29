//! Recorder 可調參數 — VAD chunking 行為。存 ~/.mori/meeting-recorder/config.json。
//! 缺檔 / parse fail / 缺欄 → 各自回預設(serde per-field default)。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_silence_split_ms() -> u64 {
    600
}
fn default_silence_threshold_db() -> f32 {
    -45.0
}
fn default_min_speech_secs() -> f32 {
    0.5
}
fn default_max_segment_secs() -> f32 {
    20.0
}
fn default_language() -> String {
    "zh".to_string()
}
fn default_traditional() -> bool {
    true
}
fn default_model() -> String {
    // 對應 ~/.mori/models/ggml-<model>.bin。目前 UI 給兩個:small / large-v3-turbo。
    "small".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecorderConfig {
    #[serde(default = "default_silence_split_ms")]
    pub silence_split_ms: u64,
    #[serde(default = "default_silence_threshold_db")]
    pub silence_threshold_db: f32,
    #[serde(default = "default_min_speech_secs")]
    pub min_speech_secs: f32,
    #[serde(default = "default_max_segment_secs")]
    pub max_segment_secs: f32,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_traditional")]
    pub traditional: bool,
    #[serde(default = "default_model")]
    pub model: String,
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            silence_split_ms: default_silence_split_ms(),
            silence_threshold_db: default_silence_threshold_db(),
            min_speech_secs: default_min_speech_secs(),
            max_segment_secs: default_max_segment_secs(),
            language: default_language(),
            traditional: default_traditional(),
            model: default_model(),
        }
    }
}

/// ~/.mori/meeting-recorder/config.json
/// (default_meetings_dir() = ~/.mori/meetings,parent = ~/.mori)
pub fn config_path() -> PathBuf {
    crate::session_store::default_meetings_dir()
        .parent()
        .map(|p| p.join("meeting-recorder").join("config.json"))
        .unwrap_or_else(|| PathBuf::from("config.json"))
}

pub fn read_config() -> RecorderConfig {
    let path = config_path();
    let s = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return RecorderConfig::default(),
    };
    serde_json::from_str(&s).unwrap_or_default()
}

pub fn write_config(cfg: &RecorderConfig) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir config dir: {e}"))?;
    }
    let s = serde_json::to_string_pretty(cfg).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&path, s).map_err(|e| format!("write config: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values_match_spec() {
        let c = RecorderConfig::default();
        assert_eq!(c.silence_split_ms, 600);
        assert_eq!(c.silence_threshold_db, -45.0);
        assert_eq!(c.min_speech_secs, 0.5);
        assert_eq!(c.max_segment_secs, 20.0);
        assert_eq!(c.language, "zh");
        assert!(c.traditional);
    }

    #[test]
    fn deserialize_full_json() {
        let json = r#"{"silence_split_ms":800,"silence_threshold_db":-50.0,"min_speech_secs":1.0,"max_segment_secs":30.0}"#;
        let c: RecorderConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.silence_split_ms, 800);
        assert_eq!(c.max_segment_secs, 30.0);
    }

    #[test]
    fn missing_field_falls_back_to_default() {
        let json = r#"{"silence_split_ms":900}"#;
        let c: RecorderConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.silence_split_ms, 900);
        assert_eq!(c.silence_threshold_db, -45.0);
        assert_eq!(c.min_speech_secs, 0.5);
        assert_eq!(c.max_segment_secs, 20.0);
    }

    #[test]
    fn empty_json_all_defaults() {
        let c: RecorderConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c, RecorderConfig::default());
    }

    #[test]
    fn missing_language_and_traditional_fall_back() {
        // JSON without the new fields should fall back to zh/true via serde defaults
        let json = r#"{"silence_split_ms":600,"silence_threshold_db":-45.0,"min_speech_secs":0.5,"max_segment_secs":20.0}"#;
        let c: RecorderConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.language, "zh");
        assert!(c.traditional);
    }
}
