//! MCP tools for the seven lab operations.

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ProtocolVersion, ServerCapabilities, ServerConfig};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, Json, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use tokio::net::TcpListener;

const DEFAULT_ADDRESS: &str = "127.0.0.1:31001";

use crate::error::A2aLabError;
use crate::logs::{ListLogSourcesRequest, LogRecord, LogSource, QueryLogsRequest};
use crate::metrics::{ListMetricsRequest, MetricDescriptor, MetricPoint, QueryMetricRequest};
use crate::page::Page;
use crate::service::{A2aLabApi, A2aLabCommand, A2aLabResult};
use crate::tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskRun,
};

/// MCP server exposing the lab operations as tools.
#[derive(Clone)]
pub struct McpServer {
    lab: Arc<dyn A2aLabApi>,
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
            tool_router: Self::tool_router(),
        }
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
    /// Accepted `Host` values are loopback plus `host.docker.internal`, so a
    /// Dockerized MCP inspector on Docker Desktop or Rancher Desktop can connect.
    pub async fn serve_http(
        self,
        listener: impl Into<Option<TcpListener>>,
    ) -> Result<(), A2aLabError> {
        let listener = match listener.into() {
            Some(listener) => listener,
            None => TcpListener::bind(DEFAULT_ADDRESS)
                .await
                .map_err(|error| A2aLabError::transport(error.to_string()))?,
        };
        let service = StreamableHttpService::new(
            move || Ok(self.clone()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default().with_allowed_hosts([
                "localhost",
                "127.0.0.1",
                "::1",
                "host.docker.internal",
            ]),
        );
        let router = axum::Router::new().nest_service("/mcp", service);
        axum::serve(listener, router)
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
}

#[allow(clippy::unused_async_trait_impl)]
#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2026_07_28)
            .with_server_info(Implementation::new("a2a-lab", env!("CARGO_PKG_VERSION")))
            .with_instructions("Lab logs, metrics, and tasks")
    }
}

fn mcp_error(error: &A2aLabError) -> ErrorData {
    let message = error.to_string();
    let data = Some(serde_json::json!({ "code": error.code() }));
    match error {
        A2aLabError::Invalid { .. }
        | A2aLabError::Protocol { .. }
        | A2aLabError::NotFound { .. } => ErrorData::invalid_params(message, data),
        A2aLabError::Unavailable { .. } | A2aLabError::Transport { .. } => {
            ErrorData::internal_error(message, data)
        }
    }
}
