//! In-memory video provider.

use std::cmp::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, MutexGuard, Notify};

use crate::error::A2aLabError;
use crate::id::{DvrTrackId, ImageId, ImageSourceId, VideoClipId, VideoFollowId, VideoSourceId};
use crate::images::{Image, ImageDescriptor};
use crate::json_object::JsonObject;
use crate::page::{Page, PageRequest, slice_page};
use crate::time::{TimeRange, UtcTimestamp};
use crate::videos::{
    DEFAULT_MJPEG_BOUNDARY, DvrFrameAtRequest, DvrFrameAtResult, DvrMediaCoverage,
    DvrMediaPackager, DvrMediaPayload, DvrMediaSegment, DvrMediaStore, DvrMediaTrack,
    DvrRetentionBounds, FollowVideoRequest, GetVideoClipRequest, GetVideoFrameAtRequest,
    GetVideoFrameAtResult, GetVideoTimelineRequest, HLS_PLAYLIST_RELATIVE_PATH,
    ListVideoSourcesRequest, PackagedMediaAsset, ReadDvrMediaWindowRequest, ResolvedMediaAsset,
    VideoClip, VideoClipStream, VideoCodec, VideoFollow, VideoFrame, VideoLiveStream,
    VideoProvider, VideoSource, VideoTimeline, encode_mjpeg_end, encode_mjpeg_multipart,
    mjpeg_content_type, normalize_media_asset_path, video_live_path,
};

/// Default clip handle lifetime. Not durable storage.
pub const DEFAULT_VIDEO_CLIP_TTL: Duration = Duration::from_secs(60);

struct StoredFrame {
    captured_at: UtcTimestamp,
    jpeg: Vec<u8>,
}

struct StoredDvrTrack {
    source_id: VideoSourceId,
    id: DvrTrackId,
    codec: VideoCodec,
    codec_initialization: Vec<u8>,
}

struct StoredDvrSegment {
    source_id: VideoSourceId,
    track_id: DvrTrackId,
    range: TimeRange,
    keyframe: bool,
    payload: DvrMediaPayload,
}

/// Default live session lifetime. Not durable storage.
pub const DEFAULT_VIDEO_FOLLOW_TTL: Duration = DEFAULT_VIDEO_CLIP_TTL;

struct StoredClip {
    clip: VideoClip,
    frames: Vec<VideoFrame>,
    track_id: DvrTrackId,
    expires_at: Instant,
}

#[derive(Clone)]
enum LiveFollowMode {
    Mjpeg,
    Encoded { track_id: DvrTrackId },
}

#[derive(Clone, Copy)]
enum LiveAnchor {
    LiveEdge,
    Fixed(UtcTimestamp),
}

struct PreparedFollow {
    follow: VideoFollow,
    source_id: VideoSourceId,
    mode: LiveFollowMode,
    anchor: LiveAnchor,
    mjpeg_opened: bool,
    expires_at: Instant,
    hls: Option<crate::videos::PackagedHlsSession>,
}

struct ActiveFollow {
    id: VideoFollowId,
    source_id: VideoSourceId,
    pending: Vec<VideoFrame>,
    notify: Arc<Notify>,
    stopped: bool,
}

struct LiveCursor {
    videos: MemoryVideos,
    id: VideoFollowId,
    notify: Arc<Notify>,
}

impl Drop for LiveCursor {
    fn drop(&mut self) {
        let id = self.id.clone();
        if let Ok(mut state) = self.videos.inner.try_lock() {
            state.follows.retain(|follow| follow.id != id);
            return;
        }
        let videos = self.videos.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                videos
                    .inner
                    .lock()
                    .await
                    .follows
                    .retain(|follow| follow.id != id);
            });
        }
    }
}

struct VideoState {
    sources: Vec<VideoSource>,
    frames: Vec<(VideoSourceId, StoredFrame)>,
    dvr_tracks: Vec<StoredDvrTrack>,
    dvr_segments: Vec<StoredDvrSegment>,
    clips: Vec<StoredClip>,
    next_clip: u64,
    ttl: Duration,
    follow_ttl: Duration,
    unavailable: Option<String>,
    live: Vec<(VideoSourceId, bool)>,
    prepared: Vec<PreparedFollow>,
    follows: Vec<ActiveFollow>,
    next_follow: u64,
}

impl VideoState {
    fn with_ttl(clip_ttl: Duration, follow_ttl: Duration) -> Self {
        Self {
            sources: Vec::new(),
            frames: Vec::new(),
            dvr_tracks: Vec::new(),
            dvr_segments: Vec::new(),
            clips: Vec::new(),
            next_clip: 0,
            ttl: clip_ttl,
            follow_ttl,
            unavailable: None,
            live: Vec::new(),
            prepared: Vec::new(),
            follows: Vec::new(),
            next_follow: 0,
        }
    }
}

/// In-memory [`VideoProvider`] for examples and tests. Not durable storage.
#[derive(Clone)]
pub struct MemoryVideos {
    inner: Arc<Mutex<VideoState>>,
}

impl MemoryVideos {
    /// Creates an empty catalog with a 60-second clip and live-session TTL.
    #[must_use]
    pub fn new() -> Self {
        Self::with_session_ttl(DEFAULT_VIDEO_CLIP_TTL, DEFAULT_VIDEO_FOLLOW_TTL)
    }

    /// Creates an empty catalog. `Duration::ZERO` expires clips immediately.
    #[must_use]
    pub fn with_clip_ttl(ttl: Duration) -> Self {
        Self::with_session_ttl(ttl, ttl)
    }

