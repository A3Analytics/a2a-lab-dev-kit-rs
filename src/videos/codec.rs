//! Video codec and session-media validation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;

/// Required MJPEG multipart media type for MJPEG sessions and sources.
pub const MJPEG_MEDIA_TYPE: &str = "multipart/x-mixed-replace; type=image/jpeg";

/// Encoded video codecs a source may retain or expose through playback sessions.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    /// Timestamped JPEG frames (`multipart/x-mixed-replace`).
    Mjpeg,
    /// H.264 elementary or packaged media.
    H264,
    /// H.265 elementary or packaged media.
    H265,
}

/// Validates ordered, non-empty source capabilities.
pub fn check_capabilities(capabilities: &[VideoCodec]) -> Result<(), A2aLabError> {
    if capabilities.is_empty() {
        return Err(A2aLabError::invalid(
            "capabilities",
            "must list at least one codec",
        ));
    }
    let mut seen = Vec::with_capacity(capabilities.len());
    for codec in capabilities {
        if seen.contains(codec) {
            return Err(A2aLabError::invalid(
                "capabilities",
                "must not repeat a codec",
            ));
        }
        seen.push(*codec);
    }
    Ok(())
}

/// Validates that `media_type` matches the chosen session codec without JPEG transcoding.
pub fn check_session_media(codec: VideoCodec, media_type: &str) -> Result<(), A2aLabError> {
    match codec {
        VideoCodec::Mjpeg => check_mjpeg_media_type(media_type),
        VideoCodec::H264 | VideoCodec::H265 => check_encoded_session_media(media_type),
    }
}

pub(crate) fn check_mjpeg_media_type(media_type: &str) -> Result<(), A2aLabError> {
    if media_type == MJPEG_MEDIA_TYPE {
        Ok(())
    } else {
        Err(A2aLabError::invalid(
            "media_type",
            format!("must be {MJPEG_MEDIA_TYPE}"),
        ))
    }
}

fn check_encoded_session_media(media_type: &str) -> Result<(), A2aLabError> {
    const ALLOWED: &[&str] = &[
        "video/mp4",
        "application/vnd.apple.mpegurl",
        "application/x-mpegURL",
        "video/MP2T",
    ];
    if ALLOWED.contains(&media_type) {
        Ok(())
    } else {
        Err(A2aLabError::invalid(
            "media_type",
            "must be a packaged encoded-video media type",
        ))
    }
}

/// Rejects absolute URLs, credentials, queries, fragments, and path traversal.
pub fn check_relative_session_path(path: &str) -> Result<(), A2aLabError> {
    let relative = path.starts_with('/') && !path.starts_with("//");
    let opaque = !path.contains("://")
        && !path.contains('?')
        && !path.contains('#')
        && !path.contains('@')
        && !path.contains("..");
    if relative && opaque {
        Ok(())
    } else {
        Err(A2aLabError::invalid(
            "path",
            "must be an authenticated relative media path",
        ))
    }
}

pub(crate) fn validate_source_media(
    capabilities: &[VideoCodec],
    media_type: &str,
) -> Result<(), A2aLabError> {
    if capabilities.contains(&VideoCodec::Mjpeg) {
        check_mjpeg_media_type(media_type)?;
    }
    if capabilities
        .iter()
        .any(|codec| matches!(codec, VideoCodec::H264 | VideoCodec::H265))
        && capabilities.len() == 1
    {
        check_session_media(capabilities[0], media_type)?;
    }
    Ok(())
}
