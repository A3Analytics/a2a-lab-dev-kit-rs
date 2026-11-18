//! Caller-supplied `a2a-lab.json` store with surgical, revision-aware writes.

use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::error::A2aLabError;

use super::strict_json::parse_strict_json;
use super::{A2aLabConfig, LoadedLabConfig};

/// How the attached configuration file can be persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigPersistence {
    /// No configuration path was supplied.
    Unconfigured,
    /// A file is attached but cannot be patched.
    ReadOnly,
    /// A writable file is attached.
    Writable,
}

impl ConfigPersistence {
    /// Stable persistence identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unconfigured => "unconfigured",
            Self::ReadOnly => "read_only",
            Self::Writable => "writable",
        }
    }
}

/// Failure from loading or patching a configuration file store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigStoreError {
    /// No configuration path is attached.
    Unconfigured,
    /// The attached file is not writable.
    ReadOnly,
    /// The supplied path cannot be used.
    Path {
        /// Human-readable explanation.
        message: String,
    },
    /// The process cannot read or replace the file.
    Permission {
        /// Human-readable explanation.
        message: String,
    },
    /// The supplied content-hash revision does not match the file.
    StaleRevision,
    /// The candidate document failed validation.
    Invalid(A2aLabError),
}

impl ConfigStoreError {
    /// Stable machine-readable kind, distinct across store failure classes.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Unconfigured => "unconfigured",
            Self::ReadOnly => "read_only",
            Self::Path { .. } => "path",
            Self::Permission { .. } => "permission",
            Self::StaleRevision => "stale_revision",
            Self::Invalid(_) => "invalid",
        }
    }

    fn path(message: impl Into<String>) -> Self {
        Self::Path {
            message: message.into(),
        }
    }

    fn permission(message: impl Into<String>) -> Self {
        Self::Permission {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ConfigStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unconfigured => formatter.write_str("configuration store is not attached"),
            Self::ReadOnly => formatter.write_str("configuration file is read-only"),
            Self::Path { message } | Self::Permission { message } => formatter.write_str(message),
            Self::StaleRevision => formatter.write_str("configuration revision is stale"),
            Self::Invalid(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ConfigStoreError {}

/// Loaded file contents plus persistence metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabConfigSnapshot {
    /// Attached-file persistence state.
    pub persistence: ConfigPersistence,
    /// SHA-256 of the current file bytes, when a file is attached.
    pub revision: Option<String>,
    /// Original JSON document. Omitted optional keys stay omitted.
    pub document: Value,
    /// Parsed envelope and resolved storage paths, when a file is attached.
    pub loaded: Option<LoadedLabConfig>,
}

/// Caller-supplied configuration file store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabConfigStore {
    path: Option<PathBuf>,
}

impl LabConfigStore {
    /// Viewing-only store with no attached file.
    #[must_use]
    pub const fn unconfigured() -> Self {
        Self { path: None }
    }

    /// Attaches `path` after reading and validating the current document.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ConfigStoreError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(ConfigStoreError::path("path must not be empty"));
        }
        let store = Self {
            path: Some(path.to_path_buf()),
        };
        store.snapshot()?;
        Ok(store)
    }

    /// Creates a minimal valid configuration when `path` is absent, then opens it.
    pub fn open_or_create(path: impl AsRef<Path>) -> Result<Self, ConfigStoreError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(ConfigStoreError::path("path must not be empty"));
        }

        match OpenOptions::new().create_new(true).write(true).open(path) {
            Ok(mut file) => {
                file.write_all(
                    br#"{"schema_version":1}
"#,
                )
                .map_err(|error| ConfigStoreError::permission(error.to_string()))?;
                file.sync_all()
                    .map_err(|error| ConfigStoreError::permission(error.to_string()))?;
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(match error.kind() {
                    ErrorKind::NotFound | ErrorKind::InvalidInput => {
                        ConfigStoreError::path(error.to_string())
                    }
                    ErrorKind::PermissionDenied => ConfigStoreError::permission(error.to_string()),
                    _ => ConfigStoreError::path(error.to_string()),
                });
            }
        }

        Self::open(path)
    }

    /// Current persistence, original document, and content-hash revision.
    pub fn snapshot(&self) -> Result<LabConfigSnapshot, ConfigStoreError> {
        let Some(path) = self.path.as_deref() else {
            return Ok(unconfigured_snapshot());
        };
        let (bytes, persistence) = read_attached(path)?;
        loaded_snapshot(path, &bytes, persistence)
    }

    /// Applies surgical JSON updates when `revision` matches the current file.
    pub fn patch(
        &self,
        revision: &str,
        changes: &Value,
    ) -> Result<LabConfigSnapshot, ConfigStoreError> {
        self.commit_patch(&self.propose_patch(revision, changes)?)
    }

    /// Validates a surgical patch without writing.
    pub(crate) fn propose_patch(
        &self,
        revision: &str,
        changes: &Value,
    ) -> Result<ProposedConfigPatch, ConfigStoreError> {
        let Some(path) = self.path.clone() else {
            return Err(ConfigStoreError::Unconfigured);
        };
        let (bytes, persistence) = read_attached(&path)?;
        if persistence == ConfigPersistence::ReadOnly {
            return Err(ConfigStoreError::ReadOnly);
        }
        if content_hash(&bytes) != revision {
            return Err(ConfigStoreError::StaleRevision);
        }
        let previous = original_document(&bytes)?;
        let mut document = previous.clone();
        apply_changes(&mut document, changes)?;
        let candidate = serde_json::to_vec(&document).map_err(|_| {
            ConfigStoreError::Invalid(A2aLabError::invalid("input", "must be valid JSON"))
        })?;
        let candidate_text = std::str::from_utf8(&candidate).map_err(|_| {
            ConfigStoreError::Invalid(A2aLabError::invalid("input", "must be UTF-8"))
        })?;
        A2aLabConfig::parse(candidate_text).map_err(ConfigStoreError::Invalid)?;
        let snapshot = loaded_snapshot(&path, &candidate, persistence)?;
        Ok(ProposedConfigPatch {
            path,
            current: bytes,
            candidate,
            previous,
            snapshot,
        })
    }

    /// Writes a previously validated patch when the file has not changed.
    pub(crate) fn commit_patch(
        &self,
        proposed: &ProposedConfigPatch,
    ) -> Result<LabConfigSnapshot, ConfigStoreError> {
        let (bytes, persistence) = read_attached(&proposed.path)?;
        if persistence == ConfigPersistence::ReadOnly {
            return Err(ConfigStoreError::ReadOnly);
        }
        if bytes != proposed.current {
            return Err(ConfigStoreError::StaleRevision);
        }
        if proposed.candidate != proposed.current {
            atomic_replace(&proposed.path, &proposed.candidate)?;
        }
        self.snapshot()
    }
}