    /// Creates an empty catalog with independent clip and live session TTLs.
    #[must_use]
    pub fn with_session_ttl(clip_ttl: Duration, follow_ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VideoState::with_ttl(clip_ttl, follow_ttl))),
        }
    }

    /// Adds a source. Live follow starts unavailable until [`Self::set_live_available`].
    pub async fn insert_source(&self, source: VideoSource) -> Result<(), A2aLabError> {
        let mut state = self.lock().await;
        if state
            .sources
            .iter()
            .any(|existing| existing.id() == source.id())
        {
            return Err(A2aLabError::invalid("id", "video source already exists"));
        }
        state.sources.push(source);
        Ok(())
    }

    /// Adds a JPEG frame for an existing source.
    ///
    /// Open live followers of this source receive the frame in capture-timestamp order.
    /// The frame is also indexed on the source's implicit MJPEG DVR track.
    pub async fn insert_frame(
        &self,
        source_id: VideoSourceId,
        captured_at: UtcTimestamp,
        jpeg: Vec<u8>,
    ) -> Result<(), A2aLabError> {
        let frame = VideoFrame::new(captured_at, jpeg)?;
        let mut state = self.lock().await;
        require_source(&state, &source_id)?;
        let period = find_source(&state, &source_id)?.frame_period_us();
        state.frames.push((
            source_id.clone(),
            StoredFrame {
                captured_at: frame.captured_at(),
                jpeg: frame.jpeg().to_vec(),
            },
        ));
        record_mjpeg_dvr_segment(
            &mut state,
            &source_id,
            frame.captured_at(),
            frame.jpeg(),
            period,
        )?;
        publish_frame(&mut state, &source_id, &frame);
        Ok(())
    }

    /// Registers an indexed DVR track for encoded media on an existing source.
    pub async fn insert_dvr_track(
        &self,
        source_id: VideoSourceId,
        track_id: DvrTrackId,
        codec: VideoCodec,
        codec_initialization: Vec<u8>,
    ) -> Result<(), A2aLabError> {
        if matches!(codec, VideoCodec::Mjpeg) {
            return Err(A2aLabError::invalid(
                "codec",
                "MJPEG tracks are created automatically from insert_frame",
            ));
        }
        if codec_initialization.is_empty() {
            return Err(A2aLabError::invalid(
                "codec_initialization",
                "encoded tracks require codec initialization bytes",
            ));
        }
        let mut state = self.lock().await;
        require_source(&state, &source_id)?;
        if state
            .dvr_tracks
            .iter()
            .any(|track| track.source_id == source_id && track.id == track_id)
        {
            return Err(A2aLabError::invalid("id", "DVR track already exists"));
        }
        state.dvr_tracks.push(StoredDvrTrack {
            source_id,
            id: track_id,
            codec,
            codec_initialization,
        });
        Ok(())
    }

    /// Indexes one encoded segment on a registered DVR track.
    pub async fn insert_dvr_segment(
        &self,
        source_id: VideoSourceId,
        track_id: DvrTrackId,
        range: TimeRange,
        keyframe: bool,
        codec: VideoCodec,
        bytes: Vec<u8>,
    ) -> Result<(), A2aLabError> {
        range.check()?;
        let mut state = self.lock().await;
        require_source(&state, &source_id)?;
        let track = state
            .dvr_tracks
            .iter()
            .find(|track| track.source_id == source_id && track.id == track_id)
            .ok_or_else(|| A2aLabError::not_found("DVR track", track_id.to_string()))?;
        if track.codec != codec {
            return Err(A2aLabError::invalid(
                "codec",
                "segment codec must match the track codec",
            ));
        }
        let payload = DvrMediaPayload::EncodedSample { codec, bytes };
        state.dvr_segments.push(StoredDvrSegment {
            source_id: source_id.clone(),
            track_id: track_id.clone(),
            range,
            keyframe,
            payload,
        });
        Ok(())
    }

    /// Returns the implicit MJPEG DVR track id for `source_id`.
    pub fn mjpeg_track_id(source_id: &VideoSourceId) -> Result<DvrTrackId, A2aLabError> {
        DvrTrackId::new(format!("{}-mjpeg", source_id.as_str()))
    }

    /// Drops in-memory frames for `source_id` captured before `earliest`, and drops expired clip copies.
    ///
    /// This does not write or delete files. Prepared clips that are still inside their TTL stay available.
    pub async fn forget_before(
        &self,
        source_id: &VideoSourceId,
        earliest: UtcTimestamp,
    ) -> Result<(), A2aLabError> {
        let mut state = self.lock().await;
        require_source(&state, source_id)?;
        state
            .frames
            .retain(|(id, frame)| id != source_id || frame.captured_at >= earliest);
        state
            .dvr_segments
            .retain(|segment| segment.source_id != *source_id || segment.range.start() >= earliest);
        let now = Instant::now();
        state.clips.retain(|clip| clip.expires_at > now);
        Ok(())
    }

    /// Whether `source_id` is currently marked live.
    pub async fn live_available(&self, source_id: &VideoSourceId) -> Result<bool, A2aLabError> {
        let state = self.lock().await;
        require_source(&state, source_id)?;
        Ok(is_live(&state, source_id))
    }

    /// Marks one source live-available or live-unavailable.
    ///
    /// Turning availability off stops open followers for that source.
    pub async fn set_live_available(
        &self,
        source_id: &VideoSourceId,
        available: bool,
    ) -> Result<(), A2aLabError> {
        let mut state = self.lock().await;
        require_source(&state, source_id)?;
        if let Some(slot) = state.live.iter_mut().find(|(id, _)| id == source_id) {
            slot.1 = available;
        } else {
            state.live.push((source_id.clone(), available));
        }
        if !available {
            stop_follows(&mut state, source_id);
        }
        Ok(())
    }

    /// Marks every prepared clip expired. Later [`VideoProvider::open_video_clip`] calls fail.
    pub async fn expire_clips(&self) {
        let mut state = self.lock().await;
        let now = Instant::now();
        for clip in &mut state.clips {
            clip.expires_at = now;
        }
    }

    /// Marks every prepared live session expired.
    pub async fn expire_follows(&self) {
        let mut state = self.lock().await;
        let now = Instant::now();
        for follow in &mut state.prepared {
            follow.expires_at = now;
        }
    }

    /// Makes later calls fail until [`Self::clear_unavailable`].
    pub async fn set_unavailable(&self, message: impl Into<String>) {
        self.lock().await.unavailable = Some(message.into());
    }

    /// Clears a simulated provider failure.
    pub async fn clear_unavailable(&self) {
        self.lock().await.unavailable = None;
    }

    async fn lock(&self) -> MutexGuard<'_, VideoState> {
        self.inner.lock().await
    }
}

