//! Shared execution of lab operations.
//!
//! Semantic frame search is an optional [`crate::videos::SemanticVideoSearchProvider`]
//! capability. [`A2aLabCommand`] and this service wire it in TASK-34; existing video
//! operations stay available when semantic search is absent.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::PackagedMediaAsset;
use crate::error::A2aLabError;
use crate::id::{RunId, VideoClipId, VideoFollowId};
use crate::images::{
    GetCurrentImageRequest, GetImageRequest, Image, ImageDescriptor, ImageProvider, ImageSource,
    ImageTransportConfig, ListImageSourcesRequest, ListImagesRequest, SearchImagesRequest,
};
use crate::logs::{ListLogSourcesRequest, LogProvider, LogRecord, LogSource, QueryLogsRequest};
use crate::metrics::{
    ListMetricsRequest, MetricDescriptor, MetricPoint, MetricProvider, QueryMetricRequest,
};
use crate::page::{Page, PageRequest};
use crate::tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskProvider,
    TaskRun, TaskState,
};
use crate::time::UtcTimestamp;
use crate::videos::{
    AppendFrameDecisionRequest, AppendFrameDecisionResult, FollowVideoRequest,
    FrameDecisionCapabilities, FrameDecisionHistoryResult, FrameDecisionLedgerProvider,
    GetFrameDecisionCapabilitiesRequest, GetSemanticIndexingCoverageRequest,
    GetSemanticSearchCapabilitiesRequest, GetVideoClipRequest, GetVideoFrameAtRequest,
    GetVideoFrameAtResult, GetVideoTimelineRequest, ListVideoSourcesRequest,
    ReadFrameDecisionHistoryRequest, ResolvedMediaAsset, SearchSemanticFramesRequest,
    SemanticFrameSearchResult, SemanticIndexingCoverage, SemanticSearchCapabilities,
    SemanticVideoSearchProvider, VideoClip, VideoClipStream, VideoFollow, VideoLiveStream,
    VideoProvider, VideoSource, VideoTimeline, authorized_assessments_for_frame,
    frame_decision_unavailable, semantic_search_unavailable,
};

/// Future returned by [`A2aLabApi`].
pub type A2aLabFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Tagged request envelope shared by A2A and MCP.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub enum A2aLabCommand {
    /// List log sources.
    ListLogSources(ListLogSourcesRequest),
    /// Query logs.
    QueryLogs(QueryLogsRequest),
    /// List metrics.
    ListMetrics(ListMetricsRequest),
    /// Query one metric.
    QueryMetric(QueryMetricRequest),
    /// List tasks.
    ListTasks(ListTasksRequest),
    /// Start a task.
    StartTask(StartTaskRequest),
    /// Read task status.
    GetTaskStatus(GetTaskStatusRequest),
    /// List image sources.
    ListImageSources(ListImageSourcesRequest),
    /// List image metadata for one source.
    ListImages(ListImagesRequest),
    /// Search image metadata.
    SearchImages(SearchImagesRequest),
    /// Read one image, including its inline bytes.
    GetImage(GetImageRequest),
    /// Read the current image for one source.
    GetCurrentImage(GetCurrentImageRequest),
    /// List video sources.
    ListVideoSources(ListVideoSourcesRequest),
    /// Read the retained frame at a UTC instant.
    ///
    /// The operation name remains `get_video_frame_at`.
    GetVideoFrameAt(GetVideoFrameAtRequest),
    /// Prepare a bounded MJPEG clip handle.
    GetVideoClip(GetVideoClipRequest),
    /// Read ordered retained coverage for one source.
    GetVideoTimeline(GetVideoTimelineRequest),
    /// Prepare a live-follow handle. JSON never includes frame bytes.
    FollowVideo(FollowVideoRequest),
    /// Discover authorized Jev append/read capabilities.
    GetFrameDecisionCapabilities(GetFrameDecisionCapabilitiesRequest),
    /// Append one authorized Jev production decision record.
    AppendFrameDecision(AppendFrameDecisionRequest),
    /// Read bounded Jev decision history.
    ReadFrameDecisionHistory(ReadFrameDecisionHistoryRequest),
    /// Discover optional semantic search schemas and operations.
    GetSemanticSearchCapabilities(GetSemanticSearchCapabilitiesRequest),
    /// Search saved frames by text or reference frame inside a bounded scope.
    SearchSemanticFrames(SearchSemanticFramesRequest),
    /// Read semantic indexing coverage for one scope and generation.
    GetSemanticIndexingCoverage(GetSemanticIndexingCoverageRequest),
}