/// Validated patch waiting for an atomic file replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProposedConfigPatch {
    path: PathBuf,
    current: Vec<u8>,
    candidate: Vec<u8>,
    #[cfg_attr(not(feature = "web-ui"), allow(dead_code))]
    previous: Value,
    #[cfg_attr(not(feature = "web-ui"), allow(dead_code))]
    snapshot: LabConfigSnapshot,
}

impl ProposedConfigPatch {
    /// Candidate snapshot after the surgical merge, before writing.
    #[cfg_attr(not(feature = "web-ui"), allow(dead_code))]
    pub(crate) fn snapshot(&self) -> &LabConfigSnapshot {
        &self.snapshot
    }

    /// Whether persisted live cadence fields changed.
    #[cfg_attr(not(feature = "web-ui"), allow(dead_code))]
    pub(crate) fn live_cadence_changed(&self) -> bool {
        live_cadence(&self.previous) != live_cadence(&self.snapshot.document)
    }
}

#[cfg_attr(not(feature = "web-ui"), allow(dead_code))]
fn live_cadence(document: &Value) -> (Value, Value) {
    (
        document
            .pointer("/videos/fps")
            .cloned()
            .unwrap_or(Value::Null),
        document
            .pointer("/videos/dvr/save_interval")
            .cloned()
            .unwrap_or(Value::Null),
    )
}

fn unconfigured_snapshot() -> LabConfigSnapshot {
    LabConfigSnapshot {
        persistence: ConfigPersistence::Unconfigured,
        revision: None,
        document: Value::Object(Map::new()),
        loaded: None,
    }
}

fn read_attached(path: &Path) -> Result<(Vec<u8>, ConfigPersistence), ConfigStoreError> {
    if path.as_os_str().is_empty() {
        return Err(ConfigStoreError::path("path must not be empty"));
    }
    let metadata = fs::metadata(path).map_err(|error| classify_io(&error))?;
    if metadata.is_dir() {
        return Err(ConfigStoreError::path("path must be a file"));
    }
    let persistence = if metadata.permissions().readonly() {
        ConfigPersistence::ReadOnly
    } else {
        ConfigPersistence::Writable
    };
    let bytes = fs::read(path).map_err(|error| classify_io(&error))?;
    Ok((bytes, persistence))
}

