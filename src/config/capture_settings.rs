//! Runtime video capture controls shown on the config/settings route.
//!
//! `videos.fps` edits change live output cadence only. `videos.dvr.save_interval`
//! edits change DVR persistence cadence only. DVR retention stays on the file
//! baseline and is not updated here.

use serde::Serialize;
use serde_json::Value;

use crate::error::A2aLabError;

use super::{
    CameraIdView, CameraSelector, ConfigDuration, SaveIntervalDuration, VideoConfig,
    VideoDvrConfig, VideoInputFormat, VideoSourceConfig,
};

/// Default live output cadence when the file omits `videos.fps`.
pub const DEFAULT_LIVE_OUTPUT_FPS: u32 = 5;

/// Shared, mutable capture settings exposed to the web shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoCaptureSettings {
    sources: Vec<VideoSourceConfig>,
    configured_fps: Option<u32>,
    live_output_fps: u32,
    live_retention: Option<ConfigDuration>,
    dvr: Option<VideoDvrConfig>,
    dvr_save_interval: Option<SaveIntervalDuration>,
    negotiated_sensor_period_ns: Option<u64>,
}

/// JSON view returned by `GET /config/settings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VideoCaptureSettingsView {
    pub sources: Vec<VideoCaptureSourceView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured_videos_fps: Option<u32>,
    pub live_output_fps: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_retention_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dvr_save_interval: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dvr_retention_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dvr_storage_path: Option<String>,
}

/// One source row for the settings route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VideoCaptureSourceView {
    pub id: String,
    pub display_name: String,
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera_id: Option<CameraIdView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_rate: Option<u32>,
}

impl VideoCaptureSettings {
    /// Builds runtime settings from a parsed file envelope.
    #[must_use]
    pub fn from_video_config(config: Option<&VideoConfig>) -> Self {
        let Some(config) = config else {
            return Self::empty();
        };
        Self {
            sources: config.sources.clone(),
            configured_fps: config.fps,
            live_output_fps: config.fps.unwrap_or(DEFAULT_LIVE_OUTPUT_FPS),
            live_retention: config.live_retention,
            dvr: config.dvr.clone(),
            dvr_save_interval: config.dvr.as_ref().and_then(|dvr| dvr.save_interval),
            negotiated_sensor_period_ns: None,
        }
    }

    /// Records the negotiated sensor spacing used to bound save-interval edits.
    #[must_use]
    pub fn with_negotiated_sensor_period_ns(mut self, period_ns: u64) -> Self {
        self.negotiated_sensor_period_ns = Some(period_ns);
        self
    }

    fn empty() -> Self {
        Self {
            sources: Vec::new(),
            configured_fps: None,
            live_output_fps: DEFAULT_LIVE_OUTPUT_FPS,
            live_retention: None,
            dvr: None,
            dvr_save_interval: None,
            negotiated_sensor_period_ns: None,
        }
    }

    /// Active live output cadence in frames per second.
    #[must_use]
    pub const fn live_output_fps(&self) -> u32 {
        self.live_output_fps
    }

    /// Live output spacing in nanoseconds.
    #[must_use]
    pub fn live_period_ns(&self) -> u64 {
        1_000_000_000u64 / u64::from(self.live_output_fps.max(1))
    }

    /// DVR save spacing in nanoseconds.
    #[must_use]
    pub fn dvr_period_ns(&self) -> u64 {
        self.dvr_save_interval()
            .map_or(self.live_period_ns(), SaveIntervalDuration::as_nanos)
    }

    /// Active DVR save cadence when one is configured.
    #[must_use]
    pub const fn dvr_save_interval(&self) -> Option<SaveIntervalDuration> {
        self.dvr_save_interval
    }

    /// File-configured `videos.fps` when the envelope set one.
    #[must_use]
    pub const fn configured_videos_fps(&self) -> Option<u32> {
        self.configured_fps
    }

