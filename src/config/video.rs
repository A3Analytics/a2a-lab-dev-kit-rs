//! Video source and DVR settings for the v1 configuration envelope.

use std::collections::HashSet;

use serde_json::Value;

use crate::error::A2aLabError;
use crate::id::VideoSourceId;
use crate::videos::{CaptureFormat, CaptureFormatPreference};

use super::duration::ConfigDuration;

/// Camera input requested from the capture library.
///
/// This is not a playback [`crate::videos::VideoCodec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoInputFormat {
    /// Encoded JPEG frames from the device.
    Mjpeg,
    /// Packed `Y0 U0 Y1 V0` bytes. `yuyv` normalizes to this variant.
    Yuy2,
}

impl VideoInputFormat {
    /// Existing raw capture format for this input.
    #[must_use]
    pub const fn capture_format(self) -> CaptureFormat {
        match self {
            Self::Mjpeg => CaptureFormat::Mjpeg,
            Self::Yuy2 => CaptureFormat::Yuy2,
        }
    }

    /// Existing profile preference for this input. Never `Auto`.
    #[must_use]
    pub const fn capture_preference(self) -> CaptureFormatPreference {
        match self {
            Self::Mjpeg => CaptureFormatPreference::Mjpeg,
            Self::Yuy2 => CaptureFormatPreference::Yuy2,
        }
    }
}

/// Shared capture and DVR overrides. Omitted fields stay absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoConfig {
    /// Operator sources. Empty means the file selects no sources.
    pub sources: Vec<VideoSourceConfig>,
    /// Positive frame rate when the file sets one.
    pub fps: Option<u32>,
    /// In-memory live window when the file sets one.
    pub live_retention: Option<ConfigDuration>,
    /// Optional DVR block. Omitted fields inside it stay absent.
    pub dvr: Option<VideoDvrConfig>,
}

/// One file-configured camera source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoSourceConfig {
    /// Stable operator id.
    pub id: VideoSourceId,
    /// Present only when the file supplied a name.
    pub name: Option<String>,
    /// Present only when the file supplied a description.
    pub description: Option<String>,
    /// Requested camera input.
    pub format: VideoInputFormat,
    /// Local capture-library index, including zero.
    pub device_index: u32,
}

impl VideoSourceConfig {
    /// File name, or the source id when the name was omitted.
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or_else(|| self.id.as_str())
    }

    /// File description, or [`Self::display_name`] when omitted.
    #[must_use]
    pub fn display_description(&self) -> &str {
        self.description
            .as_deref()
            .unwrap_or_else(|| self.display_name())
    }
}

/// Optional DVR retention and storage directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoDvrConfig {
    /// Disk retention when the file sets one.
    pub retention: Option<ConfigDuration>,
    /// Storage directory text when the file sets one. Not resolved here.
    pub storage_path: Option<String>,
}

pub(crate) fn parse_videos(value: Option<&Value>) -> Result<Option<VideoConfig>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let map = super::object_at("videos", value)?;
    super::reject_unknown_keys(map, "videos", VIDEO_KEYS)?;
    if !map.contains_key("sources") {
        return Err(A2aLabError::invalid(
            "videos.sources",
            "required field is missing",
        ));
    }
    let sources = parse_sources(map.get("sources"))?;
    let fps = parse_fps(map.get("fps"))?;
    let live_retention = parse_duration("videos.live_retention", map.get("live_retention"))?;
    let dvr = parse_dvr(map.get("dvr"))?;
    Ok(Some(VideoConfig {
        sources,
        fps,
        live_retention,
        dvr,
    }))
}

const VIDEO_KEYS: &[&str] = &["sources", "fps", "live_retention", "dvr"];
const SOURCE_KEYS: &[&str] = &["id", "name", "description", "format", "device_index"];
const DVR_KEYS: &[&str] = &["retention", "storage_path"];

