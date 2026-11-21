//! Versioned `a2a-lab.json` envelope, chat overrides, and video settings.
//!
//! File loading is separate. Parsing does not open devices or apply application
//! defaults for omitted video runtime fields.

mod camera_selector;
mod capture_settings;
mod capture_settings_watch;
mod duration;
mod image_decision;
mod load;
mod logging;
#[cfg(any(test, feature = "web-ui"))]
mod pending;
mod semantic_indexing;
mod store;
mod strict_json;
mod video;

use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use semantic_indexing::{
    A2A_LAB_CONFIG_SCHEMA_VERSION_V2 as SCHEMA_V2, parse_semantic_indexing_required,
};
use strict_json::parse_strict_json;
use video::parse_videos;

pub use camera_selector::{CameraIdView, CameraSelector};
pub use capture_settings::{
    DEFAULT_LIVE_OUTPUT_FPS, VideoCaptureSettings, VideoCaptureSettingsView, VideoCaptureSourceView,
};
pub use capture_settings_watch::VideoCaptureSettingsWatch;
pub use duration::{ConfigDuration, SaveIntervalDuration};
pub use image_decision::{
    AgentCoreImageDecisionSettings, DEFAULT_IMAGE_DECISION_TIMEOUT_SECS,
    IMAGE_DECISION_CLIENT_DOWNSTREAM_DEPENDENCY, ImageDecisionClientConfig, ImageDecisionProvider,
    MAX_IMAGE_DECISION_RESPONSE_BYTES, MAX_IMAGE_DECISION_TIMEOUT_SECS,
    ResolvedImageDecisionClient,
};
pub use load::LoadedLabConfig;
pub use logging::{
    DEFAULT_LOG_OUTPUT_FORMAT, DEFAULT_LOGGING_LEVEL, LogOutputFormat, LoggingConfig,
    OpenTelemetryLoggingConfig,
};
#[cfg(any(test, feature = "web-ui"))]
pub(crate) use pending::pending_restart_and_reindex;
pub use semantic_indexing::{
    A2A_LAB_CONFIG_SCHEMA_VERSION_V2, CredentialReference, DEFAULT_JEV_API_ENDPOINT,
    DEFAULT_SEMANTIC_DESCRIPTION_BACKEND, DEFAULT_SEMANTIC_SCORE_FLOOR, DecisionCoveragePolicy,
    EmbeddingSpaceConfig, SemanticDecisionLimits, SemanticDecisionsConfig,
    SemanticDescriptionConfig, SemanticEmbeddingsConfig, SemanticIndexingConfig,
    SemanticIndexingLimits, SemanticIndexingSearchLimits, SemanticRoiLimits, SemanticSchedule,
    SemanticStorageLimits, SemanticStreamSchedule, SemanticWorkerLimits,
};
pub use store::{ConfigPersistence, ConfigStoreError, LabConfigSnapshot, LabConfigStore};
pub use video::{VideoConfig, VideoDvrConfig, VideoInputFormat, VideoSourceConfig};

/// Supported v1 configuration schema version.
pub const A2A_LAB_CONFIG_SCHEMA_VERSION: u32 = 1;

/// Parsed configuration envelope (`schema_version` 1 or 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct A2aLabConfig {
    /// Parsed `schema_version` field (`1` or `2` only).
    pub schema_version: u32,
    /// Optional chat provider/model overrides.
    pub chat: Option<ChatConfig>,
    /// Optional video settings. Absent when the file omits `videos`.
    pub videos: Option<VideoConfig>,
    /// Optional additive semantic indexing settings.
    pub semantic_indexing: Option<SemanticIndexingConfig>,
    /// Optional synchronous generic JEV image-decision client.
    pub image_decision: Option<ImageDecisionClientConfig>,
    /// Optional application logging settings. Absent means logging stays off.
    pub logging: Option<LoggingConfig>,
}

/// Optional chat provider and model overrides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatConfig {
    /// When set, selects one of the supported chat providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ChatProvider>,
    /// When set, must contain non-whitespace text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Supported chat provider identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatProvider {
    /// Amazon Bedrock.
    Bedrock,
    /// OpenAI.
    Openai,
    /// Anthropic.
    Anthropic,
}

impl ChatProvider {
    /// Stable provider identifier text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bedrock => "bedrock",
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
        }
    }
}

impl A2aLabConfig {
    /// Parses a v1 configuration document without side effects.
    pub fn parse(document: &str) -> Result<Self, A2aLabError> {
        let value = parse_strict_json(document)?;
        Self::from_value(&value)
    }

