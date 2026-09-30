use std::sync::Arc;

use a2a_lab_sdk::{
    LabService, LogSource, McpServer, MemoryLogs, MemoryMetrics, MemoryWorkflows, Page, SourceId,
    WorkflowDefinition, WorkflowId, bind_local,
};
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ServerHandler, ServiceExt};

fn current_client() -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("a2a-lab-test", "0.1.0"),
    )
    .with_protocol_version(ProtocolVersion::V_2026_07_28)
}

fn arguments(value: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().expect("object").clone()
}

async fn service() -> Arc<dyn a2a_lab_sdk::LabApi> {
    let logs = MemoryLogs::new();
    logs.insert_source(LogSource {
        id: SourceId::new("app").unwrap(),
        name: "App".to_owned(),
        description: "Application logs".to_owned(),
    })
    .await;
    let workflows = MemoryWorkflows::new();
    workflows
        .insert(WorkflowDefinition {
            id: WorkflowId::new("build").unwrap(),
            name: "Build".to_owned(),
            description: "Build the lab".to_owned(),
        })
        .await;
    LabService::new(logs, MemoryMetrics::new(), workflows).share()
}

async fn assert_lab_tools(client: &impl ToolClient) {
    let listed = client.list().await;
    let mut names: Vec<_> = listed.iter().map(|tool| tool.name.to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "get_workflow_status",
            "list_log_sources",
            "list_metrics",
            "list_workflows",
            "query_logs",
            "query_metric",
            "start_workflow",
        ]
    );
    let logs = listed
        .iter()
        .find(|tool| tool.name == "query_logs")
        .unwrap();
    let schema = serde_json::Value::Object(logs.input_schema.as_ref().clone());
    assert!(schema.to_string().contains("source_id"), "{schema}");
    assert!(logs.output_schema.is_some());

    let page: Page<LogSource> = client
        .call(
            "list_log_sources",
            serde_json::json!({"page": {"limit": 10}}),
        )
        .await;
    assert_eq!(page.items()[0].id.as_str(), "app");

    let error = client
        .call_error(
            "query_logs",
            serde_json::json!({
                "source_id": "app",
                "range": {"start": "2024-01-01T00:00:00Z", "end": "2024-01-01T00:00:00Z"},
                "page": {"limit": 10}
            }),
        )
        .await;
    assert!(
        error.contains("invalid") || error.contains("start must be before end"),
        "{error}"
    );
}

trait ToolClient {
    async fn list(&self) -> Vec<rmcp::model::Tool>;
    async fn call(&self, name: &str, args: serde_json::Value) -> Page<LogSource>;
    async fn call_error(&self, name: &str, args: serde_json::Value) -> String;
}

impl<S> ToolClient for rmcp::service::RunningService<rmcp::RoleClient, S>
where
    S: rmcp::Service<rmcp::RoleClient>,
{
    async fn list(&self) -> Vec<rmcp::model::Tool> {
        self.list_tools(None).await.expect("tools").tools
    }

    async fn call(&self, name: &str, args: serde_json::Value) -> Page<LogSource> {
        self.call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(arguments(&args)))
            .await
            .expect("call")
            .into_typed()
            .expect("typed")
    }

    async fn call_error(&self, name: &str, args: serde_json::Value) -> String {
        self.call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(arguments(&args)))
            .await
            .expect_err("tool error")
            .to_string()
    }
}

#[tokio::test]
async fn http_transport_lists_and_calls_tools() {
    let lab = service().await;
    let (listener, address) = bind_local().await.unwrap();
    let server = McpServer::new(&lab);
    assert_eq!(
        ServerHandler::get_info(&server).protocol_version,
        ProtocolVersion::V_2026_07_28
    );
    tokio::spawn(async move {
        server.serve_http(listener).await.unwrap();
    });
    let transport = StreamableHttpClientTransport::from_uri(format!("http://{address}/mcp"));
    let client = current_client().serve(transport).await.unwrap();
    assert_lab_tools(&client).await;
}

#[tokio::test]
async fn stdio_transport_lists_and_calls_tools() {
    let lab = service().await;
    let (server_read, client_write) = tokio::io::duplex(64 * 1024);
    let (client_read, server_write) = tokio::io::duplex(64 * 1024);
    let server = McpServer::new(&lab);
    tokio::spawn(async move {
        let running = server.serve((server_read, server_write)).await.unwrap();
        running.waiting().await.unwrap();
    });
    let client = current_client()
        .serve((client_read, client_write))
        .await
        .expect("stdio client");
    assert_lab_tools(&client).await;
}