fn parse_sources(value: Option<&Value>) -> Result<Vec<VideoSourceConfig>, A2aLabError> {
    let Some(value) = value else {
        return Err(A2aLabError::invalid(
            "videos.sources",
            "required field is missing",
        ));
    };
    let Value::Array(items) = value else {
        return Err(A2aLabError::invalid(
            "videos.sources",
            "must be a JSON array",
        ));
    };
    let mut sources = Vec::with_capacity(items.len());
    let mut ids = HashSet::new();
    let mut devices = HashSet::new();
    for item in items {
        let source = parse_source(item)?;
        if !ids.insert(source.id.clone()) {
            return Err(A2aLabError::invalid(
                "videos.sources.id",
                "duplicate source id",
            ));
        }
        if !devices.insert(source.device_index) {
            return Err(A2aLabError::invalid(
                "videos.sources.device_index",
                "duplicate device index",
            ));
        }
        sources.push(source);
    }
    Ok(sources)
}

fn parse_source(value: &Value) -> Result<VideoSourceConfig, A2aLabError> {
    let map = super::object_at("videos.sources", value)?;
    super::reject_unknown_keys(map, "videos.sources", SOURCE_KEYS)?;
    let id = required_text(map.get("id"), "videos.sources.id")?;
    let id = VideoSourceId::new(id).map_err(|_| {
        A2aLabError::invalid("videos.sources.id", "must be a valid video source id")
    })?;
    let name = optional_text(map.get("name"), "videos.sources.name")?;
    let description = optional_text(map.get("description"), "videos.sources.description")?;
    let format = parse_format(map.get("format"))?;
    let device_index = match map.get("device_index") {
        Some(value) => super::parse_u32("videos.sources.device_index", value)?,
        None => {
            return Err(A2aLabError::invalid(
                "videos.sources.device_index",
                "required field is missing",
            ));
        }
    };
    Ok(VideoSourceConfig {
        id,
        name,
        description,
        format,
        device_index,
    })
}

fn parse_format(value: Option<&Value>) -> Result<VideoInputFormat, A2aLabError> {
    let Some(value) = value else {
        return Err(A2aLabError::invalid(
            "videos.sources.format",
            "required field is missing",
        ));
    };
    let Value::String(text) = value else {
        return Err(A2aLabError::invalid(
            "videos.sources.format",
            "must be a string",
        ));
    };
    match text.as_str() {
        "mjpeg" => Ok(VideoInputFormat::Mjpeg),
        "yuy2" | "yuyv" => Ok(VideoInputFormat::Yuy2),
        _ => Err(A2aLabError::invalid(
            "videos.sources.format",
            "unsupported camera input format",
        )),
    }
}

fn parse_fps(value: Option<&Value>) -> Result<Option<u32>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let fps = super::parse_u32("videos.fps", value)?;
    if fps == 0 {
        return Err(A2aLabError::invalid(
            "videos.fps",
            "must be greater than zero",
        ));
    }
    Ok(Some(fps))
}

fn parse_dvr(value: Option<&Value>) -> Result<Option<VideoDvrConfig>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let map = super::object_at("videos.dvr", value)?;
    super::reject_unknown_keys(map, "videos.dvr", DVR_KEYS)?;
    let retention = parse_duration("videos.dvr.retention", map.get("retention"))?;
    let storage_path = optional_text(map.get("storage_path"), "videos.dvr.storage_path")?;
    Ok(Some(VideoDvrConfig {
        retention,
        storage_path,
    }))
}

fn parse_duration(
    field: &'static str,
    value: Option<&Value>,
) -> Result<Option<ConfigDuration>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Value::String(text) = value else {
        return Err(A2aLabError::invalid(
            field,
            "must be a positive whole-second duration",
        ));
    };
    ConfigDuration::parse(field, text).map(Some)
}

fn required_text<'a>(
    value: Option<&'a Value>,
    field: &'static str,
) -> Result<&'a str, A2aLabError> {
    match value {
        Some(Value::String(text)) if !text.is_empty() => Ok(text.as_str()),
        Some(_) => Err(A2aLabError::invalid(field, "must be a non-empty string")),
        None => Err(A2aLabError::invalid(field, "required field is missing")),
    }
}

fn optional_text(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Option<String>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Value::String(text) = value else {
        return Err(A2aLabError::invalid(field, "must be a string"));
    };
    if text.chars().any(|character| !character.is_whitespace()) {
        Ok(Some(text.clone()))
    } else {
        Err(A2aLabError::invalid(
            field,
            "must contain non-whitespace text",
        ))
    }
}