/// Tagged result envelope shared by A2A and MCP.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "result", rename_all = "snake_case")]
pub enum A2aLabResult {
    /// A page of log sources.
    ListLogSources(Page<LogSource>),
    /// A page of log records.
    QueryLogs(Page<LogRecord>),
    /// A page of metric descriptors.
    ListMetrics(Page<MetricDescriptor>),
    /// A page of metric samples.
    QueryMetric(Page<MetricPoint>),
    /// A page of task definitions.
    ListTasks(Page<TaskDefinition>),
    /// The run created by a start request.
    StartTask(TaskRun),
    /// The current run status.
    GetTaskStatus(TaskRun),
    /// A page of image sources.
    ListImageSources(Page<ImageSource>),
    /// A page of image metadata.
    ListImages(Page<ImageDescriptor>),
    /// A page of image metadata matching the search.
    SearchImages(Page<ImageDescriptor>),
    /// One image, including its inline bytes.
    GetImage(Image),
    /// The current image for one source, including its inline bytes.
    GetCurrentImage(Image),
    /// A page of video sources.
    ListVideoSources(Page<VideoSource>),
    /// The retained frame at a UTC instant.
    ///
    /// The result type remains [`GetVideoFrameAtResult`].
    GetVideoFrameAt(GetVideoFrameAtResult),
    /// A short-lived clip handle. JSON never includes video bytes.
    GetVideoClip(VideoClip),
    /// Ordered retained coverage for one source.
    GetVideoTimeline(VideoTimeline),
    /// A live-follow handle. JSON never includes frame bytes.
    FollowVideo(VideoFollow),
    /// Advertised append/read capabilities for the active caller role.
    GetFrameDecisionCapabilities(FrameDecisionCapabilities),
    /// Stored or replayed Jev decision record.
    AppendFrameDecision(AppendFrameDecisionResult),
    /// One page of decision history.
    ReadFrameDecisionHistory(FrameDecisionHistoryResult),
    /// Advertised semantic search operations and schemas.
    GetSemanticSearchCapabilities(SemanticSearchCapabilities),
    /// Ranked semantic frame hits with metadata only.
    SearchSemanticFrames(SemanticFrameSearchResult),
    /// Indexing state counts for the scoped generation.
    GetSemanticIndexingCoverage(SemanticIndexingCoverage),
}

/// Stored view of an A2A task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskSnapshot {
    /// A2A task identifier.
    pub id: String,
    /// A2A context identifier.
    pub context_id: String,
    /// A2A task state.
    pub state: TaskState,
    /// Typed lab result carried by the task artifact.
    pub result: A2aLabResult,
}

/// Outcome of executing one lab command.
#[derive(Debug, Clone, PartialEq)]
pub struct A2aLabOutcome {
    /// Task created or updated by the command.
    pub task: TaskSnapshot,
}

/// Object-safe facade used by the protocol adapters.
pub trait A2aLabApi: Send + Sync {
    /// Executes a lab command and records its task.
    fn execute(
        &self,
        command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabOutcome, A2aLabError>>;

    /// Returns the current snapshot of a previously created task.
    fn task<'a>(&'a self, task_id: &str) -> A2aLabFuture<'a, Result<TaskSnapshot, A2aLabError>>;

    /// Cancels a started lab run. Snapshot commands are not cancelable.
    fn cancel(
        &self,
        request: GetTaskStatusRequest,
    ) -> A2aLabFuture<'_, Result<TaskSnapshot, A2aLabError>>;

    /// Opens a bounded JPEG frame stream for a previously prepared clip.
    ///
    /// HTTP adapters can mount `GET /media/video-clips/{id}` on this method.
    fn open_video_clip(
        &self,
        id: &VideoClipId,
    ) -> A2aLabFuture<'_, Result<VideoClipStream, A2aLabError>>;

    /// Packages the finite retained body for one prepared clip handle.
    fn export_finite_video_clip(
        &self,
        id: &VideoClipId,
    ) -> A2aLabFuture<'_, Result<PackagedMediaAsset, A2aLabError>>;

