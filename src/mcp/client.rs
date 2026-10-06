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

use crate::error::A2aLabError;
use crate::id::RunId;
use crate::service::{
    A2aLabApi, A2aLabCommand, A2aLabFuture, A2aLabOutcome, A2aLabResult, TaskSnapshot,
};
use crate::tasks::{GetTaskStatusRequest, TaskState};

/// Default Streamable HTTP URL for [`McpServer::serve_http`](super::McpServer::serve_http).
pub const DEFAULT_MCP_URL: &str = "http://127.0.0.1:31001/mcp";

const CONNECT_TRIES: u32 = 50;
const CONNECT_WAIT: Duration = Duration::from_millis(20);

/// `A2aLabApi` that dispatches through an MCP Streamable HTTP server.
pub struct McpLab {
    client: RunningService<rmcp::RoleClient, ClientConfig>,
}

impl McpLab {
    /// Connects to `url`, retrying until the server accepts a session.
    pub async fn connect(url: &str) -> Result<Arc<dyn A2aLabApi>, A2aLabError> {
        let mut last = A2aLabError::transport(format!("mcp not ready at {url}"));
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
    pub async fn connect_default() -> Result<Arc<dyn A2aLabApi>, A2aLabError> {
        Self::connect(DEFAULT_MCP_URL).await
    }

    async fn call<Req, Res>(&self, name: &str, request: Req) -> Result<Res, A2aLabError>
    where
        Req: Serialize,
        Res: DeserializeOwned,
    {
        let value = serde_json::to_value(request)
            .map_err(|error| A2aLabError::protocol(error.to_string()))?;
        let arguments = value
            .as_object()
            .cloned()
            .ok_or_else(|| A2aLabError::protocol("mcp tool arguments must be an object"))?;
        self.client
            .call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(arguments))
            .await
            .map_err(from_service_error)?
            .into_typed()
            .map_err(|error| A2aLabError::protocol(error.to_string()))
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

async fn connect_once(url: &str) -> Result<McpLab, A2aLabError> {
    let transport = StreamableHttpClientTransport::from_uri(url);
    let client = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("a2a-lab", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::V_2026_07_28)
    .serve(transport)
    .await
    .map_err(|error| A2aLabError::transport(error.to_string()))?;
    Ok(McpLab { client })
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