fn loaded_snapshot(
    path: &Path,
    bytes: &[u8],
    persistence: ConfigPersistence,
) -> Result<LabConfigSnapshot, ConfigStoreError> {
    let document = original_document(bytes)?;
    let text =
        std::str::from_utf8(bytes).map_err(|_| ConfigStoreError::path("file must be UTF-8"))?;
    let config = A2aLabConfig::parse(text).map_err(ConfigStoreError::Invalid)?;
    Ok(LabConfigSnapshot {
        persistence,
        revision: Some(content_hash(bytes)),
        document,
        loaded: Some(LoadedLabConfig::bind(path, config)),
    })
}

fn original_document(bytes: &[u8]) -> Result<Value, ConfigStoreError> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| ConfigStoreError::path("file must be UTF-8"))?;
    parse_strict_json(text).map_err(ConfigStoreError::Invalid)
}

fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    format!("sha256:{hex}")
}

fn apply_changes(document: &mut Value, changes: &Value) -> Result<(), ConfigStoreError> {
    let Value::Object(target) = document else {
        return Err(ConfigStoreError::Invalid(A2aLabError::invalid(
            "input",
            "must be a JSON object",
        )));
    };
    let Value::Object(patch) = changes else {
        return Err(ConfigStoreError::Invalid(A2aLabError::invalid(
            "input",
            "must be a JSON object",
        )));
    };
    merge_object(target, patch)
}

fn merge_object(
    target: &mut Map<String, Value>,
    patch: &Map<String, Value>,
) -> Result<(), ConfigStoreError> {
    for (key, value) in patch {
        if value.is_null() {
            target.remove(key);
            continue;
        }
        match target.get_mut(key) {
            Some(Value::Object(nested)) if value.is_object() => {
                let Some(patch_object) = value.as_object() else {
                    return Err(ConfigStoreError::Invalid(A2aLabError::invalid(
                        "input",
                        "must be a JSON object",
                    )));
                };
                merge_object(nested, patch_object)?;
            }
            Some(Value::Array(items)) if value.is_array() => {
                let Some(patch_items) = value.as_array() else {
                    return Err(ConfigStoreError::Invalid(A2aLabError::invalid(
                        "input",
                        "must be a JSON array",
                    )));
                };
                merge_array(items, patch_items)?;
            }
            Some(existing) => *existing = value.clone(),
            None => {
                target.insert(key.clone(), materialize(value)?);
            }
        }
    }
    Ok(())
}

fn merge_array(target: &mut Vec<Value>, patch: &[Value]) -> Result<(), ConfigStoreError> {
    for (index, value) in patch.iter().enumerate() {
        if value.is_null() {
            continue;
        }
        if index >= target.len() {
            target.push(materialize(value)?);
            continue;
        }
        match (&mut target[index], value) {
            (Value::Object(existing), Value::Object(nested)) => merge_object(existing, nested)?,
            (slot, _) => *slot = value.clone(),
        }
    }
    Ok(())
}

fn materialize(value: &Value) -> Result<Value, ConfigStoreError> {
    let Some(object) = value.as_object() else {
        return Ok(value.clone());
    };
    let mut nested = Map::new();
    merge_object(&mut nested, object)?;
    Ok(Value::Object(nested))
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), ConfigStoreError> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let directory = directory.unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| ConfigStoreError::path("path must be a file"))?;
    let tmp = directory.join(format!(
        ".{}.tmp-{}",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    let written = write_temp(&tmp, bytes)
        .and_then(|()| fs::rename(&tmp, path).map_err(|error| classify_io(&error)));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

fn write_temp(path: &Path, bytes: &[u8]) -> Result<(), ConfigStoreError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| classify_io(&error))?;
    file.write_all(bytes).map_err(|error| classify_io(&error))?;
    file.sync_all().map_err(|error| classify_io(&error))
}

fn classify_io(error: &io::Error) -> ConfigStoreError {
    match error.kind() {
        ErrorKind::NotFound | ErrorKind::NotADirectory | ErrorKind::InvalidInput => {
            ConfigStoreError::path("file is missing or path is invalid")
        }
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => {
            ConfigStoreError::permission("permission denied")
        }
        ErrorKind::AlreadyExists => ConfigStoreError::path("temporary write file already exists"),
        ErrorKind::IsADirectory => ConfigStoreError::path("path must be a file"),
        _ => ConfigStoreError::permission("file could not be replaced"),
    }
}
