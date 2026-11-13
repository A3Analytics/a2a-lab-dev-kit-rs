//! HLS packaging for sequential H.264 configuration epochs.
//!
//! Each epoch keeps its own initialization. A configuration change emits
//! `#EXT-X-DISCONTINUITY` and a new `#EXT-X-MAP`. Gaps stay empty. Overlapping
//! H.264 tracks are rejected instead of being merged into one timeline.

use super::live::{H264LiveSample, H264LiveSnapshot};
use super::packager::{
    DvrMediaPackager, DvrPackagerConfig, HLS_PLAYLIST_RELATIVE_PATH,
    HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH, PackagedMediaAsset,
};
use crate::error::A2aLabError;
use crate::id::{DvrTrackId, VideoSourceId};
use crate::page::{MAX_PAGE_LIMIT, PageRequest};
use crate::time::{TimeRange, UtcTimestamp};
use crate::videos::cmaf;
use crate::videos::{
    DvrMediaCoverage, DvrMediaPayload, DvrMediaSegment, DvrMediaStore, DvrMediaTrack,
    DvrRetentionBounds, H264TrackRelation, ReadDvrMediaWindowRequest, VideoCodec,
    h264_track_relation,
};

const DEFAULT_LIVE_SAMPLE_PERIOD_US: u64 = 200_000;

/// Finite VOD or an open live event playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochPlaylistKind {
    /// Closed playlist with `#EXT-X-ENDLIST`.
    Finite,
    /// Event playlist that a later window can refresh.
    Live,
}

/// One epoch's decoder initialization, addressed by a session-relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpochHlsInit {
    relative_path: String,
    track_id: DvrTrackId,
    initialization: Vec<u8>,
}

impl EpochHlsInit {
    /// Session-relative init path.
    #[must_use]
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    /// Track whose initialization these bytes came from.
    #[must_use]
    pub const fn track_id(&self) -> &DvrTrackId {
        &self.track_id
    }

    /// Decoder initialization copied from the track. Not replaced by a later epoch.
    #[must_use]
    pub fn initialization(&self) -> &[u8] {
        &self.initialization
    }
}

/// One media segment and the init path that must precede it.
#[derive(Debug, Clone, PartialEq)]
pub struct EpochHlsSegment {
    relative_path: String,
    range: TimeRange,
    duration_secs: f64,
    program_date_time: UtcTimestamp,
    discontinuity_before: bool,
    init_relative_path: String,
    keyframe: bool,
}

impl EpochHlsSegment {
    /// Session-relative media path.
    #[must_use]
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    /// Indexed UTC bounds.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        self.range
    }

    /// `#EXTINF` duration in seconds.
    #[must_use]
    pub const fn duration_secs(&self) -> f64 {
        self.duration_secs
    }

    /// `#EXT-X-PROGRAM-DATE-TIME` instant.
    #[must_use]
    pub const fn program_date_time(&self) -> UtcTimestamp {
        self.program_date_time
    }

    /// Whether a discontinuity tag precedes this segment.
    #[must_use]
    pub const fn discontinuity_before(&self) -> bool {
        self.discontinuity_before
    }

    /// Init path whose bytes decode this segment.
    #[must_use]
    pub fn init_relative_path(&self) -> &str {
        &self.init_relative_path
    }

    /// Whether the stored segment is a keyframe.
    #[must_use]
    pub const fn keyframe(&self) -> bool {
        self.keyframe
    }
}

/// Playlist, per-epoch inits, and an optional keyframe-only 8x playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct EpochHlsSession {
    playlist: String,
    segments: Vec<EpochHlsSegment>,
    inits: Vec<EpochHlsInit>,
    trick_playlist: Option<String>,
    /// Decode-time zero. Live refreshes keep the first session's origin so later
    /// segments continue on the same timeline after the window slides.
    timeline_origin_us: i64,
}

impl EpochHlsSession {
    /// Normal-speed playlist body.
    #[must_use]
    pub fn playlist(&self) -> &str {
        &self.playlist
    }

    /// Segments in presentation order.
    #[must_use]
    pub fn segments(&self) -> &[EpochHlsSegment] {
        &self.segments
    }

