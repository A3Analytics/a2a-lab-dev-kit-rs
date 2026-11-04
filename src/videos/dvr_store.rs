//! Backend-neutral indexed DVR media storage.
//!
//! [`DvrMediaStore`] owns retention bounds, ordered coverage with genuine gaps,
//! indexed tracks and segments, keyframe metadata, bounded window reads, and
//! frame-at-time lookup. Implementations may persist in memory, on disk, in an
//! NVR, object storage, or a remote stream. They are not required to emit HLS
//! manifests, construct finite MP4 exports, expose HTTP routes, or authenticate
//! callers.

use crate::error::A2aLabError;
use crate::id::{DvrTrackId, VideoSourceId};
use crate::page::{Page, PageRequest};
use crate::time::{TimeRange, UtcTimestamp};
use crate::videos::codec::VideoCodec;

/// Retained UTC bounds still indexed for one source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DvrRetentionBounds {
    oldest: UtcTimestamp,
    newest: UtcTimestamp,
}

impl DvrRetentionBounds {
    /// Creates bounds when `oldest <= newest`.
    pub fn new(oldest: UtcTimestamp, newest: UtcTimestamp) -> Result<Self, A2aLabError> {
        if oldest > newest {
            return Err(A2aLabError::invalid(
                "retention",
                "oldest retained instant must not follow newest",
            ));
        }
        Ok(Self { oldest, newest })
    }

    /// Oldest retained instant.
    #[must_use]
    pub const fn oldest(&self) -> UtcTimestamp {
        self.oldest
    }

    /// Newest retained instant.
    #[must_use]
    pub const fn newest(&self) -> UtcTimestamp {
        self.newest
    }
}

/// A genuine temporal gap between two retained intervals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DvrDiscontinuity {
    gap: TimeRange,
}

impl DvrDiscontinuity {
    /// Creates a half-open gap `[start, end)` with no retained media.
    pub fn new(gap: TimeRange) -> Result<Self, A2aLabError> {
        gap.check()?;
        Ok(Self { gap })
    }

    /// Half-open gap with no retained media.
    #[must_use]
    pub const fn gap(&self) -> TimeRange {
        self.gap
    }
}

/// Ordered retained intervals and explicit discontinuities between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DvrMediaCoverage {
    intervals: Vec<TimeRange>,
    discontinuities: Vec<DvrDiscontinuity>,
}

impl DvrMediaCoverage {
    /// Creates coverage when intervals are ordered, half-open, and separated by gaps.
    pub fn new(intervals: Vec<TimeRange>) -> Result<Self, A2aLabError> {
        validate_intervals(&intervals)?;
        let discontinuities = gaps_between(&intervals)?
            .into_iter()
            .map(DvrDiscontinuity::new)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            intervals,
            discontinuities,
        })
    }

    /// Ordered half-open retained intervals.
    #[must_use]
    pub fn intervals(&self) -> &[TimeRange] {
        &self.intervals
    }

    /// Ordered gaps between consecutive intervals.
    #[must_use]
    pub fn discontinuities(&self) -> &[DvrDiscontinuity] {
        &self.discontinuities
    }
}

/// One indexed media track for a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DvrMediaTrack {
    id: DvrTrackId,
    codec: VideoCodec,
    codec_initialization: Vec<u8>,
    bounds: TimeRange,
}

impl DvrMediaTrack {
    /// Creates a track when bounds are valid.
    pub fn new(
        id: DvrTrackId,
        codec: VideoCodec,
        codec_initialization: Vec<u8>,
        bounds: TimeRange,
    ) -> Result<Self, A2aLabError> {
        bounds.check()?;
        Ok(Self {
            id,
            codec,
            codec_initialization,
            bounds,
        })
    }

    /// Track identifier.
    #[must_use]
    pub const fn id(&self) -> &DvrTrackId {
        &self.id
    }

    /// Stored codec for this track.
    #[must_use]
    pub const fn codec(&self) -> VideoCodec {
        self.codec
    }