    /// DVR block copied from the file baseline. Live edits do not touch retention fields.
    #[must_use]
    pub fn dvr(&self) -> Option<&VideoDvrConfig> {
        self.dvr.as_ref()
    }

    /// Snapshot for API responses and the browser shell.
    #[must_use]
    pub fn view(&self) -> VideoCaptureSettingsView {
        VideoCaptureSettingsView {
            sources: self
                .sources
                .iter()
                .map(|source| VideoCaptureSourceView {
                    id: source.id.as_str().to_owned(),
                    display_name: source.display_name().to_owned(),
                    format: format_input(source.format),
                    camera_id: source.camera_id.as_ref().map(CameraSelector::as_view),
                    width: source.width,
                    height: source.height,
                    capture_rate: source.capture_rate,
                })
                .collect(),
            configured_videos_fps: self.configured_fps,
            live_output_fps: self.live_output_fps,
            live_retention_secs: self
                .live_retention
                .as_ref()
                .map(|duration| duration.as_secs()),
            dvr_save_interval: self.dvr_save_interval.map(SaveIntervalDuration::as_text),
            dvr_retention_secs: self
                .dvr
                .as_ref()
                .and_then(|dvr| dvr.retention.as_ref())
                .map(|duration| duration.as_secs()),
            dvr_storage_path: self.dvr.as_ref().and_then(|dvr| dvr.storage_path.clone()),
        }
    }

    /// Applies a `videos` patch. Rejects invalid values and leaves active fields unchanged.
    pub fn apply_patch(&mut self, document: &Value) -> Result<(), A2aLabError> {
        let map = super::object_at("input", document)?;
        super::reject_unknown_keys(map, "", &["videos"])?;
        let videos = map
            .get("videos")
            .ok_or_else(|| A2aLabError::invalid("videos", "required field is missing"))?;
        let videos = super::object_at("videos", videos)?;
        if videos.is_empty() {
            return Err(A2aLabError::invalid("videos", "required field is missing"));
        }
        super::reject_unknown_keys(videos, "videos", &["fps", "dvr"])?;
        if let Some(fps) = videos.get("fps") {
            let fps = parse_live_fps_value(fps)?;
            self.live_output_fps = fps;
        }
        if let Some(dvr) = videos.get("dvr") {
            let dvr = super::object_at("videos.dvr", dvr)?;
            super::reject_unknown_keys(dvr, "videos.dvr", &["save_interval"])?;
            let interval = dvr.get("save_interval").ok_or_else(|| {
                A2aLabError::invalid("videos.dvr.save_interval", "required field is missing")
            })?;
            let interval = parse_save_interval_value(interval)?;
            self.ensure_save_interval_deliverable(interval)?;
            self.dvr_save_interval = Some(interval);
        }
        Ok(())
    }

    /// Parses operator fps text from a form field.
    pub fn parse_live_fps_text(text: &str) -> Result<u32, A2aLabError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(A2aLabError::invalid("videos.fps", "must not be blank"));
        }
        if trimmed.starts_with('-') {
            return Err(A2aLabError::invalid(
                "videos.fps",
                "must be greater than zero",
            ));
        }
        if trimmed.contains('.') {
            return Err(A2aLabError::invalid(
                "videos.fps",
                "must be a positive whole number",
            ));
        }
        if !trimmed.chars().all(|character| character.is_ascii_digit()) {
            return Err(A2aLabError::invalid(
                "videos.fps",
                "must be a positive whole number",
            ));
        }
        let parsed = trimmed
            .parse::<u32>()
            .map_err(|_| A2aLabError::invalid("videos.fps", "must be a positive whole number"))?;
        if parsed == 0 {
            return Err(A2aLabError::invalid(
                "videos.fps",
                "must be greater than zero",
            ));
        }
        Ok(parsed)
    }

    /// Parses operator save-interval text from a form field.
    pub fn parse_save_interval_text(text: &str) -> Result<SaveIntervalDuration, A2aLabError> {
        SaveIntervalDuration::parse("videos.dvr.save_interval", text)
    }

    fn ensure_save_interval_deliverable(
        &self,
        interval: SaveIntervalDuration,
    ) -> Result<(), A2aLabError> {
        let Some(sensor_period_ns) = self.negotiated_sensor_period_ns else {
            return Ok(());
        };
        if interval.as_nanos() < sensor_period_ns {
            return Err(A2aLabError::invalid(
                "videos.dvr.save_interval",
                "must not be faster than the negotiated sensor period",
            ));
        }
        Ok(())
    }
}

