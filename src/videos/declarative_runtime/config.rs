//! Inputs for [`super::DeclarativeVideoRuntime::start`].

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::config::VideoConfig;
use crate::error::A2aLabError;
use crate::videos::{DvrPersistHook, SqliteChunkDvrStore};

/// Runs after the chunk DVR opens and before capture workers start.
#[async_trait]
pub trait DvrReady: Send + Sync {
    /// Initializes lab-specific DVR sidecars on the opened store.
    async fn on_dvr_opened(&self, store: &SqliteChunkDvrStore) -> Result<(), A2aLabError>;
}

/// Upper bound for one still or raw MJPEG buffer offered to encoders.
pub const DEFAULT_MAX_FRAME_BYTES: u64 = 8_000_000;

/// Declarative file config plus on-disk DVR root and optional hooks.
pub struct VideoRuntimeConfig {
    /// Operator video sources and shared cadence overrides.
    pub video: VideoConfig,
    /// SQLite chunk root for historical media.
    pub storage_root: PathBuf,
    /// Post-commit consumers such as semantic indexing tables.
    pub hooks: Vec<Arc<dyn DvrPersistHook>>,
    /// Bounded JPEG and raw frame bytes per capture pull.
    pub max_frame_bytes: u64,
    /// Prepared clip and follow TTL for the composed provider.
    pub clip_ttl: Duration,
    /// Optional hook after DVR open and before workers start.
    pub dvr_ready: Option<Arc<dyn DvrReady>>,
}

impl Clone for VideoRuntimeConfig {
    fn clone(&self) -> Self {
        Self {
            video: self.video.clone(),
            storage_root: self.storage_root.clone(),
            hooks: self.hooks.clone(),
            max_frame_bytes: self.max_frame_bytes,
            clip_ttl: self.clip_ttl,
            dvr_ready: self.dvr_ready.clone(),
        }
    }
}

impl VideoRuntimeConfig {
    /// Builds config with documented frame and clip defaults.
    #[must_use]
    pub fn new(video: VideoConfig, storage_root: impl Into<PathBuf>) -> Self {
        Self {
            video,
            storage_root: storage_root.into(),
            hooks: Vec::new(),
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            clip_ttl: Duration::from_secs(60),
            dvr_ready: None,
        }
    }

    /// Attaches a post-open initializer before workers start.
    #[must_use]
    pub fn with_dvr_ready(mut self, ready: Arc<dyn DvrReady>) -> Self {
        self.dvr_ready = Some(ready);
        self
    }

    /// Attaches DVR post-commit hooks before store open.
    #[must_use]
    pub fn with_hooks(mut self, hooks: Vec<Arc<dyn DvrPersistHook>>) -> Self {
        self.hooks = hooks;
        self
    }
}
