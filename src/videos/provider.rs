//! Video provider interface.

use futures_util::StreamExt;

use crate::PackagedMediaAsset;
use crate::error::A2aLabError;
use crate::id::{VideoClipId, VideoFollowId};
use crate::page::Page;
use crate::videos::media_asset::ResolvedMediaAsset;
use crate::videos::{
    DEFAULT_MJPEG_BOUNDARY, FollowVideoRequest, GetVideoClipRequest, GetVideoFrameAtRequest,
    GetVideoFrameAtResult, GetVideoTimelineRequest, ListVideoSourcesRequest, VideoClip,
    VideoClipStream, VideoFollow, VideoLiveStream, VideoSource, VideoTimeline, encode_mjpeg_end,
    encode_mjpeg_part, mjpeg_content_type,
};

/// Lists video sources, reads frames at a time, timelines, live follow, and clips.
///
/// The first six methods are the [`crate::videos::VideoCatalog`] control plane.
/// Implementations own recording, indexing, retention, clip expiry, and live
/// subscription. They are not required to generate HLS playlists, CMAF segments,
/// MP4 files, HTTP responses, or authentication behavior. Saved images are not
/// part of this provider.
pub trait VideoProvider: Send + Sync {
    /// Returns the video sources this agent can read.
    fn list_video_sources(
        &self,
        request: ListVideoSourcesRequest,
    ) -> impl Future<Output = Result<Page<VideoSource>, A2aLabError>> + Send;

    /// Returns the retained frame at `request.at`.
    fn get_video_frame_at(
        &self,
        request: GetVideoFrameAtRequest,
    ) -> impl Future<Output = Result<GetVideoFrameAtResult, A2aLabError>> + Send;

    /// Prepares an opaque finite playback session for a half-open `[start, end)` range.
    fn get_video_clip(
        &self,
        request: GetVideoClipRequest,
    ) -> impl Future<Output = Result<VideoClip, A2aLabError>> + Send;

    /// Opens a bounded frame stream for a previously prepared finite session.
    fn open_video_clip(
        &self,
        id: &VideoClipId,
    ) -> impl Future<Output = Result<VideoClipStream, A2aLabError>> + Send;

    /// Packages the finite retained body for one prepared clip handle.
    ///
    /// SiLA LabVideos reads this export while building `BinaryDownload`. The default
    /// implementation materializes a closed MJPEG multipart body from
    /// [`Self::open_video_clip`].
    fn export_finite_video_clip(
        &self,
        id: &VideoClipId,
    ) -> impl Future<Output = Result<PackagedMediaAsset, A2aLabError>> + Send {
        let id = id.clone();
        async move {
            let mut stream = self.open_video_clip(&id).await?;
            let mut body = Vec::new();
            while let Some(frame) = stream.next().await {
                let frame = frame?;
                body.extend_from_slice(&encode_mjpeg_part(&frame, DEFAULT_MJPEG_BOUNDARY));
            }
            body.extend_from_slice(&encode_mjpeg_end(DEFAULT_MJPEG_BOUNDARY));
            PackagedMediaAsset::new(mjpeg_content_type(DEFAULT_MJPEG_BOUNDARY), body)
        }
    }

    /// Returns ordered retained coverage for one source.
    fn get_video_timeline(
        &self,
        _request: GetVideoTimelineRequest,
    ) -> impl Future<Output = Result<VideoTimeline, A2aLabError>> + Send {
        let _ = self;
        std::future::ready(timeline_unavailable())
    }

    /// Prepares an opaque live playback session for frames captured after the stream opens.
    fn follow_video(
        &self,
        _request: FollowVideoRequest,
    ) -> impl Future<Output = Result<VideoFollow, A2aLabError>> + Send {
        let _ = self;
        std::future::ready(timeline_unavailable())
    }

    /// Opens the frame stream for a previously prepared live session.
    fn open_video_follow(
        &self,
        _id: &VideoFollowId,
    ) -> impl Future<Output = Result<VideoLiveStream, A2aLabError>> + Send {
        let _ = self;
        std::future::ready(timeline_unavailable())
    }

    /// Re-anchors a prepared live session to a retained instant.
    fn rebase_video_follow(
        &self,
        _id: &VideoFollowId,
        _anchor: crate::time::UtcTimestamp,
    ) -> impl Future<Output = Result<VideoFollow, A2aLabError>> + Send {
        let _ = self;
        std::future::ready(timeline_unavailable())
    }

    /// Resolves one gateway asset for a prepared live session.
    fn resolve_live_playback_asset(
        &self,
        id: &VideoFollowId,
        relative_path: Option<&str>,
    ) -> impl Future<Output = Result<ResolvedMediaAsset, A2aLabError>> + Send {
        let _ = (self, id, relative_path);
        std::future::ready(Err(A2aLabError::not_found("video follow", id.to_string())))
    }
}

fn timeline_unavailable<T>() -> Result<T, A2aLabError> {
    Err(A2aLabError::unavailable(
        "video timeline and live follow are not available",
    ))
}