impl Default for MemoryVideos {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoProvider for MemoryVideos {
    async fn list_video_sources(
        &self,
        request: ListVideoSourcesRequest,
    ) -> Result<Page<VideoSource>, A2aLabError> {
        let state = self.lock().await;
        require_available(&state)?;
        let mut sources = state.sources.clone();
        sources.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        slice_page(&sources, request.page())
    }

    async fn get_video_frame_at(
        &self,
        request: GetVideoFrameAtRequest,
    ) -> Result<GetVideoFrameAtResult, A2aLabError> {
        let state = self.lock().await;
        require_available(&state)?;
        let source = find_source(&state, request.source_id())?;
        let mut frames = frames_for(&state, request.source_id());
        frames.sort_by(captured_order);
        let selected = nearest_frame(
            &frames,
            request.at(),
            request.max_delta_us(),
            source.frame_period_us(),
        )?;
        let image = still_image(source, selected)?;
        Ok(GetVideoFrameAtResult::new(request.at(), image))
    }

    async fn get_video_clip(&self, request: GetVideoClipRequest) -> Result<VideoClip, A2aLabError> {
        request.check()?;
        let mut state = self.lock().await;
        require_available(&state)?;
        let source_id = request.source_id().clone();
        let period = find_source(&state, &source_id)?.frame_period_us();
        let mut frames = frames_for(&state, &source_id);
        frames.sort_by(captured_order);
        if frames.is_empty()
            && let Some((track_id, codec)) = encoded_live_track(&state, &source_id)
        {
            let effective_range =
                encoded_clip_effective_range(&state, &source_id, &track_id, request.range())?;
            state.next_clip += 1;
            let id = VideoClipId::new(format!("clip-{}", state.next_clip))?;
            let clip = VideoClip::new(
                id,
                source_id.clone(),
                request.range(),
                effective_range,
                codec,
                "video/mp4",
            )?;
            let expires_at = Instant::now() + state.ttl;
            state.clips.push(StoredClip {
                clip: clip.clone(),
                frames: Vec::new(),
                track_id,
                expires_at,
            });
            return Ok(clip);
        }
        let (included, effective_range) = clip_frames(&frames, request.range(), period)?;
        let track_id = MemoryVideos::mjpeg_track_id(&source_id)?;
        state.next_clip += 1;
        let id = VideoClipId::new(format!("clip-{}", state.next_clip))?;
        let clip = VideoClip::new_mjpeg(id, source_id, request.range(), effective_range)?;
        let expires_at = Instant::now() + state.ttl;
        state.clips.push(StoredClip {
            clip: clip.clone(),
            frames: included,
            track_id,
            expires_at,
        });
        Ok(clip)
    }

    async fn export_finite_video_clip(
        &self,
        id: &VideoClipId,
    ) -> Result<PackagedMediaAsset, A2aLabError> {
        let (clip, track_id, frames) = {
            let state = self.lock().await;
            require_available(&state)?;
            let stored = state
                .clips
                .iter()
                .find(|clip| clip.clip.id() == id)
                .ok_or_else(|| A2aLabError::not_found("video clip", id.to_string()))?;
            if stored.expires_at <= Instant::now() {
                return Err(A2aLabError::clip_expired(format!(
                    "video clip `{id}` has expired"
                )));
            }
            (
                stored.clip.clone(),
                stored.track_id.clone(),
                stored.frames.clone(),
            )
        };
        if clip.codec() == VideoCodec::Mjpeg && !frames.is_empty() {
            let mut body = encode_mjpeg_multipart(&frames, DEFAULT_MJPEG_BOUNDARY);
            body.extend_from_slice(&encode_mjpeg_end(DEFAULT_MJPEG_BOUNDARY));
            return PackagedMediaAsset::new(mjpeg_content_type(DEFAULT_MJPEG_BOUNDARY), body);
        }
        let packager = DvrMediaPackager::default();
        match clip.codec() {
            VideoCodec::Mjpeg => {
                packager
                    .export_finite_mjpeg(self, clip.source_id(), &track_id, clip.effective_range())
                    .await
            }
            VideoCodec::H264 | VideoCodec::H265 => {
                packager
                    .export_finite_mp4(self, clip.source_id(), &track_id, clip.effective_range())
                    .await
            }
        }
    }

    async fn open_video_clip(&self, id: &VideoClipId) -> Result<VideoClipStream, A2aLabError> {
        let state = self.lock().await;
        require_available(&state)?;
        let stored = state
            .clips
            .iter()
            .find(|clip| clip.clip.id() == id)
            .ok_or_else(|| A2aLabError::not_found("video clip", id.to_string()))?;
        if stored.expires_at <= Instant::now() {
            return Err(A2aLabError::clip_expired(format!(
                "video clip `{id}` has expired"
            )));
        }
        let frames = stored.frames.clone();
        Ok(Box::pin(futures_util::stream::iter(
            frames.into_iter().map(Ok),
        )))
    }

    async fn get_video_timeline(
        &self,
        request: GetVideoTimelineRequest,
    ) -> Result<VideoTimeline, A2aLabError> {
        let state = self.lock().await;
        require_available(&state)?;
        let (frames, intervals) = prepared_coverage(&state, request.source_id())?;
        VideoTimeline::new(
            request.source_id().clone(),
            intervals,
            frames.last().map(|frame| frame.captured_at),
            is_live(&state, request.source_id()),
        )
    }

