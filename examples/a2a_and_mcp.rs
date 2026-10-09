//! One lab service on MCP Streamable HTTP, with A2A calling that MCP server.
//!
//! Both listeners are ephemeral. The process lists log and image sources and
//! reads the current and a specific inline image on each protocol, then stops
//! the servers and exits.

use std::error::Error;
use std::fmt::Display;
use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabError, A2aLabService, A2aServer, GetCurrentImageRequest,
    GetImageRequest, Image, ImageDescriptor, ImageId, ImageSource, ImageSourceId, JsonObject,
    ListImageSourcesRequest, ListLogSourcesRequest, LogSource, McpLab, McpServer, MemoryImages,
    MemoryLogs, MemoryMetrics, MemoryTasks, Page, PageRequest, SourceId, UtcTimestamp, bind_local,
};
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
};
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpClientTransport;
use serde::de::DeserializeOwned;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let service = A2aLabService::new(app_logs().await?, MemoryMetrics::new(), MemoryTasks::new())
        .with_images(bench_images().await?)
        .share();

    let (a2a_listener, a2a_address) = bind_local().await?;
    let (mcp_listener, mcp_address) = bind_local().await?;
    let mcp = spawn_mcp(Arc::clone(&service), mcp_listener);
    let mcp_lab: Arc<dyn A2aLabApi> =
        Arc::new(McpLab::connect(&format!("http://{mcp_address}/mcp")).await?);
    let a2a = spawn_a2a(mcp_lab, a2a_listener);

    demo_a2a(&a2a_address).await?;
    demo_mcp(&mcp_address).await?;
    a2a.abort();
    mcp.abort();
    Ok(())
}

async fn app_logs() -> Result<MemoryLogs, A2aLabError> {
    let logs = MemoryLogs::new();
    logs.insert_source(LogSource {
        id: SourceId::new("app")?,
        name: "App".to_owned(),
        description: "Application logs".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
    Ok(logs)
}

async fn bench_images() -> Result<MemoryImages, A2aLabError> {
    let images = MemoryImages::new();
    images.insert_source(bench_source()?).await?;
    images
        .insert_image(frame("earlier", "2024-01-01T00:00:00Z", vec![1, 2, 3])?)
        .await?;
    images
        .insert_image(frame("current", "2024-01-01T01:00:00Z", vec![4, 5, 6, 7])?)
        .await?;
    images
        .set_current(ImageSourceId::new("bench")?, ImageId::new("current")?)
        .await?;
    Ok(images)
}

async fn demo_a2a(address: impl Display) -> Result<(), Box<dyn Error + Send + Sync>> {
    let client = A2aClient::new(&format!("http://{address}"))?;
    let sources = client
        .list_log_sources(ListLogSourcesRequest {
            page: PageRequest::new(None, 10)?,
        })
        .await?;
    println!("a2a list_log_sources: {}", sources.items()[0].id);
    let image_sources = client
        .list_image_sources(ListImageSourcesRequest::new(PageRequest::new(None, 10)?)?)
        .await?;
    println!("a2a list_image_sources: {}", image_sources.items()[0].id);
    let current = client
        .get_current_image(GetCurrentImageRequest::new(ImageSourceId::new("bench")?))
        .await?;
    print_image("a2a", "get_current_image", &current);
    let specific = client
        .get_image(GetImageRequest::new(ImageId::new("earlier")?))
        .await?;
    print_image("a2a", "get_image", &specific);
    Ok(())
}

async fn demo_mcp(address: impl Display) -> Result<(), Box<dyn Error + Send + Sync>> {
    let transport = StreamableHttpClientTransport::from_uri(format!("http://{address}/mcp"));
    let client = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("a2a-lab-example", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::V_2026_07_28)
    .serve(transport)
    .await?;
    let page: Page<LogSource> = mcp_call(
        &client,
        "list_log_sources",
        serde_json::json!({"page": {"limit": 10}}),
    )
    .await?;
    println!("mcp list_log_sources: {}", page.items()[0].id);
    let image_page: Page<ImageSource> = mcp_call(
        &client,
        "list_image_sources",
        serde_json::json!({"page": {"limit": 10}}),
    )
    .await?;
    println!("mcp list_image_sources: {}", image_page.items()[0].id);
    let current: Image = mcp_call(
        &client,
        "get_current_image",
        serde_json::json!({"source_id": "bench"}),
    )
    .await?;
    print_image("mcp", "get_current_image", &current);
    let specific: Image =
        mcp_call(&client, "get_image", serde_json::json!({"id": "earlier"})).await?;
    print_image("mcp", "get_image", &specific);
    client.cancel().await?;
    Ok(())
}

fn print_image(protocol: &str, operation: &str, image: &Image) {
    println!(
        "{protocol} {operation}: {} {}",
        image.descriptor().id(),
        image.data().len()
    );
}

fn bench_source() -> Result<ImageSource, A2aLabError> {
    Ok(ImageSource {
        id: ImageSourceId::new("bench")?,
        name: "Bench".to_owned(),
        description: "Bench camera".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
}

fn frame(id: &str, stamp: &str, bytes: Vec<u8>) -> Result<Image, A2aLabError> {
    Image::new(
        ImageDescriptor::new(
            ImageId::new(id)?,
            ImageSourceId::new("bench")?,
            UtcTimestamp::parse(stamp)?,
            "image/png",
            1,
            1,
            None,
            JsonObject::empty(),
        )?,
        bytes,
    )
}

async fn mcp_call<T: DeserializeOwned>(
    client: &RunningService<rmcp::RoleClient, ClientConfig>,
    name: &str,
    arguments: serde_json::Value,
) -> Result<T, Box<dyn Error + Send + Sync>> {
    let arguments = arguments
        .as_object()
        .cloned()
        .ok_or("tool arguments must be an object")?;
    Ok(client
        .call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(arguments))
        .await?
        .into_typed()?)
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
