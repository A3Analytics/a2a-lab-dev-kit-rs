//! Lab API that calls MCP tools.

use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation,
    ProtocolVersion,
};
use rmcp::service::{RunningService, ServiceError};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{IntoTransport, StreamableHttpClientTransport};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::A2aLabError;
use crate::id::RunId;
use crate::images::{DEFAULT_MAX_IMAGE_BYTES, Image, ImageTransportConfig};
use crate::service::{
    A2aLabApi, A2aLabCommand, A2aLabFuture, A2aLabOutcome, A2aLabResult, TaskSnapshot,
};
use crate::tasks::{GetTaskStatusRequest, TaskState};

/// Default Streamable HTTP URL for [`McpServer::serve_http`](super::McpServer::serve_http).
pub const DEFAULT_MCP_URL: &str = "http://127.0.0.1:31001/mcp";

const CONNECT_TRIES: u32 = 50;
const CONNECT_WAIT: Duration = Duration::from_millis(20);

/// `A2aLabApi` that dispatches through an MCP server.
pub struct McpLab {
    client: RunningService<rmcp::RoleClient, ClientConfig>,
    image_transport: ImageTransportConfig,
}

impl McpLab {
    /// Adapts a connected MCP session.
    ///
    /// The decoded-image maximum starts at 64 MiB.
    #[must_use]
    pub fn from_session(client: RunningService<rmcp::RoleClient, ClientConfig>) -> Self {
        Self {
            client,
            image_transport: ImageTransportConfig::default(),
        }
    }

    /// Connects to `url`, retrying until the server accepts a session.
    pub async fn connect(url: &str) -> Result<Self, A2aLabError> {
        Self::connect_with(url, ImageTransportConfig::default()).await
    }

    /// Connects to `url` and decodes image results with `image_transport`.
    ///
    /// The HTTP event budget follows this limit, so a raised maximum is in
    /// effect before the first image result is read.
    pub async fn connect_with(
        url: &str,
        image_transport: ImageTransportConfig,
    ) -> Result<Self, A2aLabError> {
        let mut last = A2aLabError::transport(format!("mcp not ready at {url}"));
        for _ in 0..CONNECT_TRIES {
            match connect_once(url, image_transport).await {
                Ok(lab) => return Ok(lab),
                Err(error) => last = error,
            }
            tokio::time::sleep(CONNECT_WAIT).await;
        }
        Err(last)
    }

    /// Connects to [`DEFAULT_MCP_URL`].
    pub async fn connect_default() -> Result<Self, A2aLabError> {
        Self::connect(DEFAULT_MCP_URL).await
    }

    /// Connects over any MCP transport the rmcp client supports, including stdio.
    pub async fn connect_transport<T, E, A>(transport: T) -> Result<Self, A2aLabError>
    where
        T: IntoTransport<rmcp::RoleClient, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let client = client_config()
            .serve(transport)
            .await
            .map_err(|error| A2aLabError::transport(error.to_string()))?;
        Ok(Self::from_session(client))
    }

    /// Sets the decoded-byte maximum for image results.
    ///
    /// The default is 64 MiB, matching [`ImageTransportConfig::default`].
    #[must_use]
    pub fn with_image_transport(mut self, image_transport: ImageTransportConfig) -> Self {
        self.image_transport = image_transport;
        self
    }

    /// Returns the decoded-byte maximum applied when image JSON is decoded.
    #[must_use]
    pub const fn max_image_bytes(&self) -> u64 {
        self.image_transport.max_image_bytes()
    }

    async fn call<Req, Res>(&self, name: &str, request: Req) -> Result<Res, A2aLabError>
    where
        Req: Serialize,
        Res: DeserializeOwned,
    {
        self.invoke(name, request)
            .await?
            .into_typed()
            .map_err(|error| A2aLabError::protocol(error.to_string()))
    }

    async fn call_image<Req>(&self, name: &str, request: Req) -> Result<Image, A2aLabError>
    where
        Req: Serialize,
    {
        // `Image`'s `Deserialize` is fixed at the 64 MiB default. Image results use
        // the caller's limit so a raised maximum applies while the JSON is decoded.
        let value = tool_json(self.invoke(name, request).await?)?;
        decode_image(&value, self.image_transport)
    }

    async fn invoke<Req>(&self, name: &str, request: Req) -> Result<CallToolResult, A2aLabError>
    where
        Req: Serialize,
    {
        let value = serde_json::to_value(request)
            .map_err(|error| A2aLabError::protocol(error.to_string()))?;
        let arguments = value
            .as_object()
            .cloned()
            .ok_or_else(|| A2aLabError::protocol("mcp tool arguments must be an object"))?;
        let result = self
            .client
            .call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(arguments))
            .await
            .map_err(from_service_error)?;
        if result.is_error == Some(true) {
            let message = result
                .content
                .iter()
                .find_map(|block| block.as_text())
                .map_or_else(|| "mcp tool failed".to_owned(), |text| text.text.clone());
            return Err(A2aLabError::invalid("request", message));
        }
        Ok(result)
    }

    async fn execute_command(&self, command: A2aLabCommand) -> Result<A2aLabOutcome, A2aLabError> {
        let result = match command {
            A2aLabCommand::ListLogSources(request) => {
                A2aLabResult::ListLogSources(self.call("list_log_sources", request).await?)
            }
            A2aLabCommand::QueryLogs(request) => {
                A2aLabResult::QueryLogs(self.call("query_logs", request).await?)
            }
            A2aLabCommand::ListMetrics(request) => {
                A2aLabResult::ListMetrics(self.call("list_metrics", request).await?)
            }
            A2aLabCommand::QueryMetric(request) => {
                A2aLabResult::QueryMetric(self.call("query_metric", request).await?)
            }
            A2aLabCommand::ListTasks(request) => {
                A2aLabResult::ListTasks(self.call("list_tasks", request).await?)
            }
            A2aLabCommand::StartTask(request) => {
                A2aLabResult::StartTask(self.call("start_task", request).await?)
            }
            A2aLabCommand::GetTaskStatus(request) => {
                A2aLabResult::GetTaskStatus(self.call("get_task_status", request).await?)
            }
            A2aLabCommand::ListImageSources(request) => {
                A2aLabResult::ListImageSources(self.call("list_image_sources", request).await?)
            }
            A2aLabCommand::ListImages(request) => {
                A2aLabResult::ListImages(self.call("list_images", request).await?)
            }
            A2aLabCommand::SearchImages(request) => {
                A2aLabResult::SearchImages(self.call("search_images", request).await?)
            }
            A2aLabCommand::GetImage(request) => {
                A2aLabResult::GetImage(self.call_image("get_image", request).await?)
            }
            A2aLabCommand::GetCurrentImage(request) => {
                A2aLabResult::GetCurrentImage(self.call_image("get_current_image", request).await?)
            }
        };
        Ok(outcome(result))
    }
}