    async fn follow_video(&self, request: FollowVideoRequest) -> Result<VideoFollow, A2aLabError> {
        let mut state = self.lock().await;
        require_available(&state)?;
        require_source(&state, request.source_id())?;
        if !is_live(&state, request.source_id()) {
            return Err(A2aLabError::live_unavailable(
                "live follow is not available",
            ));
        }
        purge_expired_follows(&mut state);
        state.next_follow += 1;
        let id = VideoFollowId::new(format!("follow-{}", state.next_follow))?;
        let expires_at = Instant::now() + state.follow_ttl;
        let anchor = match request.anchor_at() {
            Some(at) => LiveAnchor::Fixed(at),
            None => LiveAnchor::LiveEdge,
        };
        if let Some((track_id, codec)) = encoded_live_track(&state, request.source_id()) {
            let source_id = request.source_id().clone();
            let follow = VideoFollow::new_encoded(id, source_id.clone(), codec)?;
            drop(state);
            let playback_8x =
                encoded_live_playback_8x(self, &follow, &source_id, &track_id, anchor).await?;
            let follow = follow.with_playback_8x(playback_8x);
            let mut state = self.lock().await;
            state.prepared.push(PreparedFollow {
                follow: follow.clone(),
                source_id,
                mode: LiveFollowMode::Encoded { track_id },
                anchor,
                mjpeg_opened: false,
                expires_at,
                hls: None,
            });
            return Ok(follow);
        }
        let follow = VideoFollow::new_mjpeg(id, request.source_id().clone())?;
        state.prepared.push(PreparedFollow {
            follow: follow.clone(),
            source_id: request.source_id().clone(),
            mode: LiveFollowMode::Mjpeg,
            anchor,
            mjpeg_opened: false,
            expires_at,
            hls: None,
        });
        Ok(follow)
    }

    async fn open_video_follow(&self, id: &VideoFollowId) -> Result<VideoLiveStream, A2aLabError> {
        let notify = {
            let mut state = self.lock().await;
            require_available(&state)?;
            let index = state
                .prepared
                .iter()
                .position(|follow| follow.follow.id() == id)
                .ok_or_else(|| A2aLabError::not_found("video follow", id.to_string()))?;
            if state.prepared[index].expires_at <= Instant::now() {
                state.prepared.remove(index);
                return Err(A2aLabError::clip_expired(format!(
                    "live session `{id}` has expired"
                )));
            }
            if !matches!(state.prepared[index].mode, LiveFollowMode::Mjpeg) {
                return Err(A2aLabError::invalid(
                    "id",
                    "encoded live sessions use packaged HLS assets",
                ));
            }
            let source_id = state.prepared[index].source_id.clone();
            if !is_live(&state, &source_id) {
                return Err(A2aLabError::live_unavailable(
                    "live follow is not available",
                ));
            }
            if state.prepared[index].mjpeg_opened {
                return Err(A2aLabError::invalid("id", "live follow is already open"));
            }
            state.prepared[index].mjpeg_opened = true;
            let notify = Arc::new(Notify::new());
            state.follows.push(ActiveFollow {
                id: id.clone(),
                source_id,
                pending: Vec::new(),
                notify: Arc::clone(&notify),
                stopped: false,
            });
            notify
        };
        let cursor = LiveCursor {
            videos: self.clone(),
            id: id.clone(),
            notify,
        };
        Ok(Box::pin(futures_util::stream::unfold(
            cursor,
            live_follow_next,
        )))
    }

    async fn rebase_video_follow(
        &self,
        id: &VideoFollowId,
        anchor: UtcTimestamp,
    ) -> Result<VideoFollow, A2aLabError> {
        let mut state = self.lock().await;
        require_available(&state)?;
        let prepared = state
            .prepared
            .iter_mut()
            .find(|follow| follow.follow.id() == id)
            .ok_or_else(|| A2aLabError::not_found("video follow", id.to_string()))?;
        if prepared.expires_at <= Instant::now() {
            return Err(A2aLabError::clip_expired(format!(
                "live session `{id}` has expired"
            )));
        }
        prepared.anchor = LiveAnchor::Fixed(anchor);
        prepared.hls = None;
        if let LiveFollowMode::Encoded { ref track_id } = prepared.mode {
            let follow = prepared.follow.clone();
            let source_id = prepared.source_id.clone();
            let track_id = track_id.clone();
            let anchor_mode = prepared.anchor;
            drop(state);
            let playback_8x =
                encoded_live_playback_8x(self, &follow, &source_id, &track_id, anchor_mode).await?;
            let mut state = self.lock().await;
            let prepared = state
                .prepared
                .iter_mut()
                .find(|follow| follow.follow.id() == id)
                .ok_or_else(|| A2aLabError::not_found("video follow", id.to_string()))?;
            prepared.follow = prepared.follow.clone().with_playback_8x(playback_8x);
            return Ok(prepared.follow.clone());
        }
        Ok(prepared.follow.clone())
    }

