//! Authenticated video clip and live-follow delivery for [`super::A2aLabService`].

use crate::PackagedMediaAsset;
use crate::error::A2aLabError;
use crate::id::{VideoClipId, VideoFollowId};
use crate::time::UtcTimestamp;
use crate::videos::{ResolvedMediaAsset, VideoClipStream, VideoFollow, VideoLiveStream};

use super::A2aLabService;

impl<L, M, W, I, V> A2aLabService<L, M, W, I, V>
where
    L: crate::logs::LogProvider,
    M: crate::metrics::MetricProvider,
    W: crate::tasks::TaskProvider,
    I: crate::images::ImageProvider,
    V: crate::videos::VideoProvider,
{
    /// Opens a bounded JPEG frame stream for a previously prepared clip.
    pub async fn open_video_clip(&self, id: &VideoClipId) -> Result<VideoClipStream, A2aLabError> {
        self.videos.open_video_clip(id).await
    }

    /// Packages the finite retained body for one prepared clip handle.
    pub async fn export_finite_video_clip(
        &self,
        id: &VideoClipId,
    ) -> Result<PackagedMediaAsset, A2aLabError> {
        self.videos.export_finite_video_clip(id).await
    }

    /// Opens the JPEG frame stream for a previously prepared live-follow handle.
    pub async fn open_video_follow(
        &self,
        id: &VideoFollowId,
    ) -> Result<VideoLiveStream, A2aLabError> {
        self.videos.open_video_follow(id).await
    }

    /// Resolves one authenticated H.264 clip asset. MJPEG clips return not-found.
    pub async fn resolve_clip_playback_asset(
        &self,
        id: &VideoClipId,
        relative_path: Option<&str>,
    ) -> Result<ResolvedMediaAsset, A2aLabError> {
        self.videos
            .resolve_clip_playback_asset(id, relative_path)
            .await
    }

    /// Resolves one authenticated live session asset or MJPEG stream.
    pub async fn resolve_live_playback_asset(
        &self,
        id: &VideoFollowId,
        relative_path: Option<&str>,
    ) -> Result<ResolvedMediaAsset, A2aLabError> {
        self.videos
            .resolve_live_playback_asset(id, relative_path)
            .await
    }

    /// Re-anchors a prepared live session to a retained instant.
    pub async fn rebase_video_follow(
        &self,
        id: &VideoFollowId,
        anchor: UtcTimestamp,
    ) -> Result<VideoFollow, A2aLabError> {
        self.videos.rebase_video_follow(id, anchor).await
    }
}
