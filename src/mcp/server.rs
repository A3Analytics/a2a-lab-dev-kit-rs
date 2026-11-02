//! MCP tools for lab logs, metrics, tasks, images, and videos.

use std::collections::HashMap;
use std::sync::Arc;

use a2a_types::{SecurityRequirement, SecurityScheme};
use axum::Router;
use axum::extract::Request;
use axum::middleware::{Next, from_fn};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ProtocolVersion, ServerCapabilities, ServerConfig};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, Json, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use tokio::net::TcpListener;

const DEFAULT_ADDRESS: &str = "127.0.0.1:31001";

use crate::a2a::{AccessGate, Authenticator, auth_response, security_configuration_error};
use crate::error::A2aLabError;
use crate::images::{
    GetCurrentImageRequest, GetImageRequest, Image, ImageDescriptor, ImageSource,
    ListImageSourcesRequest, ListImagesRequest, SearchImagesRequest,
};
use crate::logs::{ListLogSourcesRequest, LogRecord, LogSource, QueryLogsRequest};
use crate::metrics::{ListMetricsRequest, MetricDescriptor, MetricPoint, QueryMetricRequest};
use crate::page::Page;
use crate::service::{A2aLabApi, A2aLabCommand, A2aLabResult};
use crate::tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskRun,
};
use crate::videos::{
    FollowVideoRequest, GetVideoClipRequest, GetVideoFrameAtRequest, GetVideoFrameAtResult,
    GetVideoTimelineRequest, ListVideoSourcesRequest, SharedMediaAssetProvider, VideoClip,
    VideoFollow, VideoSource, VideoTimeline,
};

/// MCP server exposing the lab operations as tools.
#[derive(Clone)]
pub struct McpServer {
    lab: Arc<dyn A2aLabApi>,
    security_schemes: Option<HashMap<String, SecurityScheme>>,
    security_requirements: Option<Vec<SecurityRequirement>>,
    authenticator: Option<Arc<dyn Authenticator>>,
    media_assets: Option<SharedMediaAssetProvider>,
    tool_router: ToolRouter<Self>,
}

impl McpServer {
    /// Creates a server backed by `lab`.
    ///
    /// The server stores its own handle to the same service.
    #[must_use]
    pub fn new(lab: &Arc<dyn A2aLabApi>) -> Self {
        Self {
            lab: Arc::clone(lab),
            security_schemes: None,
            security_requirements: None,
            authenticator: None,
            media_assets: None,
            tool_router: Self::tool_router(),
        }
    }

    /// Declares the same HTTP bearer requirements as [`crate::A2aServer::with_security`].
    ///
    /// When requirements are non-empty, every `/media/video-*` gateway route on this
    /// listener uses the access gate. `/mcp` stays on the host allowlist only.
    #[must_use]
    pub fn with_security(
        mut self,
        schemes: HashMap<String, SecurityScheme>,
        requirements: Vec<SecurityRequirement>,
    ) -> Self {
        self.security_schemes = Some(schemes);
        self.security_requirements = Some(requirements);
        self
    }

    /// Validates bearer tokens for OAuth2, OpenID Connect, and HTTP bearer schemes.
    #[must_use]
    pub fn with_authenticator(mut self, authenticator: Arc<dyn Authenticator>) -> Self {
        self.authenticator = Some(authenticator);
        self
    }

    /// Uses `assets` for `/media/video-*` gateway routes instead of a default local provider.
    #[must_use]
    pub fn with_media_assets(mut self, assets: SharedMediaAssetProvider) -> Self {
        self.media_assets = Some(assets);
        self
    }

