//! Explicit configuration-file loading. Callers choose the path.

use std::path::{Path, PathBuf};

use crate::error::A2aLabError;

use super::A2aLabConfig;

/// Validated document plus the file-relative DVR directory, when one was set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedLabConfig {
    /// Parsed v1 document. Omitted fields stay omitted.
    pub config: A2aLabConfig,
    /// `videos.dvr.storage_path` resolved from the selected file's directory.
    pub dvr_storage_path: Option<PathBuf>,
}

impl LoadedLabConfig {
    /// Reads and validates `path`. Does not create storage or contact a service.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, A2aLabError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(A2aLabError::invalid("config", "path must not be empty"));
        }
        let bytes = std::fs::read(path)
            .map_err(|_| A2aLabError::invalid("config", "file is missing or unreadable"))?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| A2aLabError::invalid("config", "file must be UTF-8"))?;
        let config = A2aLabConfig::parse(text)?;
        let dvr_storage_path = config
            .videos
            .as_ref()
            .and_then(|videos| videos.dvr.as_ref())
            .and_then(|dvr| dvr.storage_path.as_deref())
            .map(|storage| resolve_storage_path(path, storage));
        Ok(Self {
            config,
            dvr_storage_path,
        })
    }
}

fn resolve_storage_path(config_file: &Path, storage: &str) -> PathBuf {
    let storage = Path::new(storage);
    if storage.is_absolute() {
        return storage.to_path_buf();
    }
    config_file.parent().unwrap_or(Path::new("")).join(storage)
}
