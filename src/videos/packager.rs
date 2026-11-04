//! Shared HLS/CMAF and finite export packaging over [`DvrMediaStore`].
//!
//! Stores retain indexed media only. This layer turns bounded store windows into
//! session-relative manifests, init segments, `.m4s` media segments, finite MP4
//! exports, and closed MJPEG multipart bodies.

use crate::error::A2aLabError;
use crate::id::{DvrTrackId, VideoSourceId};
use crate::page::{MAX_PAGE_LIMIT, PageRequest};
use crate::time::{TimeRange, UtcTimestamp};
use crate::videos::codec::VideoCodec;
use crate::videos::dvr_store::{
    DvrMediaCoverage, DvrMediaPayload, DvrMediaSegment, DvrMediaStore, DvrMediaTrack,
    DvrRetentionBounds, ReadDvrMediaWindowRequest,
};
use crate::videos::model::{
    DEFAULT_MJPEG_BOUNDARY, VideoFrame, VideoPlayback8xMetadata, encode_mjpeg_end,
    encode_mjpeg_multipart,
};

use crate::videos::cmaf;

/// Default half-width of a sliding HLS manifest around an anchor instant.
pub const DEFAULT_PLAYBACK_WINDOW_US: u64 = 120_000_000;

/// Session-relative init segment for CMAF/HLS.
pub const HLS_INIT_RELATIVE_PATH: &str = "init.mp4";

/// Session-relative media playlist for a bounded window.
pub const HLS_PLAYLIST_RELATIVE_PATH: &str = "index.m3u8";

/// Session-relative 8x trick-play playlist when the indexed media supports it.
pub const HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH: &str = "index-8x.m3u8";

/// Reason code when trick play is omitted for sparse keyframes.
pub const TRICK_8X_UNAVAILABLE_INSUFFICIENT_KEYFRAMES: &str = "insufficient_keyframes";

/// Reason code when trick play would not reduce samples for the bounded window.
pub const TRICK_8X_UNAVAILABLE_NO_REDUCTION: &str = "no_sample_reduction";

const TRICK_8X_DECIMATION: usize = 8;
const NORMAL_SEGMENT_PREFIX: &str = "seg-";
const TRICK_8X_SEGMENT_PREFIX: &str = "seg-8x-";
const HLS_SEGMENT_TARGET_US: i64 = 5_000_000;

/// Configuration for bounded manifest generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DvrPackagerConfig {
    playback_window_us: u64,
}

impl DvrPackagerConfig {
    /// Creates config when the playback window is non-zero.
    pub fn new(playback_window_us: u64) -> Result<Self, A2aLabError> {
        if playback_window_us == 0 {
            return Err(A2aLabError::invalid(
                "playback_window_us",
                "must be greater than zero",
            ));
        }
        Ok(Self { playback_window_us })
    }

    /// Maximum UTC span covered by one generated HLS manifest.
    #[must_use]
    pub const fn playback_window_us(&self) -> u64 {
        self.playback_window_us
    }
}

impl Default for DvrPackagerConfig {
    fn default() -> Self {
        Self {
            playback_window_us: DEFAULT_PLAYBACK_WINDOW_US,
        }
    }
}

/// One packaged byte asset with its HTTP content type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagedMediaAsset {
    content_type: String,
    bytes: Vec<u8>,
}

impl PackagedMediaAsset {
    /// Creates an asset when the payload is non-empty.
    pub fn new(content_type: impl Into<String>, bytes: Vec<u8>) -> Result<Self, A2aLabError> {
        if bytes.is_empty() {
            return Err(A2aLabError::invalid("bytes", "must not be empty"));
        }
        Ok(Self {
            content_type: content_type.into(),
            bytes,
        })
    }

    /// Declared HTTP `Content-Type`.
    #[must_use]
    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    /// Packaged bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Metadata for one segment listed in an HLS manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct PackagedHlsSegment {
    relative_path: String,
    range: TimeRange,
    duration_secs: f64,
    program_date_time: UtcTimestamp,
    discontinuity_before: bool,
}

impl PackagedHlsSegment {
    /// Session-relative `.m4s` path.
    #[must_use]
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    /// UTC bounds of the indexed store segment.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        self.range
    }

    /// `#EXTINF` duration in seconds.
    #[must_use]
    pub fn duration_secs(&self) -> f64 {
        self.duration_secs
    }

    /// Wall-clock anchor for `#EXT-X-PROGRAM-DATE-TIME`.
    #[must_use]
    pub const fn program_date_time(&self) -> UtcTimestamp {
        self.program_date_time
    }

    /// Whether `#EXT-X-DISCONTINUITY` precedes this segment.
    #[must_use]
    pub const fn discontinuity_before(&self) -> bool {
        self.discontinuity_before
    }
}

