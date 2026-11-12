//! Video source and DVR settings for the v1 configuration envelope.

use std::collections::HashSet;

use serde_json::Value;

use crate::error::A2aLabError;
use crate::id::VideoSourceId;
use crate::videos::{CaptureFormat, CaptureFormatPreference};

use super::duration::ConfigDuration;
use super::semantic_indexing::{SemanticStreamSchedule, parse_schedule};

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
    /// Explicit frame width. Required for non-legacy source ids.
    pub width: Option<u32>,
    /// Explicit frame height. Required for non-legacy source ids.
    pub height: Option<u32>,
    /// Optional per-source semantic indexing stage schedules.
    pub semantic_indexing: Option<SemanticStreamSchedule>,
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

pub(crate) fn parse_videos(
    value: Option<&Value>,
    allow_semantic_indexing: bool,
) -> Result<Option<VideoConfig>, A2aLabError> {
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
    let sources = parse_sources(map.get("sources"), allow_semantic_indexing)?;
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
const SOURCE_KEYS_V1: &[&str] = &[
    "id",
    "name",
    "description",
    "format",
    "device_index",
    "width",
    "height",
];
const SOURCE_KEYS_V2: &[&str] = &[
    "id",
    "name",
    "description",
    "format",
    "device_index",
    "width",
    "height",
    "semantic_indexing",
];
const DVR_KEYS: &[&str] = &["retention", "storage_path"];

fn parse_sources(
    value: Option<&Value>,
    allow_semantic_indexing: bool,
) -> Result<Vec<VideoSourceConfig>, A2aLabError> {
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
        let source = parse_source(item, allow_semantic_indexing)?;
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

fn parse_source(
    value: &Value,
    allow_semantic_indexing: bool,
) -> Result<VideoSourceConfig, A2aLabError> {
    let map = super::object_at("videos.sources", value)?;
    super::reject_unknown_keys(
        map,
        "videos.sources",
        if allow_semantic_indexing {
            SOURCE_KEYS_V2
        } else {
            SOURCE_KEYS_V1
        },
    )?;
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
    let width = parse_optional_positive_u32(map.get("width"), "videos.sources.width")?;
    let height = parse_optional_positive_u32(map.get("height"), "videos.sources.height")?;
    let semantic_indexing = if allow_semantic_indexing {
        parse_source_semantic_indexing(map.get("semantic_indexing"), id.as_str())?
    } else {
        None
    };
    Ok(VideoSourceConfig {
        id,
        name,
        description,
        format,
        device_index,
        width,
        height,
        semantic_indexing,
    })
}

fn parse_optional_positive_u32(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Option<u32>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let parsed = super::parse_u32(field, value)?;
    if parsed == 0 {
        return Err(A2aLabError::invalid(field, "must be greater than zero"));
    }
    Ok(Some(parsed))
}

fn parse_source_semantic_indexing(
    value: Option<&Value>,
    stream: &str,
) -> Result<Option<SemanticStreamSchedule>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let map = super::object_at("videos.sources.semantic_indexing", value)?;
    super::reject_unknown_keys(
        map,
        "videos.sources.semantic_indexing",
        &[
            "description",
            "description_model",
            "embeddings",
            "embedding_model",
        ],
    )?;
    let description_model = optional_model(
        map.get("description_model"),
        "videos.sources.semantic_indexing.description_model",
    )?;
    let embedding_model = optional_model(
        map.get("embedding_model"),
        "videos.sources.semantic_indexing.embedding_model",
    )?;
    Ok(Some(SemanticStreamSchedule {
        stream: stream.to_owned(),
        description: parse_schedule(
            map.get("description"),
            "videos.sources.semantic_indexing.description",
        )?,
        embeddings: parse_schedule(
            map.get("embeddings"),
            "videos.sources.semantic_indexing.embeddings",
        )?,
        description_model,
        embedding_model,
        embedding_configured: map.contains_key("embeddings") || map.contains_key("embedding_model"),
    }))
}

fn optional_model(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Option<String>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Value::String(model) = value else {
        return Err(A2aLabError::invalid(field, "must be a non-empty string"));
    };
    if model.trim().is_empty() {
        return Err(A2aLabError::invalid(field, "must be a non-empty string"));
    }
    Ok(Some(model.clone()))
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
