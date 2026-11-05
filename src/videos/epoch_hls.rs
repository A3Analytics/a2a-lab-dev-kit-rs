//! HLS packaging for sequential H.264 configuration epochs.
//!
//! Each epoch keeps its own initialization. A configuration change emits
//! `#EXT-X-DISCONTINUITY` and a new `#EXT-X-MAP`. Gaps stay empty. Overlapping
//! H.264 tracks are rejected instead of being merged into one timeline.

use crate::error::A2aLabError;
use crate::id::{DvrTrackId, VideoSourceId};
use crate::page::{MAX_PAGE_LIMIT, PageRequest};
use crate::time::{TimeRange, UtcTimestamp};
use crate::videos::cmaf;
use crate::videos::{
    DvrMediaCoverage, DvrMediaSegment, DvrMediaStore, DvrMediaTrack, H264TrackRelation,
    ReadDvrMediaWindowRequest, VideoCodec, h264_track_relation,
};

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
}

/// Packages sequential H.264 epochs that intersect `window`.
///
/// A single H.264 track is packaged as one epoch. Two or more overlapping
/// H.264 tracks return an error and are not stitched together.
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
            segments.push(EpochHlsSegment {
                relative_path: format!("seg-{index}-{offset}.m4s"),
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
    Ok(EpochHlsSession {
        playlist,
        segments,
        inits,
        trick_playlist,
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