    /// Inits referenced by the segments. Earlier epochs keep their own bytes.
    #[must_use]
    pub fn inits(&self) -> &[EpochHlsInit] {
        &self.inits
    }

    /// Keyframe-only playlist using the same init paths and timestamps.
    #[must_use]
    pub fn trick_playlist(&self) -> Option<&str> {
        self.trick_playlist.as_deref()
    }

    /// Microsecond instant encoded as decode time zero for this session.
    #[must_use]
    pub const fn timeline_origin_us(&self) -> i64 {
        self.timeline_origin_us
    }

    /// Keeps segment timestamps on the timeline established by an earlier refresh.
    pub fn pin_timeline_origin(&mut self, origin_us: i64) {
        self.timeline_origin_us = origin_us;
    }
}

/// Packages sequential H.264 epochs that intersect `window`.
///
/// A single H.264 track is packaged as one epoch. Two or more overlapping
/// H.264 tracks return an error and are not stitched together.
/// Newest instant suitable for anchoring a live follow window over one snapshot.
pub fn live_h264_edge_anchor(snapshot: &H264LiveSnapshot) -> Result<UtcTimestamp, A2aLabError> {
    let last = snapshot
        .samples()
        .last()
        .ok_or_else(|| A2aLabError::live_unavailable("live follow has no retained samples"))?;
    Ok(last.captured_at().saturating_add_micros(-1))
}

/// Builds a refreshable live HLS session from bounded in-memory samples.
pub fn package_h264_live_at(
    snapshot: &H264LiveSnapshot,
    anchor: UtcTimestamp,
    playback_window_us: u64,
) -> Result<EpochHlsSession, A2aLabError> {
    package_h264_live_snapshot(
        snapshot,
        anchor,
        playback_window_us,
        EpochPlaylistKind::Live,
    )
}

/// Packages one bounded live window without consulting disk DVR.
pub fn package_h264_live_snapshot(
    snapshot: &H264LiveSnapshot,
    anchor: UtcTimestamp,
    playback_window_us: u64,
    kind: EpochPlaylistKind,
) -> Result<EpochHlsSession, A2aLabError> {
    let retention = live_retention_bounds(snapshot)?;
    let window = live_anchor_window(anchor, playback_window_us, &retention)?;
    let selected = live_segments_in_window(snapshot, window)?;
    if selected.is_empty() {
        return Err(A2aLabError::no_recording(
            "live playback window contains no samples",
        ));
    }
    let epochs = group_live_epochs(snapshot, &selected)?;
    let epoch_refs: Vec<(&DvrMediaTrack, Vec<DvrMediaSegment>)> = epochs
        .iter()
        .map(|(track, segments)| (track, segments.clone()))
        .collect();
    let coverage = DvrMediaCoverage::new(vec![window])?;
    build_session(&epoch_refs, &coverage, kind)
}

/// Loads session-relative packaged assets from the live snapshot used to build the session.
pub fn load_live_hls_asset(
    snapshot: &H264LiveSnapshot,
    session: &EpochHlsSession,
    relative_path: &str,
) -> Result<PackagedMediaAsset, A2aLabError> {
    if relative_path == HLS_PLAYLIST_RELATIVE_PATH {
        return PackagedMediaAsset::new(
            "application/vnd.apple.mpegurl",
            session.playlist().as_bytes().to_vec(),
        );
    }
    if relative_path == HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH {
        let trick = session
            .trick_playlist()
            .ok_or_else(|| A2aLabError::not_found("packaged asset", relative_path.to_owned()))?;
        return PackagedMediaAsset::new("application/vnd.apple.mpegurl", trick.as_bytes().to_vec());
    }
    if let Some(init) = session
        .inits()
        .iter()
        .find(|init| init.relative_path() == relative_path)
    {
        let track = live_track_for_epoch(snapshot, init.track_id())?;
        let bytes = cmaf::init_segment(&track)?;
        return PackagedMediaAsset::new("video/mp4", bytes);
    }
    let segment = session
        .segments()
        .iter()
        .find(|segment| segment.relative_path() == relative_path)
        .ok_or_else(|| A2aLabError::not_found("packaged asset", relative_path.to_owned()))?;
    let track_id = track_for_init(session, segment)?;
    let frame = live_segment_payload(snapshot, track_id, segment.range())?;
    let origin = session.timeline_origin_us();
    let start = frame.range().start().as_micros();
    let natural = cmaf::segment_duration_us(frame.range());
    let index = session
        .segments()
        .iter()
        .position(|item| item.relative_path() == relative_path)
        .unwrap_or(0);
    let next_start = session
        .segments()
        .get(index.saturating_add(1))
        .map(|next| next.range().start().as_micros());
    let decode_time_us = u64::try_from(start.saturating_sub(origin)).unwrap_or(0);
    let duration_us = abutted_duration_us(start, natural, next_start);
    let sequence = u32::try_from(start.saturating_sub(origin) / 1_000).unwrap_or(u32::MAX);
    let timing = cmaf::FragmentTiming {
        sequence,
        decode_time_us,
        duration_us,
    };
    let track = live_track_for_epoch(snapshot, track_id)?;
    let bytes = cmaf::media_fragments(&track, &[(&frame, timing)])?;
    PackagedMediaAsset::new("video/iso.segment", bytes)
}

