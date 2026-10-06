//! One lab service on MCP Streamable HTTP, with A2A calling that MCP server.
//!
//! Both listeners are ephemeral. The process makes one client call on each
//! protocol, then stops the servers and exits.

use std::error::Error;
use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabError, A2aLabService, A2aServer, ListLogSourcesRequest, LogSource,
    McpLab, McpServer, MemoryLogs, MemoryMetrics, MemoryTasks, Page, PageRequest, SourceId,
    bind_local,
};
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
};
use rmcp::transport::StreamableHttpClientTransport;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let logs = MemoryLogs::new();
    logs.insert_source(LogSource {
        id: SourceId::new("app")?,
        name: "App".to_owned(),
        description: "Application logs".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
    let service = A2aLabService::new(logs, MemoryMetrics::new(), MemoryTasks::new()).share();

    let (a2a_listener, a2a_address) = bind_local().await?;
    let (mcp_listener, mcp_address) = bind_local().await?;
    let mcp = spawn_mcp(Arc::clone(&service), mcp_listener);
    let mcp_lab = McpLab::connect(&format!("http://{mcp_address}/mcp")).await?;
    let a2a = spawn_a2a(mcp_lab, a2a_listener);

    let client = A2aClient::new(&format!("http://{a2a_address}"))?;
    let sources = client
        .list_log_sources(ListLogSourcesRequest {
            page: PageRequest::new(None, 10)?,
        })
        .await?;
    println!("a2a list_log_sources: {}", sources.items()[0].id);

    let transport = StreamableHttpClientTransport::from_uri(format!("http://{mcp_address}/mcp"));
    let mcp_client = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("a2a-lab-example", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::V_2026_07_28)
    .serve(transport)
    .await?;
    let arguments = serde_json::json!({"page": {"limit": 10}})
        .as_object()
        .cloned()
        .ok_or("tool arguments must be an object")?;
    let page: Page<LogSource> = mcp_client
        .call_tool(CallToolRequestParams::new("list_log_sources").with_arguments(arguments))
        .await?
        .into_typed()?;
    println!("mcp list_log_sources: {}", page.items()[0].id);
    mcp_client.cancel().await?;

    a2a.abort();
    mcp.abort();
    Ok(())
}

fn spawn_a2a(
    service: Arc<dyn A2aLabApi>,
    listener: TcpListener,
) -> JoinHandle<Result<(), A2aLabError>> {
    tokio::spawn(async move { A2aServer::new(&service).listen(listener).await })
}

fn spawn_mcp(
    service: Arc<dyn A2aLabApi>,
    listener: TcpListener,
) -> JoinHandle<Result<(), A2aLabError>> {
    tokio::spawn(async move { McpServer::new(&service).serve_http(listener).await })
}