    fn from_value(value: &serde_json::Value) -> Result<Self, A2aLabError> {
        let map = object_at("", value)?;
        let schema_version = parse_schema_version(map.get("schema_version"))?;
        let allowed_envelope = match schema_version {
            A2A_LAB_CONFIG_SCHEMA_VERSION | SCHEMA_V2 => &[
                "schema_version",
                "chat",
                "videos",
                "semantic_indexing",
                "image_decision",
                "logging",
            ][..],
            _ => {
                return Err(A2aLabError::invalid(
                    "schema_version",
                    "unsupported schema version",
                ));
            }
        };
        reject_unknown_keys(map, "", allowed_envelope)?;
        let chat = parse_chat(map.get("chat"))?;
        let has_semantic_indexing = map.contains_key("semantic_indexing");
        let videos = parse_videos(
            map.get("videos"),
            schema_version == SCHEMA_V2 || has_semantic_indexing,
        )?;
        let semantic_indexing = match schema_version {
            A2A_LAB_CONFIG_SCHEMA_VERSION | SCHEMA_V2 if has_semantic_indexing => Some(
                parse_semantic_indexing_required(map.get("semantic_indexing"))?,
            ),
            _ => None,
        };
        if let Some(semantic) = semantic_indexing.as_ref() {
            semantic.reject_source_embedding_overrides(videos.as_ref())?;
        }
        let image_decision = match map.get("image_decision") {
            None => None,
            Some(value) => Some(ImageDecisionClientConfig::parse_value(value)?),
        };
        let logging = logging::parse_logging(map.get("logging"))?;
        Ok(Self {
            schema_version,
            chat,
            videos,
            semantic_indexing,
            image_decision,
            logging,
        })
    }
}

pub(crate) fn object_at<'a>(
    path: &'static str,
    value: &'a serde_json::Value,
) -> Result<&'a serde_json::Map<String, serde_json::Value>, A2aLabError> {
    match value {
        serde_json::Value::Object(map) => Ok(map),
        _ => Err(A2aLabError::invalid(
            path_or_input(path),
            "must be a JSON object",
        )),
    }
}

fn path_or_input(path: &'static str) -> &'static str {
    if path.is_empty() { "input" } else { path }
}

pub(crate) fn reject_unknown_keys(
    map: &serde_json::Map<String, serde_json::Value>,
    path: &'static str,
    allowed: &[&str],
) -> Result<(), A2aLabError> {
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(A2aLabError::invalid(
                unknown_field(path, key),
                "unknown field",
            ));
        }
    }
    Ok(())
}

fn unknown_field(parent: &'static str, key: &str) -> &'static str {
    match (parent, key) {
        ("", "semantic_indexing") => "semantic_indexing",
        ("videos", "disk_retention_secs") => "videos.disk_retention_secs",
        ("videos", "live_retention_secs") => "videos.live_retention_secs",
        ("videos", "dvr_root") => "videos.dvr_root",
        ("videos", "backend") => "videos.backend",
        ("videos", "output_format") => "videos.output_format",
        ("videos", "semantic_indexing") => "videos.semantic_indexing",
        ("videos.sources", "backend") => "videos.sources.backend",
        ("videos.sources", "output_format") => "videos.sources.output_format",
        ("videos.sources", "h264") => "videos.sources.h264",
        ("videos.dvr", "disk_retention_secs") => "videos.dvr.disk_retention_secs",
        ("videos.dvr", "dvr_root") => "videos.dvr.dvr_root",
        ("chat", _) => "chat",
        ("videos", _) => "videos",
        ("videos.sources", _) => "videos.sources",
        ("videos.dvr", _) => "videos.dvr",
        _ => "input",
    }
}

pub(crate) fn parse_u32(
    field: &'static str,
    value: &serde_json::Value,
) -> Result<u32, A2aLabError> {
    let invalid = || A2aLabError::invalid(field, "must be an integer");
    match value {
        serde_json::Value::Number(number) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(invalid),
        _ => Err(invalid()),
    }
}

fn parse_schema_version(value: Option<&serde_json::Value>) -> Result<u32, A2aLabError> {
    let Some(value) = value else {
        return Err(A2aLabError::invalid(
            "schema_version",
            "required field is missing",
        ));
    };
    let version = positive_integer("schema_version", value)?;
    if version == A2A_LAB_CONFIG_SCHEMA_VERSION || version == SCHEMA_V2 {
        Ok(version)
    } else {
        Err(A2aLabError::invalid(
            "schema_version",
            "unsupported schema version",
        ))
    }
}

fn positive_integer(field: &'static str, value: &serde_json::Value) -> Result<u32, A2aLabError> {
    match value {
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                u32::try_from(value).map_err(|_| A2aLabError::invalid(field, "must be an integer"))
            } else if let Some(value) = number.as_i64() {
                if value < 0 {
                    return Err(A2aLabError::invalid(field, "must be an integer"));
                }
                u32::try_from(value)
                    .map_err(|_| A2aLabError::invalid(field, "unsupported schema version"))
            } else {
                Err(A2aLabError::invalid(field, "must be an integer"))
            }
        }
        _ => Err(A2aLabError::invalid(field, "must be an integer")),
    }
}