pub async fn package_h264_epochs(
    store: &impl DvrMediaStore,
    source_id: &VideoSourceId,
    window: TimeRange,
    kind: EpochPlaylistKind,
) -> Result<EpochHlsSession, A2aLabError> {
    window.check()?;
    let tracks = store.list_tracks(source_id).await?;
    let epochs = ordered_epochs(&tracks)?;
    let coverage = store.media_coverage(source_id).await?;
    let mut selected = Vec::new();
    for track in epochs {
        if track.bounds().end() <= window.start() || track.bounds().start() >= window.end() {
            continue;
        }
        let segments = read_segments(store, source_id, track.id(), window).await?;
        if segments.is_empty() {
            continue;
        }
        selected.push((track, segments));
    }
    if selected.is_empty() {
        return Err(A2aLabError::no_recording(
            "playback window contains no H.264 epochs",
        ));
    }
    let session = build_session(&selected, &coverage, kind)?;
    Ok(session)
}

/// One finite MP4 for a single H.264 epoch inside the requested window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiniteH264Clip {
    track_id: DvrTrackId,
    initialization: Vec<u8>,
    range: TimeRange,
    mp4: Vec<u8>,
}

impl FiniteH264Clip {
    /// Epoch whose initialization was written into this asset.
    #[must_use]
    pub const fn track_id(&self) -> &DvrTrackId {
        &self.track_id
    }

    /// Decoder initialization retained for this epoch.
    #[must_use]
    pub fn initialization(&self) -> &[u8] {
        &self.initialization
    }

    /// UTC span of the retained segments in this asset.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        self.range
    }

    /// Finite fragmented MP4 using only this epoch's initialization.
    #[must_use]
    pub fn mp4(&self) -> &[u8] {
        &self.mp4
    }
}

/// Exports each sequential epoch that intersects `window` as its own finite MP4.
///
/// Differing declared picture sizes fail before any asset is built. Gaps stay
/// empty. This does not re-encode samples or change the single-track exporter.
pub async fn export_finite_h264_epochs(
    store: &impl DvrMediaStore,
    source_id: &VideoSourceId,
    window: TimeRange,
) -> Result<Vec<FiniteH264Clip>, A2aLabError> {
    window.check()?;
    let tracks = store.list_tracks(source_id).await?;
    let epochs = ordered_epochs(&tracks)?;
    let mut selected = Vec::new();
    for track in epochs {
        if track.bounds().end() <= window.start() || track.bounds().start() >= window.end() {
            continue;
        }
        let segments = read_segments(store, source_id, track.id(), window).await?;
        if segments.is_empty() {
            continue;
        }
        selected.push((track, segments));
    }
    if selected.is_empty() {
        return Err(A2aLabError::no_recording(
            "export window contains no H.264 epochs",
        ));
    }
    reject_dimension_change(&selected)?;
    let packager = DvrMediaPackager::new(DvrPackagerConfig::default());
    let mut clips = Vec::new();
    for (track, segments) in selected {
        let start = segments[0].range().start();
        let end = segments[segments.len() - 1].range().end();
        let asset = packager
            .export_finite_mp4(store, source_id, track.id(), window)
            .await?;
        clips.push(FiniteH264Clip {
            track_id: track.id().clone(),
            initialization: track.codec_initialization().to_vec(),
            range: TimeRange::new(start, end)?,
            mp4: asset.bytes().to_vec(),
        });
    }
    Ok(clips)
}

