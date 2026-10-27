//! Video sources, frame-at-time lookup, retained timelines, live follow, and bounded clips.
//!
//! Saved images stay in the images domain. This module does not list or own them.

use std::pin::Pin;

use futures_util::Stream;
use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::A2aLabError;
use crate::id::{VideoClipId, VideoSourceId};
use crate::images::Image;
use crate::page::PageRequest;
use crate::time::{TimeRange, UtcTimestamp};

/// Required MJPEG multipart media type for video sources and clips.
pub const MJPEG_MEDIA_TYPE: &str = "multipart/x-mixed-replace; type=image/jpeg";

/// Longest clip [`TimeRange`] the kit accepts (60 seconds).
pub const MAX_VIDEO_CLIP_DURATION_US: u64 = 60 * 1_000_000;

/// Default multipart boundary used by [`encode_mjpeg_multipart`].
pub const DEFAULT_MJPEG_BOUNDARY: &str = "a2alabframe";

/// Relative download prefix for clip handles. Never a host or scheme.
pub const VIDEO_CLIP_PATH_PREFIX: &str = "/media/video-clips/";

/// Object-safe stream of JPEG frames for one clip handle.
pub type VideoClipStream = Pin<Box<dyn Stream<Item = Result<VideoFrame, A2aLabError>> + Send>>;

/// A source that can supply timestamped MJPEG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct VideoSource {
    id: VideoSourceId,
    name: String,
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    asset_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    semantic_id: Option<String>,
    width: u32,
    height: u32,
    fps: u32,
    media_type: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VideoSourceRaw {
    id: VideoSourceId,
    name: String,
    description: String,
    #[serde(default)]
    asset_id: Option<String>,
    #[serde(default)]
    semantic_id: Option<String>,
    width: u32,
    height: u32,
    fps: u32,
    media_type: String,
}

impl TryFrom<VideoSourceRaw> for VideoSource {
    type Error = A2aLabError;

    fn try_from(raw: VideoSourceRaw) -> Result<Self, Self::Error> {
        Self::new(
            raw.id,
            raw.name,
            raw.description,
            raw.asset_id,
            raw.semantic_id,
            raw.width,
            raw.height,
            raw.fps,
            raw.media_type,
        )
    }
}

impl<'de> Deserialize<'de> for VideoSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        VideoSourceRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

impl VideoSource {
    /// Creates a source when dimensions, fps, and media type are valid.
    pub fn new(
        id: VideoSourceId,
        name: impl Into<String>,
        description: impl Into<String>,
        asset_id: Option<String>,
        semantic_id: Option<String>,
        width: u32,
        height: u32,
        fps: u32,
        media_type: impl Into<String>,
    ) -> Result<Self, A2aLabError> {
        let media_type = media_type.into();
        check_mjpeg_media_type(&media_type)?;
        check_positive("width", width)?;
        check_positive("height", height)?;
        check_positive("fps", fps)?;
        Ok(Self {
            id,
            name: name.into(),
            description: description.into(),
            asset_id,
            semantic_id,
            width,
            height,
            fps,
            media_type,
        })
    }

    /// Stable source identifier.
    #[must_use]
    pub const fn id(&self) -> &VideoSourceId {
        &self.id
    }

    /// Human-readable name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What this source records.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Asset identifier from the industrial catalog.
    #[must_use]
    pub fn asset_id(&self) -> Option<&str> {
        self.asset_id.as_deref()
    }

    /// Semantic identifier from the industrial catalog.
    #[must_use]
    pub fn semantic_id(&self) -> Option<&str> {
        self.semantic_id.as_deref()
    }

    /// Frame width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Frame height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Nominal frames per second.
    #[must_use]
    pub const fn fps(&self) -> u32 {
        self.fps
    }

    /// MJPEG multipart media type.
    #[must_use]
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    /// Nominal frame period in microseconds.
    #[must_use]
    pub fn frame_period_us(&self) -> u64 {
        (1_000_000 / u64::from(self.fps)).max(1)
    }
}

/// Request for a page of video sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ListVideoSourcesRequest {
    page: PageRequest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListVideoSourcesRequestRaw {
    page: PageRequest,
}

impl ListVideoSourcesRequest {
    /// Creates a request when `page` is valid.
    pub fn new(page: PageRequest) -> Result<Self, A2aLabError> {
        page.check()?;
        Ok(Self { page })
    }

    /// Page bounds.
    #[must_use]
    pub const fn page(&self) -> &PageRequest {
        &self.page
    }
}

impl TryFrom<ListVideoSourcesRequestRaw> for ListVideoSourcesRequest {
    type Error = A2aLabError;

    fn try_from(raw: ListVideoSourcesRequestRaw) -> Result<Self, Self::Error> {
        Self::new(raw.page)
    }
}