    /// Codec initialization bytes (empty for MJPEG).
    #[must_use]
    pub fn codec_initialization(&self) -> &[u8] {
        &self.codec_initialization
    }

    /// UTC bounds of indexed media on this track.
    #[must_use]
    pub const fn bounds(&self) -> TimeRange {
        self.bounds
    }
}

/// Payload for one indexed segment or frame-at-time sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DvrMediaPayload {
    /// One MJPEG frame.
    MjpegFrame(Vec<u8>),
    /// One encoded sample for H.264 or H.265.
    EncodedSample {
        /// Stored codec.
        codec: VideoCodec,
        /// Encoded bytes (not packaged as MP4 or HLS).
        bytes: Vec<u8>,
    },
}

/// One indexed segment inside a bounded read window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DvrMediaSegment {
    track_id: DvrTrackId,
    range: TimeRange,
    keyframe: bool,
    payload: DvrMediaPayload,
}

impl DvrMediaSegment {
    /// Creates a segment when its range is valid.
    pub fn new(
        track_id: DvrTrackId,
        range: TimeRange,
        keyframe: bool,
        payload: DvrMediaPayload,
    ) -> Result<Self, A2aLabError> {
        range.check()?;
        Ok(Self {
            track_id,
            range,
            keyframe,
            payload,
        })
    }

    /// Track this segment belongs to.
    #[must_use]
    pub const fn track_id(&self) -> &DvrTrackId {
        &self.track_id
    }

    /// Half-open UTC bounds for this segment.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        self.range
    }

    /// Whether this segment starts on a keyframe (always true for MJPEG frames).
    #[must_use]
    pub const fn keyframe(&self) -> bool {
        self.keyframe
    }

    /// Stored media bytes for this segment.
    #[must_use]
    pub const fn payload(&self) -> &DvrMediaPayload {
        &self.payload
    }
}

/// Request for a bounded segment read inside one retained window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadDvrMediaWindowRequest {
    source_id: VideoSourceId,
    track_id: DvrTrackId,
    window: TimeRange,
    page: PageRequest,
}

impl ReadDvrMediaWindowRequest {
    /// Creates a window read after validating the range and page.
    pub fn new(
        source_id: VideoSourceId,
        track_id: DvrTrackId,
        window: TimeRange,
        page: PageRequest,
    ) -> Result<Self, A2aLabError> {
        window.check()?;
        page.check()?;
        Ok(Self {
            source_id,
            track_id,
            window,
            page,
        })
    }

    /// Source to read.
    #[must_use]
    pub const fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }

    /// Track to read.
    #[must_use]
    pub const fn track_id(&self) -> &DvrTrackId {
        &self.track_id
    }

    /// Half-open UTC window `[start, end)` to intersect with retained segments.
    #[must_use]
    pub const fn window(&self) -> TimeRange {
        self.window
    }

    /// Pagination for segments inside the window.
    #[must_use]
    pub const fn page(&self) -> &PageRequest {
        &self.page
    }
}

/// Frame-at-time lookup against indexed retained media.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DvrFrameAtRequest {
    source_id: VideoSourceId,
    track_id: DvrTrackId,
    at: UtcTimestamp,
    max_delta_us: Option<u64>,
}

impl DvrFrameAtRequest {
    /// Creates a lookup for `at` on `track_id`.
    #[must_use]
    pub const fn new(
        source_id: VideoSourceId,
        track_id: DvrTrackId,
        at: UtcTimestamp,
        max_delta_us: Option<u64>,
    ) -> Self {
        Self {
            source_id,
            track_id,
            at,
            max_delta_us,
        }
    }

    /// Source to read.
    #[must_use]
    pub const fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }

    /// Track to read.
    #[must_use]
    pub const fn track_id(&self) -> &DvrTrackId {
        &self.track_id
    }

    /// Requested UTC instant.
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

/// Frame-at-time result with requested and actual UTC timestamps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DvrFrameAtResult {
    requested_at: UtcTimestamp,
    actual_at: UtcTimestamp,
    delta_us: i64,
    payload: DvrMediaPayload,
}

