//! Settings document parse, core drafts, and surgical PATCH bodies.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value, json};

pub const SCHEMA_VERSION: &str = "schema_version";
pub const CHAT_PROVIDER: &str = "chat.provider";
pub const CHAT_MODEL: &str = "chat.model";
pub const LOGGING_ENABLED: &str = "logging";
pub const LOGGING_LEVEL: &str = "logging.level";
pub const LOGGING_FORMAT: &str = "logging.format";
pub const LOGGING_FILTER: &str = "logging.filter";
pub const LOGGING_OTEL: &str = "logging.opentelemetry.endpoint";

const DEFAULT_LEVEL: &str = "info";
const DEFAULT_FORMAT: &str = "text";
const PROVIDERS: &[&str] = &["bedrock", "openai", "anthropic"];
const LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error", "off"];
const FORMATS: &[&str] = &["text", "json"];

pub type FieldErrors = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CaptureSettingsView {
    pub sources: Vec<SourceRow>,
    pub configured_videos_fps: Option<u32>,
    pub live_output_fps: u32,
    pub live_retention_secs: Option<u64>,
    pub dvr_save_interval: Option<String>,
    pub dvr_retention_secs: Option<u64>,
    pub dvr_storage_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum CameraIdRow {
    Index(u32),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SourceRow {
    pub id: String,
    pub display_name: String,
    pub format: String,
    pub camera_id: Option<CameraIdRow>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub capture_rate: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsSnapshot {
    pub document: Value,
    pub revision: Option<String>,
    pub persistence: String,
    pub live: Option<CaptureSettingsView>,
    pub pending_restart: Vec<String>,
    pub pending_reindex: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDraft {
    pub schema_version: String,
    pub chat_provider: String,
    pub chat_model: String,
    pub logging_enabled: bool,
    pub logging_level: String,
    pub logging_format: String,
    pub logging_filter: String,
    pub logging_otel: String,
}

impl CoreDraft {
    pub fn empty() -> Self {
        Self {
            schema_version: String::new(),
            chat_provider: String::new(),
            chat_model: String::new(),
            logging_enabled: false,
            logging_level: DEFAULT_LEVEL.to_owned(),
            logging_format: DEFAULT_FORMAT.to_owned(),
            logging_filter: String::new(),
            logging_otel: String::new(),
        }
    }
}

pub fn parse_snapshot(body: &str) -> Result<SettingsSnapshot, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "Could not read capture settings.".to_owned())?;
    let persistence = value
        .get("persistence")
        .and_then(Value::as_str)
        .unwrap_or("unconfigured")
        .to_owned();
    let revision = value
        .get("revision")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    let document = value.get("document").cloned().unwrap_or_else(|| json!({}));
    let live = value
        .get("live")
        .cloned()
        .and_then(|live| serde_json::from_value(live).ok());
    let pending_restart = string_list(value.get("pending_restart"));
    let pending_reindex = string_list(value.get("pending_reindex"));
    Ok(SettingsSnapshot {
        document,
        revision,
        persistence,
        live,
        pending_restart,
        pending_reindex,
    })
}

pub fn draft_from_document(document: &Value) -> CoreDraft {
    let chat = document.get("chat");
    let logging = document.get("logging");
    CoreDraft {
        schema_version: number_text(document.get(SCHEMA_VERSION)),
        chat_provider: string_text(chat.and_then(|value| value.get("provider"))),
        chat_model: string_text(chat.and_then(|value| value.get("model"))),
        logging_enabled: logging.is_some(),
        logging_level: string_or(logging.and_then(|value| value.get("level")), DEFAULT_LEVEL),
        logging_format: string_or(
            logging.and_then(|value| value.get("format")),
            DEFAULT_FORMAT,
        ),
        logging_filter: string_text(logging.and_then(|value| value.get("filter"))),
        logging_otel: string_text(
            logging
                .and_then(|value| value.get("opentelemetry"))
                .and_then(|value| value.get("endpoint")),
        ),
    }
}

pub fn patch_envelope(revision: &str, changes: &Value) -> Value {
    json!({ "revision": revision, "changes": changes })
}

pub fn surgical_core_changes(original: &Value, draft: &CoreDraft) -> Result<Value, FieldErrors> {
    let errors = validate_draft(draft);
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut changes = Map::new();
    schema_change(original, draft, &mut changes);
    chat_change(original, draft, &mut changes);
    logging_change(original, draft, &mut changes);
    Ok(Value::Object(changes))
}

pub fn validate_draft(draft: &CoreDraft) -> FieldErrors {
    let mut errors = FieldErrors::new();
    if !draft.schema_version.is_empty()
        && draft.schema_version != "1"
        && draft.schema_version != "2"
    {
        errors.insert(
            SCHEMA_VERSION.to_owned(),
            "schema_version must be 1 or 2.".to_owned(),
        );
    }
    let provider = draft.chat_provider.trim();
    if !provider.is_empty() && !PROVIDERS.contains(&provider) {
        errors.insert(
            CHAT_PROVIDER.to_owned(),
            "chat.provider is not a supported provider.".to_owned(),
        );
    }
    if !draft.chat_model.is_empty() && draft.chat_model.trim().is_empty() {
        errors.insert(
            CHAT_MODEL.to_owned(),
            "chat.model must contain non-whitespace text.".to_owned(),
        );
    }
    if draft.logging_enabled {
        if !LEVELS.contains(&draft.logging_level.as_str()) {
            errors.insert(
                LOGGING_LEVEL.to_owned(),
                "logging.level is not a supported log level.".to_owned(),
            );
        }
        if !FORMATS.contains(&draft.logging_format.as_str()) {
            errors.insert(
                LOGGING_FORMAT.to_owned(),
                "logging.format is not a supported output format.".to_owned(),
            );
        }
        reject_blank(
            &mut errors,
            LOGGING_FILTER,
            &draft.logging_filter,
            "logging.filter must contain non-whitespace text.",
        );
        reject_blank(
            &mut errors,
            LOGGING_OTEL,
            &draft.logging_otel,
            "logging.opentelemetry.endpoint must contain non-whitespace text.",
        );
    }
    errors
}

pub fn restart_required(path: &str) -> bool {
    if path == "videos.fps" || path == "videos.dvr.save_interval" {
        return false;
    }
    path == SCHEMA_VERSION
        || path.starts_with("chat.")
        || path == "chat"
        || path.starts_with("logging.")
        || path == "logging"
        || path == "videos"
        || path.starts_with("videos.")
        || path == "semantic_indexing"
        || path.starts_with("semantic_indexing.")
}

pub fn parse_fps_text(text: &str) -> Result<u32, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("videos.fps must not be blank.".to_owned());
    }
    if trimmed.starts_with('-') || trimmed.contains('.') {
        return Err("videos.fps must be a positive whole number.".to_owned());
    }
    if !trimmed.chars().all(|character| character.is_ascii_digit()) {
        return Err("videos.fps must be a positive whole number.".to_owned());
    }
    let fps = trimmed
        .parse::<u32>()
        .map_err(|_| "videos.fps must be a positive whole number.".to_owned())?;
    if fps == 0 {
        return Err("videos.fps must be greater than zero.".to_owned());
    }
    Ok(fps)
}