fn parse_chat(value: Option<&serde_json::Value>) -> Result<Option<ChatConfig>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Err(A2aLabError::invalid("chat", "must be a JSON object"));
    }
    let map = object_at("chat", value)?;
    reject_unknown_keys(map, "chat", &["provider", "model"])?;
    let provider = parse_provider(map.get("provider"))?;
    let model = parse_model(map.get("model"))?;
    if provider.is_none() && model.is_none() {
        return Ok(Some(ChatConfig {
            provider: None,
            model: None,
        }));
    }
    Ok(Some(ChatConfig { provider, model }))
}

fn parse_provider(value: Option<&serde_json::Value>) -> Result<Option<ChatProvider>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(text) = value.as_str() else {
        return Err(A2aLabError::invalid("chat.provider", "must be a string"));
    };
    match text {
        "bedrock" => Ok(Some(ChatProvider::Bedrock)),
        "openai" => Ok(Some(ChatProvider::Openai)),
        "anthropic" => Ok(Some(ChatProvider::Anthropic)),
        _ => Err(A2aLabError::invalid(
            "chat.provider",
            "unsupported chat provider",
        )),
    }
}

fn parse_model(value: Option<&serde_json::Value>) -> Result<Option<String>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(text) = value.as_str() else {
        return Err(A2aLabError::invalid("chat.model", "must be a string"));
    };
    if text.chars().any(|character| !character.is_whitespace()) {
        Ok(Some(text.to_owned()))
    } else {
        Err(A2aLabError::invalid(
            "chat.model",
            "model must contain non-whitespace text",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_envelope_accepts_schema_version_only() {
        let config = A2aLabConfig::parse(r#"{"schema_version":1}"#).unwrap();
        assert_eq!(config.schema_version, 1);
        assert!(config.chat.is_none());
        assert!(config.videos.is_none());
        assert!(config.semantic_indexing.is_none());
        assert!(config.image_decision.is_none());
        assert!(config.logging.is_none());
    }

    #[test]
    fn chat_only_overrides_are_optional() {
        let config = A2aLabConfig::parse(
            r#"{"schema_version":1,"chat":{"provider":"openai","model":"gpt-test"}}"#,
        )
        .unwrap();
        assert_eq!(
            config.chat.as_ref().unwrap().provider,
            Some(ChatProvider::Openai)
        );
        assert_eq!(
            config.chat.as_ref().unwrap().model.as_deref(),
            Some("gpt-test")
        );
    }

    #[test]
    fn empty_chat_object_is_valid() {
        let config = A2aLabConfig::parse(r#"{"schema_version":1,"chat":{}}"#).unwrap();
        assert_eq!(
            config.chat,
            Some(ChatConfig {
                provider: None,
                model: None,
            })
        );
    }

    #[test]
    fn rejects_malformed_json() {
        let error = A2aLabConfig::parse("{").unwrap_err();
        assert_eq!(error.code(), "invalid");
        match error {
            A2aLabError::Invalid { field, .. } => assert_eq!(field, "input"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn rejects_duplicate_envelope_keys() {
        let error = A2aLabConfig::parse(r#"{"schema_version":1,"schema_version":1}"#).unwrap_err();
        assert_eq!(error.code(), "invalid");
        match error {
            A2aLabError::Invalid { field, message } => {
                assert_eq!(field, "input");
                assert_eq!(message, "duplicate JSON object key");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_envelope_field() {
        let error = A2aLabConfig::parse(r#"{"schema_version":1,"retired_envelope_field":true}"#)
            .unwrap_err();
        match error {
            A2aLabError::Invalid { field, message } => {
                assert_eq!(field, "input");
                assert_eq!(message, "unknown field");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn rejects_missing_and_invalid_schema_version() {
        assert!(A2aLabConfig::parse(r"{}").is_err());
        assert!(A2aLabConfig::parse(r#"{"schema_version":"1"}"#).is_err());
        assert!(A2aLabConfig::parse(r#"{"schema_version":3}"#).is_err());
        assert!(A2aLabConfig::parse(r#"{"schema_version":1.5}"#).is_err());
    }

    #[test]
    fn rejects_chat_validation_failures() {
        assert!(
            A2aLabConfig::parse(r#"{"schema_version":1,"chat":{"provider":"unknown"}}"#).is_err()
        );
        assert!(A2aLabConfig::parse(r#"{"schema_version":1,"chat":{"model":"   "}}"#).is_err());
        assert!(A2aLabConfig::parse(r#"{"schema_version":1,"chat":{"extra":true}}"#).is_err());
        assert!(
            A2aLabConfig::parse(
                r#"{"schema_version":1,"chat":{"provider":"openai","provider":"anthropic"}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn diagnostics_do_not_echo_rejected_values() {
        let error =
            A2aLabConfig::parse(r#"{"schema_version":1,"chat":{"provider":"not-a-provider"}}"#)
                .unwrap_err();
        let display = error.to_string();
        assert!(!display.contains("not-a-provider"));
        assert!(display.contains("chat.provider"));
    }
}