fn reject_dimension_change(
    epochs: &[(&DvrMediaTrack, Vec<DvrMediaSegment>)],
) -> Result<(), A2aLabError> {
    let mut seen = None;
    for (track, _) in epochs {
        let Some(picture) = track.picture() else {
            continue;
        };
        let size = (picture.width(), picture.height());
        if seen.is_some_and(|previous| previous != size) {
            return Err(A2aLabError::invalid(
                "picture",
                "H.264 epoch dimensions cannot change inside one export",
            ));
        }
        seen = Some(size);
    }
    Ok(())
}

fn ordered_epochs(tracks: &[DvrMediaTrack]) -> Result<Vec<&DvrMediaTrack>, A2aLabError> {
    let mut epochs: Vec<&DvrMediaTrack> = tracks
        .iter()
        .filter(|track| track.codec() == VideoCodec::H264)
        .collect();
    if epochs.is_empty() {
        return Err(A2aLabError::no_recording("source has no H.264 epochs"));
    }
    if epochs.len() >= 2 {
        match h264_track_relation(tracks)? {
            H264TrackRelation::SequentialEpoch => {}
            H264TrackRelation::SimultaneousAlternate => {
                return Err(A2aLabError::invalid(
                    "tracks",
                    "overlapping H.264 epochs cannot share one playlist",
                ));
            }
        }
    }
    epochs.sort_by_key(|track| track.bounds().start());
    Ok(epochs)
}