    /// Opens the JPEG frame stream for a previously prepared live-follow handle.
    ///
    /// HTTP adapters can mount `GET /media/video-live/{id}` on this method.
    /// The default is unavailable until an implementor provides a stream.
    fn open_video_follow(
        &self,
        id: &VideoFollowId,
    ) -> A2aLabFuture<'_, Result<VideoLiveStream, A2aLabError>> {
        let _ = (self, id);
        Box::pin(std::future::ready(Err(A2aLabError::unavailable(
            "live follow is not available",
        ))))
    }

    /// Resolves one authenticated H.264 clip asset. MJPEG clips return not-found.
    fn resolve_clip_playback_asset(
        &self,
        id: &VideoClipId,
        relative_path: Option<&str>,
    ) -> A2aLabFuture<'_, Result<ResolvedMediaAsset, A2aLabError>> {
        let _ = (self, id, relative_path);
        Box::pin(std::future::ready(Err(A2aLabError::not_found(
            "video clip",
            id.to_string(),
        ))))
    }

    /// Resolves one authenticated live session asset or MJPEG stream.
    fn resolve_live_playback_asset(
        &self,
        id: &VideoFollowId,
        relative_path: Option<&str>,
    ) -> A2aLabFuture<'_, Result<ResolvedMediaAsset, A2aLabError>> {
        let _ = (self, id, relative_path);
        Box::pin(std::future::ready(Err(A2aLabError::not_found(
            "video follow",
            id.to_string(),
        ))))
    }

    /// Re-anchors a prepared live session to a retained instant.
    fn rebase_video_follow(
        &self,
        id: &VideoFollowId,
        anchor: UtcTimestamp,
    ) -> A2aLabFuture<'_, Result<VideoFollow, A2aLabError>> {
        let _ = (self, id, anchor);
        Box::pin(std::future::ready(Err(A2aLabError::unavailable(
            "live follow is not available",
        ))))
    }
}

#[derive(Clone)]
struct StoredTask {
    context_id: String,
    body: StoredBody,
}

#[derive(Clone)]
enum StoredBody {
    Snapshot {
        state: TaskState,
        result: Box<A2aLabResult>,
    },
    Run(RunId),
}

/// Image provider used when a service is built without [`A2aLabService::with_images`].
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyImages;

impl ImageProvider for EmptyImages {
    fn list_image_sources(
        &self,
        _request: ListImageSourcesRequest,
    ) -> impl Future<Output = Result<Page<ImageSource>, A2aLabError>> + Send {
        std::future::ready(Err(images_unavailable()))
    }

    fn list_images(
        &self,
        _request: ListImagesRequest,
    ) -> impl Future<Output = Result<Page<ImageDescriptor>, A2aLabError>> + Send {
        std::future::ready(Err(images_unavailable()))
    }

    fn search_images(
        &self,
        _request: SearchImagesRequest,
    ) -> impl Future<Output = Result<Page<ImageDescriptor>, A2aLabError>> + Send {
        std::future::ready(Err(images_unavailable()))
    }

    fn get_image(
        &self,
        _request: GetImageRequest,
    ) -> impl Future<Output = Result<Image, A2aLabError>> + Send {
        std::future::ready(Err(images_unavailable()))
    }

    fn get_current_image(
        &self,
        _request: GetCurrentImageRequest,
    ) -> impl Future<Output = Result<Image, A2aLabError>> + Send {
        std::future::ready(Err(images_unavailable()))
    }
}

fn images_unavailable() -> A2aLabError {
    A2aLabError::unavailable("image provider is not configured")
}

/// Video provider used when a service is built without [`A2aLabService::with_videos`].
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyVideos;

impl VideoProvider for EmptyVideos {
    fn list_video_sources(
        &self,
        _request: ListVideoSourcesRequest,
    ) -> impl Future<Output = Result<Page<VideoSource>, A2aLabError>> + Send {
        std::future::ready(Err(videos_unavailable()))
    }

    fn get_video_frame_at(
        &self,
        _request: GetVideoFrameAtRequest,
    ) -> impl Future<Output = Result<GetVideoFrameAtResult, A2aLabError>> + Send {
        std::future::ready(Err(videos_unavailable()))
    }

    fn get_video_clip(
        &self,
        _request: GetVideoClipRequest,
    ) -> impl Future<Output = Result<VideoClip, A2aLabError>> + Send {
        std::future::ready(Err(videos_unavailable()))
    }