    /// Serves newline-delimited JSON-RPC on standard input and output.
    ///
    /// Diagnostics must stay on stderr so stdout remains protocol-clean.
    pub async fn serve_stdio(self) -> Result<(), A2aLabError> {
        let running = self
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|error| A2aLabError::transport(error.to_string()))?;
        running
            .waiting()
            .await
            .map(|_| ())
            .map_err(|error| A2aLabError::transport(error.to_string()))
    }

    /// Serves MCP Streamable HTTP at `/mcp`.
    ///
    /// `listener` defaults to `127.0.0.1:31001` when it is `None`.
    /// A loopback listener accepts `Host` values of loopback plus
    /// `host.docker.internal`. A listener on every interface accepts any `Host`.
    pub async fn serve_http(
        self,
        listener: impl Into<Option<TcpListener>>,
    ) -> Result<(), A2aLabError> {
        security_configuration_error(
            self.security_schemes.as_ref().unwrap_or(&HashMap::new()),
            self.security_requirements.as_deref().unwrap_or(&[]),
            self.authenticator.is_some(),
        )
        .map_err(|message| A2aLabError::invalid("security", message))?;
        let listener = match listener.into() {
            Some(listener) => listener,
            None => TcpListener::bind(DEFAULT_ADDRESS)
                .await
                .map_err(|error| A2aLabError::transport(error.to_string()))?,
        };
        let mut config = StreamableHttpServerConfig::default().with_allowed_hosts([
            "localhost",
            "127.0.0.1",
            "::1",
            "host.docker.internal",
        ]);
        if listener
            .local_addr()
            .is_ok_and(|address| address.ip().is_unspecified())
        {
            config = config.disable_allowed_hosts();
        }
        let lab = Arc::clone(&self.lab);
        let gate = AccessGate::new(
            self.security_schemes.clone(),
            self.security_requirements.clone(),
            self.authenticator.clone(),
        );
        let media = self.media_assets.clone().unwrap_or_else(|| {
            Arc::new(crate::videos::LocalMediaAssetProvider::new(Arc::clone(
                &lab,
            )))
        });
        let service = StreamableHttpService::new(
            move || Ok(self.clone()),
            Arc::new(LocalSessionManager::default()),
            config,
        );
        let router = Router::new()
            .nest_service("/mcp", service)
            .merge(crate::videos::http::router(media));
        let app = if gate.enforced() {
            let http_gate = gate;
            router.layer(from_fn(move |request: Request, next: Next| {
                let gate = http_gate.clone();
                async move {
                    if !crate::videos::http::is_gated_media_path(request.uri().path()) {
                        return next.run(request).await;
                    }
                    match gate
                        .authenticate(request.headers(), request.uri().query())
                        .await
                    {
                        Ok(_caller) => next.run(request).await,
                        Err(error) => {
                            let mut response = auth_response(&error);
                            if crate::videos::http::is_gated_media_path(request.uri().path()) {
                                crate::videos::http::apply_media_security_headers(&mut response);
                            }
                            response
                        }
                    }
                }
            }))
        } else {
            router
        };
        axum::serve(listener, app)
            .await
            .map_err(|error| A2aLabError::transport(error.to_string()))
    }

    async fn take<T>(
        &self,
        command: A2aLabCommand,
        pick: impl FnOnce(A2aLabResult) -> Option<T>,
    ) -> Result<Json<T>, ErrorData> {
        let outcome = self
            .lab
            .execute(command)
            .await
            .map_err(|error| mcp_error(&error))?;
        pick(outcome.task.result)
            .map(Json)
            .ok_or_else(|| ErrorData::internal_error("unexpected result variant", None))
    }
}

