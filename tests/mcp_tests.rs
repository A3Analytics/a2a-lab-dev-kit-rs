use std::sync::Arc;

use a2a_lab_sdk::{
    A2aClient, A2aServer, JsonObject, LabService, ListLogSourcesRequest, LogSource, McpLab,
    McpServer, MemoryLogs, MemoryMetrics, MemoryTasks, Page, PageRequest, SourceId,
    StartTaskRequest, TaskDefinition, TaskId, bind_local,
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
        asset_id: None,
        semantic_id: None,
    })
    .await;
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("build").unwrap(),
            name: "Build".to_owned(),
            description: "Build the lab".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    LabService::new(logs, MemoryMetrics::new(), tasks).share()
}

async fn assert_lab_tools(client: &impl ToolClient) {
    let listed = client.list().await;
    let mut names: Vec<_> = listed.iter().map(|tool| tool.name.to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "get_task_status",
            "list_log_sources",
            "list_metrics",
            "list_tasks",
            "query_logs",
            "query_metric",
            "start_task",
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
async fn http_transport_accepts_docker_host_and_rejects_other_hosts() {
    let lab = service().await;
    let (listener, address) = bind_local().await.unwrap();
    let port = address.port();
    let server = McpServer::new(&lab);
    tokio::spawn(async move {
        server.serve_http(listener).await.unwrap();
    });
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"probe","version":"0"}}}"#;
    let allowed = post_mcp(port, "host.docker.internal", body).await;
    assert!(allowed.starts_with("HTTP/1.1 200"), "{allowed}");
    let rejected = post_mcp(port, "evil.example", body).await;
    assert!(rejected.contains("403"), "{rejected}");
}

async fn post_mcp(port: u16, host: &str, body: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::time::{Duration, timeout};

    let mut stream = timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(stream) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                return stream;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("mcp server");
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {host}:{port}\r\ncontent-type: application/json\r\naccept: application/json, text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    timeout(Duration::from_secs(2), async {
        let mut buf = [0_u8; 1024];
        let read = stream.read(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf[..read]).into_owned()
    })
    .await
    .expect("mcp response")
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

#[tokio::test]
async fn a2a_calls_mcp_http_tools() {
    let lab = service().await;
    let (mcp_listener, mcp_address) = bind_local().await.unwrap();
    let server = McpServer::new(&lab);
    tokio::spawn(async move {
        server.serve_http(mcp_listener).await.unwrap();
    });
    let mcp_lab = McpLab::connect(&format!("http://{mcp_address}/mcp"))
        .await
        .unwrap();
    let (a2a_listener, a2a_address) = bind_local().await.unwrap();
    tokio::spawn(async move {
        A2aServer::new(&mcp_lab).listen(a2a_listener).await.unwrap();
    });
    let client = A2aClient::new(&format!("http://{a2a_address}")).unwrap();
    let page = client
        .list_log_sources(ListLogSourcesRequest {
            page: PageRequest::new(None, 10).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(page.items()[0].id.as_str(), "app");
    let missing = client
        .start_task(
            StartTaskRequest::new(TaskId::new("missing").unwrap(), JsonObject::empty()).immediate(),
        )
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");
}