    fn open_video_clip(
        &self,
        _id: &VideoClipId,
    ) -> impl Future<Output = Result<VideoClipStream, A2aLabError>> + Send {
        std::future::ready(Err(videos_unavailable()))
    }

    fn get_video_timeline(
        &self,
        _request: GetVideoTimelineRequest,
    ) -> impl Future<Output = Result<VideoTimeline, A2aLabError>> + Send {
        std::future::ready(Err(videos_unavailable()))
    }

    fn follow_video(
        &self,
        _request: FollowVideoRequest,
    ) -> impl Future<Output = Result<VideoFollow, A2aLabError>> + Send {
        std::future::ready(Err(videos_unavailable()))
    }

    fn open_video_follow(
        &self,
        _id: &VideoFollowId,
    ) -> impl Future<Output = Result<VideoLiveStream, A2aLabError>> + Send {
        std::future::ready(Err(videos_unavailable()))
    }
}

fn videos_unavailable() -> A2aLabError {
    A2aLabError::unavailable("video provider is not configured")
}

/// Routes lab commands to provider traits.
pub struct A2aLabService<L, M, W, I = EmptyImages, V = EmptyVideos> {
    logs: L,
    metrics: M,
    tasks: W,
    images: I,
    videos: V,
    frame_decisions: Option<Arc<dyn FrameDecisionLedgerProvider>>,
    semantic_search: Option<Arc<dyn SemanticVideoSearchProvider>>,
    image_transport: ImageTransportConfig,
    task_store: Mutex<BTreeMap<String, StoredTask>>,
    ids: AtomicU64,
}

impl<L, M, W> A2aLabService<L, M, W, EmptyImages, EmptyVideos>
where
    L: LogProvider,
    M: MetricProvider,
    W: TaskProvider,
{
    /// Creates a service over log, metric, and task providers.
    ///
    /// Image operations stay unavailable until [`Self::with_images`].
    /// Video operations stay unavailable until [`Self::with_videos`].
    #[must_use]
    pub const fn new(logs: L, metrics: M, tasks: W) -> Self {
        Self {
            logs,
            metrics,
            tasks,
            images: EmptyImages,
            videos: EmptyVideos,
            frame_decisions: None,
            semantic_search: None,
            image_transport: ImageTransportConfig::from_default(),
            task_store: Mutex::const_new(BTreeMap::new()),
            ids: AtomicU64::new(0),
        }
    }
}

