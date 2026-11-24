//! Explicit configuration-file loading. Callers choose the path.

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::A2aLabError;
use crate::layered_config::{Config, File, FileFormat};

use super::A2aLabConfig;
use super::strict_json::parse_strict_json;

const TYPESAFE_API_KEY: &str = "TYPESAFE_API_KEY";
const LAYER_KEYS: &[&str] = &[
    "schema_version",
    "chat",
    "videos",
    "semantic_indexing",
    "image_decision",
    "logging",
    "typesafe",
];

/// Validated document plus file-relative storage paths, when set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedLabConfig {
    /// Parsed document. Omitted fields stay omitted.
    pub config: A2aLabConfig,
    /// `videos.dvr.storage_path` resolved from the selected file's directory.
    pub dvr_storage_path: Option<PathBuf>,
    /// `semantic_indexing.limits.storage.vector_index_path` resolved from the file directory.
    pub semantic_vector_index_path: Option<PathBuf>,
    /// `semantic_indexing.limits.storage.sqlite_path` resolved from the file directory.
    pub semantic_sqlite_path: Option<PathBuf>,
    typesafe: Option<TypeSafeCredential>,
}

/// Runtime TypeSafe key. Debug output never includes the secret.
#[derive(Clone, PartialEq, Eq)]
struct TypeSafeCredential {
    api_key: String,
}

impl fmt::Debug for TypeSafeCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TypeSafeCredential")
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl TypeSafeCredential {
    fn api_key(&self) -> &str {
        &self.api_key
    }
}

#[derive(Clone, Copy)]
enum LayerSource {
    Base,
    Secret,
}

#[derive(Clone, Copy)]
enum KeyOrigin {
    Base,
    Secret,
    Env,
}

impl LoadedLabConfig {
    /// Reads and validates `path`. Does not create storage or contact a service.
    ///
    /// Precedence is the base file, then its optional `<stem>.secret.<ext>` sibling,
    /// then `TYPESAFE_API_KEY`. Later sources override earlier values.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, A2aLabError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(A2aLabError::invalid("config", "path must not be empty"));
        }
        let bytes = std::fs::read(path)
            .map_err(|_| A2aLabError::invalid("config", "file is missing or unreadable"))?;
        Self::load_bytes(path, &bytes)
    }

    /// Validates `bytes` as the base document at `path`, then applies the sibling and env layers.
    pub(crate) fn load_bytes(path: &Path, bytes: &[u8]) -> Result<Self, A2aLabError> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| A2aLabError::invalid("config", "file must be UTF-8"))?;
        let base = parse_layer(text, LayerSource::Base)?;
        validate_layer(&base, LayerSource::Base)?;
        let secret = read_secret_layer(path)?;
        let env_key = read_typesafe_env()?;
        let origin = key_origin(&base, secret.as_ref(), env_key.as_deref());
        let mut merged = merge_layers(&base, secret.as_ref(), env_key.as_deref())?;
        let typesafe = take_typesafe_key(&mut merged, origin)?;
        let config = A2aLabConfig::from_value(&merged)?;
        Ok(Self::assemble(path, config, typesafe))
    }

    /// Layered TypeSafe key, when `typesafe.api_key` or `TYPESAFE_API_KEY` supplied one.
    #[must_use]
    pub fn typesafe_api_key(&self) -> Option<&str> {
        self.typesafe.as_ref().map(TypeSafeCredential::api_key)
    }

    /// Layered TypeSafe key, otherwise the image-decision `credentials.env` value.
    pub fn image_decision_api_key(&self) -> Result<Option<String>, A2aLabError> {
        if let Some(key) = self.typesafe_api_key() {
            return Ok(Some(key.to_owned()));
        }
        let Some(name) = self
            .config
            .image_decision
            .as_ref()
            .and_then(|client| client.credentials_env())
        else {
            return Ok(None);
        };
        read_named_env(name)
    }

    /// Layered TypeSafe key, otherwise the semantic-decision `credentials.env` value.
    pub fn semantic_decision_api_key(&self) -> Result<Option<String>, A2aLabError> {
        if let Some(key) = self.typesafe_api_key() {
            return Ok(Some(key.to_owned()));
        }
        let Some(name) = self.config.semantic_indexing.as_ref().and_then(|semantic| {
            semantic
                .decisions
                .credentials()
                .map(super::semantic_indexing::CredentialReference::env_var)
        }) else {
            return Ok(None);
        };
        read_named_env(name)
    }

    fn assemble(path: &Path, config: A2aLabConfig, typesafe: Option<TypeSafeCredential>) -> Self {
        let dvr_storage_path = config
            .videos
            .as_ref()
            .and_then(|videos| videos.dvr.as_ref())
            .and_then(|dvr| dvr.storage_path.as_deref())
            .map(|storage| resolve_relative_path(path, storage));
        let semantic_vector_index_path = config.semantic_indexing.as_ref().map(|semantic| {
            resolve_relative_path(path, &semantic.limits.storage.vector_index_path)
        });
        let semantic_sqlite_path = config
            .semantic_indexing
            .as_ref()
            .map(|semantic| resolve_relative_path(path, &semantic.limits.storage.sqlite_path));
        Self {
            config,
            dvr_storage_path,
            semantic_vector_index_path,
            semantic_sqlite_path,
            typesafe,
        }
    }
}