impl<'de> Deserialize<'de> for ListVideoSourcesRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ListVideoSourcesRequestRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// Request for the retained frame at a UTC instant.
///
/// The public operation remains `get_video_frame_at`. This is a frame-at-time
/// lookup, not a saved-image query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GetVideoFrameAtRequest {
    source_id: VideoSourceId,
    at: UtcTimestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_delta_us: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetVideoFrameAtRequestRaw {
    source_id: VideoSourceId,
    at: UtcTimestamp,
    #[serde(default)]
    max_delta_us: Option<u64>,
}

impl GetVideoFrameAtRequest {
    /// Creates a frame-at-time request for `source_id` at `at`.
    #[must_use]
    pub const fn new(
        source_id: VideoSourceId,
        at: UtcTimestamp,
        max_delta_us: Option<u64>,
    ) -> Self {
        Self {
            source_id,
            at,
            max_delta_us,
        }
    }

    /// Source to read.
    #[must_use]
    pub const fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }

    /// Instant to look up.
    #[must_use]
    pub const fn at(&self) -> UtcTimestamp {
        self.at
    }

    /// Optional maximum absolute delta from `at`, in microseconds.
    #[must_use]
    pub const fn max_delta_us(&self) -> Option<u64> {
        self.max_delta_us
    }
}

impl TryFrom<GetVideoFrameAtRequestRaw> for GetVideoFrameAtRequest {
    type Error = A2aLabError;

    fn try_from(raw: GetVideoFrameAtRequestRaw) -> Result<Self, Self::Error> {
        Ok(Self::new(raw.source_id, raw.at, raw.max_delta_us))
    }
}

impl<'de> Deserialize<'de> for GetVideoFrameAtRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        GetVideoFrameAtRequestRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// Retained frame at the requested time, with requested and actual UTC timestamps.
///
/// The result type remains [`GetVideoFrameAtResult`]. The embedded JPEG is the
/// frame at that time. It is not a saved image record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GetVideoFrameAtResult {
    requested_at: UtcTimestamp,
    actual_captured_at: UtcTimestamp,
    delta_us: i64,
    image: Image,
}

impl GetVideoFrameAtResult {
    /// Pairs a still with the lookup timestamps.
    #[must_use]
    pub fn new(requested_at: UtcTimestamp, image: Image) -> Self {
        let actual_captured_at = image.descriptor().captured_at();
        Self {
            requested_at,
            actual_captured_at,
            delta_us: actual_captured_at.as_micros() - requested_at.as_micros(),
            image,
        }
    }

    /// Instant the caller asked for.
    #[must_use]
    pub const fn requested_at(&self) -> UtcTimestamp {
        self.requested_at
    }

    /// Capture time of the selected frame.
    #[must_use]
    pub const fn actual_captured_at(&self) -> UtcTimestamp {
        self.actual_captured_at
    }

    /// Signed delta `actual - requested` in microseconds.
    #[must_use]
    pub const fn delta_us(&self) -> i64 {
        self.delta_us
    }

    /// JPEG for the frame at the requested time (`image/*`).
    #[must_use]
    pub const fn image(&self) -> &Image {
        &self.image
    }
}

/// Request for a bounded clip handle over a half-open UTC range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GetVideoClipRequest {
    source_id: VideoSourceId,
    range: TimeRange,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetVideoClipRequestRaw {
    source_id: VideoSourceId,
    range: TimeRange,
}

impl GetVideoClipRequest {
    /// Creates a request when `range` is valid and within the duration limit.
    pub fn new(source_id: VideoSourceId, range: TimeRange) -> Result<Self, A2aLabError> {
        let request = Self { source_id, range };
        request.check()?;
        Ok(request)
    }

    /// Rejects an invalid, reversed, or over-limit range.
    pub fn check(&self) -> Result<(), A2aLabError> {
        self.range.check()?;
        if self.range.duration_us() > MAX_VIDEO_CLIP_DURATION_US {
            return Err(A2aLabError::invalid(
                "range",
                format!("must be at most {MAX_VIDEO_CLIP_DURATION_US} microseconds"),
            ));
        }
        Ok(())
    }

    /// Source to clip.
    #[must_use]
    pub const fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }

    /// Half-open UTC interval `[start, end)`.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        self.range
    }
}

impl TryFrom<GetVideoClipRequestRaw> for GetVideoClipRequest {
    type Error = A2aLabError;

    fn try_from(raw: GetVideoClipRequestRaw) -> Result<Self, Self::Error> {
        Self::new(raw.source_id, raw.range)
    }
}

impl<'de> Deserialize<'de> for GetVideoClipRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        GetVideoClipRequestRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// Constructors reserved for the malformed video requests in TCK profile 1.2.0.
///
/// These requests intentionally fail normal validation and exist only so a
/// conformance runner can verify rejection through the typed public clients.
pub struct TckMalformedVideoRequests;