#[tool_router(router = tool_router)]
impl McpServer {
    #[tool(
        name = "list_log_sources",
        description = "List the log sources this agent can read"
    )]
    async fn list_log_sources(
        &self,
        params: Parameters<ListLogSourcesRequest>,
    ) -> Result<Json<Page<LogSource>>, ErrorData> {
        self.take(
            A2aLabCommand::ListLogSources(params.0),
            |result| match result {
                A2aLabResult::ListLogSources(page) => Some(page),
                _ => None,
            },
        )
        .await
    }

    #[tool(
        name = "query_logs",
        description = "Read structured logs from a source over a UTC time range"
    )]
    async fn query_logs(
        &self,
        params: Parameters<QueryLogsRequest>,
    ) -> Result<Json<Page<LogRecord>>, ErrorData> {
        self.take(A2aLabCommand::QueryLogs(params.0), |result| match result {
            A2aLabResult::QueryLogs(page) => Some(page),
            _ => None,
        })
        .await
    }

    #[tool(
        name = "list_metrics",
        description = "List the metrics this agent can read"
    )]
    async fn list_metrics(
        &self,
        params: Parameters<ListMetricsRequest>,
    ) -> Result<Json<Page<MetricDescriptor>>, ErrorData> {
        self.take(
            A2aLabCommand::ListMetrics(params.0),
            |result| match result {
                A2aLabResult::ListMetrics(page) => Some(page),
                _ => None,
            },
        )
        .await
    }

    #[tool(
        name = "query_metric",
        description = "Read metric samples over a UTC time range"
    )]
    async fn query_metric(
        &self,
        params: Parameters<QueryMetricRequest>,
    ) -> Result<Json<Page<MetricPoint>>, ErrorData> {
        self.take(
            A2aLabCommand::QueryMetric(params.0),
            |result| match result {
                A2aLabResult::QueryMetric(page) => Some(page),
                _ => None,
            },
        )
        .await
    }

    #[tool(
        name = "list_tasks",
        description = "List the tasks this agent can start"
    )]
    async fn list_tasks(
        &self,
        params: Parameters<ListTasksRequest>,
    ) -> Result<Json<Page<TaskDefinition>>, ErrorData> {
        self.take(A2aLabCommand::ListTasks(params.0), |result| match result {
            A2aLabResult::ListTasks(page) => Some(page),
            _ => None,
        })
        .await
    }

    #[tool(
        name = "start_task",
        description = "Start a task with a JSON object input. Waits until the run is terminal unless wait is false."
    )]
    async fn start_task(
        &self,
        params: Parameters<StartTaskRequest>,
    ) -> Result<Json<TaskRun>, ErrorData> {
        self.take(A2aLabCommand::StartTask(params.0), |result| match result {
            A2aLabResult::StartTask(run) => Some(run),
            _ => None,
        })
        .await
    }

    #[tool(
        name = "get_task_status",
        description = "Read the status of an A2A task"
    )]
    async fn get_task_status(
        &self,
        params: Parameters<GetTaskStatusRequest>,
    ) -> Result<Json<TaskRun>, ErrorData> {
        self.take(
            A2aLabCommand::GetTaskStatus(params.0),
            |result| match result {
                A2aLabResult::GetTaskStatus(run) => Some(run),
                _ => None,
            },
        )
        .await
    }

    #[tool(
        name = "list_image_sources",
        description = "List the image sources this agent can read"
    )]
    async fn list_image_sources(
        &self,
        params: Parameters<ListImageSourcesRequest>,
    ) -> Result<Json<Page<ImageSource>>, ErrorData> {
        self.take(
            A2aLabCommand::ListImageSources(params.0),
            |result| match result {
                A2aLabResult::ListImageSources(page) => Some(page),
                _ => None,
            },
        )
        .await
    }

    #[tool(
        name = "list_images",
        description = "List image metadata from one source"
    )]
    async fn list_images(
        &self,
        params: Parameters<ListImagesRequest>,
    ) -> Result<Json<Page<ImageDescriptor>>, ErrorData> {
        self.take(A2aLabCommand::ListImages(params.0), |result| match result {
            A2aLabResult::ListImages(page) => Some(page),
            _ => None,
        })
        .await
    }

    #[tool(
        name = "search_images",
        description = "Search image metadata by time range or text"
    )]
    async fn search_images(
        &self,
        params: Parameters<SearchImagesRequest>,
    ) -> Result<Json<Page<ImageDescriptor>>, ErrorData> {
        self.take(
            A2aLabCommand::SearchImages(params.0),
            |result| match result {
                A2aLabResult::SearchImages(page) => Some(page),
                _ => None,
            },
        )
        .await
    }

    #[tool(
        name = "get_image",
        description = "Read one image, including its inline bytes"
    )]
    async fn get_image(
        &self,
        params: Parameters<GetImageRequest>,
    ) -> Result<Json<Image>, ErrorData> {
        self.take(A2aLabCommand::GetImage(params.0), |result| match result {
            A2aLabResult::GetImage(image) => Some(image),
            _ => None,
        })
        .await
    }

    #[tool(
        name = "get_current_image",
        description = "Read the current image for one source, including its inline bytes"
    )]
    async fn get_current_image(
        &self,
        params: Parameters<GetCurrentImageRequest>,
    ) -> Result<Json<Image>, ErrorData> {
        self.take(
            A2aLabCommand::GetCurrentImage(params.0),
            |result| match result {
                A2aLabResult::GetCurrentImage(image) => Some(image),
                _ => None,
            },
        )
        .await
    }

    #[tool(
        name = "list_video_sources",
        description = "List the video sources this agent can read"
    )]
    async fn list_video_sources(
        &self,
        params: Parameters<ListVideoSourcesRequest>,
    ) -> Result<Json<Page<VideoSource>>, ErrorData> {
        self.take(
            A2aLabCommand::ListVideoSources(params.0),
            |result| match result {
                A2aLabResult::ListVideoSources(page) => Some(page),
                _ => None,
            },
        )
        .await
    }

    #[tool(
        name = "get_video_frame_at",
        description = "Read the retained frame at a UTC instant"
    )]
    async fn get_video_frame_at(
        &self,
        params: Parameters<GetVideoFrameAtRequest>,
    ) -> Result<Json<GetVideoFrameAtResult>, ErrorData> {
        self.take(
            A2aLabCommand::GetVideoFrameAt(params.0),
            |result| match result {
                A2aLabResult::GetVideoFrameAt(frame) => Some(frame),
                _ => None,
            },
        )
        .await
    }

    #[tool(
        name = "get_video_clip",
        description = "Prepare a bounded MJPEG clip handle. Bytes are not embedded."
    )]
    async fn get_video_clip(
        &self,
        params: Parameters<GetVideoClipRequest>,
    ) -> Result<Json<VideoClip>, ErrorData> {
        self.take(
            A2aLabCommand::GetVideoClip(params.0),
            |result| match result {
                A2aLabResult::GetVideoClip(clip) => Some(clip),
                _ => None,
            },
        )
        .await
    }

    #[tool(
        name = "get_video_timeline",
        description = "Read ordered retained coverage for one source. Bytes are not embedded."
    )]
    async fn get_video_timeline(
        &self,
        params: Parameters<GetVideoTimelineRequest>,
    ) -> Result<Json<VideoTimeline>, ErrorData> {
        self.take(
            A2aLabCommand::GetVideoTimeline(params.0),
            |result| match result {
                A2aLabResult::GetVideoTimeline(timeline) => Some(timeline),
                _ => None,
            },
        )
        .await
    }

    #[tool(
        name = "follow_video",
        description = "Prepare a live MJPEG follow handle. Bytes are not embedded."
    )]
    async fn follow_video(
        &self,
        params: Parameters<FollowVideoRequest>,
    ) -> Result<Json<VideoFollow>, ErrorData> {
        self.take(
            A2aLabCommand::FollowVideo(params.0),
            |result| match result {
                A2aLabResult::FollowVideo(follow) => Some(follow),
                _ => None,
            },
        )
        .await
    }
}

#[allow(clippy::unused_async_trait_impl)]
#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2026_07_28)
            .with_server_info(Implementation::new("a2a-lab", env!("CARGO_PKG_VERSION")))
            .with_instructions("Lab logs, metrics, tasks, images, and videos")
    }
}

fn mcp_error(error: &A2aLabError) -> ErrorData {
    let message = error.to_string();
    let data = Some(serde_json::json!({ "code": error.code() }));
    match error {
        A2aLabError::Unavailable { .. }
        | A2aLabError::Transport { .. }
        | A2aLabError::Video {
            availability: crate::error::VideoAvailability::LiveUnavailable,
            ..
        } => ErrorData::internal_error(message, data),
        A2aLabError::Invalid { .. }
        | A2aLabError::Protocol { .. }
        | A2aLabError::NotFound { .. }
        | A2aLabError::Video { .. } => ErrorData::invalid_params(message, data),
    }
}
