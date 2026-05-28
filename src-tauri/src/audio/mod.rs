//! 音訊 capture / write — per-track WAV writer + 平台 capture impl(linux / windows)。

pub mod writer;

use serde::{Deserialize, Serialize};

/// 一個 source 的「分類」— 決定預設 visibility + 在 segment 上的 source_kind 欄。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    MeetingSystem,
    MicInternal,
}

/// Segment / 匯出檔的 visibility。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Internal,
}

impl SourceKind {
    pub fn default_visibility(self) -> Visibility {
        match self {
            Self::MeetingSystem => Visibility::Public,
            Self::MicInternal => Visibility::Internal,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::MeetingSystem => "meeting_system",
            Self::MicInternal => "mic_internal",
        }
    }

    pub fn track_name(self) -> &'static str {
        match self {
            Self::MeetingSystem => "system",
            Self::MicInternal => "mic-internal",
        }
    }
}