impl TckMalformedVideoRequests {
    /// Lists video sources with the profile's invalid zero page limit.
    #[must_use]
    pub const fn list_video_sources_zero_limit() -> ListVideoSourcesRequest {
        ListVideoSourcesRequest {
            page: PageRequest::tck_zero_limit(),
        }
    }

    /// Prepares a clip with the profile's reversed `[start, end)` range.
    #[must_use]
    pub fn get_video_clip_reversed_range(
        source_id: VideoSourceId,
        range: TimeRange,
    ) -> GetVideoClipRequest {
        GetVideoClipRequest {
            source_id,
            range: TimeRange::tck_unchecked(range.end(), range.start()),
        }
    }

    /// Prepares a clip whose duration is one microsecond over the 60-second limit.
    #[must_use]
    pub fn get_video_clip_over_limit(
        source_id: VideoSourceId,
        start: UtcTimestamp,
    ) -> GetVideoClipRequest {
        GetVideoClipRequest {
            source_id,
            range: TimeRange::tck_unchecked(
                start,
                start.saturating_add_micros(over_limit_delta_us()),
            ),
        }
    }
}

fn over_limit_delta_us() -> i64 {
    i64::try_from(MAX_VIDEO_CLIP_DURATION_US.saturating_add(1)).unwrap_or(i64::MAX)
}

/// Short-lived clip handle. JSON never includes video bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct VideoClip {
    id: VideoClipId,
    source_id: VideoSourceId,
    requested_range: TimeRange,
    effective_range: TimeRange,
    media_type: String,
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VideoClipRaw {
    id: VideoClipId,
    source_id: VideoSourceId,
    requested_range: TimeRange,
    effective_range: TimeRange,
    media_type: String,
    path: String,
}

impl TryFrom<VideoClipRaw> for VideoClip {
    type Error = A2aLabError;

    fn try_from(raw: VideoClipRaw) -> Result<Self, Self::Error> {
        let clip = Self::new(
            raw.id,
            raw.source_id,
            raw.requested_range,
            raw.effective_range,
            raw.media_type,
        )?;
        if clip.path != raw.path {
            return Err(A2aLabError::invalid(
                "path",
                "must be a relative /media/video-clips/{id} path",
            ));
        }
        Ok(clip)
    }
}

impl<'de> Deserialize<'de> for VideoClip {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        VideoClipRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

impl VideoClip {
    /// Creates a clip handle with the required MJPEG type and relative path.
    pub fn new(
        id: VideoClipId,
        source_id: VideoSourceId,
        requested_range: TimeRange,
        effective_range: TimeRange,
        media_type: impl Into<String>,
    ) -> Result<Self, A2aLabError> {
        requested_range.check()?;
        effective_range.check()?;
        let media_type = media_type.into();
        check_mjpeg_media_type(&media_type)?;
        Ok(Self {
            path: video_clip_path(&id),
            id,
            source_id,
            requested_range,
            effective_range,
            media_type,
        })
    }

    /// Clip identifier.
    #[must_use]
    pub const fn id(&self) -> &VideoClipId {
        &self.id
    }

    /// Source the clip was cut from.
    #[must_use]
    pub const fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }

    /// Range the caller requested.
    #[must_use]
    pub const fn requested_range(&self) -> TimeRange {
        self.requested_range
    }

    /// Range actually covered by retained frames.
    #[must_use]
    pub const fn effective_range(&self) -> TimeRange {
        self.effective_range
    }

    /// MJPEG multipart media type.
    #[must_use]
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    /// Relative download path. Never a URL and never bytes.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// One JPEG frame in a clip stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    captured_at: UtcTimestamp,
    jpeg: Vec<u8>,
}

impl VideoFrame {
    /// Creates a frame when the JPEG payload is non-empty.
    pub fn new(captured_at: UtcTimestamp, jpeg: Vec<u8>) -> Result<Self, A2aLabError> {
        if jpeg.is_empty() {
            return Err(A2aLabError::invalid("jpeg", "must not be empty"));
        }
        Ok(Self { captured_at, jpeg })
    }

    /// Capture timestamp in UTC.
    #[must_use]
    pub const fn captured_at(&self) -> UtcTimestamp {
        self.captured_at
    }

    /// JPEG bytes.
    #[must_use]
    pub fn jpeg(&self) -> &[u8] {
        &self.jpeg
    }
}

/// Relative path `/media/video-clips/{id}`.
#[must_use]
pub fn video_clip_path(id: &VideoClipId) -> String {
    format!("{VIDEO_CLIP_PATH_PREFIX}{id}")
}