pub fn field_error_from_message(message: &str) -> Option<(String, String)> {
    let message = message.trim();
    let rest = message.strip_prefix("invalid ")?;
    let (field, detail) = rest.split_once(": ")?;
    if field.is_empty() || field == "input" {
        return None;
    }
    Some((field.to_owned(), format!("invalid {field}: {detail}")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsWriteError {
    Stale(String),
    Field { field: String, message: String },
    Message(String),
}

pub fn classify_settings_write(
    status: u16,
    body: &str,
) -> Result<SettingsSnapshot, SettingsWriteError> {
    if (200..300).contains(&status) {
        return parse_snapshot(body).map_err(SettingsWriteError::Message);
    }
    if status == 409 {
        return Err(SettingsWriteError::Stale(
            api_error_message(body).unwrap_or_else(|| "configuration revision is stale".to_owned()),
        ));
    }
    if status == 400 {
        let message =
            api_error_message(body).unwrap_or_else(|| "Settings were rejected.".to_owned());
        return Err(match field_error_from_message(&message) {
            Some((field, detail)) => SettingsWriteError::Field {
                field,
                message: detail,
            },
            None => SettingsWriteError::Message(message),
        });
    }
    if status == 403 {
        return Err(SettingsWriteError::Message(
            api_error_message(body).unwrap_or_else(|| "Settings cannot be saved.".to_owned()),
        ));
    }
    Err(SettingsWriteError::Message(
        "Could not save settings.".to_owned(),
    ))
}

fn api_error_message(body: &str) -> Option<String> {
    serde_json::from_str::<Value>(body).ok().and_then(|value| {
        value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

pub fn persistence_message(persistence: &str) -> Option<&'static str> {
    match persistence {
        "unconfigured" => Some("Configuration file is not attached. Settings cannot be saved."),
        "read_only" => Some("Configuration file is read-only."),
        _ => None,
    }
}

pub fn writable(persistence: &str) -> bool {
    persistence == "writable"
}

fn schema_change(original: &Value, draft: &CoreDraft, changes: &mut Map<String, Value>) {
    if draft.schema_version.is_empty() {
        return;
    }
    let current = number_text(original.get(SCHEMA_VERSION));
    if current == draft.schema_version {
        return;
    }
    let version: u32 = draft.schema_version.parse().unwrap_or(0);
    changes.insert(SCHEMA_VERSION.to_owned(), json!(version));
}

fn chat_change(original: &Value, draft: &CoreDraft, changes: &mut Map<String, Value>) {
    let chat = original.get("chat");
    let mut nested = Map::new();
    push_optional(
        &mut nested,
        "provider",
        chat.and_then(|value| value.get("provider"))
            .and_then(Value::as_str),
        draft.chat_provider.trim(),
    );
    push_optional(
        &mut nested,
        "model",
        chat.and_then(|value| value.get("model"))
            .and_then(Value::as_str),
        draft.chat_model.trim(),
    );
    if !nested.is_empty() {
        changes.insert("chat".to_owned(), Value::Object(nested));
    }
}

fn logging_change(original: &Value, draft: &CoreDraft, changes: &mut Map<String, Value>) {
    let logging = original.get("logging");
    if !draft.logging_enabled {
        if logging.is_some() {
            changes.insert("logging".to_owned(), Value::Null);
        }
        return;
    }
    let mut nested = Map::new();
    push_defaulted(
        &mut nested,
        "level",
        logging
            .and_then(|value| value.get("level"))
            .and_then(Value::as_str),
        draft.logging_level.trim(),
        DEFAULT_LEVEL,
        logging.is_some(),
    );
    push_defaulted(
        &mut nested,
        "format",
        logging
            .and_then(|value| value.get("format"))
            .and_then(Value::as_str),
        draft.logging_format.trim(),
        DEFAULT_FORMAT,
        logging.is_some(),
    );
    push_optional(
        &mut nested,
        "filter",
        logging
            .and_then(|value| value.get("filter"))
            .and_then(Value::as_str),
        draft.logging_filter.trim(),
    );
    otel_change(logging, draft, &mut nested);
    if logging.is_none() || !nested.is_empty() {
        changes.insert("logging".to_owned(), Value::Object(nested));
    }
}

fn otel_change(logging: Option<&Value>, draft: &CoreDraft, nested: &mut Map<String, Value>) {
    let current = logging
        .and_then(|value| value.get("opentelemetry"))
        .and_then(|value| value.get("endpoint"))
        .and_then(Value::as_str);
    let next = draft.logging_otel.trim();
    match (current, next) {
        (None, "") => {}
        (Some(current), next) if current == next => {}
        (Some(_), "") => {
            nested.insert("opentelemetry".to_owned(), Value::Null);
        }
        (_, next) => {
            nested.insert("opentelemetry".to_owned(), json!({ "endpoint": next }));
        }
    }
}

fn push_optional(target: &mut Map<String, Value>, key: &str, current: Option<&str>, next: &str) {
    match (current, next) {
        (None, "") => {}
        (Some(current), next) if current == next => {}
        (Some(_), "") => {
            target.insert(key.to_owned(), Value::Null);
        }
        (_, next) => {
            target.insert(key.to_owned(), json!(next));
        }
    }
}

fn push_defaulted(
    target: &mut Map<String, Value>,
    key: &str,
    current: Option<&str>,
    next: &str,
    default: &str,
    present: bool,
) {
    match (current, next) {
        (None, next) if next == default => {
            if !present {
                // Creating logging with defaults leaves omitted keys omitted.
            }
        }
        (Some(current), next) if current == next => {}
        (_, next) => {
            target.insert(key.to_owned(), json!(next));
        }
    }
}

pub(super) fn reject_blank(errors: &mut FieldErrors, field: &str, value: &str, message: &str) {
    if !value.is_empty() && !value.chars().any(|character| !character.is_whitespace()) {
        errors.insert(field.to_owned(), message.to_owned());
    }
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn number_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::String(text)) => text.clone(),
        _ => String::new(),
    }
}

fn string_text(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn string_or(value: Option<&Value>, fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINAL: &str = r#"{
        "document": {
            "schema_version": 1,
            "chat": {"provider": "openai", "model": "keep-me"},
            "videos": {"fps": 5, "sources": [{"id": "camera", "format": "yuyv"}]}
        },
        "revision": "sha256:abc",
        "persistence": "writable",
        "live": {
            "sources": [],
            "configured_videos_fps": 5,
            "live_output_fps": 5,
            "live_retention_secs": null,
            "dvr_save_interval": null,
            "dvr_retention_secs": null,
            "dvr_storage_path": null
        },
        "pending_restart": ["chat.model"],
        "pending_reindex": []
    }"#;

    #[test]
    fn parse_snapshot_keeps_omitted_logging_and_states() {
        let snapshot = parse_snapshot(ORIGINAL).unwrap();
        assert_eq!(snapshot.persistence, "writable");
        assert_eq!(snapshot.revision.as_deref(), Some("sha256:abc"));
        assert!(snapshot.document.get("logging").is_none());
        assert_eq!(snapshot.pending_restart, vec!["chat.model"]);
        assert_eq!(snapshot.pending_reindex, Vec::<String>::new());
        assert_eq!(snapshot.live.unwrap().live_output_fps, 5);
    }

    #[test]
    fn surgical_patch_omits_unchanged_optional_keys() {
        let original = json!({
            "schema_version": 1,
            "chat": {"provider": "openai", "model": "keep-me"},
            "videos": {"fps": 5}
        });
        let mut draft = draft_from_document(&original);
        draft.chat_model = "other".to_owned();
        let changes = surgical_core_changes(&original, &draft).unwrap();
        assert_eq!(changes, json!({ "chat": { "model": "other" } }));
        assert!(changes.get("schema_version").is_none());
        assert!(changes.get("videos").is_none());
        assert!(changes.get("logging").is_none());
        assert!(changes["chat"].get("provider").is_none());
    }

    #[test]
    fn enabling_logging_with_defaults_omits_default_keys() {
        let original = json!({ "schema_version": 1 });
        let mut draft = draft_from_document(&original);
        draft.logging_enabled = true;
        let changes = surgical_core_changes(&original, &draft).unwrap();
        assert_eq!(changes, json!({ "logging": {} }));
    }

    #[test]
    fn chat_and_logging_paths_require_restart() {
        assert!(restart_required(CHAT_MODEL));
        assert!(restart_required(LOGGING_LEVEL));
        assert!(restart_required(SCHEMA_VERSION));
        assert!(!restart_required("videos.fps"));
        assert!(!restart_required("videos.dvr.save_interval"));
        assert!(restart_required("videos.dvr.storage_path"));
        assert!(restart_required("videos.sources.0.name"));
        assert!(restart_required("semantic_indexing.description.model"));
        assert!(restart_required("semantic_indexing.decisions.provider"));
        assert!(restart_required(
            "semantic_indexing.embeddings.credentials.env"
        ));
    }

    #[test]
    fn validation_errors_do_not_build_a_patch() {
        let original = json!({ "schema_version": 1 });
        let mut draft = draft_from_document(&original);
        draft.schema_version = "9".to_owned();
        draft.chat_provider = "unknown".to_owned();
        let errors = surgical_core_changes(&original, &draft).unwrap_err();
        assert!(errors.contains_key(SCHEMA_VERSION));
        assert!(errors.contains_key(CHAT_PROVIDER));
    }

    #[test]
    fn unconfigured_and_read_only_have_visible_messages() {
        assert!(
            persistence_message("unconfigured")
                .unwrap()
                .contains("not attached")
        );
        assert!(
            persistence_message("read_only")
                .unwrap()
                .contains("read-only")
        );
        assert!(persistence_message("writable").is_none());
        assert!(!writable("unconfigured"));
    }

    #[test]
    fn fps_and_envelope_stay_surgical() {
        assert_eq!(parse_fps_text(" 12 ").unwrap(), 12);
        assert!(parse_fps_text("0").is_err());
        let body = patch_envelope("sha256:abc", &json!({ "videos": { "fps": 12 } }));
        assert_eq!(
            body,
            json!({ "revision": "sha256:abc", "changes": { "videos": { "fps": 12 } } })
        );
    }

    #[test]
    fn server_field_errors_map_to_inline_paths() {
        let mapped =
            field_error_from_message("invalid chat.model: model must contain non-whitespace text")
                .unwrap();
        assert_eq!(mapped.0, CHAT_MODEL);
        assert!(field_error_from_message("configuration revision is stale").is_none());
    }

    #[test]
    fn write_classifier_splits_conflict_and_inline_errors() {
        let conflict = classify_settings_write(
            409,
            r#"{"error":{"code":"stale_revision","message":"configuration revision is stale"}}"#,
        );
        assert!(matches!(
            conflict,
            Err(SettingsWriteError::Stale(message)) if message == "configuration revision is stale"
        ));
        let inline = classify_settings_write(
            400,
            r#"{"error":{"code":"invalid","message":"invalid videos.fps: videos.fps must be greater than zero"}}"#,
        );
        assert!(matches!(
            inline,
            Err(SettingsWriteError::Field { field, message })
                if field == "videos.fps" && message.contains("greater than zero")
        ));
        let locked = classify_settings_write(
            403,
            r#"{"error":{"code":"read_only","message":"configuration file is read-only"}}"#,
        );
        assert!(matches!(
            locked,
            Err(SettingsWriteError::Message(message)) if message.contains("read-only")
        ));
    }

    #[test]
    fn representative_live_restart_and_reindex_labels() {
        assert!(!restart_required("videos.fps"));
        assert!(!restart_required("videos.dvr.save_interval"));
        assert!(restart_required(CHAT_MODEL));
        assert!(restart_required(LOGGING_LEVEL));
        assert!(restart_required("semantic_indexing.description.model"));
        assert!(restart_required(
            "videos.sources.0.semantic_indexing.description_model"
        ));
        let snapshot = parse_snapshot(
            r#"{
                "document": {},
                "revision": "sha256:abc",
                "persistence": "writable",
                "live": {
                    "sources": [],
                    "live_output_fps": 5
                },
                "pending_restart": ["semantic_indexing.description.model"],
                "pending_reindex": ["semantic_indexing.description.model"]
            }"#,
        )
        .unwrap();
        assert_eq!(snapshot.pending_restart, snapshot.pending_reindex);
        assert_eq!(
            snapshot.pending_reindex,
            vec!["semantic_indexing.description.model"]
        );
    }
}