    async fn resolve_live_playback_asset(
        &self,
        id: &VideoFollowId,
        relative_path: Option<&str>,
    ) -> Result<ResolvedMediaAsset, A2aLabError> {
        let relative = normalize_media_asset_path(relative_path)?;
        let mut state = self.lock().await;
        require_available(&state)?;
        let index = state
            .prepared
            .iter()
            .position(|follow| follow.follow.id() == id)
            .ok_or_else(|| A2aLabError::not_found("video follow", id.to_string()))?;
        if state.prepared[index].expires_at <= Instant::now() {
            state.prepared.remove(index);
            return Err(A2aLabError::clip_expired(format!(
                "live session `{id}` has expired"
            )));
        }
        state.prepared[index].expires_at = Instant::now() + state.follow_ttl;
        let source_id = state.prepared[index].source_id.clone();
        if !is_live(&state, &source_id)
            && matches!(state.prepared[index].anchor, LiveAnchor::LiveEdge)
        {
            return Err(A2aLabError::live_unavailable(
                "live follow is not available",
            ));
        }
        match state.prepared[index].mode {
            LiveFollowMode::Mjpeg => {
                if relative.is_some() {
                    return Err(A2aLabError::not_found(
                        "packaged asset",
                        relative.unwrap_or_default(),
                    ));
                }
                drop(state);
                let stream = self.open_video_follow(id).await?;
                Ok(ResolvedMediaAsset::LiveStream(stream))
            }
            LiveFollowMode::Encoded { ref track_id } => {
                let track_id = track_id.clone();
                let anchor = state.prepared[index].anchor;
                let root = video_live_path(id);
                let cached = state.prepared[index].hls.clone();
                drop(state);
                let packager = DvrMediaPackager::default();
                if let Some(path) = relative.as_deref()
                    && path != HLS_PLAYLIST_RELATIVE_PATH
                    && let Some(session) = cached.as_ref()
                    && cached_asset(session, path)
                {
                    let asset = packager.load_hls_asset(self, session, path).await?;
                    return Ok(ResolvedMediaAsset::Bytes(asset));
                }
                let timeline_origin_us = cached
                    .as_ref()
                    .map(crate::videos::PackagedHlsSession::timeline_origin_us);
                let anchor = resolve_live_anchor(self, &source_id, anchor).await?;
                let mut session = packager
                    .package_live_hls_at(self, &root, &source_id, &track_id, anchor)
                    .await?;
                if let Some(origin_us) = timeline_origin_us {
                    session.pin_timeline_origin(origin_us);
                }
                let path = relative.as_deref().unwrap_or(HLS_PLAYLIST_RELATIVE_PATH);
                let asset = packager.load_hls_asset(self, &session, path).await?;
                let mut state = self.lock().await;
                if let Some(prepared) = state
                    .prepared
                    .iter_mut()
                    .find(|follow| follow.follow.id() == id)
                {
                    prepared.follow = prepared
                        .follow
                        .clone()
                        .with_playback_8x(session.playback_8x_metadata());
                    prepared.hls = Some(session);
                }
                Ok(ResolvedMediaAsset::Bytes(asset))
            }
        }
    }
}

fn cached_asset(session: &crate::videos::PackagedHlsSession, path: &str) -> bool {
    if path == session.init_relative_path() {
        return true;
    }
    if session
        .segments()
        .iter()
        .any(|item| item.relative_path() == path)
    {
        return true;
    }
    session.trick_8x().is_some_and(|trick| {
        path == trick.playlist_relative_path()
            || trick
                .segments()
                .iter()
                .any(|item| item.relative_path() == path)
    })
}

async fn live_follow_next(
    cursor: LiveCursor,
) -> Option<(Result<VideoFrame, A2aLabError>, LiveCursor)> {
    loop {
        let notify = Arc::clone(&cursor.notify);
        let notified = notify.notified();
        let next = {
            let mut state = cursor.videos.lock().await;
            let follow = state
                .follows
                .iter_mut()
                .find(|follow| follow.id == cursor.id);
            match follow {
                None => return None,
                Some(follow) if follow.stopped => return None,
                Some(follow) => take_earliest(&mut follow.pending),
            }
        };
        if let Some(frame) = next {
            drop(notified);
            return Some((Ok(frame), cursor));
        }
        notified.await;
    }
}

fn still_image(source: &VideoSource, frame: &StoredFrame) -> Result<Image, A2aLabError> {
    let descriptor = ImageDescriptor::new(
        ImageId::new(format!(
            "{}-{}",
            source.id().as_str(),
            frame.captured_at.as_micros()
        ))?,
        ImageSourceId::new(source.id().as_str())?,
        frame.captured_at,
        "image/jpeg",
        source.width(),
        source.height(),
        None,
        JsonObject::empty(),
    )?;
    Image::new(descriptor, frame.jpeg.clone())
}

fn nearest_frame(
    frames: &[StoredFrame],
    at: UtcTimestamp,
    max_delta_us: Option<u64>,
    period_us: u64,
) -> Result<&StoredFrame, A2aLabError> {
    let missing = || A2aLabError::not_found("video frame", at.to_string());
    let Some((first, last)) = frames
        .first()
        .zip(frames.last())
        .map(|(first, last)| (first.captured_at, last.captured_at))
    else {
        return Err(missing());
    };
    if at < first || at > last {
        return Err(missing());
    }
    let covered = coverage_segments(frames, period_us)
        .into_iter()
        .any(|(start, end)| at >= start && at < end);
    if !covered {
        return Err(missing());
    }
    let selected = frames
        .iter()
        .min_by(|left, right| {
            frame_delta(left, at)
                .cmp(&frame_delta(right, at))
                .then_with(|| left.captured_at.cmp(&right.captured_at))
        })
        .ok_or_else(missing)?;
    if let Some(limit) = max_delta_us
        && frame_delta(selected, at) > limit
    {
        return Err(missing());
    }
    Ok(selected)
}

fn frame_delta(frame: &StoredFrame, at: UtcTimestamp) -> u64 {
    frame.captured_at.as_micros().abs_diff(at.as_micros())
}

fn prepared_coverage(
    state: &VideoState,
    source_id: &VideoSourceId,
) -> Result<(Vec<StoredFrame>, Vec<TimeRange>), A2aLabError> {
    let period = find_source(state, source_id)?.frame_period_us();
    let mut frames = frames_for(state, source_id);
    frames.sort_by(captured_order);
    let intervals = retained_intervals(&frames, period)?;
    Ok((frames, intervals))
}

fn encoded_clip_effective_range(
    state: &VideoState,
    source_id: &VideoSourceId,
    track_id: &DvrTrackId,
    range: TimeRange,
) -> Result<TimeRange, A2aLabError> {
    let segments: Vec<_> = state
        .dvr_segments
        .iter()
        .filter(|segment| &segment.source_id == source_id && &segment.track_id == track_id)
        .filter(|segment| {
            segment.range.start() < range.end() && range.start() < segment.range.end()
        })
        .collect();
    if segments.is_empty() {
        let label = format!("{}..{}", range.start(), range.end());
        return Err(A2aLabError::no_recording(format!(
            "requested range {label} has no retained recording"
        )));
    }
    let mut starts: Vec<_> = segments
        .iter()
        .map(|segment| segment.range.start())
        .collect();
    let mut ends: Vec<_> = segments.iter().map(|segment| segment.range.end()).collect();
    starts.sort();
    ends.sort();
    TimeRange::new(
        *starts.first().expect("segments"),
        *ends.last().expect("segments"),
    )
}

fn clip_frames(
    frames: &[StoredFrame],
    range: TimeRange,
    period_us: u64,
) -> Result<(Vec<VideoFrame>, TimeRange), A2aLabError> {
    let included: Vec<VideoFrame> = frames
        .iter()
        .filter(|frame| range.contains(frame.captured_at))
        .map(|frame| VideoFrame::new(frame.captured_at, frame.jpeg.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    if included.is_empty() {
        let label = format!("{}..{}", range.start(), range.end());
        return Err(A2aLabError::no_recording(format!(
            "requested range {label} has no retained recording"
        )));
    }
    let period = i64::try_from(period_us.max(1)).unwrap_or(1);
    let first = included.first().expect("included").captured_at();
    let last = included.last().expect("included").captured_at();
    let effective_range = TimeRange::new(first, last.saturating_add_micros(period))?;
    Ok((included, effective_range))
}

fn retained_intervals(
    frames: &[StoredFrame],
    period_us: u64,
) -> Result<Vec<TimeRange>, A2aLabError> {
    coverage_segments(frames, period_us)
        .into_iter()
        .map(|(start, end)| TimeRange::new(start, end))
        .collect()
}

fn coverage_segments(frames: &[StoredFrame], period_us: u64) -> Vec<(UtcTimestamp, UtcTimestamp)> {
    let Some(first) = frames.first() else {
        return Vec::new();
    };
    let period = i64::try_from(period_us.max(1)).unwrap_or(1);
    let mut segments = Vec::new();
    let mut start = first.captured_at;
    let mut last = first.captured_at;
    for frame in frames.iter().skip(1) {
        let gap = frame
            .captured_at
            .as_micros()
            .saturating_sub(last.as_micros());
        if gap > period {
            segments.push((start, last.saturating_add_micros(period)));
            start = frame.captured_at;
        }
        last = frame.captured_at;
    }
    segments.push((start, last.saturating_add_micros(period)));
    segments
}

fn captured_order(left: &StoredFrame, right: &StoredFrame) -> Ordering {
    left.captured_at.cmp(&right.captured_at)
}

fn frames_for(state: &VideoState, source_id: &VideoSourceId) -> Vec<StoredFrame> {
    state
        .frames
        .iter()
        .filter(|(id, _)| id == source_id)
        .map(|(_, frame)| StoredFrame {
            captured_at: frame.captured_at,
            jpeg: frame.jpeg.clone(),
        })
        .collect()
}

fn publish_frame(state: &mut VideoState, source_id: &VideoSourceId, frame: &VideoFrame) {
    for follow in &mut state.follows {
        if follow.stopped || &follow.source_id != source_id {
            continue;
        }
        follow.pending.push(frame.clone());
        follow.notify.notify_one();
    }
}

fn stop_follows(state: &mut VideoState, source_id: &VideoSourceId) {
    for follow in &mut state.follows {
        if &follow.source_id == source_id {
            follow.stopped = true;
            follow.notify.notify_one();
        }
    }
}

fn take_earliest(frames: &mut Vec<VideoFrame>) -> Option<VideoFrame> {
    let index = frames
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| left.captured_at().cmp(&right.captured_at()))
        .map(|(index, _)| index)?;
    Some(frames.remove(index))
}

fn is_live(state: &VideoState, source_id: &VideoSourceId) -> bool {
    state
        .live
        .iter()
        .find(|(id, _)| id == source_id)
        .is_some_and(|(_, available)| *available)
}

fn find_source<'a>(
    state: &'a VideoState,
    source_id: &VideoSourceId,
) -> Result<&'a VideoSource, A2aLabError> {
    state
        .sources
        .iter()
        .find(|source| source.id() == source_id)
        .ok_or_else(|| A2aLabError::not_found("video source", source_id.to_string()))
}