/// Removes `typesafe` from a document returned to settings or logs.
pub(crate) fn strip_typesafe_secret(document: &mut Value) {
    if let Some(map) = document.as_object_mut() {
        map.remove("typesafe");
    }
}

fn resolve_relative_path(config_file: &Path, storage: &str) -> PathBuf {
    let storage = Path::new(storage);
    if storage.is_absolute() {
        return storage.to_path_buf();
    }
    config_file.parent().unwrap_or(Path::new("")).join(storage)
}

fn secret_sibling(path: &Path) -> PathBuf {
    let mut name = path.file_stem().unwrap_or_default().to_os_string();
    name.push(".secret");
    if let Some(extension) = path.extension() {
        name.push(".");
        name.push(extension);
    }
    path.with_file_name(name)
}

fn parse_layer(text: &str, source: LayerSource) -> Result<Value, A2aLabError> {
    parse_strict_json(text).map_err(|error| match (source, error) {
        (LayerSource::Base, error) => error,
        (LayerSource::Secret, A2aLabError::Invalid { message, .. }) => {
            A2aLabError::invalid("config.secret", message)
        }
        (LayerSource::Secret, other) => other,
    })
}

fn read_secret_layer(path: &Path) -> Result<Option<Value>, A2aLabError> {
    let secret_path = secret_sibling(path);
    match std::fs::read(&secret_path) {
        Ok(bytes) => {
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| A2aLabError::invalid("config.secret", "file must be UTF-8"))?;
            let value = parse_layer(text, LayerSource::Secret)?;
            validate_layer(&value, LayerSource::Secret)?;
            Ok(Some(value))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(A2aLabError::invalid("config.secret", "file is unreadable")),
    }
}

fn validate_layer(value: &Value, source: LayerSource) -> Result<(), A2aLabError> {
    let Some(map) = value.as_object() else {
        return Err(layer_error(source, "must be a JSON object"));
    };
    for key in map.keys() {
        if !LAYER_KEYS.contains(&key.as_str()) {
            return Err(layer_error(source, "unknown field"));
        }
    }
    validate_typesafe(map.get("typesafe"), source)?;
    reject_known_nested(map, source)
}

fn reject_known_nested(
    map: &serde_json::Map<String, Value>,
    source: LayerSource,
) -> Result<(), A2aLabError> {
    if let Some(chat) = map.get("chat") {
        super::parse_chat(Some(chat)).map_err(|error| remap_layer(source, error))?;
    }
    if let Some(logging) = map.get("logging") {
        super::logging::parse_logging(Some(logging)).map_err(|error| remap_layer(source, error))?;
    }
    if let Some(image_decision) = map.get("image_decision") {
        super::image_decision::ImageDecisionClientConfig::parse_value(image_decision)
            .map_err(|error| remap_layer(source, error))?;
    }
    Ok(())
}

fn remap_layer(source: LayerSource, error: A2aLabError) -> A2aLabError {
    match source {
        LayerSource::Base => error,
        LayerSource::Secret => match error {
            A2aLabError::Invalid { message, .. } => A2aLabError::invalid("config.secret", message),
            other => other,
        },
    }
}

fn validate_typesafe(value: Option<&Value>, source: LayerSource) -> Result<(), A2aLabError> {
    let Some(value) = value else {
        return Ok(());
    };
    let Some(map) = value.as_object() else {
        return Err(layer_error(source, "typesafe must be a JSON object"));
    };
    for key in map.keys() {
        if key != "api_key" {
            return Err(layer_error(source, "unknown field"));
        }
    }
    match map.get("api_key") {
        None | Some(Value::String(_)) => Ok(()),
        Some(_) => Err(layer_error(source, "typesafe.api_key must be a string")),
    }
}