/// Optional bounded 8x trick-play rendition for one HLS session.
#[derive(Debug, Clone, PartialEq)]
pub struct PackagedHlsTrick8xRendition {
    playlist_relative_path: String,
    playlist: String,
    segments: Vec<PackagedHlsSegment>,
}

impl PackagedHlsTrick8xRendition {
    /// Session-relative trick-play playlist path.
    #[must_use]
    pub fn playlist_relative_path(&self) -> &str {
        &self.playlist_relative_path
    }

    /// Generated `#EXTM3U` body for the 8x rendition.
    #[must_use]
    pub fn playlist(&self) -> &str {
        &self.playlist
    }

    /// Ordered trick-play segment entries referenced by the playlist.
    #[must_use]
    pub fn segments(&self) -> &[PackagedHlsSegment] {
        &self.segments
    }
}

/// Bounded HLS/CMAF session rooted at `session_root`.
#[derive(Debug, Clone, PartialEq)]
pub struct PackagedHlsSession {
    session_root: String,
    source_id: VideoSourceId,
    track_id: DvrTrackId,
    anchor: UtcTimestamp,
    window: TimeRange,
    codec: VideoCodec,
    init_relative_path: String,
    playlist_relative_path: String,
    playlist: String,
    segments: Vec<PackagedHlsSegment>,
    /// Decode-time zero for every segment in this follow. Live refreshes keep the
    /// first session's origin so later segments continue on the same timeline.
    timeline_origin_us: i64,
    track: DvrMediaTrack,
    trick_8x: Option<PackagedHlsTrick8xRendition>,
    trick_8x_unavailable_reason: Option<String>,
}

impl PackagedHlsSession {
    /// Opaque authenticated gateway root ending in `/`.
    #[must_use]
    pub fn session_root(&self) -> &str {
        &self.session_root
    }

    /// Source packaged by this session.
    #[must_use]
    pub fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }

    /// Track packaged by this session.
    #[must_use]
    pub fn track_id(&self) -> &DvrTrackId {
        &self.track_id
    }

    /// Anchor instant for this bounded window.
    #[must_use]
    pub const fn anchor(&self) -> UtcTimestamp {
        self.anchor
    }

    /// UTC span covered by the manifest.
    #[must_use]
    pub const fn window(&self) -> TimeRange {
        self.window
    }

    /// Encoded codec for this session.
    #[must_use]
    pub const fn codec(&self) -> VideoCodec {
        self.codec
    }

    /// Session-relative init segment path.
    #[must_use]
    pub fn init_relative_path(&self) -> &str {
        &self.init_relative_path
    }

    /// Session-relative media playlist path.
    #[must_use]
    pub fn playlist_relative_path(&self) -> &str {
        &self.playlist_relative_path
    }

    /// Generated `#EXTM3U` body.
    #[must_use]
    pub fn playlist(&self) -> &str {
        &self.playlist
    }

    /// Ordered segment entries referenced by the playlist.
    #[must_use]
    pub fn segments(&self) -> &[PackagedHlsSegment] {
        &self.segments
    }

    /// Microsecond instant encoded as decode time zero for this session.
    #[must_use]
    pub const fn timeline_origin_us(&self) -> i64 {
        self.timeline_origin_us
    }

    /// Keeps segment timestamps on the timeline established by an earlier refresh.
    pub(crate) fn pin_timeline_origin(&mut self, origin_us: i64) {
        self.timeline_origin_us = origin_us;
    }

    /// Optional 8x trick-play rendition for the same bounded window.
    #[must_use]
    pub fn trick_8x(&self) -> Option<&PackagedHlsTrick8xRendition> {
        self.trick_8x.as_ref()
    }

    /// Stable reason code when [`Self::trick_8x`] is absent.
    #[must_use]
    pub fn trick_8x_unavailable_reason(&self) -> Option<&str> {
        self.trick_8x_unavailable_reason.as_deref()
    }

    /// Session metadata for optional 8x trick play and recoverable 1x fallback.
    #[must_use]
    pub fn playback_8x_metadata(&self) -> VideoPlayback8xMetadata {
        if let Some(trick) = &self.trick_8x {
            VideoPlayback8xMetadata::available(
                trick.playlist_relative_path(),
                HLS_PLAYLIST_RELATIVE_PATH,
            )
        } else {
            VideoPlayback8xMetadata::unavailable(
                HLS_PLAYLIST_RELATIVE_PATH,
                self.trick_8x_unavailable_reason
                    .as_deref()
                    .unwrap_or(TRICK_8X_UNAVAILABLE_INSUFFICIENT_KEYFRAMES),
            )
        }
    }
}

/// Shared packaging layer over any [`DvrMediaStore`].
#[derive(Debug, Clone, Copy, Default)]
pub struct DvrMediaPackager {
    config: DvrPackagerConfig,
}

impl DvrMediaPackager {
    /// Creates a packager with explicit window configuration.
    #[must_use]
    pub const fn new(config: DvrPackagerConfig) -> Self {
        Self { config }
    }