async fn read_segments(
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

fn build_session(
    epochs: &[(&DvrMediaTrack, Vec<DvrMediaSegment>)],
    coverage: &DvrMediaCoverage,
    kind: EpochPlaylistKind,
) -> Result<EpochHlsSession, A2aLabError> {
    let mut inits = Vec::new();
    let mut segments = Vec::new();
    for (index, (track, stored)) in epochs.iter().enumerate() {
        let init_path = format!("init-{index}.mp4");
        inits.push(EpochHlsInit {
            relative_path: init_path.clone(),
            track_id: track.id().clone(),
            initialization: track.codec_initialization().to_vec(),
        });
        for (offset, stored_segment) in stored.iter().enumerate() {
            let previous = segments.last();
            let epoch_changed = previous
                .is_some_and(|segment: &EpochHlsSegment| segment.init_relative_path != init_path);
            let gap = previous.is_some_and(|segment| {
                stored_segment.range().start() > segment.range().end()
                    || coverage
                        .discontinuities()
                        .iter()
                        .any(|item| item.gap().end() == stored_segment.range().start())
            });
            let duration_us = u64::try_from(
                stored_segment
                    .range()
                    .end()
                    .as_micros()
                    .saturating_sub(stored_segment.range().start().as_micros()),
            )
            .unwrap_or(0);
            #[allow(clippy::cast_precision_loss)]
            let duration_secs = duration_us as f64 / 1_000_000.0;
            let relative_path = match kind {
                EpochPlaylistKind::Live => format!(
                    "seg-{index}-{}.m4s",
                    stored_segment.range().start().as_micros()
                ),
                EpochPlaylistKind::Finite => format!("seg-{index}-{offset}.m4s"),
            };
            segments.push(EpochHlsSegment {
                relative_path,
                range: stored_segment.range(),
                duration_secs,
                program_date_time: stored_segment.range().start(),
                discontinuity_before: epoch_changed || gap,
                init_relative_path: init_path.clone(),
                keyframe: stored_segment.keyframe(),
            });
        }
    }
    let playlist = render_playlist(&segments, kind)?;
    let trick_playlist = render_trick(&segments, kind)?;
    let timeline_origin_us = segments
        .first()
        .map_or(0, |segment| segment.range().start().as_micros());
    Ok(EpochHlsSession {
        playlist,
        segments,
        inits,
        trick_playlist,
        timeline_origin_us,
    })
}

fn render_trick(
    segments: &[EpochHlsSegment],
    kind: EpochPlaylistKind,
) -> Result<Option<String>, A2aLabError> {
    let keyframes: Vec<EpochHlsSegment> = segments
        .iter()
        .filter(|segment| segment.keyframe())
        .cloned()
        .collect();
    if keyframes.is_empty() {
        return Ok(None);
    }
    Ok(Some(render_playlist(&keyframes, kind)?))
}

fn render_playlist(
    segments: &[EpochHlsSegment],
    kind: EpochPlaylistKind,
) -> Result<String, A2aLabError> {
    if segments.is_empty() {
        return Err(A2aLabError::no_recording(
            "playback window contains no segments",
        ));
    }
    let target = segments
        .iter()
        .map(EpochHlsSegment::duration_secs)
        .fold(0.0_f64, f64::max)
        .ceil()
        .max(1.0);
    let mut lines = vec![
        "#EXTM3U".to_owned(),
        "#EXT-X-VERSION:7".to_owned(),
        format!("#EXT-X-TARGETDURATION:{target:.0}"),
    ];
    match kind {
        EpochPlaylistKind::Finite => lines.push("#EXT-X-PLAYLIST-TYPE:VOD".to_owned()),
        EpochPlaylistKind::Live => {
            lines.push("#EXT-X-PLAYLIST-TYPE:EVENT".to_owned());
            lines.push("#EXT-X-MEDIA-SEQUENCE:0".to_owned());
        }
    }
    let mut current_init: Option<&str> = None;
    for segment in segments {
        let init_changed =
            current_init.is_some() && current_init != Some(segment.init_relative_path());
        if init_changed || segment.discontinuity_before() {
            lines.push("#EXT-X-DISCONTINUITY".to_owned());
        }
        if current_init != Some(segment.init_relative_path()) {
            lines.push(format!(
                "#EXT-X-MAP:URI=\"{}\"",
                segment.init_relative_path()
            ));
            current_init = Some(segment.init_relative_path());
        }
        lines.push(format!(
            "#EXT-X-PROGRAM-DATE-TIME:{}",
            cmaf::program_date_time(segment.program_date_time())
        ));
        lines.push(format!("#EXTINF:{:.3},", segment.duration_secs()));
        lines.push(segment.relative_path().to_owned());
    }
    if kind == EpochPlaylistKind::Finite {
        lines.push("#EXT-X-ENDLIST".to_owned());
    }
    Ok(lines.join("\n"))
}

/// Loads one session-relative asset from a multi-epoch HLS session.
pub async fn load_epoch_hls_asset(
    store: &impl DvrMediaStore,
    source_id: &VideoSourceId,
    session: &EpochHlsSession,
    relative_path: &str,
) -> Result<PackagedMediaAsset, A2aLabError> {
    if relative_path == HLS_PLAYLIST_RELATIVE_PATH {
        return PackagedMediaAsset::new(
            "application/vnd.apple.mpegurl",
            session.playlist().as_bytes().to_vec(),
        );
    }
    if relative_path == HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH {
        let trick = session
            .trick_playlist()
            .ok_or_else(|| A2aLabError::not_found("packaged asset", relative_path.to_owned()))?;
        return PackagedMediaAsset::new("application/vnd.apple.mpegurl", trick.as_bytes().to_vec());
    }
    if let Some(init) = session
        .inits()
        .iter()
        .find(|init| init.relative_path() == relative_path)
    {
        let track = find_track(store, source_id, init.track_id()).await?;
        let bytes = cmaf::init_segment(&track)?;
        return PackagedMediaAsset::new("video/mp4", bytes);
    }
    let segment = session
        .segments()
        .iter()
        .find(|segment| segment.relative_path() == relative_path)
        .ok_or_else(|| A2aLabError::not_found("packaged asset", relative_path.to_owned()))?;
    let track = find_track(store, source_id, track_for_init(session, segment)?).await?;
    let frames = read_segments(store, source_id, track.id(), segment.range()).await?;
    let frame = frames
        .into_iter()
        .find(|frame| frame.range() == segment.range())
        .ok_or_else(|| {
            A2aLabError::not_found("packaged segment", segment.relative_path().to_owned())
        })?;
    let origin = session.timeline_origin_us();
    let start = frame.range().start().as_micros();
    let natural = cmaf::segment_duration_us(frame.range());
    let index = session
        .segments()
        .iter()
        .position(|item| item.relative_path() == relative_path)
        .unwrap_or(0);
    let next_start = session
        .segments()
        .get(index.saturating_add(1))
        .map(|next| next.range().start().as_micros());
    let decode_time_us = u64::try_from(start.saturating_sub(origin)).unwrap_or(0);
    let duration_us = abutted_duration_us(start, natural, next_start);
    let sequence = u32::try_from(start.saturating_sub(origin) / 1_000).unwrap_or(u32::MAX);
    let timing = cmaf::FragmentTiming {
        sequence,
        decode_time_us,
        duration_us,
    };
    let bytes = cmaf::media_fragments(&track, &[(&frame, timing)])?;
    PackagedMediaAsset::new("video/iso.segment", bytes)
}

fn track_for_init<'a>(
    session: &'a EpochHlsSession,
    segment: &'a EpochHlsSegment,
) -> Result<&'a DvrTrackId, A2aLabError> {
    session
        .inits()
        .iter()
        .find(|init| init.relative_path() == segment.init_relative_path())
        .map(EpochHlsInit::track_id)
        .ok_or_else(|| {
            A2aLabError::invalid(
                "init_relative_path",
                "epoch segment references an unknown initialization",
            )
        })
}

