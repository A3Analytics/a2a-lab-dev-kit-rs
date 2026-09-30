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

use crate::error::SdkError;
use crate::logs::{ListLogSourcesRequest, LogRecord, LogSource, QueryLogsRequest};
use crate::metrics::{ListMetricsRequest, MetricDescriptor, MetricPoint, QueryMetricRequest};
use crate::page::Page;
use crate::service::{LabApi, LabCommand, LabResult};
use crate::workflows::{
    GetWorkflowStatusRequest, ListWorkflowsRequest, StartWorkflowRequest, WorkflowDefinition,
    WorkflowRun,
};

/// MCP server exposing the lab operations as tools.
#[derive(Clone)]
pub struct McpServer {
    lab: Arc<dyn LabApi>,
    tool_router: ToolRouter<Self>,
}

impl McpServer {
    /// Creates a server backed by `lab`.
    ///
    /// The server stores its own handle to the same service.
    #[must_use]
    pub fn new(lab: &Arc<dyn LabApi>) -> Self {
        Self {
            lab: Arc::clone(lab),
            tool_router: Self::tool_router(),
        }
    }

    /// Serves newline-delimited JSON-RPC on standard input and output.
    ///
    /// Diagnostics must stay on stderr so stdout remains protocol-clean.
    pub async fn serve_stdio(self) -> Result<(), SdkError> {
        let running = self
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|error| SdkError::transport(error.to_string()))?;
        running
            .waiting()
            .await
            .map(|_| ())
            .map_err(|error| SdkError::transport(error.to_string()))
    }

    /// Serves MCP Streamable HTTP at `/mcp`.
    ///
    /// `listener` defaults to `127.0.0.1:31001` when it is `None`.
    pub async fn serve_http(
        self,
        listener: impl Into<Option<TcpListener>>,
    ) -> Result<(), SdkError> {
        let listener = match listener.into() {
            Some(listener) => listener,
            None => TcpListener::bind(DEFAULT_ADDRESS)
                .await
                .map_err(|error| SdkError::transport(error.to_string()))?,
        };
        let service = StreamableHttpService::new(
            move || Ok(self.clone()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        let router = axum::Router::new().nest_service("/mcp", service);
        axum::serve(listener, router)
            .await
            .map_err(|error| SdkError::transport(error.to_string()))
    }

    async fn take<T>(
        &self,
        command: LabCommand,
        pick: impl FnOnce(LabResult) -> Option<T>,
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
            LabCommand::ListLogSources(params.0),
            |result| match result {
                LabResult::ListLogSources(page) => Some(page),
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
        self.take(LabCommand::QueryLogs(params.0), |result| match result {
            LabResult::QueryLogs(page) => Some(page),
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
        self.take(LabCommand::ListMetrics(params.0), |result| match result {
            LabResult::ListMetrics(page) => Some(page),
            _ => None,
        })
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
        self.take(LabCommand::QueryMetric(params.0), |result| match result {
            LabResult::QueryMetric(page) => Some(page),
            _ => None,
        })
        .await
    }

    #[tool(
        name = "list_workflows",
        description = "List the workflows this agent can start"
    )]
    async fn list_workflows(
        &self,
        params: Parameters<ListWorkflowsRequest>,
    ) -> Result<Json<Page<WorkflowDefinition>>, ErrorData> {
        self.take(LabCommand::ListWorkflows(params.0), |result| match result {
            LabResult::ListWorkflows(page) => Some(page),
            _ => None,
        })
        .await
    }

    #[tool(
        name = "start_workflow",
        description = "Start a workflow with a JSON object input and return the run"
    )]
    async fn start_workflow(
        &self,
        params: Parameters<StartWorkflowRequest>,
    ) -> Result<Json<WorkflowRun>, ErrorData> {
        self.take(LabCommand::StartWorkflow(params.0), |result| match result {
            LabResult::StartWorkflow(run) => Some(run),
            _ => None,
        })
        .await
    }

    #[tool(
        name = "get_workflow_status",
        description = "Read the status of a workflow run"
    )]
    async fn get_workflow_status(
        &self,
        params: Parameters<GetWorkflowStatusRequest>,
    ) -> Result<Json<WorkflowRun>, ErrorData> {
        self.take(
            LabCommand::GetWorkflowStatus(params.0),
            |result| match result {
                LabResult::GetWorkflowStatus(run) => Some(run),
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
            .with_instructions("Lab logs, metrics, and workflows")
    }
}

fn mcp_error(error: &SdkError) -> ErrorData {
    let message = error.to_string();
    match error {
        SdkError::Invalid { .. } | SdkError::Protocol { .. } | SdkError::NotFound { .. } => {
            ErrorData::invalid_params(message, None)
        }
        SdkError::Unavailable { .. } | SdkError::Transport { .. } => {
            ErrorData::internal_error(message, None)
        }
    }
}
