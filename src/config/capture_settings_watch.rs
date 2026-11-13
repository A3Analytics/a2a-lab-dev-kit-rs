//! Live `VideoCaptureSettings` updates for capture threads.

use std::sync::{Arc, RwLock};

use serde_json::Value;
use tokio::sync::watch;

use crate::error::A2aLabError;

use super::VideoCaptureSettings;

/// Thread-safe settings with a tokio watch stream for non-blocking readers.
#[derive(Clone, Debug)]
pub struct VideoCaptureSettingsWatch {
    inner: Arc<RwLock<VideoCaptureSettings>>,
    updates: watch::Sender<VideoCaptureSettings>,
}

impl VideoCaptureSettingsWatch {
    /// Opens the watch at the current settings snapshot.
    #[must_use]
    pub fn new(settings: VideoCaptureSettings) -> Self {
        let (updates, _) = watch::channel(settings.clone());
        Self {
            inner: Arc::new(RwLock::new(settings)),
            updates,
        }
    }

    /// Current settings snapshot for HTTP handlers and tests.
    pub fn snapshot(&self) -> VideoCaptureSettings {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Live subscriber for capture loops without blocking on shell writes.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<VideoCaptureSettings> {
        self.updates.subscribe()
    }

    /// Replaces the active settings and notifies subscribers.
    pub fn replace(&self, settings: VideoCaptureSettings) {
        {
            let mut guard = self
                .inner
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *guard = settings.clone();
        }
        let _ = self.updates.send(settings);
    }

    /// Applies a shell patch and publishes the merged snapshot.
    pub fn apply_patch(&self, document: &Value) -> Result<(), A2aLabError> {
        let mut settings = self.snapshot();
        settings.apply_patch(document)?;
        self.replace(settings);
        Ok(())
    }
}