impl A2aLabApi for McpLab {
    fn execute(
        &self,
        command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabOutcome, A2aLabError>> {
        Box::pin(self.execute_command(command))
    }

    fn task<'a>(&'a self, task_id: &str) -> A2aLabFuture<'a, Result<TaskSnapshot, A2aLabError>> {
        let task_id = task_id.to_owned();
        Box::pin(async move {
            let id = RunId::new(task_id)?;
            Ok(outcome(A2aLabResult::GetTaskStatus(
                self.call("get_task_status", GetTaskStatusRequest { id })
                    .await?,
            ))
            .task)
        })
    }

    fn cancel(
        &self,
        _request: GetTaskStatusRequest,
    ) -> A2aLabFuture<'_, Result<TaskSnapshot, A2aLabError>> {
        Box::pin(async { Err(A2aLabError::unavailable("task is not cancelable")) })
    }
}

async fn connect_once(
    url: &str,
    image_transport: ImageTransportConfig,
) -> Result<McpLab, A2aLabError> {
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url)
            .max_sse_event_size(sse_event_limit(image_transport.max_image_bytes())),
    );
    let mut lab = McpLab::connect_transport(transport).await?;
    lab.image_transport = image_transport;
    Ok(lab)
}

fn sse_event_limit(max_image_bytes: u64) -> usize {
    let allowed = max_image_bytes.max(DEFAULT_MAX_IMAGE_BYTES);
    let encoded = allowed.div_ceil(3).saturating_mul(4);
    // Tool JSON repeats the base64 payload in text content and structured content.
    let budget = encoded.saturating_mul(3).saturating_add(1024 * 1024);
    usize::try_from(budget).unwrap_or(usize::MAX)
}

fn client_config() -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("a2a-lab", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::V_2026_07_28)
}

fn tool_json(result: CallToolResult) -> Result<serde_json::Value, A2aLabError> {
    if let Some(value) = result.structured_content {
        return Ok(value);
    }
    let text = result
        .content
        .first()
        .and_then(|block| block.as_text())
        .map(|text| text.text.as_str())
        .ok_or_else(|| A2aLabError::protocol("mcp tool result is missing json"))?;
    serde_json::from_str(text).map_err(|error| A2aLabError::protocol(error.to_string()))
}

fn decode_image(
    value: &serde_json::Value,
    transport: ImageTransportConfig,
) -> Result<Image, A2aLabError> {
    let descriptor = serde_json::from_value(
        value
            .get("descriptor")
            .cloned()
            .ok_or_else(|| A2aLabError::protocol("image descriptor is missing"))?,
    )
    .map_err(|error| A2aLabError::protocol(error.to_string()))?;
    let encoded = value
        .get("data")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| A2aLabError::protocol("image data is missing"))?;
    Image::from_base64(descriptor, encoded, transport)
}

fn outcome(result: A2aLabResult) -> A2aLabOutcome {
    let (id, state) = match &result {
        A2aLabResult::StartTask(run) | A2aLabResult::GetTaskStatus(run) => {
            (run.id.as_str().to_owned(), run.state)
        }
        _ => ("mcp".to_owned(), TaskState::Completed),
    };
    A2aLabOutcome {
        task: TaskSnapshot {
            context_id: format!("ctx-{id}"),
            id,
            state,
            result,
        },
    }
}

fn from_service_error(error: ServiceError) -> A2aLabError {
    match error {
        ServiceError::McpError(data) => {
            let code = data
                .data
                .as_ref()
                .and_then(|value| value.get("code"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("protocol");
            A2aLabError::from_code(code, data.message.to_string())
        }
        other => A2aLabError::transport(other.to_string()),
    }
}