fn abutted_duration_us(start: i64, natural: u64, next_start: Option<i64>) -> u64 {
    match next_start {
        Some(next) if next > start => u64::try_from(next.saturating_sub(start)).unwrap_or(natural),
        _ => natural.max(1),
    }
}

fn live_retention_bounds(snapshot: &H264LiveSnapshot) -> Result<DvrRetentionBounds, A2aLabError> {
    let first = snapshot
        .samples()
        .first()
        .ok_or_else(|| A2aLabError::live_unavailable("live follow has no retained samples"))?;
    let last = snapshot
        .samples()
        .last()
        .ok_or_else(|| A2aLabError::live_unavailable("live follow has no retained samples"))?;
    let period = live_sample_period_us(snapshot.samples(), snapshot.samples().len() - 1);
    let newest = last
        .captured_at()
        .saturating_add_micros(i64::try_from(period).unwrap_or(i64::MAX));
    DvrRetentionBounds::new(first.captured_at(), newest)
}

fn live_anchor_window(
    anchor: UtcTimestamp,
    window_us: u64,
    retention: &DvrRetentionBounds,
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
            "live anchor does not intersect retained samples",
        ));
    }
    TimeRange::new(start, end)
}

fn live_sample_period_us(samples: &[H264LiveSample], index: usize) -> u64 {
    let Some(sample) = samples.get(index) else {
        return DEFAULT_LIVE_SAMPLE_PERIOD_US;
    };
    if let Some(measured) = sample_delta_us(sample, samples.get(index + 1)) {
        return measured;
    }
    if index > 0
        && let Some(previous) = samples.get(index - 1)
        && let Some(measured) = sample_delta_us(previous, Some(sample))
    {
        return measured;
    }
    DEFAULT_LIVE_SAMPLE_PERIOD_US
}

fn sample_delta_us(sample: &H264LiveSample, next: Option<&H264LiveSample>) -> Option<u64> {
    let next = next?;
    let delta = next.captured_at().as_micros() - sample.captured_at().as_micros();
    if delta > 0 {
        u64::try_from(delta).ok()
    } else {
        None
    }
}

