//! Lifecycle and configuration events.

use tracing::{info, warn};

use crate::config::{A2aLabConfig, LoadedLabConfig, LogOutputFormat, LoggingConfig};

use super::redact::redact_field;

/// Emits one startup event when logging is initialized.
pub fn log_logging_ready(config: &LoggingConfig) {
    info!(
        target: "a2a_lab_dev_kit::lifecycle",
        level = %config.level,
        format = ?config.format,
        opentelemetry = config.opentelemetry.is_some(),
        event = "logging_initialized",
        "application logging initialized"
    );
}

/// Logs safe configuration selections without secrets.
pub fn log_effective_config(loaded: &LoadedLabConfig) {
    let summary = summarize_config(&loaded.config);
    info!(
        target: "a2a_lab_dev_kit::lifecycle",
        schema_version = loaded.config.schema_version,
        summary,
        dvr_storage_configured = loaded.dvr_storage_path.is_some(),
        semantic_vector_index_configured = loaded.semantic_vector_index_path.is_some(),
        event = "configuration_selected",
        "effective configuration loaded"
    );
}

/// Logs adapter readiness without binding credentials.
pub fn log_server_ready(adapter: &str, address: &str) {
    info!(
        target: "a2a_lab_dev_kit::lifecycle",
        adapter,
        address = %redact_field("address", address),
        event = "server_ready",
        "server ready"
    );
}

/// Logs orderly shutdown when the logging guard is dropped.
pub fn log_shutdown() {
    info!(
        target: "a2a_lab_dev_kit::lifecycle",
        event = "logging_shutdown",
        "application logging shutdown"
    );
}

/// Warns when logging was already initialized.
pub fn log_duplicate_init_attempt() {
    warn!(
        target: "a2a_lab_dev_kit::lifecycle",
        event = "logging_duplicate_init",
        "application logging already initialized"
    );
}

fn summarize_config(config: &A2aLabConfig) -> String {
    let mut parts = Vec::new();
    if let Some(chat) = &config.chat {
        if let Some(provider) = chat.provider {
            parts.push(format!("chat.provider={}", provider.as_str()));
        }
        if chat.model.is_some() {
            parts.push("chat.model=<redacted>".to_owned());
        }
    }
    if config.videos.is_some() {
        parts.push("videos=present".to_owned());
    }
    if config.semantic_indexing.is_some() {
        parts.push("semantic_indexing=present".to_owned());
    }
    if let Some(logging) = &config.logging {
        parts.push(format!(
            "logging.level={};logging.format={}",
            logging.level,
            logging_format(logging.format)
        ));
    }
    parts.join(",")
}

const fn logging_format(format: LogOutputFormat) -> &'static str {
    match format {
        LogOutputFormat::Text => "text",
        LogOutputFormat::Json => "json",
    }
}
