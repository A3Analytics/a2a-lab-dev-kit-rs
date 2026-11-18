//! Live `VideoCaptureSettings` updates for capture threads.

use std::sync::{Arc, RwLock};

use serde_json::Value;
use tokio::sync::watch;

use crate::error::A2aLabError;

use super::{VideoCaptureSettings, VideoConfig};

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

    /// Publishes persisted fps/save-interval cadence without other file fields.
    pub fn apply_persisted_cadence(&self, videos: Option<&VideoConfig>) -> Result<(), A2aLabError> {
        let mut settings = self.snapshot();
        let before = settings.clone();
        settings.apply_persisted_cadence(videos)?;
        if settings != before {
            self.replace(settings);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::VideoCaptureSettingsWatch;
    use crate::config::{
        CameraSelector, ConfigDuration, SaveIntervalDuration, VideoCaptureSettings, VideoConfig,
        VideoDvrConfig, VideoInputFormat, VideoSourceConfig,
    };
    use crate::id::VideoSourceId;
    use crate::videos::SENSOR_30FPS_PERIOD_NS;

    fn sample_settings() -> VideoCaptureSettings {
        let config = VideoConfig {
            sources: vec![VideoSourceConfig {
                id: VideoSourceId::new("bench").unwrap(),
                name: Some("Bench".to_owned()),
                description: None,
                format: VideoInputFormat::Mjpeg,
                camera_id: Some(CameraSelector::Index(1)),
                width: Some(640),
                height: Some(480),
                capture_rate: Some(30),
                semantic_indexing: None,
            }],
            fps: Some(5),
            live_retention: ConfigDuration::parse("videos.live_retention", "30s").ok(),
            dvr: Some(VideoDvrConfig {
                retention: ConfigDuration::parse("videos.dvr.retention", "1h").ok(),
                save_interval: SaveIntervalDuration::parse("videos.dvr.save_interval", "1s").ok(),
                storage_path: Some("./dvr".to_owned()),
            }),
        };
        VideoCaptureSettings::from_video_config(Some(&config))
            .with_negotiated_sensor_period_ns(SENSOR_30FPS_PERIOD_NS)
    }

    #[test]
    fn persisted_cadence_notifies_subscribers() {
        let watch = VideoCaptureSettingsWatch::new(sample_settings());
        let mut rx = watch.subscribe();
        let _ = rx.borrow_and_update();
        let mut videos = VideoConfig {
            sources: Vec::new(),
            fps: Some(10),
            live_retention: None,
            dvr: None,
        };
        watch.apply_persisted_cadence(Some(&videos)).unwrap();
        assert!(rx.has_changed().unwrap());
        assert_eq!(watch.snapshot().live_output_fps(), 10);
        videos.fps = Some(5);
        videos.dvr = Some(VideoDvrConfig {
            retention: ConfigDuration::parse("videos.dvr.retention", "2h").ok(),
            save_interval: SaveIntervalDuration::parse("videos.dvr.save_interval", "2s").ok(),
            storage_path: Some("./other".to_owned()),
        });
        watch.apply_persisted_cadence(Some(&videos)).unwrap();
        let snapshot = watch.snapshot();
        assert_eq!(snapshot.live_output_fps(), 5);
        assert_eq!(
            snapshot
                .dvr_save_interval()
                .map(SaveIntervalDuration::as_text)
                .as_deref(),
            Some("2s")
        );
        assert_eq!(snapshot.view().dvr_retention_secs, Some(3_600));
        assert_eq!(snapshot.view().dvr_storage_path.as_deref(), Some("./dvr"));
    }

    #[test]
    fn invalid_persisted_cadence_leaves_watch_unchanged() {
        let watch = VideoCaptureSettingsWatch::new(sample_settings());
        let before = watch.snapshot();
        let videos = VideoConfig {
            sources: Vec::new(),
            fps: Some(5),
            live_retention: None,
            dvr: Some(VideoDvrConfig {
                retention: None,
                save_interval: SaveIntervalDuration::parse("videos.dvr.save_interval", "10ms").ok(),
                storage_path: None,
            }),
        };
        assert!(watch.apply_persisted_cadence(Some(&videos)).is_err());
        assert_eq!(watch.snapshot(), before);
    }
}