    /// Returns the configured playback window.
    #[must_use]
    pub const fn config(&self) -> DvrPackagerConfig {
        self.config
    }

    /// Builds a bounded HLS/CMAF session around `anchor`.
    pub async fn package_hls_at(
        &self,
        store: &impl DvrMediaStore,
        session_root: &str,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        anchor: UtcTimestamp,
    ) -> Result<PackagedHlsSession, A2aLabError> {
        let retention = store.retention_bounds(source_id).await?;
        let coverage = store.media_coverage(source_id).await?;
        let track = find_track(store, source_id, track_id).await?;
        ensure_encoded_track(&track)?;
        let window = playback_window(
            anchor,
            self.config.playback_window_us(),
            retention,
            &coverage,
        )?;
        self.build_hls_session(
            store,
            session_root,
            source_id,
            track,
            anchor,
            window,
            &coverage,
            PlaylistKind::Finite,
        )
        .await
    }

    /// Packages the retained range itself as one finite HLS session.
    pub async fn package_hls_range(
        &self,
        store: &impl DvrMediaStore,
        session_root: &str,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        range: TimeRange,
    ) -> Result<PackagedHlsSession, A2aLabError> {
        range.check()?;
        let coverage = store.media_coverage(source_id).await?;
        let track = find_track(store, source_id, track_id).await?;
        ensure_encoded_track(&track)?;
        self.build_hls_session(
            store,
            session_root,
            source_id,
            track,
            range.start(),
            range,
            &coverage,
            PlaylistKind::Finite,
        )
        .await
    }

    /// Re-anchors an existing session to a new bounded window at `new_anchor`.
    pub async fn rebase_hls(
        &self,
        store: &impl DvrMediaStore,
        session: &PackagedHlsSession,
        new_anchor: UtcTimestamp,
    ) -> Result<PackagedHlsSession, A2aLabError> {
        self.package_hls_at(
            store,
            session.session_root(),
            session.source_id(),
            session.track_id(),
            new_anchor,
        )
        .await
    }

    /// Builds a bounded live HLS/CMAF session around `anchor` for refreshable follow.
    pub async fn package_live_hls_at(
        &self,
        store: &impl DvrMediaStore,
        session_root: &str,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        anchor: UtcTimestamp,
    ) -> Result<PackagedHlsSession, A2aLabError> {
        let retention = store.retention_bounds(source_id).await?;
        let coverage = store.media_coverage(source_id).await?;
        let track = find_track(store, source_id, track_id).await?;
        ensure_encoded_track(&track)?;
        let window = playback_window(
            anchor,
            self.config.playback_window_us(),
            retention,
            &coverage,
        )?;
        self.build_hls_session(
            store,
            session_root,
            source_id,
            track,
            anchor,
            window,
            &coverage,
            PlaylistKind::Live,
        )
        .await
    }

    /// Returns the newest retained instant suitable for anchoring a live-edge window.
    pub async fn live_edge_anchor(
        store: &impl DvrMediaStore,
        source_id: &VideoSourceId,
    ) -> Result<UtcTimestamp, A2aLabError> {
        let retention = store.retention_bounds(source_id).await?;
        Ok(retention.newest().saturating_add_micros(-1))
    }

    /// Loads one session-relative packaged asset (`init.mp4`, `index.m3u8`, or `.m4s`).
    pub async fn load_hls_asset(
        &self,
        store: &impl DvrMediaStore,
        session: &PackagedHlsSession,
        relative_path: &str,
    ) -> Result<PackagedMediaAsset, A2aLabError> {
        if relative_path == session.init_relative_path() {
            let bytes = cmaf::init_segment(&session.track)?;
            return PackagedMediaAsset::new("video/mp4", bytes);
        }
        if relative_path == session.playlist_relative_path() {
            return PackagedMediaAsset::new(
                "application/vnd.apple.mpegurl",
                session.playlist().as_bytes().to_vec(),
            );
        }
        if let Some(trick) = session.trick_8x.as_ref() {
            if relative_path == trick.playlist_relative_path() {
                return PackagedMediaAsset::new(
                    "application/vnd.apple.mpegurl",
                    trick.playlist().as_bytes().to_vec(),
                );
            }
            if let Some(segment) = trick
                .segments()
                .iter()
                .find(|item| item.relative_path() == relative_path)
            {
                return self
                    .load_hls_segment_asset(store, session, segment, trick.segments())
                    .await;
            }
        }
        let segment = session
            .segments()
            .iter()
            .find(|item| item.relative_path() == relative_path)
            .ok_or_else(|| A2aLabError::not_found("packaged asset", relative_path.to_owned()))?;
        self.load_hls_segment_asset(store, session, segment, session.segments())
            .await
    }