fn require_source(state: &VideoState, source_id: &VideoSourceId) -> Result<(), A2aLabError> {
    find_source(state, source_id).map(|_| ())
}

fn require_available(state: &VideoState) -> Result<(), A2aLabError> {
    match &state.unavailable {
        Some(message) => Err(A2aLabError::unavailable(message.clone())),
        None => Ok(()),
    }
}

fn purge_expired_follows(state: &mut VideoState) {
    let now = Instant::now();
    state.prepared.retain(|follow| follow.expires_at > now);
}

fn encoded_live_track(
    state: &VideoState,
    source_id: &VideoSourceId,
) -> Option<(DvrTrackId, VideoCodec)> {
    let mut tracks: Vec<_> = state
        .dvr_tracks
        .iter()
        .filter(|track| &track.source_id == source_id)
        .filter(|track| matches!(track.codec, VideoCodec::H264 | VideoCodec::H265))
        .collect();
    tracks.sort_by(|left, right| {
        encoded_track_rank(left.codec)
            .cmp(&encoded_track_rank(right.codec))
            .then_with(|| left.id.as_str().cmp(right.id.as_str()))
    });
    tracks.first().map(|track| (track.id.clone(), track.codec))
}

fn encoded_track_rank(codec: VideoCodec) -> u8 {
    match codec {
        VideoCodec::H264 => 0,
        VideoCodec::H265 => 1,
        VideoCodec::Mjpeg => 2,
    }
}

