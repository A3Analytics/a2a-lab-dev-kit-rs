//! Optional application logging settings in `a2a-lab.json`.

use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;

use super::{object_at, reject_unknown_keys};

/// Default filter when `logging` is present but `level` is omitted.
pub const DEFAULT_LOGGING_LEVEL: &str = "info";

/// Structured output mode for application logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogOutputFormat {
    /// Human-readable lines on stderr.
    Text,
    /// Newline-delimited JSON events on stderr.
    Json,
}

/// Default stderr renderer when `logging.format` is omitted.
pub const DEFAULT_LOG_OUTPUT_FORMAT: LogOutputFormat = LogOutputFormat::Text;

/// Optional OpenTelemetry export settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenTelemetryLoggingConfig {
    /// OTLP gRPC endpoint, for example `http://127.0.0.1:4317`.
    pub endpoint: String,
}

/// Application logging configuration. Absent in the file means logging stays off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Maximum level for the default `a2a_lab_dev_kit` target filter.
    #[serde(default = "default_level")]
    pub level: String,
    /// Selects JSON or human-readable stderr output.
    #[serde(default = "default_format")]
    pub format: LogOutputFormat,
    /// Optional `tracing-subscriber` env-filter directives appended after the crate default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// Optional OTLP trace export (`otel` Cargo feature).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opentelemetry: Option<OpenTelemetryLoggingConfig>,
}

const fn default_format() -> LogOutputFormat {
    DEFAULT_LOG_OUTPUT_FORMAT
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: DEFAULT_LOGGING_LEVEL.to_owned(),
            format: DEFAULT_LOG_OUTPUT_FORMAT,
            filter: None,
            opentelemetry: None,
        }
    }
}

fn default_level() -> String {
    DEFAULT_LOGGING_LEVEL.to_owned()
}

pub(crate) fn parse_logging(
    value: Option<&serde_json::Value>,
) -> Result<Option<LoggingConfig>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Err(A2aLabError::invalid("logging", "must be a JSON object"));
    }
    let map = object_at("logging", value)?;
    reject_unknown_keys(
        map,
        "logging",
        &["level", "format", "filter", "opentelemetry"],
    )?;
    let level = parse_level(map.get("level"))?;
    let format = parse_format(map.get("format"))?;
    let filter = parse_optional_filter(map.get("filter"))?;
    let opentelemetry = parse_opentelemetry(map.get("opentelemetry"))?;
    Ok(Some(LoggingConfig {
        level,
        format,
        filter,
        opentelemetry,
    }))
}

fn parse_level(value: Option<&serde_json::Value>) -> Result<String, A2aLabError> {
    let Some(value) = value else {
        return Ok(DEFAULT_LOGGING_LEVEL.to_owned());
    };
    let Some(text) = value.as_str() else {
        return Err(A2aLabError::invalid("logging.level", "must be a string"));
    };
    if is_valid_level(text) {
        Ok(text.to_owned())
    } else {
        Err(A2aLabError::invalid(
            "logging.level",
            "unsupported log level",
        ))
    }
}

fn parse_format(value: Option<&serde_json::Value>) -> Result<LogOutputFormat, A2aLabError> {
    let Some(value) = value else {
        return Ok(DEFAULT_LOG_OUTPUT_FORMAT);
    };
    let Some(text) = value.as_str() else {
        return Err(A2aLabError::invalid("logging.format", "must be a string"));
    };
    match text {
        "text" => Ok(LogOutputFormat::Text),
        "json" => Ok(LogOutputFormat::Json),
        _ => Err(A2aLabError::invalid(
            "logging.format",
            "unsupported output format",
        )),
    }
}

fn parse_optional_filter(value: Option<&serde_json::Value>) -> Result<Option<String>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(text) = value.as_str() else {
        return Err(A2aLabError::invalid("logging.filter", "must be a string"));
    };
    if text.chars().any(|character| !character.is_whitespace()) {
        Ok(Some(text.to_owned()))
    } else {
        Err(A2aLabError::invalid(
            "logging.filter",
            "filter must contain non-whitespace text",
        ))
    }
}

fn parse_opentelemetry(
    value: Option<&serde_json::Value>,
) -> Result<Option<OpenTelemetryLoggingConfig>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Err(A2aLabError::invalid(
            "logging.opentelemetry",
            "must be a JSON object",
        ));
    }
    let map = object_at("logging.opentelemetry", value)?;
    reject_unknown_keys(map, "logging.opentelemetry", &["endpoint"])?;
    let endpoint = parse_endpoint(map.get("endpoint"))?;
    Ok(Some(OpenTelemetryLoggingConfig { endpoint }))
}

fn parse_endpoint(value: Option<&serde_json::Value>) -> Result<String, A2aLabError> {
    let Some(value) = value else {
        return Err(A2aLabError::invalid(
            "logging.opentelemetry.endpoint",
            "required field is missing",
        ));
    };
    let Some(text) = value.as_str() else {
        return Err(A2aLabError::invalid(
            "logging.opentelemetry.endpoint",
            "must be a string",
        ));
    };
    if text.chars().any(|character| !character.is_whitespace()) {
        Ok(text.to_owned())
    } else {
        Err(A2aLabError::invalid(
            "logging.opentelemetry.endpoint",
            "endpoint must contain non-whitespace text",
        ))
    }
}

fn is_valid_level(level: &str) -> bool {
    matches!(level, "trace" | "debug" | "info" | "warn" | "error" | "off")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_logging_field() {
        let error = parse_logging(Some(&serde_json::json!({"extra": true}))).unwrap_err();
        assert_eq!(error.code(), "invalid");
    }

    #[test]
    fn rejects_invalid_level_without_echoing_value() {
        let error = parse_logging(Some(&serde_json::json!({"level": "verbose"}))).unwrap_err();
        assert_eq!(error.code(), "invalid");
        assert!(!format!("{error}").contains("verbose"));
    }

    #[test]
    fn accepts_json_format() {
        let config = parse_logging(Some(&serde_json::json!({"format": "json"})))
            .unwrap()
            .unwrap();
        assert_eq!(config.format, LogOutputFormat::Json);
    }

    #[test]
    fn rejects_invalid_format_without_echoing_value() {
        let error = parse_logging(Some(&serde_json::json!({"format": "yaml"}))).unwrap_err();
        assert_eq!(error.code(), "invalid");
        assert!(!format!("{error}").contains("yaml"));
    }

    #[test]
    fn default_logging_config_uses_human_readable_text() {
        let config = LoggingConfig::default();
        assert_eq!(config.format, DEFAULT_LOG_OUTPUT_FORMAT);
        assert_eq!(config.format, LogOutputFormat::Text);
    }
}
