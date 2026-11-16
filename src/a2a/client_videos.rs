//! Video, frame-decision, and semantic-search calls on [`super::A2aClient`].

use crate::error::A2aLabError;
use crate::id::{VideoClipId, VideoFollowId};
use crate::page::Page;
use crate::service::{A2aLabCommand, A2aLabResult};
use crate::videos::{
    AppendFrameDecisionRequest, AppendFrameDecisionResult, DEFAULT_MJPEG_BOUNDARY,
    FollowVideoRequest, FrameDecisionCapabilities, FrameDecisionHistoryResult,
    GetFrameDecisionCapabilitiesRequest, GetSemanticIndexingCoverageRequest,
    GetSemanticSearchCapabilitiesRequest, GetVideoClipRequest, GetVideoFrameAtRequest,
    GetVideoFrameAtResult, GetVideoTimelineRequest, ListVideoSourcesRequest,
    ReadFrameDecisionHistoryRequest, SearchSemanticFramesRequest, SemanticFrameSearchResult,
    SemanticIndexingCoverage, SemanticSearchCapabilities, VideoClip, VideoClipStream, VideoFollow,
    VideoSource, VideoTimeline, decode_mjpeg_multipart,
};

use super::A2aClient;

impl A2aClient {
    /// Lists video sources.
    pub async fn list_video_sources(
        &self,
        request: ListVideoSourcesRequest,
    ) -> Result<Page<VideoSource>, A2aLabError> {
        self.result(
            A2aLabCommand::ListVideoSources(request),
            |result| match result {
                A2aLabResult::ListVideoSources(page) => Some(page),
                _ => None,
            },
        )
        .await
    }

    /// Reads the retained frame at a UTC instant.
    ///
    /// The operation name remains `get_video_frame_at`.
    pub async fn get_video_frame_at(
        &self,
        request: GetVideoFrameAtRequest,
    ) -> Result<GetVideoFrameAtResult, A2aLabError> {
        self.result(
            A2aLabCommand::GetVideoFrameAt(request),
            |result| match result {
                A2aLabResult::GetVideoFrameAt(frame) => Some(frame),
                _ => None,
            },
        )
        .await
    }

    /// Prepares a bounded MJPEG clip handle. JSON never includes video bytes.
    pub async fn get_video_clip(
        &self,
        request: GetVideoClipRequest,
    ) -> Result<VideoClip, A2aLabError> {
        self.result(
            A2aLabCommand::GetVideoClip(request),
            |result| match result {
                A2aLabResult::GetVideoClip(clip) => Some(clip),
                _ => None,
            },
        )
        .await
    }

    /// Reads ordered retained coverage for one source. JSON never includes media bytes.
    pub async fn get_video_timeline(
        &self,
        request: GetVideoTimelineRequest,
    ) -> Result<VideoTimeline, A2aLabError> {
        self.result(
            A2aLabCommand::GetVideoTimeline(request),
            |result| match result {
                A2aLabResult::GetVideoTimeline(timeline) => Some(timeline),
                _ => None,
            },
        )
        .await
    }

    /// Prepares a live-follow handle. JSON never includes frame bytes.
    pub async fn follow_video(
        &self,
        request: FollowVideoRequest,
    ) -> Result<VideoFollow, A2aLabError> {
        self.result(A2aLabCommand::FollowVideo(request), |result| match result {
            A2aLabResult::FollowVideo(follow) => Some(follow),
            _ => None,
        })
        .await
    }

    /// Discovers authorized Jev append/read capabilities.
    pub async fn get_frame_decision_capabilities(
        &self,
        request: GetFrameDecisionCapabilitiesRequest,
    ) -> Result<FrameDecisionCapabilities, A2aLabError> {
        self.result(
            A2aLabCommand::GetFrameDecisionCapabilities(request),
            |result| match result {
                A2aLabResult::GetFrameDecisionCapabilities(capabilities) => Some(capabilities),
                _ => None,
            },
        )
        .await
    }

    /// Appends one authorized Jev production decision record.
    pub async fn append_frame_decision(
        &self,
        request: AppendFrameDecisionRequest,
    ) -> Result<AppendFrameDecisionResult, A2aLabError> {
        self.result(
            A2aLabCommand::AppendFrameDecision(request),
            |result| match result {
                A2aLabResult::AppendFrameDecision(append) => Some(append),
                _ => None,
            },
        )
        .await
    }

    /// Reads bounded Jev decision history.
    pub async fn read_frame_decision_history(
        &self,
        request: ReadFrameDecisionHistoryRequest,
    ) -> Result<FrameDecisionHistoryResult, A2aLabError> {
        self.result(
            A2aLabCommand::ReadFrameDecisionHistory(request),
            |result| match result {
                A2aLabResult::ReadFrameDecisionHistory(history) => Some(history),
                _ => None,
            },
        )
        .await
    }

    /// Discovers optional semantic search schemas and operations.
    pub async fn get_semantic_search_capabilities(
        &self,
        request: GetSemanticSearchCapabilitiesRequest,
    ) -> Result<SemanticSearchCapabilities, A2aLabError> {
        self.result(
            A2aLabCommand::GetSemanticSearchCapabilities(request),
            |result| match result {
                A2aLabResult::GetSemanticSearchCapabilities(capabilities) => Some(capabilities),
                _ => None,
            },
        )
        .await
    }

    /// Searches saved frames by text or reference frame. JSON never includes media bytes.
    pub async fn search_semantic_frames(
        &self,
        request: SearchSemanticFramesRequest,
    ) -> Result<SemanticFrameSearchResult, A2aLabError> {
        self.result(
            A2aLabCommand::SearchSemanticFrames(request),
            |result| match result {
                A2aLabResult::SearchSemanticFrames(search) => Some(search),
                _ => None,
            },
        )
        .await
    }

    /// Reads semantic indexing coverage for one scope and generation.
    pub async fn get_semantic_indexing_coverage(
        &self,
        request: GetSemanticIndexingCoverageRequest,
    ) -> Result<SemanticIndexingCoverage, A2aLabError> {
        self.result(
            A2aLabCommand::GetSemanticIndexingCoverage(request),
            |result| match result {
                A2aLabResult::GetSemanticIndexingCoverage(coverage) => Some(coverage),
                _ => None,
            },
        )
        .await
    }

    /// Origin-qualified URL for `GET` of a live-follow handle.
    ///
    /// The handle's own path stays relative. This joins that path to the client origin.
    #[must_use]
    pub fn video_live_url(&self, id: &VideoFollowId) -> String {
        crate::videos::http::live_url(&self.base, id)
    }

    /// Downloads a prepared clip over `GET /media/video-clips/{id}` using protocol auth.
    pub async fn open_video_clip(&self, id: &VideoClipId) -> Result<VideoClipStream, A2aLabError> {
        let mut request = reqwest::Client::new().get(crate::videos::http::clip_url(&self.base, id));
        if let Some(token) = &self.bearer {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|error| A2aLabError::transport(error.to_string()))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| A2aLabError::transport(error.to_string()))?;
        if !status.is_success() {
            return Err(super::clip_http_error(status, &body));
        }
        let frames = decode_mjpeg_multipart(&body, DEFAULT_MJPEG_BOUNDARY)?;
        Ok(Box::pin(futures_util::stream::iter(
            frames.into_iter().map(Ok),
        )))
    }
}