async fn resolve_live_anchor(
    store: &MemoryVideos,
    source_id: &VideoSourceId,
    anchor: LiveAnchor,
) -> Result<UtcTimestamp, A2aLabError> {
    match anchor {
        LiveAnchor::Fixed(at) => Ok(at),
        LiveAnchor::LiveEdge => DvrMediaPackager::live_edge_anchor(store, source_id).await,
    }
}

async fn encoded_live_playback_8x(
    videos: &MemoryVideos,
    follow: &VideoFollow,
    source_id: &VideoSourceId,
    track_id: &DvrTrackId,
    anchor: LiveAnchor,
) -> Result<crate::videos::VideoPlayback8xMetadata, A2aLabError> {
    let packager = DvrMediaPackager::default();
    let root = video_live_path(follow.id());
    let anchor_ts = resolve_live_anchor(videos, source_id, anchor).await?;
    let session = packager
        .package_live_hls_at(videos, &root, source_id, track_id, anchor_ts)
        .await?;
    Ok(session.playback_8x_metadata())
}

impl DvrMediaStore for MemoryVideos {
    async fn retention_bounds(
        &self,
        source_id: &VideoSourceId,
    ) -> Result<DvrRetentionBounds, A2aLabError> {
        let state = self.lock().await;
        require_available(&state)?;
        require_source(&state, source_id)?;
        dvr_retention_bounds(&state, source_id)
    }

    async fn media_coverage(
        &self,
        source_id: &VideoSourceId,
    ) -> Result<DvrMediaCoverage, A2aLabError> {
        let state = self.lock().await;
        require_available(&state)?;
        let period = find_source(&state, source_id)?.frame_period_us();
        dvr_media_coverage(&state, source_id, period)
    }

    async fn list_tracks(
        &self,
        source_id: &VideoSourceId,
    ) -> Result<Vec<DvrMediaTrack>, A2aLabError> {
        let state = self.lock().await;
        require_available(&state)?;
        require_source(&state, source_id)?;
        dvr_list_tracks(&state, source_id)
    }

    async fn read_media_window(
        &self,
        request: ReadDvrMediaWindowRequest,
    ) -> Result<Page<DvrMediaSegment>, A2aLabError> {
        let state = self.lock().await;
        require_available(&state)?;
        require_source(&state, request.source_id())?;
        let matched = dvr_segments_in_window(
            &state,
            request.source_id(),
            request.track_id(),
            request.window(),
        )?;
        paginate_dvr_segments(&matched, request.page())
    }

    async fn frame_at(&self, request: DvrFrameAtRequest) -> Result<DvrFrameAtResult, A2aLabError> {
        let state = self.lock().await;
        require_available(&state)?;
        let period = find_source(&state, request.source_id())?.frame_period_us();
        dvr_frame_at(&state, &request, period)
    }
}

fn segment_order(left: &StoredDvrSegment, right: &StoredDvrSegment) -> Ordering {
    left.source_id
        .as_str()
        .cmp(right.source_id.as_str())
        .then_with(|| left.track_id.as_str().cmp(right.track_id.as_str()))
        .then_with(|| left.range.start().cmp(&right.range.start()))
}

fn record_mjpeg_dvr_segment(
    state: &mut VideoState,
    source_id: &VideoSourceId,
    captured_at: UtcTimestamp,
    jpeg: &[u8],
    period_us: u64,
) -> Result<(), A2aLabError> {
    let track_id = MemoryVideos::mjpeg_track_id(source_id)?;
    ensure_mjpeg_track(state, source_id, &track_id);
    let end = captured_at.saturating_add_micros(i64::try_from(period_us.max(1)).unwrap_or(1));
    let range = TimeRange::new(captured_at, end)?;
    state.dvr_segments.push(StoredDvrSegment {
        source_id: source_id.clone(),
        track_id,
        range,
        keyframe: true,
        payload: DvrMediaPayload::MjpegFrame(jpeg.to_vec()),
    });
    Ok(())
}

fn ensure_mjpeg_track(state: &mut VideoState, source_id: &VideoSourceId, track_id: &DvrTrackId) {
    if state
        .dvr_tracks
        .iter()
        .any(|track| track.source_id == *source_id && track.id == *track_id)
    {
        return;
    }
    state.dvr_tracks.push(StoredDvrTrack {
        source_id: source_id.clone(),
        id: track_id.clone(),
        codec: VideoCodec::Mjpeg,
        codec_initialization: Vec::new(),
    });
}

fn dvr_segments_for_source<'a>(
    state: &'a VideoState,
    source_id: &VideoSourceId,
) -> Vec<&'a StoredDvrSegment> {
    let mut segments: Vec<&'a StoredDvrSegment> = state
        .dvr_segments
        .iter()
        .filter(|segment| &segment.source_id == source_id)
        .collect();
    segments.sort_by(|left, right| segment_order(left, right));
    segments
}

fn dvr_retention_bounds(
    state: &VideoState,
    source_id: &VideoSourceId,
) -> Result<DvrRetentionBounds, A2aLabError> {
    let segments = dvr_segments_for_source(state, source_id);
    let Some(first) = segments.first() else {
        return Err(A2aLabError::no_recording(
            "source has no retained DVR media",
        ));
    };
    let oldest = segments
        .iter()
        .map(|segment| segment.range.start())
        .min()
        .unwrap_or(first.range.start());
    let newest = segments
        .iter()
        .map(|segment| segment.range.end())
        .max()
        .unwrap_or(first.range.end());
    DvrRetentionBounds::new(oldest, newest)
}

