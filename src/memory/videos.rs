//! In-memory video provider.

use std::cmp::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, MutexGuard, Notify};

use crate::error::A2aLabError;
use crate::id::{ImageId, ImageSourceId, VideoClipId, VideoFollowId, VideoSourceId};
use crate::images::{Image, ImageDescriptor};
use crate::json_object::JsonObject;
use crate::page::{Page, slice_page};
use crate::time::{TimeRange, UtcTimestamp};
use crate::videos::{
    FollowVideoRequest, GetVideoClipRequest, GetVideoFrameAtRequest, GetVideoFrameAtResult,
    GetVideoTimelineRequest, ListVideoSourcesRequest, MJPEG_MEDIA_TYPE, VideoClip, VideoClipStream,
    VideoFollow, VideoFrame, VideoLiveStream, VideoProvider, VideoSource, VideoTimeline,
};

/// Default clip handle lifetime. Not durable storage.
pub const DEFAULT_VIDEO_CLIP_TTL: Duration = Duration::from_secs(10);

struct StoredFrame {
    captured_at: UtcTimestamp,
    jpeg: Vec<u8>,
}

struct StoredClip {
    clip: VideoClip,
    frames: Vec<VideoFrame>,
    expires_at: Instant,
}

struct PreparedFollow {
    id: VideoFollowId,
    source_id: VideoSourceId,
    opened: bool,
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
    clips: Vec<StoredClip>,
    next_clip: u64,
    ttl: Duration,
    unavailable: Option<String>,
    live: Vec<(VideoSourceId, bool)>,
    prepared: Vec<PreparedFollow>,
    follows: Vec<ActiveFollow>,
    next_follow: u64,
}

impl VideoState {
    fn with_ttl(ttl: Duration) -> Self {
        Self {
            sources: Vec::new(),
            frames: Vec::new(),
            clips: Vec::new(),
            next_clip: 0,
            ttl,
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
    /// Creates an empty catalog with a 10-second clip TTL.
    #[must_use]
    pub fn new() -> Self {
        Self::with_clip_ttl(DEFAULT_VIDEO_CLIP_TTL)
    }

    /// Creates an empty catalog. `Duration::ZERO` expires clips immediately.
    #[must_use]
    pub fn with_clip_ttl(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VideoState::with_ttl(ttl))),
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
    pub async fn insert_frame(
        &self,
        source_id: VideoSourceId,
        captured_at: UtcTimestamp,
        jpeg: Vec<u8>,
    ) -> Result<(), A2aLabError> {
        let frame = VideoFrame::new(captured_at, jpeg)?;
        let mut state = self.lock().await;
        require_source(&state, &source_id)?;
        state.frames.push((
            source_id.clone(),
            StoredFrame {
                captured_at: frame.captured_at(),
                jpeg: frame.jpeg().to_vec(),
            },
        ));
        publish_frame(&mut state, &source_id, &frame);
        Ok(())
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
        let now = Instant::now();
        state.clips.retain(|clip| clip.expires_at > now);
        Ok(())
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
        let period = find_source(&state, request.source_id())?.frame_period_us();
        let mut frames = frames_for(&state, request.source_id());
        frames.sort_by(captured_order);
        let (included, effective_range) = clip_frames(&frames, request.range(), period)?;
        state.next_clip += 1;
        let id = VideoClipId::new(format!("clip-{}", state.next_clip))?;
        let clip = VideoClip::new(
            id,
            request.source_id().clone(),
            request.range(),
            effective_range,
            MJPEG_MEDIA_TYPE,
        )?;
        let expires_at = Instant::now() + state.ttl;
        state.clips.push(StoredClip {
            clip: clip.clone(),
            frames: included,
            expires_at,
        });
        Ok(clip)
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
        state.next_follow += 1;
        let id = VideoFollowId::new(format!("follow-{}", state.next_follow))?;
        let follow = VideoFollow::new(id.clone(), request.source_id().clone(), MJPEG_MEDIA_TYPE)?;
        state.prepared.push(PreparedFollow {
            id,
            source_id: request.source_id().clone(),
            opened: false,
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
                .position(|follow| &follow.id == id)
                .ok_or_else(|| A2aLabError::not_found("video follow", id.to_string()))?;
            let source_id = state.prepared[index].source_id.clone();
            if !is_live(&state, &source_id) {
                return Err(A2aLabError::live_unavailable(
                    "live follow is not available",
                ));
            }
            if state.prepared[index].opened {
                return Err(A2aLabError::invalid("id", "live follow is already open"));
            }
            state.prepared[index].opened = true;
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