impl DvrFrameAtResult {
    /// Pairs a sample with lookup timestamps.
    #[must_use]
    pub fn new(
        requested_at: UtcTimestamp,
        actual_at: UtcTimestamp,
        payload: DvrMediaPayload,
    ) -> Self {
        Self {
            requested_at,
            actual_at,
            delta_us: actual_at.as_micros() - requested_at.as_micros(),
            payload,
        }
    }

    /// Instant the caller asked for.
    #[must_use]
    pub const fn requested_at(&self) -> UtcTimestamp {
        self.requested_at
    }

    /// UTC instant of the selected sample.
    #[must_use]
    pub const fn actual_at(&self) -> UtcTimestamp {
        self.actual_at
    }

    /// Signed delta `actual - requested` in microseconds.
    #[must_use]
    pub const fn delta_us(&self) -> i64 {
        self.delta_us
    }

    /// Selected retained sample.
    #[must_use]
    pub const fn payload(&self) -> &DvrMediaPayload {
        &self.payload
    }
}

/// Indexed recording storage shared by DVR backends.
pub trait DvrMediaStore: Send + Sync {
    /// Returns the UTC bounds still indexed for `source_id`.
    fn retention_bounds(
        &self,
        source_id: &VideoSourceId,
    ) -> impl Future<Output = Result<DvrRetentionBounds, A2aLabError>> + Send;

    /// Returns ordered retained intervals and explicit discontinuities.
    fn media_coverage(
        &self,
        source_id: &VideoSourceId,
    ) -> impl Future<Output = Result<DvrMediaCoverage, A2aLabError>> + Send;

    /// Lists indexed tracks with codec initialization and UTC bounds.
    fn list_tracks(
        &self,
        source_id: &VideoSourceId,
    ) -> impl Future<Output = Result<Vec<DvrMediaTrack>, A2aLabError>> + Send;

    /// Reads up to one page of segments intersecting `request.window()`.
    fn read_media_window(
        &self,
        request: ReadDvrMediaWindowRequest,
    ) -> impl Future<Output = Result<Page<DvrMediaSegment>, A2aLabError>> + Send;

    /// Lists the same page as [`Self::read_media_window`] without loading payloads.
    ///
    /// Playlist planning only needs ranges and keyframe flags. The default
    /// reads payloads; stores with large samples override this.
    fn read_media_spans(
        &self,
        request: ReadDvrMediaWindowRequest,
    ) -> impl Future<Output = Result<Page<DvrMediaSegment>, A2aLabError>> + Send {
        self.read_media_window(request)
    }

    /// Returns the nearest retained sample at `request.at()`.
    fn frame_at(
        &self,
        request: DvrFrameAtRequest,
    ) -> impl Future<Output = Result<DvrFrameAtResult, A2aLabError>> + Send;
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

fn gaps_between(intervals: &[TimeRange]) -> Result<Vec<TimeRange>, A2aLabError> {
    let mut gaps = Vec::new();
    let mut previous: Option<TimeRange> = None;
    for interval in intervals {
        if let Some(prev) = previous {
            let gap = TimeRange::new(prev.end(), interval.start())?;
            gaps.push(gap);
        }
        previous = Some(*interval);
    }
    Ok(gaps)
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn coverage_derives_ordered_discontinuities() {
        let start = UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap();
        let gap_end = UtcTimestamp::parse("2024-01-01T00:00:02Z").unwrap();
        let second = UtcTimestamp::parse("2024-01-01T00:00:04Z").unwrap();
        let end = UtcTimestamp::parse("2024-01-01T00:00:06Z").unwrap();
        let coverage = DvrMediaCoverage::new(vec![
            TimeRange::new(start, gap_end).unwrap(),
            TimeRange::new(second, end).unwrap(),
        ])
        .unwrap();
        assert_eq!(coverage.intervals().len(), 2);
        assert_eq!(coverage.discontinuities().len(), 1);
        assert_eq!(coverage.discontinuities()[0].gap().start(), gap_end);
        assert_eq!(coverage.discontinuities()[0].gap().end(), second);
    }
}