    async fn load_hls_segment_asset(
        &self,
        store: &impl DvrMediaStore,
        session: &PackagedHlsSession,
        segment: &PackagedHlsSegment,
        listed: &[PackagedHlsSegment],
    ) -> Result<PackagedMediaAsset, A2aLabError> {
        let frames = read_all_segments(
            store,
            session.source_id(),
            session.track_id(),
            segment.range(),
        )
        .await?;
        if frames.is_empty() {
            return Err(A2aLabError::not_found(
                "packaged segment",
                segment.relative_path().to_owned(),
            ));
        }
        let index = listed
            .iter()
            .position(|item| item.relative_path() == segment.relative_path())
            .unwrap_or(0);
        let origin = session.timeline_origin_us();
        let next_segment_start = listed
            .get(index.saturating_add(1))
            .map(|next| next.range().start().as_micros());
        let mut parts = Vec::with_capacity(frames.len());
        for (frame_index, frame) in frames.iter().enumerate() {
            let start = frame.range().start().as_micros();
            let natural = cmaf::segment_duration_us(frame.range());
            let next_start = frames
                .get(frame_index.saturating_add(1))
                .map(|next| next.range().start().as_micros())
                .or(next_segment_start);
            let sequence = u32::try_from(start.saturating_sub(origin) / 1000).unwrap_or(u32::MAX);
            parts.push((
                frame,
                cmaf::FragmentTiming {
                    sequence,
                    decode_time_us: u64::try_from(start.saturating_sub(origin)).unwrap_or(0),
                    duration_us: abutted_duration_us(start, natural, next_start),
                },
            ));
        }
        let bytes = cmaf::media_fragments(&session.track, &parts)?;
        PackagedMediaAsset::new("video/iso.segment", bytes)
    }

    /// Packages a finite MP4 export for an encoded retained range.
    pub async fn export_finite_mp4(
        &self,
        store: &impl DvrMediaStore,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        range: TimeRange,
    ) -> Result<PackagedMediaAsset, A2aLabError> {
        range.check()?;
        let track = find_track(store, source_id, track_id).await?;
        ensure_encoded_track(&track)?;
        let segments = read_all_segments(store, source_id, track_id, range).await?;
        if segments.is_empty() {
            return Err(A2aLabError::no_recording(
                "retained range contains no encoded segments",
            ));
        }
        let bytes = cmaf::fragmented_mp4_export(&track, &segments)?;
        PackagedMediaAsset::new("video/mp4", bytes)
    }

