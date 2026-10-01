//! Lab API that calls the seven MCP tools.

use std::sync::Arc;
use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
};
use rmcp::service::{RunningService, ServiceError};
use rmcp::transport::StreamableHttpClientTransport;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::SdkError;
use crate::id::RunId;
use crate::service::{LabApi, LabCommand, LabFuture, LabOutcome, LabResult, TaskSnapshot};
use crate::tasks::{GetTaskStatusRequest, TaskState};

/// Default Streamable HTTP URL for [`McpServer::serve_http`](super::McpServer::serve_http).
pub const DEFAULT_MCP_URL: &str = "http://127.0.0.1:31001/mcp";

const CONNECT_TRIES: u32 = 50;
const CONNECT_WAIT: Duration = Duration::from_millis(20);

/// `LabApi` that dispatches through an MCP Streamable HTTP server.
pub struct McpLab {
    client: RunningService<rmcp::RoleClient, ClientConfig>,
}

impl McpLab {
    /// Connects to `url`, retrying until the server accepts a session.
    pub async fn connect(url: &str) -> Result<Arc<dyn LabApi>, SdkError> {
        let mut last = SdkError::transport(format!("mcp not ready at {url}"));
        for _ in 0..CONNECT_TRIES {
            match connect_once(url).await {
                Ok(lab) => return Ok(Arc::new(lab)),
                Err(error) => last = error,
            }
            tokio::time::sleep(CONNECT_WAIT).await;
        }
        Err(last)
    }

    /// Connects to [`DEFAULT_MCP_URL`].
    pub async fn connect_default() -> Result<Arc<dyn LabApi>, SdkError> {
        Self::connect(DEFAULT_MCP_URL).await
    }

    async fn call<Req, Res>(&self, name: &str, request: Req) -> Result<Res, SdkError>
    where
        Req: Serialize,
        Res: DeserializeOwned,
    {
        let value =
            serde_json::to_value(request).map_err(|error| SdkError::protocol(error.to_string()))?;
        let arguments = value
            .as_object()
            .cloned()
            .ok_or_else(|| SdkError::protocol("mcp tool arguments must be an object"))?;
        self.client
            .call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(arguments))
            .await
            .map_err(from_service_error)?
            .into_typed()
            .map_err(|error| SdkError::protocol(error.to_string()))
    }

    async fn execute_command(&self, command: LabCommand) -> Result<LabOutcome, SdkError> {
        let result = match command {
            LabCommand::ListLogSources(request) => {
                LabResult::ListLogSources(self.call("list_log_sources", request).await?)
            }
            LabCommand::QueryLogs(request) => {
                LabResult::QueryLogs(self.call("query_logs", request).await?)
            }
            LabCommand::ListMetrics(request) => {
                LabResult::ListMetrics(self.call("list_metrics", request).await?)
            }
            LabCommand::QueryMetric(request) => {
                LabResult::QueryMetric(self.call("query_metric", request).await?)
            }
            LabCommand::ListTasks(request) => {
                LabResult::ListTasks(self.call("list_tasks", request).await?)
            }
            LabCommand::StartTask(request) => {
                LabResult::StartTask(self.call("start_task", request).await?)
            }
            LabCommand::GetTaskStatus(request) => {
                LabResult::GetTaskStatus(self.call("get_task_status", request).await?)
            }
        };
        Ok(outcome(result))
    }
}

impl LabApi for McpLab {
    fn execute(&self, command: LabCommand) -> LabFuture<'_, Result<LabOutcome, SdkError>> {
        Box::pin(self.execute_command(command))
    }

    fn task<'a>(&'a self, task_id: &str) -> LabFuture<'a, Result<TaskSnapshot, SdkError>> {
        let task_id = task_id.to_owned();
        Box::pin(async move {
            let id = RunId::new(task_id)?;
            Ok(outcome(LabResult::GetTaskStatus(
                self.call("get_task_status", GetTaskStatusRequest { id })
                    .await?,
            ))
            .task)
        })
    }

    fn cancel(
        &self,
        _request: GetTaskStatusRequest,
    ) -> LabFuture<'_, Result<TaskSnapshot, SdkError>> {
        Box::pin(async { Err(SdkError::unavailable("task is not cancelable")) })
    }
}

async fn connect_once(url: &str) -> Result<McpLab, SdkError> {
    let transport = StreamableHttpClientTransport::from_uri(url);
    let client = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("a2a-lab", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::V_2026_07_28)
    .serve(transport)
    .await
    .map_err(|error| SdkError::transport(error.to_string()))?;
    Ok(McpLab { client })
}

fn outcome(result: LabResult) -> LabOutcome {
    let (id, state) = match &result {
        LabResult::StartTask(run) | LabResult::GetTaskStatus(run) => {
            (run.id.as_str().to_owned(), run.state)
        }
        _ => ("mcp".to_owned(), TaskState::Completed),
    };
    LabOutcome {
        task: TaskSnapshot {
            context_id: format!("ctx-{id}"),
            id,
            state,
            result,
        },
    }
}

fn from_service_error(error: ServiceError) -> SdkError {
    match error {
        ServiceError::McpError(data) => {
            let code = data
                .data
                .as_ref()
                .and_then(|value| value.get("code"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("protocol");
            SdkError::from_code(code, data.message.to_string())
        }
        other => SdkError::transport(other.to_string()),
    }
}