fn dvr_media_coverage(
    state: &VideoState,
    source_id: &VideoSourceId,
    period_us: u64,
) -> Result<DvrMediaCoverage, A2aLabError> {
    let segments = dvr_segments_for_source(state, source_id);
    if segments.is_empty() {
        return Err(A2aLabError::no_recording(
            "source has no retained DVR media",
        ));
    }
    let mut points: Vec<UtcTimestamp> = segments
        .iter()
        .map(|segment| segment.range.start())
        .collect();
    points.sort();
    points.dedup();
    let pseudo_frames: Vec<StoredFrame> = points
        .into_iter()
        .map(|captured_at| StoredFrame {
            captured_at,
            jpeg: Vec::new(),
        })
        .collect();
    retained_intervals(&pseudo_frames, period_us).and_then(DvrMediaCoverage::new)
}

fn dvr_list_tracks(
    state: &VideoState,
    source_id: &VideoSourceId,
) -> Result<Vec<DvrMediaTrack>, A2aLabError> {
    let tracks: Vec<_> = state
        .dvr_tracks
        .iter()
        .filter(|track| track.source_id == *source_id)
        .collect();
    if tracks.is_empty() {
        return Err(A2aLabError::no_recording(
            "source has no retained DVR media",
        ));
    }
    tracks
        .into_iter()
        .map(|track| {
            let bounds = track_bounds(state, source_id, &track.id)?;
            DvrMediaTrack::new(
                track.id.clone(),
                track.codec,
                track.codec_initialization.clone(),
                bounds,
            )
        })
        .collect()
}

fn track_bounds(
    state: &VideoState,
    source_id: &VideoSourceId,
    track_id: &DvrTrackId,
) -> Result<TimeRange, A2aLabError> {
    let segments: Vec<_> = state
        .dvr_segments
        .iter()
        .filter(|segment| segment.source_id == *source_id && segment.track_id == *track_id)
        .collect();
    let Some(first) = segments.first() else {
        return Err(A2aLabError::no_recording("track has no retained segments"));
    };
    let start = segments
        .iter()
        .map(|segment| segment.range.start())
        .min()
        .unwrap_or(first.range.start());
    let end = segments
        .iter()
        .map(|segment| segment.range.end())
        .max()
        .unwrap_or(first.range.end());
    TimeRange::new(start, end)
}

fn dvr_segments_in_window(
    state: &VideoState,
    source_id: &VideoSourceId,
    track_id: &DvrTrackId,
    window: TimeRange,
) -> Result<Vec<DvrMediaSegment>, A2aLabError> {
    let mut track_segments: Vec<_> = state
        .dvr_segments
        .iter()
        .filter(|segment| segment.source_id == *source_id && segment.track_id == *track_id)
        .collect();
    track_segments.sort_by(|left, right| segment_order(left, right));
    let start_index =
        track_segments.partition_point(|segment| segment.range.end() <= window.start());
    let mut matched = Vec::new();
    for segment in &track_segments[start_index..] {
        if segment.range.start() >= window.end() {
            break;
        }
        if dvr_ranges_overlap(segment.range, window) {
            matched.push(stored_segment_to_public(segment)?);
        }
    }
    Ok(matched)
}

fn dvr_ranges_overlap(left: TimeRange, right: TimeRange) -> bool {
    left.start() < right.end() && right.start() < left.end()
}

fn stored_segment_to_public(segment: &StoredDvrSegment) -> Result<DvrMediaSegment, A2aLabError> {
    DvrMediaSegment::new(
        segment.track_id.clone(),
        segment.range,
        segment.keyframe,
        segment.payload.clone(),
    )
}

fn paginate_dvr_segments(
    items: &[DvrMediaSegment],
    page: &PageRequest,
) -> Result<Page<DvrMediaSegment>, A2aLabError> {
    page.check()?;
    let start = match page.cursor() {
        None => 0,
        Some(cursor) => cursor
            .parse::<usize>()
            .map_err(|_| A2aLabError::invalid("cursor", "is not a valid page cursor"))?,
    };
    if start > items.len() {
        return Err(A2aLabError::invalid("cursor", "is past the end"));
    }
    let limit = usize::try_from(page.limit()).unwrap_or(usize::MAX);
    let end = start.saturating_add(limit).min(items.len());
    let next_cursor = (end < items.len()).then(|| end.to_string());
    Ok(Page::new(items[start..end].to_vec(), next_cursor))
}

fn dvr_frame_at(
    state: &VideoState,
    request: &DvrFrameAtRequest,
    period_us: u64,
) -> Result<DvrFrameAtResult, A2aLabError> {
    let at = request.at();
    let coverage = dvr_media_coverage(state, request.source_id(), period_us)?;
    if !coverage
        .intervals()
        .iter()
        .any(|interval| interval.contains(at))
    {
        if coverage
            .discontinuities()
            .iter()
            .any(|gap| gap.gap().contains(at))
        {
            return Err(A2aLabError::recording_gap("requested instant is in a gap"));
        }
        return Err(A2aLabError::no_recording(
            "requested instant is not retained",
        ));
    }
    let candidates: Vec<_> = state
        .dvr_segments
        .iter()
        .filter(|segment| segment.source_id == *request.source_id())
        .filter(|segment| segment.track_id == *request.track_id())
        .filter(|segment| segment.range.contains(at) || segment.range.start() <= at)
        .collect();
    let selected = candidates
        .iter()
        .min_by(|left, right| {
            dvr_segment_delta(left, at)
                .cmp(&dvr_segment_delta(right, at))
                .then_with(|| left.range.start().cmp(&right.range.start()))
        })
        .ok_or_else(|| A2aLabError::not_found("DVR frame", at.to_string()))?;
    if let Some(limit) = request.max_delta_us()
        && dvr_segment_delta(selected, at) > limit
    {
        return Err(A2aLabError::not_found("DVR frame", at.to_string()));
    }
    Ok(DvrFrameAtResult::new(
        at,
        selected.range.start(),
        selected.payload.clone(),
    ))
}

fn dvr_segment_delta(segment: &StoredDvrSegment, at: UtcTimestamp) -> u64 {
    segment.range.start().as_micros().abs_diff(at.as_micros())
}