fn parse_live_fps_value(value: &Value) -> Result<u32, A2aLabError> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .and_then(|fps| u32::try_from(fps).ok())
            .filter(|fps| *fps > 0)
            .ok_or_else(|| A2aLabError::invalid("videos.fps", "must be greater than zero")),
        Value::String(text) => VideoCaptureSettings::parse_live_fps_text(text),
        _ => Err(A2aLabError::invalid(
            "videos.fps",
            "must be a positive whole number",
        )),
    }
}

fn parse_save_interval_value(value: &Value) -> Result<SaveIntervalDuration, A2aLabError> {
    match value {
        Value::String(text) => VideoCaptureSettings::parse_save_interval_text(text),
        _ => Err(A2aLabError::invalid(
            "videos.dvr.save_interval",
            "must be a positive duration",
        )),
    }
}

fn format_input(format: VideoInputFormat) -> String {
    match format {
        VideoInputFormat::Auto => "auto".to_owned(),
        VideoInputFormat::Mjpeg => "mjpeg".to_owned(),
        VideoInputFormat::Yuy2 => "yuy2".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::VideoSourceId;
    use crate::videos::CaptureRate;
    use serde_json::json;

    fn sample_config() -> VideoConfig {
        VideoConfig {
            sources: vec![VideoSourceConfig {
                id: VideoSourceId::new("bench").unwrap(),
                name: Some("Bench".to_owned()),
                description: None,
                format: VideoInputFormat::Mjpeg,
                camera_id: Some(CameraSelector::Index(2)),
                width: Some(1280),
                height: Some(720),
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
        }
    }

    #[test]
    fn view_lists_every_source_field_and_configured_fps() {
        let settings = VideoCaptureSettings::from_video_config(Some(&sample_config()));
        let view = settings.view();
        assert_eq!(view.sources.len(), 1);
        let source = &view.sources[0];
        assert_eq!(source.id, "bench");
        assert_eq!(source.display_name, "Bench");
        assert_eq!(source.format, "mjpeg");
        assert_eq!(source.camera_id, Some(CameraIdView::Index(2)));
        assert_eq!(source.width, Some(1280));
        assert_eq!(source.height, Some(720));
        assert_eq!(source.capture_rate, Some(30));
        assert_eq!(view.configured_videos_fps, Some(5));
        assert_eq!(view.live_output_fps, 5);
        assert_eq!(view.dvr_save_interval.as_deref(), Some("1s"));
        assert_eq!(view.dvr_retention_secs, Some(3_600));
        assert_eq!(view.dvr_storage_path.as_deref(), Some("./dvr"));
    }

    #[test]
    fn valid_edit_updates_live_output_only() {
        let config = sample_config();
        let dvr_before = config.dvr.clone();
        let mut settings = VideoCaptureSettings::from_video_config(Some(&config));
        settings
            .apply_patch(&json!({ "videos": { "fps": 12 } }))
            .unwrap();
        assert_eq!(settings.live_output_fps(), 12);
        assert_eq!(settings.configured_videos_fps(), Some(5));
        assert_eq!(settings.dvr().cloned(), dvr_before);
        assert_eq!(
            settings.dvr_save_interval(),
            dvr_before.and_then(|dvr| dvr.save_interval)
        );
    }

    #[test]
    fn invalid_edit_leaves_live_output_unchanged() {
        let mut settings = VideoCaptureSettings::from_video_config(Some(&sample_config()));
        for document in [
            json!({ "videos": { "fps": 0 } }),
            json!({ "videos": { "fps": -3 } }),
            json!({ "videos": { "fps": "1.5" } }),
            json!({ "videos": { "fps": "" } }),
            json!({ "videos": { "fps": "nope" } }),
        ] {
            assert!(settings.apply_patch(&document).is_err());
            assert_eq!(settings.live_output_fps(), 5);
        }
        for text in ["", "0", "-1", "1.5", "abc"] {
            assert!(VideoCaptureSettings::parse_live_fps_text(text).is_err());
            assert_eq!(settings.live_output_fps(), 5);
        }
    }

    #[test]
    fn live_versus_dvr_semantics_stay_separate() {
        let config = sample_config();
        let mut settings = VideoCaptureSettings::from_video_config(Some(&config));
        let live_before = settings.view().live_retention_secs;
        let dvr_before = settings.view().dvr_retention_secs;
        settings
            .apply_patch(&json!({ "videos": { "fps": 30 } }))
            .unwrap();
        let view = settings.view();
        assert_eq!(view.live_output_fps, 30);
        assert_eq!(view.live_retention_secs, live_before);
        assert_eq!(view.dvr_retention_secs, dvr_before);
        assert_eq!(view.configured_videos_fps, Some(5));
        assert_eq!(view.dvr_save_interval.as_deref(), Some("1s"));
    }

    #[test]
    fn save_interval_examples_round_trip_through_settings() {
        let mut settings = VideoCaptureSettings::from_video_config(Some(&sample_config()))
            .with_negotiated_sensor_period_ns(CaptureRate::fps(30).unwrap().period_ns().unwrap());
        for text in ["500ms", "1s", "5s", "30s"] {
            settings
                .apply_patch(&json!({ "videos": { "dvr": { "save_interval": text } } }))
                .unwrap_or_else(|_| panic!("{text}"));
            assert_eq!(settings.view().dvr_save_interval.as_deref(), Some(text));
        }
    }

    #[test]
    fn arbitrary_save_interval_is_stored_as_duration_not_fps() {
        let interval = SaveIntervalDuration::parse("videos.dvr.save_interval", "750ms").unwrap();
        let mut settings = VideoCaptureSettings::from_video_config(Some(&sample_config()))
            .with_negotiated_sensor_period_ns(CaptureRate::fps(30).unwrap().period_ns().unwrap());
        settings
            .apply_patch(&json!({ "videos": { "dvr": { "save_interval": "750ms" } } }))
            .unwrap();
        assert_eq!(settings.dvr_save_interval(), Some(interval));
        assert_eq!(settings.view().dvr_save_interval.as_deref(), Some("750ms"));
    }

    #[test]
    fn too_fast_save_interval_is_rejected() {
        let mut settings = VideoCaptureSettings::from_video_config(Some(&sample_config()))
            .with_negotiated_sensor_period_ns(CaptureRate::fps(30).unwrap().period_ns().unwrap());
        let before = settings.dvr_save_interval();
        let err = settings
            .apply_patch(&json!({ "videos": { "dvr": { "save_interval": "10ms" } } }))
            .unwrap_err();
        assert_eq!(err.code(), "invalid");
        assert_eq!(settings.dvr_save_interval(), before);
    }

    #[test]
    fn invalid_save_interval_leaves_active_value_unchanged() {
        let mut settings = VideoCaptureSettings::from_video_config(Some(&sample_config()))
            .with_negotiated_sensor_period_ns(CaptureRate::fps(30).unwrap().period_ns().unwrap());
        let before = settings.dvr_save_interval();
        for document in [
            json!({ "videos": { "dvr": { "save_interval": "" } } }),
            json!({ "videos": { "dvr": { "save_interval": "0ms" } } }),
            json!({ "videos": { "dvr": { "save_interval": "-1s" } } }),
            json!({ "videos": { "dvr": { "save_interval": "nope" } } }),
        ] {
            assert!(settings.apply_patch(&document).is_err());
            assert_eq!(settings.dvr_save_interval(), before);
        }
    }
}