fn layer_error(source: LayerSource, message: &str) -> A2aLabError {
    let field = match source {
        LayerSource::Base => "input",
        LayerSource::Secret => "config.secret",
    };
    A2aLabError::invalid(field, message)
}

fn read_typesafe_env() -> Result<Option<String>, A2aLabError> {
    match std::env::var(TYPESAFE_API_KEY) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(A2aLabError::invalid(
            TYPESAFE_API_KEY,
            "must be valid UTF-8",
        )),
    }
}

fn read_named_env(name: &str) -> Result<Option<String>, A2aLabError> {
    match std::env::var(name) {
        Ok(value) if !is_blank(&value) => Ok(Some(value)),
        Ok(_) => Err(A2aLabError::invalid(
            "credentials.env",
            format!("{name} must not be empty"),
        )),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(A2aLabError::invalid(
            "credentials.env",
            format!("{name} must be valid UTF-8"),
        )),
    }
}

fn key_origin(base: &Value, secret: Option<&Value>, env_key: Option<&str>) -> Option<KeyOrigin> {
    if env_key.is_some() {
        return Some(KeyOrigin::Env);
    }
    if secret.is_some_and(contains_api_key) {
        return Some(KeyOrigin::Secret);
    }
    if contains_api_key(base) {
        return Some(KeyOrigin::Base);
    }
    None
}

fn contains_api_key(value: &Value) -> bool {
    value
        .get("typesafe")
        .and_then(Value::as_object)
        .is_some_and(|typesafe| typesafe.contains_key("api_key"))
}

fn merge_layers(
    base: &Value,
    secret: Option<&Value>,
    env_key: Option<&str>,
) -> Result<Value, A2aLabError> {
    let base_json = serde_json::to_string(base).map_err(|_| merge_error())?;
    let mut builder = Config::builder().add_source(File::from_str(&base_json, FileFormat::Json));
    if let Some(secret) = secret {
        let secret_json = serde_json::to_string(secret).map_err(|_| merge_error())?;
        builder = builder.add_source(File::from_str(&secret_json, FileFormat::Json));
    }
    if let Some(env_key) = env_key {
        builder = builder
            .set_override("typesafe.api_key", env_key)
            .map_err(|_| merge_error())?;
    }
    builder
        .build()
        .and_then(Config::try_deserialize)
        .map_err(|_| merge_error())
}

fn merge_error() -> A2aLabError {
    A2aLabError::invalid("config", "could not merge configuration")
}

fn take_typesafe_key(
    merged: &mut Value,
    origin: Option<KeyOrigin>,
) -> Result<Option<TypeSafeCredential>, A2aLabError> {
    let typesafe = merged
        .as_object_mut()
        .and_then(|map| map.remove("typesafe"));
    let Some(typesafe) = typesafe else {
        return Ok(None);
    };
    let Some(map) = typesafe.as_object() else {
        return Err(empty_or_type_error(origin));
    };
    match map.get("api_key") {
        Some(Value::String(text)) if !is_blank(text) => Ok(Some(TypeSafeCredential {
            api_key: text.clone(),
        })),
        Some(Value::String(_)) => Err(empty_key_error(origin)),
        Some(_) => Err(empty_or_type_error(origin)),
        None => Ok(None),
    }
}

fn empty_key_error(origin: Option<KeyOrigin>) -> A2aLabError {
    match origin {
        Some(KeyOrigin::Secret) => {
            A2aLabError::invalid("config.secret", "typesafe.api_key must not be empty")
        }
        Some(KeyOrigin::Env) => A2aLabError::invalid(TYPESAFE_API_KEY, "must not be empty"),
        Some(KeyOrigin::Base) | None => {
            A2aLabError::invalid("typesafe.api_key", "must not be empty")
        }
    }
}

fn empty_or_type_error(origin: Option<KeyOrigin>) -> A2aLabError {
    match origin {
        Some(KeyOrigin::Secret) => {
            A2aLabError::invalid("config.secret", "typesafe.api_key must be a string")
        }
        Some(KeyOrigin::Env) => A2aLabError::invalid(TYPESAFE_API_KEY, "must be a string"),
        Some(KeyOrigin::Base) | None => {
            A2aLabError::invalid("typesafe.api_key", "must be a string")
        }
    }
}

fn is_blank(text: &str) -> bool {
    text.chars().all(char::is_whitespace)
}