/// HTTP `Content-Type` for a finite MJPEG multipart body using `boundary`.
#[must_use]
pub fn mjpeg_content_type(boundary: &str) -> String {
    format!("multipart/x-mixed-replace; boundary={boundary}; type=image/jpeg")
}

/// Encodes one JPEG part, including the leading boundary and trailing CRLF.
#[must_use]
pub fn encode_mjpeg_part(frame: &VideoFrame, boundary: &str) -> Vec<u8> {
    let captured = frame.captured_at.to_rfc3339();
    let len = frame.jpeg.len();
    let mut part = Vec::with_capacity(boundary.len() + captured.len() + len + 80);
    part.extend_from_slice(b"--");
    part.extend_from_slice(boundary.as_bytes());
    part.extend_from_slice(b"\r\nContent-Type: image/jpeg\r\nX-Captured-At: ");
    part.extend_from_slice(captured.as_bytes());
    part.extend_from_slice(format!("\r\nContent-Length: {len}\r\n\r\n").as_bytes());
    part.extend_from_slice(&frame.jpeg);
    part.extend_from_slice(b"\r\n");
    part
}

/// Closed multipart epilogue `--boundary--\r\n`.
#[must_use]
pub fn encode_mjpeg_end(boundary: &str) -> Vec<u8> {
    let mut end = Vec::with_capacity(boundary.len() + 6);
    end.extend_from_slice(b"--");
    end.extend_from_slice(boundary.as_bytes());
    end.extend_from_slice(b"--\r\n");
    end
}

/// Encodes JPEG frames as `multipart/x-mixed-replace` with a closed final boundary.
///
/// Each part includes `Content-Type: image/jpeg` and `X-Captured-At` as RFC 3339 UTC.
#[must_use]
pub fn encode_mjpeg_multipart<'a, I>(frames: I, boundary: &str) -> Vec<u8>
where
    I: IntoIterator<Item = &'a VideoFrame>,
{
    let mut body = Vec::new();
    for frame in frames {
        body.extend_from_slice(&encode_mjpeg_part(frame, boundary));
    }
    body.extend_from_slice(&encode_mjpeg_end(boundary));
    body
}

/// Parses a finite MJPEG multipart body produced by [`encode_mjpeg_multipart`].
pub fn decode_mjpeg_multipart(body: &[u8], boundary: &str) -> Result<Vec<VideoFrame>, A2aLabError> {
    let marker = format!("--{boundary}").into_bytes();
    let mut frames = Vec::new();
    let mut pos = 0;
    let mut closed = false;
    while pos < body.len() {
        let Some(start) = find_bytes(&body[pos..], &marker) else {
            break;
        };
        pos += start + marker.len();
        if body.get(pos..pos.saturating_add(2)) == Some(&b"--"[..]) {
            closed = true;
            break;
        }
        if body.get(pos..pos.saturating_add(2)) != Some(&b"\r\n"[..]) {
            return Err(A2aLabError::invalid(
                "clip",
                "multipart part is missing a header",
            ));
        }
        pos += 2;
        let Some(header_end) = find_bytes(&body[pos..], b"\r\n\r\n") else {
            return Err(A2aLabError::invalid(
                "clip",
                "multipart headers are truncated",
            ));
        };
        let headers = std::str::from_utf8(&body[pos..pos + header_end])
            .map_err(|_| A2aLabError::invalid("clip", "multipart headers must be UTF-8"))?;
        pos += header_end + 4;
        let captured = header_value(headers, "X-Captured-At")
            .ok_or_else(|| A2aLabError::invalid("clip", "X-Captured-At is required"))?;
        let length = header_value(headers, "Content-Length")
            .ok_or_else(|| A2aLabError::invalid("clip", "Content-Length is required"))?
            .parse::<usize>()
            .map_err(|_| A2aLabError::invalid("clip", "Content-Length is invalid"))?;
        let end = pos
            .checked_add(length)
            .ok_or_else(|| A2aLabError::invalid("clip", "Content-Length is invalid"))?;
        if end > body.len() {
            return Err(A2aLabError::invalid("clip", "multipart body is truncated"));
        }
        let jpeg = body[pos..end].to_vec();
        pos = end;
        if body.get(pos..pos.saturating_add(2)) == Some(&b"\r\n"[..]) {
            pos += 2;
        }
        frames.push(VideoFrame::new(UtcTimestamp::parse(captured)?, jpeg)?);
    }
    if !closed {
        return Err(A2aLabError::invalid(
            "clip",
            "multipart is missing the final boundary",
        ));
    }
    Ok(frames)
}

fn header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name)
            .then(|| value.trim())
            .filter(|value| !value.is_empty())
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
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

fn check_positive(field: &'static str, value: u32) -> Result<(), A2aLabError> {
    if value == 0 {
        return Err(A2aLabError::invalid(field, "must be positive"));
    }
    Ok(())
}
