//! Retained source timelines and live-follow handles.
//!
//! Saved images stay in the images domain.

use std::pin::Pin;

use futures_util::Stream;
use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::A2aLabError;
use crate::id::{VideoFollowId, VideoSourceId};
use crate::time::{TimeRange, UtcTimestamp};
use crate::videos::model::{VideoFrame, check_mjpeg_media_type};

/// Relative media prefix for live-follow handles. Never a host or scheme.
pub const VIDEO_LIVE_PATH_PREFIX: &str = "/media/video-live/";

/// Object-safe stream of JPEG frames for one live-follow handle.
pub type VideoLiveStream = Pin<Box<dyn Stream<Item = Result<VideoFrame, A2aLabError>> + Send>>;

/// Request for one source's retained timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetVideoTimelineRequest {
    source_id: VideoSourceId,
}

impl GetVideoTimelineRequest {
    /// Creates a request for `source_id`.
    #[must_use]
    pub const fn new(source_id: VideoSourceId) -> Self {
        Self { source_id }
    }

    /// Source whose retained coverage is requested.
    #[must_use]
    pub const fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }
}

/// Ordered retained coverage for one video source.
///
/// Intervals are half-open `[start, end)` ranges. Temporally continuous coverage
/// is one interval. A genuine temporal gap is a separate later interval. Rotated
/// files and storage partitions are not represented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct VideoTimeline {
    source_id: VideoSourceId,
    intervals: Vec<TimeRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    newest_at: Option<UtcTimestamp>,
    live_available: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VideoTimelineRaw {
    source_id: VideoSourceId,
    intervals: Vec<TimeRange>,
    #[serde(default)]
    newest_at: Option<UtcTimestamp>,
    live_available: bool,
}

impl TryFrom<VideoTimelineRaw> for VideoTimeline {
    type Error = A2aLabError;

    fn try_from(raw: VideoTimelineRaw) -> Result<Self, Self::Error> {
        Self::new(
            raw.source_id,
            raw.intervals,
            raw.newest_at,
            raw.live_available,
        )
    }
}

impl<'de> Deserialize<'de> for VideoTimeline {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        VideoTimelineRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

impl VideoTimeline {
    /// Creates a timeline when intervals are ordered, half-open, and separated by gaps.
    pub fn new(
        source_id: VideoSourceId,
        intervals: Vec<TimeRange>,
        newest_at: Option<UtcTimestamp>,
        live_available: bool,
    ) -> Result<Self, A2aLabError> {
        validate_intervals(&intervals)?;
        validate_newest(intervals.last().copied(), newest_at)?;
        Ok(Self {
            source_id,
            intervals,
            newest_at,
            live_available,
        })
    }

    /// Source this coverage belongs to.
    #[must_use]
    pub const fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }

    /// Ordered half-open retained intervals.
    #[must_use]
    pub fn intervals(&self) -> &[TimeRange] {
        &self.intervals
    }

    /// Newest retained timestamp, when the provider knows one.
    #[must_use]
    pub const fn newest_at(&self) -> Option<UtcTimestamp> {
        self.newest_at
    }

    /// Whether live follow can be prepared for this source right now.
    #[must_use]
    pub const fn live_available(&self) -> bool {
        self.live_available
    }
}

/// Request to prepare a live-follow handle for one source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FollowVideoRequest {
    source_id: VideoSourceId,
}

impl FollowVideoRequest {
    /// Creates a request for `source_id`.
    #[must_use]
    pub const fn new(source_id: VideoSourceId) -> Self {
        Self { source_id }
    }

    /// Source to follow.
    #[must_use]
    pub const fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }
}

/// Metadata for a genuine live-follow stream. JSON never includes frame bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct VideoFollow {
    id: VideoFollowId,
    source_id: VideoSourceId,
    media_type: String,
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VideoFollowRaw {
    id: VideoFollowId,
    source_id: VideoSourceId,
    media_type: String,
    path: String,
}

impl TryFrom<VideoFollowRaw> for VideoFollow {
    type Error = A2aLabError;

    fn try_from(raw: VideoFollowRaw) -> Result<Self, Self::Error> {
        check_relative_media_path(&raw.path)?;
        let follow = Self::new(raw.id, raw.source_id, raw.media_type)?;
        if follow.path != raw.path {
            return Err(A2aLabError::invalid(
                "path",
                "must be a relative /media/video-live/{id} path",
            ));
        }
        Ok(follow)
    }
}

impl<'de> Deserialize<'de> for VideoFollow {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        VideoFollowRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

impl VideoFollow {
    /// Creates a live-follow handle with the required MJPEG type and relative path.
    pub fn new(
        id: VideoFollowId,
        source_id: VideoSourceId,
        media_type: impl Into<String>,
    ) -> Result<Self, A2aLabError> {
        let media_type = media_type.into();
        check_mjpeg_media_type(&media_type)?;
        let path = video_live_path(&id);
        check_relative_media_path(&path)?;
        Ok(Self {
            id,
            source_id,
            media_type,
            path,
        })
    }

    /// Opaque follow identifier.
    #[must_use]
    pub const fn id(&self) -> &VideoFollowId {
        &self.id
    }

    /// Source being followed.
    #[must_use]
    pub const fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }

    /// MJPEG multipart media type.
    #[must_use]
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    /// Authenticated relative media path. Never a URL and never frame bytes.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// Relative path `/media/video-live/{id}`.
#[must_use]
pub fn video_live_path(id: &VideoFollowId) -> String {
    format!("{VIDEO_LIVE_PATH_PREFIX}{id}")
}

fn validate_intervals(intervals: &[TimeRange]) -> Result<(), A2aLabError> {
    let mut previous_end = None;
    for interval in intervals {
        interval.check()?;
        if let Some(end) = previous_end
            && interval.start() <= end
        {
            return Err(A2aLabError::invalid(
                "intervals",
                "must be ordered, non-overlapping, and separated by a genuine gap",
            ));
        }
        previous_end = Some(interval.end());
    }
    Ok(())
}

fn validate_newest(
    newest_interval: Option<TimeRange>,
    newest_at: Option<UtcTimestamp>,
) -> Result<(), A2aLabError> {
    let Some(newest_at) = newest_at else {
        return Ok(());
    };
    let Some(interval) = newest_interval else {
        return Err(A2aLabError::invalid(
            "newest_at",
            "must be absent when no interval is retained",
        ));
    };
    if interval.contains(newest_at) {
        Ok(())
    } else {
        Err(A2aLabError::invalid(
            "newest_at",
            "must lie in the newest retained interval",
        ))
    }
}

fn check_relative_media_path(path: &str) -> Result<(), A2aLabError> {
    let relative = path.starts_with('/') && !path.starts_with("//");
    let opaque =
        !path.contains("://") && !path.contains('?') && !path.contains('#') && !path.contains('@');
    if relative && opaque {
        Ok(())
    } else {
        Err(A2aLabError::invalid(
            "path",
            "must be an authenticated relative media path",
        ))
    }
}