impl<L, M, W, I, V> A2aLabService<L, M, W, I, V>
where
    L: LogProvider,
    M: MetricProvider,
    W: TaskProvider,
    I: ImageProvider,
    V: VideoProvider,
{
    /// Replaces the image provider without changing the three-provider constructor.
    #[must_use]
    pub fn with_images<P>(self, images: P) -> A2aLabService<L, M, W, P, V>
    where
        P: ImageProvider,
    {
        A2aLabService {
            logs: self.logs,
            metrics: self.metrics,
            tasks: self.tasks,
            images,
            videos: self.videos,
            frame_decisions: self.frame_decisions,
            semantic_search: self.semantic_search,
            image_transport: self.image_transport,
            task_store: self.task_store,
            ids: self.ids,
        }
    }

    /// Replaces the video provider without changing the three-provider constructor.
    #[must_use]
    pub fn with_videos<P>(self, videos: P) -> A2aLabService<L, M, W, I, P>
    where
        P: VideoProvider,
    {
        A2aLabService {
            logs: self.logs,
            metrics: self.metrics,
            tasks: self.tasks,
            images: self.images,
            videos,
            frame_decisions: self.frame_decisions,
            semantic_search: self.semantic_search,
            image_transport: self.image_transport,
            task_store: self.task_store,
            ids: self.ids,
        }
    }

    /// Attaches a Jev decision ledger provider for append/read APIs.
    #[must_use]
    pub fn with_frame_decisions(mut self, provider: Arc<dyn FrameDecisionLedgerProvider>) -> Self {
        self.frame_decisions = Some(provider);
        self
    }

    /// Attaches an optional semantic frame search provider.
    #[must_use]
    pub fn with_semantic_search(mut self, provider: Arc<dyn SemanticVideoSearchProvider>) -> Self {
        self.semantic_search = Some(provider);
        self
    }

    /// Sets the decoded-byte maximum applied before image payloads are returned.
    #[must_use]
    pub fn with_image_transport(mut self, image_transport: ImageTransportConfig) -> Self {
        self.image_transport = image_transport;
        self
    }

    /// Shares the service with protocol adapters.
    #[must_use]
    pub fn share(self) -> Arc<dyn A2aLabApi>
    where
        L: 'static,
        M: 'static,
        W: 'static,
        I: 'static,
        V: 'static,
    {
        Arc::new(self)
    }

    /// Executes a command and stores the resulting task.
    pub async fn execute(&self, command: A2aLabCommand) -> Result<A2aLabOutcome, A2aLabError> {
        let operation = crate::observability::operation_name(&command);
        let task = crate::observability::trace_operation(operation, self.run(command)).await?;
        Ok(A2aLabOutcome { task })
    }

    async fn run(&self, command: A2aLabCommand) -> Result<TaskSnapshot, A2aLabError> {
        match command {
            A2aLabCommand::ListVideoSources(request) => self.list_video_sources(request).await,
            A2aLabCommand::GetVideoFrameAt(request) => self.get_video_frame_at(request).await,
            A2aLabCommand::GetVideoClip(request) => self.get_video_clip(request).await,
            A2aLabCommand::GetVideoTimeline(request) => self.get_video_timeline(request).await,
            A2aLabCommand::FollowVideo(request) => self.follow_video(request).await,
            A2aLabCommand::GetFrameDecisionCapabilities(request) => {
                self.get_frame_decision_capabilities(request).await
            }
            A2aLabCommand::AppendFrameDecision(request) => {
                self.append_frame_decision(request).await
            }
            A2aLabCommand::ReadFrameDecisionHistory(request) => {
                self.read_frame_decision_history(request).await
            }
            A2aLabCommand::GetSemanticSearchCapabilities(_)
            | A2aLabCommand::SearchSemanticFrames(_)
            | A2aLabCommand::GetSemanticIndexingCoverage(_) => {
                self.run_semantic_search(command).await
            }
            other => self.run_lab(other).await,
        }
    }

    async fn run_lab(&self, command: A2aLabCommand) -> Result<TaskSnapshot, A2aLabError> {
        match command {
            A2aLabCommand::ListLogSources(request) => self.list_sources(request).await,
            A2aLabCommand::QueryLogs(request) => self.query_logs(request).await,
            A2aLabCommand::ListMetrics(request) => self.list_metrics(request).await,
            A2aLabCommand::QueryMetric(request) => self.query_metric(request).await,
            A2aLabCommand::ListTasks(request) => self.list_tasks(request).await,
            A2aLabCommand::StartTask(request) => self.start_task(request).await,
            A2aLabCommand::GetTaskStatus(request) => self.task_status(request).await,
            A2aLabCommand::ListImageSources(request) => self.list_image_sources(request).await,
            A2aLabCommand::ListImages(request) => self.list_images(request).await,
            A2aLabCommand::SearchImages(request) => self.search_images(request).await,
            A2aLabCommand::GetImage(request) => self.get_image(request).await,
            A2aLabCommand::GetCurrentImage(request) => self.get_current_image(request).await,
            A2aLabCommand::ListVideoSources(_)
            | A2aLabCommand::GetVideoFrameAt(_)
            | A2aLabCommand::GetVideoClip(_)
            | A2aLabCommand::GetVideoTimeline(_)
            | A2aLabCommand::FollowVideo(_)
            | A2aLabCommand::GetFrameDecisionCapabilities(_)
            | A2aLabCommand::AppendFrameDecision(_)
            | A2aLabCommand::ReadFrameDecisionHistory(_)
            | A2aLabCommand::GetSemanticSearchCapabilities(_)
            | A2aLabCommand::SearchSemanticFrames(_)
            | A2aLabCommand::GetSemanticIndexingCoverage(_) => {
                unreachable!("handled in run")
            }
        }
    }

    /// Cancels a started lab run.
    pub async fn cancel(&self, request: GetTaskStatusRequest) -> Result<TaskSnapshot, A2aLabError> {
        let run = self.tasks.cancel(request).await?;
        Ok(TaskSnapshot {
            id: run.id.as_str().to_owned(),
            context_id: context_id(run.id.as_str()),
            state: run.state,
            result: A2aLabResult::StartTask(run),
        })
    }

    /// Returns a task, refreshing task runs from the provider.
    pub async fn task(&self, task_id: &str) -> Result<TaskSnapshot, A2aLabError> {
        let stored = self
            .task_store
            .lock()
            .await
            .get(task_id)
            .cloned()
            .ok_or_else(|| A2aLabError::not_found("task", task_id))?;
        self.materialize(task_id, stored).await
    }

    async fn list_sources(
        &self,
        request: ListLogSourcesRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        check_page(&request.page)?;
        let page = self.logs.list_sources(request).await?;
        self.store_snapshot(A2aLabResult::ListLogSources(page))
            .await
    }

    async fn query_logs(&self, request: QueryLogsRequest) -> Result<TaskSnapshot, A2aLabError> {
        check_page(&request.page)?;
        request.range.check()?;
        let page = self.logs.query(request).await?;
        self.store_snapshot(A2aLabResult::QueryLogs(page)).await
    }

    async fn list_metrics(&self, request: ListMetricsRequest) -> Result<TaskSnapshot, A2aLabError> {
        check_page(&request.page)?;
        let page = self.metrics.list_metrics(request).await?;
        self.store_snapshot(A2aLabResult::ListMetrics(page)).await
    }

    async fn query_metric(&self, request: QueryMetricRequest) -> Result<TaskSnapshot, A2aLabError> {
        check_page(&request.page)?;
        request.range.check()?;
        let page = self.metrics.query(request).await?;
        self.store_snapshot(A2aLabResult::QueryMetric(page)).await
    }

    async fn list_tasks(&self, request: ListTasksRequest) -> Result<TaskSnapshot, A2aLabError> {
        check_page(&request.page)?;
        let page = self.tasks.list_tasks(request).await?;
        self.store_snapshot(A2aLabResult::ListTasks(page)).await
    }

    async fn start_task(&self, request: StartTaskRequest) -> Result<TaskSnapshot, A2aLabError> {
        let run = crate::tasks::start_run(&self.tasks, request).await?;
        let id = run.id.as_str().to_owned();
        let stored = StoredTask {
            context_id: context_id(&id),
            body: StoredBody::Run(run.id.clone()),
        };
        self.task_store
            .lock()
            .await
            .insert(id.clone(), stored.clone());
        self.materialize(&id, stored).await
    }

    async fn task_status(
        &self,
        request: GetTaskStatusRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        let run = self.tasks.status(request).await?;
        self.store_snapshot(A2aLabResult::GetTaskStatus(run)).await
    }

    async fn list_image_sources(
        &self,
        request: ListImageSourcesRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        check_page(request.page())?;
        let page = self.images.list_image_sources(request).await?;
        self.store_snapshot(A2aLabResult::ListImageSources(page))
            .await
    }

    async fn list_images(&self, request: ListImagesRequest) -> Result<TaskSnapshot, A2aLabError> {
        check_page(request.page())?;
        let page = self.images.list_images(request).await?;
        self.store_snapshot(A2aLabResult::ListImages(page)).await
    }

    async fn search_images(
        &self,
        request: SearchImagesRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        request.check()?;
        let page = self.images.search_images(request).await?;
        self.store_snapshot(A2aLabResult::SearchImages(page)).await
    }

    async fn get_image(&self, request: GetImageRequest) -> Result<TaskSnapshot, A2aLabError> {
        let image = self.accepted_image(self.images.get_image(request).await?)?;
        self.store_snapshot(A2aLabResult::GetImage(image)).await
    }

    async fn get_current_image(
        &self,
        request: GetCurrentImageRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        let image = self.accepted_image(self.images.get_current_image(request).await?)?;
        self.store_snapshot(A2aLabResult::GetCurrentImage(image))
            .await
    }

    async fn list_video_sources(
        &self,
        request: ListVideoSourcesRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        check_page(request.page())?;
        let page = self.videos.list_video_sources(request).await?;
        self.store_snapshot(A2aLabResult::ListVideoSources(page))
            .await
    }

    async fn get_video_frame_at(
        &self,
        request: GetVideoFrameAtRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        let result = self.videos.get_video_frame_at(request).await?;
        self.image_transport.check_payload(result.image().data())?;
        self.store_snapshot(A2aLabResult::GetVideoFrameAt(result))
            .await
    }

    async fn get_video_clip(
        &self,
        request: GetVideoClipRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        request.check()?;
        let clip = self.videos.get_video_clip(request).await?;
        self.store_snapshot(A2aLabResult::GetVideoClip(clip)).await
    }

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

    async fn get_video_timeline(
        &self,
        request: GetVideoTimelineRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        let timeline = self.videos.get_video_timeline(request).await?;
        self.store_snapshot(A2aLabResult::GetVideoTimeline(timeline))
            .await
    }

    async fn follow_video(&self, request: FollowVideoRequest) -> Result<TaskSnapshot, A2aLabError> {
        let follow = self.videos.follow_video(request).await?;
        self.store_snapshot(A2aLabResult::FollowVideo(follow)).await
    }

    fn frame_decisions(&self) -> Result<&Arc<dyn FrameDecisionLedgerProvider>, A2aLabError> {
        self.frame_decisions
            .as_ref()
            .ok_or_else(frame_decision_unavailable)
    }

    async fn get_frame_decision_capabilities(
        &self,
        request: GetFrameDecisionCapabilitiesRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        let capabilities = self
            .frame_decisions()?
            .get_frame_decision_capabilities(request)
            .await?;
        self.store_snapshot(A2aLabResult::GetFrameDecisionCapabilities(capabilities))
            .await
    }

    async fn append_frame_decision(
        &self,
        request: AppendFrameDecisionRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        request.check()?;
        let result = self
            .frame_decisions()?
            .append_frame_decision(request)
            .await?;
        self.store_snapshot(A2aLabResult::AppendFrameDecision(result))
            .await
    }

    async fn read_frame_decision_history(
        &self,
        request: ReadFrameDecisionHistoryRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        request.check()?;
        let result = self
            .frame_decisions()?
            .read_frame_decision_history(request)
            .await?;
        self.store_snapshot(A2aLabResult::ReadFrameDecisionHistory(result))
            .await
    }

    async fn attach_search_assessments(
        &self,
        request: &SearchSemanticFramesRequest,
        result: SemanticFrameSearchResult,
    ) -> Result<SemanticFrameSearchResult, A2aLabError> {
        let Some(ledger) = self.frame_decisions.as_ref() else {
            return Ok(result);
        };
        let max_rows = request.limits().max_assessment_rows_per_hit();
        let mut items = result.hits().items().to_vec();
        for hit in &mut items {
            if hit.authorized_assessments().is_some() {
                continue;
            }
            let assessments =
                authorized_assessments_for_frame(ledger.as_ref(), hit.frame(), max_rows).await?;
            *hit = hit.clone().with_authorized_assessments(Some(assessments))?;
        }
        Ok(SemanticFrameSearchResult::new(
            Page::new(items, result.hits().next_cursor().map(str::to_owned)),
            result.incomplete_indexing(),
            result.candidate_budget_exhausted(),
        ))
    }

    async fn run_semantic_search(
        &self,
        command: A2aLabCommand,
    ) -> Result<TaskSnapshot, A2aLabError> {
        match command {
            A2aLabCommand::GetSemanticSearchCapabilities(_) => {
                let capabilities = match self.semantic_search.as_ref() {
                    Some(provider) => {
                        SemanticSearchCapabilities::registered(provider.active_provenance().ok())
                    }
                    None => SemanticSearchCapabilities::unregistered(),
                };
                self.store_snapshot(A2aLabResult::GetSemanticSearchCapabilities(capabilities))
                    .await
            }
            A2aLabCommand::SearchSemanticFrames(request) => {
                request.check()?;
                let result = self
                    .semantic_search
                    .as_ref()
                    .ok_or_else(semantic_search_unavailable)?
                    .search_semantic_frames(request.clone())
                    .await?;
                let result = self.attach_search_assessments(&request, result).await?;
                self.store_snapshot(A2aLabResult::SearchSemanticFrames(result))
                    .await
            }
            A2aLabCommand::GetSemanticIndexingCoverage(request) => {
                let result = self
                    .semantic_search
                    .as_ref()
                    .ok_or_else(semantic_search_unavailable)?
                    .get_semantic_indexing_coverage(request)
                    .await?;
                self.store_snapshot(A2aLabResult::GetSemanticIndexingCoverage(result))
                    .await
            }
            _ => Err(A2aLabError::protocol("unexpected semantic search command")),
        }
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

    fn accepted_image(&self, image: Image) -> Result<Image, A2aLabError> {
        self.image_transport.check_payload(image.data())?;
        Ok(image)
    }

    async fn store_snapshot(&self, result: A2aLabResult) -> Result<TaskSnapshot, A2aLabError> {
        let id = self.allocate("task");
        let stored = StoredTask {
            context_id: context_id(&id),
            body: StoredBody::Snapshot {
                state: TaskState::Completed,
                result: Box::new(result),
            },
        };
        self.task_store
            .lock()
            .await
            .insert(id.clone(), stored.clone());
        self.materialize(&id, stored).await
    }

    async fn materialize(&self, id: &str, stored: StoredTask) -> Result<TaskSnapshot, A2aLabError> {
        let (state, result) = match stored.body {
            StoredBody::Snapshot { state, result } => (state, *result),
            StoredBody::Run(id) => {
                let run = self.tasks.status(GetTaskStatusRequest { id }).await?;
                (run.state, A2aLabResult::StartTask(run))
            }
        };
        Ok(TaskSnapshot {
            id: id.to_owned(),
            context_id: stored.context_id,
            state,
            result,
        })
    }

    fn allocate(&self, prefix: &str) -> String {
        let number = self.ids.fetch_add(1, Ordering::Relaxed) + 1;
        format!("{prefix}-{number}")
    }
}

impl<L, M, W, I, V> A2aLabApi for A2aLabService<L, M, W, I, V>
where
    L: LogProvider + 'static,
    M: MetricProvider + 'static,
    W: TaskProvider + 'static,
    I: ImageProvider + 'static,
    V: VideoProvider + 'static,
{
    fn execute(
        &self,
        command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabOutcome, A2aLabError>> {
        Box::pin(A2aLabService::execute(self, command))
    }

    fn task<'a>(&'a self, task_id: &str) -> A2aLabFuture<'a, Result<TaskSnapshot, A2aLabError>> {
        let task_id = task_id.to_owned();
        Box::pin(async move { A2aLabService::task(self, &task_id).await })
    }

    fn cancel(
        &self,
        request: GetTaskStatusRequest,
    ) -> A2aLabFuture<'_, Result<TaskSnapshot, A2aLabError>> {
        Box::pin(A2aLabService::cancel(self, request))
    }

    fn open_video_clip(
        &self,
        id: &VideoClipId,
    ) -> A2aLabFuture<'_, Result<VideoClipStream, A2aLabError>> {
        let id = id.clone();
        Box::pin(async move { A2aLabService::open_video_clip(self, &id).await })
    }

    fn export_finite_video_clip(
        &self,
        id: &VideoClipId,
    ) -> A2aLabFuture<'_, Result<PackagedMediaAsset, A2aLabError>> {
        let id = id.clone();
        Box::pin(async move { A2aLabService::export_finite_video_clip(self, &id).await })
    }

    fn open_video_follow(
        &self,
        id: &VideoFollowId,
    ) -> A2aLabFuture<'_, Result<VideoLiveStream, A2aLabError>> {
        let id = id.clone();
        Box::pin(async move { A2aLabService::open_video_follow(self, &id).await })
    }

    fn resolve_clip_playback_asset(
        &self,
        id: &VideoClipId,
        relative_path: Option<&str>,
    ) -> A2aLabFuture<'_, Result<ResolvedMediaAsset, A2aLabError>> {
        let id = id.clone();
        let relative_path = relative_path.map(str::to_owned);
        Box::pin(async move {
            A2aLabService::resolve_clip_playback_asset(self, &id, relative_path.as_deref()).await
        })
    }

    fn resolve_live_playback_asset(
        &self,
        id: &VideoFollowId,
        relative_path: Option<&str>,
    ) -> A2aLabFuture<'_, Result<ResolvedMediaAsset, A2aLabError>> {
        let id = id.clone();
        let relative_path = relative_path.map(str::to_owned);
        Box::pin(async move {
            A2aLabService::resolve_live_playback_asset(self, &id, relative_path.as_deref()).await
        })
    }

    fn rebase_video_follow(
        &self,
        id: &VideoFollowId,
        anchor: UtcTimestamp,
    ) -> A2aLabFuture<'_, Result<VideoFollow, A2aLabError>> {
        let id = id.clone();
        Box::pin(async move { A2aLabService::rebase_video_follow(self, &id, anchor).await })
    }
}

fn check_page(page: &PageRequest) -> Result<(), A2aLabError> {
    page.check()
}

fn context_id(task_id: &str) -> String {
    format!("ctx-{task_id}")
}