    /// Packages a finite closed MJPEG multipart export for a retained range.
    pub async fn export_finite_mjpeg(
        &self,
        store: &impl DvrMediaStore,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        range: TimeRange,
    ) -> Result<PackagedMediaAsset, A2aLabError> {
        range.check()?;
        let track = find_track(store, source_id, track_id).await?;
        if track.codec() != VideoCodec::Mjpeg {
            return Err(A2aLabError::invalid(
                "codec",
                "MJPEG export requires an MJPEG track",
            ));
        }
        let segments = read_all_segments(store, source_id, track_id, range).await?;
        if segments.is_empty() {
            return Err(A2aLabError::no_recording(
                "retained range contains no MJPEG frames",
            ));
        }
        let frames = segments
            .into_iter()
            .map(|segment| match segment.payload() {
                DvrMediaPayload::MjpegFrame(jpeg) => {
                    VideoFrame::new(segment.range().start(), jpeg.clone())
                }
                DvrMediaPayload::EncodedSample { .. } => Err(A2aLabError::invalid(
                    "payload",
                    "MJPEG export must not transcode encoded samples",
                )),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut body = encode_mjpeg_multipart(&frames, DEFAULT_MJPEG_BOUNDARY);
        body.extend_from_slice(&encode_mjpeg_end(DEFAULT_MJPEG_BOUNDARY));
        PackagedMediaAsset::new(
            crate::videos::model::mjpeg_content_type(DEFAULT_MJPEG_BOUNDARY),
            body,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaylistKind {
    Finite,
    Live,
}

impl DvrMediaPackager {
    async fn build_hls_session(
        &self,
        store: &impl DvrMediaStore,
        session_root: &str,
        source_id: &VideoSourceId,
        track: DvrMediaTrack,
        anchor: UtcTimestamp,
        window: TimeRange,
        coverage: &DvrMediaCoverage,
        kind: PlaylistKind,
    ) -> Result<PackagedHlsSession, A2aLabError> {
        validate_session_root(session_root)?;
        cmaf::init_segment(&track)?;
        let stored = read_all_spans(store, source_id, track.id(), window).await?;
        let segments = plan_hls_segments(&stored, coverage, window, NORMAL_SEGMENT_PREFIX, kind);
        let playlist = match kind {
            PlaylistKind::Finite => render_finite_playlist(&segments)?,
            PlaylistKind::Live => render_live_playlist(&segments)?,
        };
        let (trick_8x, trick_8x_unavailable_reason) =
            plan_trick_8x_rendition(&stored, coverage, window, kind, segments.len());
        let timeline_origin_us = segments.first().map_or_else(
            || window.start().as_micros(),
            |segment| segment.range().start().as_micros(),
        );
        Ok(PackagedHlsSession {
            session_root: normalize_session_root(session_root),
            source_id: source_id.clone(),
            track_id: track.id().clone(),
            anchor,
            window,
            codec: track.codec(),
            init_relative_path: HLS_INIT_RELATIVE_PATH.to_owned(),
            playlist_relative_path: HLS_PLAYLIST_RELATIVE_PATH.to_owned(),
            playlist,
            segments,
            timeline_origin_us,
            track,
            trick_8x,
            trick_8x_unavailable_reason,
        })
    }
}

async fn find_track(
    store: &impl DvrMediaStore,
    source_id: &VideoSourceId,
    track_id: &DvrTrackId,
) -> Result<DvrMediaTrack, A2aLabError> {
    store
        .list_tracks(source_id)
        .await?
        .into_iter()
        .find(|track| track.id() == track_id)
        .ok_or_else(|| A2aLabError::not_found("DVR track", track_id.to_string()))
}

fn ensure_encoded_track(track: &DvrMediaTrack) -> Result<(), A2aLabError> {
    match track.codec() {
        VideoCodec::H264 | VideoCodec::H265 => Ok(()),
        VideoCodec::Mjpeg => Err(A2aLabError::invalid(
            "codec",
            "HLS/CMAF packaging requires H.264 or H.265",
        )),
    }
}

fn playback_window(
    anchor: UtcTimestamp,
    window_us: u64,
    retention: DvrRetentionBounds,
    coverage: &DvrMediaCoverage,
) -> Result<TimeRange, A2aLabError> {
    let half = i64::try_from(window_us / 2).unwrap_or(i64::MAX);
    let mut start = anchor.saturating_add_micros(-half);
    let mut end = anchor.saturating_add_micros(
        i64::try_from(window_us.saturating_sub(window_us / 2)).unwrap_or(i64::MAX),
    );
    start = start.max(retention.oldest());
    end = end.min(retention.newest());
    if start >= end {
        return Err(A2aLabError::no_recording(
            "anchor does not intersect retained media",
        ));
    }
    let window = TimeRange::new(start, end)?;
    if !coverage
        .intervals()
        .iter()
        .any(|interval| intervals_overlap(*interval, window))
    {
        return Err(A2aLabError::no_recording(
            "playback window does not intersect retained coverage",
        ));
    }
    Ok(window)
}

fn intervals_overlap(left: TimeRange, right: TimeRange) -> bool {
    left.start() < right.end() && right.start() < left.end()
}

async fn read_all_spans(
    store: &impl DvrMediaStore,
    source_id: &VideoSourceId,
    track_id: &DvrTrackId,
    window: TimeRange,
) -> Result<Vec<DvrMediaSegment>, A2aLabError> {
    let mut cursor = None;
    let mut collected = Vec::new();
    loop {
        let page = store
            .read_media_spans(ReadDvrMediaWindowRequest::new(
                source_id.clone(),
                track_id.clone(),
                window,
                PageRequest::new(cursor, MAX_PAGE_LIMIT)?,
            )?)
            .await?;
        collected.extend(page.items().iter().cloned());
        cursor = page.next_cursor().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    Ok(collected)
}

async fn read_all_segments(
    store: &impl DvrMediaStore,
    source_id: &VideoSourceId,
    track_id: &DvrTrackId,
    window: TimeRange,
) -> Result<Vec<DvrMediaSegment>, A2aLabError> {
    let mut cursor = None;
    let mut collected = Vec::new();
    loop {
        let page = store
            .read_media_window(ReadDvrMediaWindowRequest::new(
                source_id.clone(),
                track_id.clone(),
                window,
                PageRequest::new(cursor, MAX_PAGE_LIMIT)?,
            )?)
            .await?;
        collected.extend(page.items().iter().cloned());
        cursor = page.next_cursor().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    Ok(collected)
}

fn plan_hls_segments(
    stored: &[DvrMediaSegment],
    coverage: &DvrMediaCoverage,
    window: TimeRange,
    segment_prefix: &str,
    kind: PlaylistKind,
) -> Vec<PackagedHlsSegment> {
    let frames: Vec<&DvrMediaSegment> = stored
        .iter()
        .filter(|segment| intervals_overlap(segment.range(), window))
        .collect();
    let mut groups: Vec<(bool, Vec<&DvrMediaSegment>)> = Vec::new();
    let mut current: Vec<&DvrMediaSegment> = Vec::new();
    let mut current_break = false;
    for frame in frames {
        if let Some(previous) = current.last().copied() {
            let bucket_changed = frame
                .range()
                .start()
                .as_micros()
                .div_euclid(HLS_SEGMENT_TARGET_US)
                != current[0]
                    .range()
                    .start()
                    .as_micros()
                    .div_euclid(HLS_SEGMENT_TARGET_US);
            let gap = frames_are_separated(previous, frame)
                || discontinuity_before_segment(frame.range().start(), coverage);
            if gap || bucket_changed {
                groups.push((current_break, std::mem::take(&mut current)));
                current_break = gap;
            }
        } else {
            current_break = discontinuity_before_segment(frame.range().start(), coverage);
        }
        current.push(frame);
    }
    if !current.is_empty() {
        let span = current
            .last()
            .map_or(0, |frame| frame.range().end().as_micros())
            .saturating_sub(current[0].range().start().as_micros());
        if kind != PlaylistKind::Live || span >= HLS_SEGMENT_TARGET_US || groups.is_empty() {
            groups.push((current_break, current));
        }
    }
    groups
        .into_iter()
        .map(|(discontinuity_before, frames)| {
            grouped_segment(segment_prefix, discontinuity_before, &frames)
        })
        .collect()
}

fn grouped_segment(
    segment_prefix: &str,
    discontinuity_before: bool,
    frames: &[&DvrMediaSegment],
) -> PackagedHlsSegment {
    let start = frames[0].range().start();
    let end = frames.last().map_or(start, |frame| frame.range().end());
    let duration_us = u64::try_from(end.as_micros().saturating_sub(start.as_micros())).unwrap_or(0);
    #[allow(clippy::cast_precision_loss)]
    let duration_secs = duration_us as f64 / 1_000_000.0;
    PackagedHlsSegment {
        relative_path: format!(
            "{segment_prefix}{}.m4s",
            frames[0].range().start().as_micros()
        ),
        range: TimeRange::new(start, end).unwrap_or_else(|_| frames[0].range()),
        duration_secs,
        program_date_time: start,
        discontinuity_before,
    }
}

fn frames_are_separated(previous: &DvrMediaSegment, next: &DvrMediaSegment) -> bool {
    let period = i64::try_from(cmaf::segment_duration_us(previous.range())).unwrap_or(1);
    next.range()
        .start()
        .as_micros()
        .saturating_sub(previous.range().end().as_micros())
        > period.max(1)
}

fn abutted_duration_us(start: i64, natural: u64, next_start: Option<i64>) -> u64 {
    let Some(next) = next_start else {
        return natural.max(1);
    };
    let gap = u64::try_from(next.saturating_sub(start)).unwrap_or(natural);
    if gap > 0 && gap < natural.saturating_mul(2) {
        gap
    } else {
        natural.max(1)
    }
}

fn segments_in_window(stored: &[DvrMediaSegment], window: TimeRange) -> Vec<DvrMediaSegment> {
    stored
        .iter()
        .filter(|segment| intervals_overlap(segment.range(), window))
        .cloned()
        .collect()
}

fn plan_trick_8x_rendition(
    stored: &[DvrMediaSegment],
    coverage: &DvrMediaCoverage,
    window: TimeRange,
    kind: PlaylistKind,
    normal_segment_count: usize,
) -> (Option<PackagedHlsTrick8xRendition>, Option<String>) {
    let in_window = segments_in_window(stored, window);
    if in_window.is_empty() || normal_segment_count == 0 {
        return (
            None,
            Some(TRICK_8X_UNAVAILABLE_INSUFFICIENT_KEYFRAMES.to_owned()),
        );
    }
    if !in_window[0].keyframe() {
        return (
            None,
            Some(TRICK_8X_UNAVAILABLE_INSUFFICIENT_KEYFRAMES.to_owned()),
        );
    }

    let keyframe_only: Vec<DvrMediaSegment> = in_window
        .iter()
        .filter(|segment| segment.keyframe())
        .cloned()
        .collect();
    if keyframe_only.len() < normal_segment_count
        && let Ok(trick) = build_trick_8x_rendition(
            &keyframe_only,
            coverage,
            window,
            kind,
            TRICK_8X_SEGMENT_PREFIX,
        )
    {
        return (Some(trick), None);
    }

    let decimated: Vec<DvrMediaSegment> = in_window
        .iter()
        .enumerate()
        .filter(|&(index, _)| index % TRICK_8X_DECIMATION == 0)
        .map(|(_, segment)| segment.clone())
        .collect();
    if decimated.len() < normal_segment_count
        && decimated.iter().all(DvrMediaSegment::keyframe)
        && let Ok(trick) =
            build_trick_8x_rendition(&decimated, coverage, window, kind, TRICK_8X_SEGMENT_PREFIX)
    {
        return (Some(trick), None);
    }

    (
        None,
        Some(if keyframe_only.len() >= normal_segment_count {
            TRICK_8X_UNAVAILABLE_NO_REDUCTION.to_owned()
        } else {
            TRICK_8X_UNAVAILABLE_INSUFFICIENT_KEYFRAMES.to_owned()
        }),
    )
}

fn build_trick_8x_rendition(
    stored: &[DvrMediaSegment],
    coverage: &DvrMediaCoverage,
    window: TimeRange,
    kind: PlaylistKind,
    segment_prefix: &str,
) -> Result<PackagedHlsTrick8xRendition, A2aLabError> {
    let segments = plan_hls_segments(stored, coverage, window, segment_prefix, kind);
    if segments.is_empty() {
        return Err(A2aLabError::no_recording(
            "trick-play window contains no segments",
        ));
    }
    let playlist = match kind {
        PlaylistKind::Finite => render_finite_playlist(&segments)?,
        PlaylistKind::Live => render_live_playlist(&segments)?,
    };
    Ok(PackagedHlsTrick8xRendition {
        playlist_relative_path: HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH.to_owned(),
        playlist,
        segments,
    })
}

fn discontinuity_before_segment(start: UtcTimestamp, coverage: &DvrMediaCoverage) -> bool {
    coverage
        .discontinuities()
        .iter()
        .any(|gap| gap.gap().end() == start)
}

fn render_finite_playlist(segments: &[PackagedHlsSegment]) -> Result<String, A2aLabError> {
    if segments.is_empty() {
        return Err(A2aLabError::no_recording(
            "playback window contains no segments",
        ));
    }
    let target = target_duration(segments);
    let mut lines = playlist_header(target, PlaylistKind::Finite);
    append_segment_lines(&mut lines, segments);
    lines.push("#EXT-X-ENDLIST".to_owned());
    Ok(lines.join("\n"))
}

fn render_live_playlist(segments: &[PackagedHlsSegment]) -> Result<String, A2aLabError> {
    if segments.is_empty() {
        return Err(A2aLabError::no_recording(
            "playback window contains no segments",
        ));
    }
    let target = target_duration(segments);
    let mut lines = playlist_header(target, PlaylistKind::Live);
    append_segment_lines(&mut lines, segments);
    Ok(lines.join("\n"))
}

fn target_duration(segments: &[PackagedHlsSegment]) -> f64 {
    segments
        .iter()
        .map(PackagedHlsSegment::duration_secs)
        .fold(0.0_f64, f64::max)
        .ceil()
        .max(1.0)
}

fn playlist_header(target: f64, kind: PlaylistKind) -> Vec<String> {
    let mut lines = vec![
        "#EXTM3U".to_owned(),
        "#EXT-X-VERSION:7".to_owned(),
        format!("#EXT-X-TARGETDURATION:{target:.0}"),
        format!("#EXT-X-MAP:URI=\"{HLS_INIT_RELATIVE_PATH}\""),
    ];
    match kind {
        PlaylistKind::Finite => {
            lines.push("#EXT-X-PLAYLIST-TYPE:VOD".to_owned());
        }
        PlaylistKind::Live => {
            lines.push("#EXT-X-PLAYLIST-TYPE:EVENT".to_owned());
            lines.push("#EXT-X-MEDIA-SEQUENCE:0".to_owned());
        }
    }
    lines
}

fn append_segment_lines(lines: &mut Vec<String>, segments: &[PackagedHlsSegment]) {
    for segment in segments {
        if segment.discontinuity_before() {
            lines.push("#EXT-X-DISCONTINUITY".to_owned());
        }
        lines.push(format!(
            "#EXT-X-PROGRAM-DATE-TIME:{}",
            cmaf::program_date_time(segment.program_date_time())
        ));
        lines.push(format!("#EXTINF:{:.3},", segment.duration_secs()));
        lines.push(segment.relative_path().to_owned());
    }
}

fn validate_session_root(session_root: &str) -> Result<(), A2aLabError> {
    if !session_root.starts_with('/') || session_root.contains("..") || session_root.contains("://")
    {
        return Err(A2aLabError::invalid(
            "session_root",
            "must be a relative authenticated gateway root",
        ));
    }
    Ok(())
}

fn normalize_session_root(session_root: &str) -> String {
    if session_root.ends_with('/') {
        session_root.to_owned()
    } else {
        format!("{session_root}/")
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::videos::dvr_store::DvrMediaPayload;

    #[test]
    fn playlist_includes_program_date_time_and_discontinuity() {
        let start = UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap();
        let gap = UtcTimestamp::parse("2024-01-01T00:00:02Z").unwrap();
        let second = UtcTimestamp::parse("2024-01-01T00:00:04Z").unwrap();
        let end = UtcTimestamp::parse("2024-01-01T00:00:05Z").unwrap();
        let coverage = DvrMediaCoverage::new(vec![
            TimeRange::new(start, gap).unwrap(),
            TimeRange::new(second, end).unwrap(),
        ])
        .unwrap();
        let first = DvrMediaSegment::new(
            DvrTrackId::new("h264").unwrap(),
            TimeRange::new(start, gap).unwrap(),
            true,
            DvrMediaPayload::EncodedSample {
                codec: VideoCodec::H264,
                bytes: vec![1, 2],
            },
        )
        .unwrap();
        let second_segment = DvrMediaSegment::new(
            DvrTrackId::new("h264").unwrap(),
            TimeRange::new(second, end).unwrap(),
            true,
            DvrMediaPayload::EncodedSample {
                codec: VideoCodec::H264,
                bytes: vec![3, 4],
            },
        )
        .unwrap();
        let window = TimeRange::new(start, end).unwrap();
        let planned = plan_hls_segments(
            &[first, second_segment],
            &coverage,
            window,
            NORMAL_SEGMENT_PREFIX,
            PlaylistKind::Finite,
        );
        let playlist = render_finite_playlist(&planned).unwrap();
        assert!(playlist.contains("#EXT-X-PROGRAM-DATE-TIME:"));
        assert!(playlist.contains("#EXT-X-DISCONTINUITY"));
        assert!(playlist.contains(&planned[1].relative_path));
        assert_ne!(planned[0].relative_path, planned[1].relative_path);
    }

    #[test]
    fn live_playlist_is_event_without_endlist() {
        let start = UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap();
        let end = UtcTimestamp::parse("2024-01-01T00:00:02Z").unwrap();
        let coverage = DvrMediaCoverage::new(vec![TimeRange::new(start, end).unwrap()]).unwrap();
        let segment = DvrMediaSegment::new(
            DvrTrackId::new("h264").unwrap(),
            TimeRange::new(start, end).unwrap(),
            true,
            DvrMediaPayload::EncodedSample {
                codec: VideoCodec::H264,
                bytes: vec![1, 2],
            },
        )
        .unwrap();
        let window = TimeRange::new(start, end).unwrap();
        let planned = plan_hls_segments(
            &[segment],
            &coverage,
            window,
            NORMAL_SEGMENT_PREFIX,
            PlaylistKind::Live,
        );
        let playlist = render_live_playlist(&planned).unwrap();
        assert!(playlist.contains("#EXT-X-PLAYLIST-TYPE:EVENT"));
        assert!(playlist.contains("#EXT-X-PROGRAM-DATE-TIME:"));
        assert!(!playlist.contains("#EXT-X-ENDLIST"));
    }

    #[test]
    fn frames_group_into_five_second_segments() {
        let start = UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap();
        let frames: Vec<DvrMediaSegment> = (0..30)
            .map(|index| {
                let at = start.saturating_add_micros(index * 200_000);
                DvrMediaSegment::new(
                    DvrTrackId::new("h264").unwrap(),
                    TimeRange::new(at, at.saturating_add_micros(200_000)).unwrap(),
                    true,
                    DvrMediaPayload::EncodedSample {
                        codec: VideoCodec::H264,
                        bytes: vec![1],
                    },
                )
                .unwrap()
            })
            .collect();
        let end = frames.last().unwrap().range().end();
        let coverage = DvrMediaCoverage::new(vec![TimeRange::new(start, end).unwrap()]).unwrap();
        let window = TimeRange::new(start, end).unwrap();
        let finite = plan_hls_segments(
            &frames,
            &coverage,
            window,
            NORMAL_SEGMENT_PREFIX,
            PlaylistKind::Finite,
        );
        assert_eq!(finite.len(), 2);
        assert!((finite[0].duration_secs() - 5.0).abs() < 0.001);
        assert!((finite[1].duration_secs() - 1.0).abs() < 0.001);
        let live = plan_hls_segments(
            &frames,
            &coverage,
            window,
            NORMAL_SEGMENT_PREFIX,
            PlaylistKind::Live,
        );
        assert_eq!(live.len(), 1);
    }

    #[test]
    fn segment_boundaries_stay_on_absolute_five_second_marks() {
        let start = UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap();
        let frames: Vec<DvrMediaSegment> = (5..30)
            .map(|index| {
                let at = start.saturating_add_micros(index * 200_000);
                DvrMediaSegment::new(
                    DvrTrackId::new("h264").unwrap(),
                    TimeRange::new(at, at.saturating_add_micros(200_000)).unwrap(),
                    true,
                    DvrMediaPayload::EncodedSample {
                        codec: VideoCodec::H264,
                        bytes: vec![1],
                    },
                )
                .unwrap()
            })
            .collect();
        let window = TimeRange::new(
            frames[0].range().start(),
            frames.last().unwrap().range().end(),
        )
        .unwrap();
        let coverage =
            DvrMediaCoverage::new(vec![TimeRange::new(start, window.end()).unwrap()]).unwrap();
        let finite = plan_hls_segments(
            &frames,
            &coverage,
            window,
            NORMAL_SEGMENT_PREFIX,
            PlaylistKind::Finite,
        );
        assert_eq!(finite.len(), 2);
        assert!((finite[0].duration_secs() - 4.0).abs() < 0.001);
        let boundary = start.saturating_add_micros(5_000_000).as_micros();
        assert!(finite[1].relative_path().contains(&boundary.to_string()));
    }
}