fn live_segments_in_window(
    snapshot: &H264LiveSnapshot,
    window: TimeRange,
) -> Result<Vec<DvrMediaSegment>, A2aLabError> {
    let samples = snapshot.samples();
    let mut segments = Vec::new();
    for (index, sample) in samples.iter().enumerate() {
        if sample.captured_at() < window.start() {
            continue;
        }
        if sample.captured_at() >= window.end() {
            break;
        }
        let period = if index + 1 == samples.len() {
            DEFAULT_LIVE_SAMPLE_PERIOD_US
        } else {
            live_sample_period_us(samples, index)
        };
        let end = sample
            .captured_at()
            .saturating_add_micros(i64::try_from(period).unwrap_or(i64::MAX));
        let range = TimeRange::new(sample.captured_at(), end.min(window.end()))?;
        segments.push(DvrMediaSegment::new(
            sample.track_id().clone(),
            range,
            sample.keyframe(),
            DvrMediaPayload::EncodedSample {
                codec: VideoCodec::H264,
                bytes: sample.bytes().to_vec(),
            },
        )?);
    }
    Ok(segments)
}

fn group_live_epochs(
    snapshot: &H264LiveSnapshot,
    segments: &[DvrMediaSegment],
) -> Result<Vec<(DvrMediaTrack, Vec<DvrMediaSegment>)>, A2aLabError> {
    if segments.is_empty() {
        return Err(A2aLabError::no_recording(
            "live playback window contains no samples",
        ));
    }
    let mut grouped: Vec<(DvrTrackId, Vec<DvrMediaSegment>)> = Vec::new();
    for segment in segments {
        match grouped.last_mut() {
            Some((track_id, bucket)) if track_id == segment.track_id() => {
                bucket.push(segment.clone());
            }
            _ => grouped.push((segment.track_id().clone(), vec![segment.clone()])),
        }
    }
    let mut epochs = Vec::new();
    for (track_id, stored) in grouped {
        let epoch = snapshot
            .epochs()
            .iter()
            .find(|item| item.track_id() == &track_id)
            .ok_or_else(|| {
                A2aLabError::invalid("track_id", "live sample references an unknown epoch")
            })?;
        let bounds = TimeRange::new(
            stored[0].range().start(),
            stored[stored.len() - 1].range().end(),
        )?;
        let track = DvrMediaTrack::with_picture(
            track_id,
            VideoCodec::H264,
            epoch.initialization().to_vec(),
            bounds,
            epoch.picture(),
        )?;
        epochs.push((track, stored));
    }
    Ok(epochs)
}

fn live_track_for_epoch(
    snapshot: &H264LiveSnapshot,
    track_id: &DvrTrackId,
) -> Result<DvrMediaTrack, A2aLabError> {
    let epoch = snapshot
        .epochs()
        .iter()
        .find(|item| item.track_id() == track_id)
        .ok_or_else(|| A2aLabError::not_found("H.264 epoch", track_id.to_string()))?;
    let bounds = live_epoch_bounds(snapshot, track_id)
        .ok_or_else(|| A2aLabError::not_found("H.264 epoch samples", track_id.to_string()))?;
    DvrMediaTrack::with_picture(
        track_id.clone(),
        VideoCodec::H264,
        epoch.initialization().to_vec(),
        bounds,
        epoch.picture(),
    )
}

fn live_epoch_bounds(snapshot: &H264LiveSnapshot, track_id: &DvrTrackId) -> Option<TimeRange> {
    let samples: Vec<&H264LiveSample> = snapshot
        .samples()
        .iter()
        .filter(|sample| sample.track_id() == track_id)
        .collect();
    let first = samples.first()?;
    let last = samples.last()?;
    let period = live_sample_period_us(
        snapshot.samples(),
        snapshot.samples().len().saturating_sub(1),
    );
    let end = last
        .captured_at()
        .saturating_add_micros(i64::try_from(period).unwrap_or(i64::MAX));
    TimeRange::new(first.captured_at(), end).ok()
}

fn live_segment_payload(
    snapshot: &H264LiveSnapshot,
    track_id: &DvrTrackId,
    range: TimeRange,
) -> Result<DvrMediaSegment, A2aLabError> {
    let sample = snapshot
        .samples()
        .iter()
        .find(|item| item.track_id() == track_id && item.captured_at() == range.start())
        .ok_or_else(|| A2aLabError::not_found("packaged segment", range.start().to_string()))?;
    DvrMediaSegment::new(
        track_id.clone(),
        range,
        sample.keyframe(),
        DvrMediaPayload::EncodedSample {
            codec: VideoCodec::H264,
            bytes: sample.bytes().to_vec(),
        },
    )
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
