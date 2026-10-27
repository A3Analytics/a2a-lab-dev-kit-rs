//! Video provider interface.

use crate::error::A2aLabError;
use crate::id::{VideoClipId, VideoFollowId};
use crate::page::Page;
use crate::videos::{
    FollowVideoRequest, GetVideoClipRequest, GetVideoFrameAtRequest, GetVideoFrameAtResult,
    GetVideoTimelineRequest, ListVideoSourcesRequest, VideoClip, VideoClipStream, VideoFollow,
    VideoLiveStream, VideoSource, VideoTimeline,
};

/// Lists video sources, reads frames at a time, timelines, live follow, and clips.
///
/// Implementations own recording, indexing, retention, clip expiry, and live
/// subscription. This crate does not capture cameras. Saved images are not part
/// of this provider.
pub trait VideoProvider: Send + Sync {
    /// Returns the video sources this agent can read.
    fn list_video_sources(
        &self,
        request: ListVideoSourcesRequest,
    ) -> impl Future<Output = Result<Page<VideoSource>, A2aLabError>> + Send;

    /// Returns the retained frame at `request.at`.
    ///
    /// This is a frame-at-time lookup. Ties choose the earlier frame. An optional
    /// maximum delta is honored. Empty coverage, a request outside retained
    /// coverage, or a coverage gap is `not_found`. Coverage is never silently clamped.
    fn get_video_frame_at(
        &self,
        request: GetVideoFrameAtRequest,
    ) -> impl Future<Output = Result<GetVideoFrameAtResult, A2aLabError>> + Send;

    /// Prepares a short-lived clip handle for a half-open `[start, end)` range.
    ///
    /// A range with no retained recording is [`A2aLabError::no_recording`]. A range
    /// that crosses a gap between retained intervals is [`A2aLabError::recording_gap`].
    /// Invalid, reversed, or over-limit ranges are `invalid`. JSON metadata never
    /// includes video bytes.
    fn get_video_clip(
        &self,
        request: GetVideoClipRequest,
    ) -> impl Future<Output = Result<VideoClip, A2aLabError>> + Send;

    /// Opens a bounded JPEG frame stream for a previously prepared clip.
    ///
    /// An expired handle is [`A2aLabError::clip_expired`]. An unknown handle is `not_found`.
    fn open_video_clip(
        &self,
        id: &VideoClipId,
    ) -> impl Future<Output = Result<VideoClipStream, A2aLabError>> + Send;

    /// Returns ordered retained coverage for one source.
    ///
    /// Temporally continuous coverage is one logical interval. Genuine temporal
    /// gaps remain separate intervals. The result has no storage-file or partition
    /// boundary.
    fn get_video_timeline(
        &self,
        _request: GetVideoTimelineRequest,
    ) -> impl Future<Output = Result<VideoTimeline, A2aLabError>> + Send {
        let _ = self;
        std::future::ready(timeline_unavailable())
    }

    /// Prepares a live-follow handle for frames captured after the stream opens.
    ///
    /// A source that cannot be followed is [`A2aLabError::live_unavailable`].
    /// The handle contains no frame bytes.
    fn follow_video(
        &self,
        _request: FollowVideoRequest,
    ) -> impl Future<Output = Result<VideoFollow, A2aLabError>> + Send {
        let _ = self;
        std::future::ready(timeline_unavailable())
    }

    /// Opens the JPEG frame stream for a previously prepared live-follow handle.
    ///
    /// Unknown handles are `not_found`. The stream stays open across the retained
    /// live edge and ends when the consumer drops it or the provider stops the source.
    fn open_video_follow(
        &self,
        _id: &VideoFollowId,
    ) -> impl Future<Output = Result<VideoLiveStream, A2aLabError>> + Send {
        let _ = self;
        std::future::ready(timeline_unavailable())
    }
}

fn timeline_unavailable<T>() -> Result<T, A2aLabError> {
    Err(A2aLabError::unavailable(
        "video timeline and live follow are not available",
    ))
}
