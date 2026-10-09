use std::sync::Arc;
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aLabApi, A2aLabCommand, A2aLabResult, A2aLabService, DEFAULT_MAX_IMAGE_BYTES,
    GetCurrentImageRequest, GetImageRequest, Image, ImageDescriptor, ImageId, ImageSource,
    ImageSourceId, ImageTransportConfig, JsonObject, ListImageSourcesRequest, ListImagesRequest,
    McpLab, McpServer, MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks, PageRequest,
    SearchImagesRequest, TimeRange, UtcTimestamp, bind_local,
};
use base64::Engine;
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation,
    ProtocolVersion,
};
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::json;

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

fn page(limit: u32) -> PageRequest {
    PageRequest::new(None, limit).unwrap()
}

fn source(id: &str) -> ImageSource {
    ImageSource {
        id: ImageSourceId::new(id).unwrap(),
        name: id.to_owned(),
        description: format!("{id} frames"),
        asset_id: None,
        semantic_id: None,
    }
}

fn frame(id: &str, source_id: &str, stamp: &str, caption: Option<&str>, bytes: Vec<u8>) -> Image {
    Image::new(
        ImageDescriptor::new(
            ImageId::new(id).unwrap(),
            ImageSourceId::new(source_id).unwrap(),
            timestamp(stamp),
            "image/png",
            1,
            1,
            caption.map(str::to_owned),
            JsonObject::empty(),
        )
        .unwrap(),
        bytes,
    )
    .unwrap()
}

fn service(images: MemoryImages, transport: Option<ImageTransportConfig>) -> Arc<dyn A2aLabApi> {
    let mut built = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_images(images);
    if let Some(transport) = transport {
        built = built.with_image_transport(transport);
    }
    built.share()
}

async fn catalog() -> (MemoryImages, Vec<u8>) {
    let images = MemoryImages::new();
    images.insert_source(source("zeta")).await.unwrap();
    images.insert_source(source("mid")).await.unwrap();
    images.insert_source(source("alpha")).await.unwrap();
    let bytes = vec![9, 8, 7, 6];
    for (id, source_id, stamp, caption, payload) in [
        (
            "c",
            "alpha",
            "2024-01-01T01:00:00Z",
            Some("Well"),
            vec![1, 2, 3, 4],
        ),
        (
            "a",
            "alpha",
            "2024-01-01T00:00:00Z",
            Some("Plate"),
            bytes.clone(),
        ),
        (
            "b",
            "alpha",
            "2024-01-01T00:30:00Z",
            Some("plate well"),
            vec![1, 2, 3, 4],
        ),
        (
            "m",
            "mid",
            "2024-01-01T00:15:00Z",
            Some("other"),
            vec![1, 2, 3, 4],
        ),
        (
            "z",
            "zeta",
            "2024-01-01T03:00:00Z",
            Some("Other"),
            vec![1, 2, 3, 4],
        ),
    ] {
        images
            .insert_image(frame(id, source_id, stamp, caption, payload))
            .await
            .unwrap();
    }
    images
        .set_current(
            ImageSourceId::new("alpha").unwrap(),
            ImageId::new("a").unwrap(),
        )
        .await
        .unwrap();
    (images, bytes)
}

fn ids(descriptors: &[ImageDescriptor]) -> Vec<&str> {
    descriptors
        .iter()
        .map(|descriptor| descriptor.id().as_str())
        .collect()
}

fn raw_client() -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("a2a-lab-test", "0.1.0"),
    )
    .with_protocol_version(ProtocolVersion::V_2026_07_28)
}

fn arguments(value: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().expect("object").clone()
}

async fn call_tool(
    client: &RunningService<rmcp::RoleClient, ClientConfig>,
    name: &str,
    args: serde_json::Value,
) -> CallToolResult {
    client
        .call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(arguments(&args)))
        .await
        .expect(name)
}

fn structured(result: &CallToolResult) -> serde_json::Value {
    result.structured_content.clone().expect("structured json")
}

fn assert_json_envelope(result: &CallToolResult) {
    assert!(
        result.content.iter().all(|block| block.as_text().is_some()),
        "image tools return JSON text, not MCP image or resource blocks"
    );
    assert!(result.content.iter().all(|block| {
        block.as_image().is_none()
            && block.as_resource().is_none()
            && block.as_resource_link().is_none()
    }));
}

async fn serve_http(lab: Arc<dyn A2aLabApi>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    let server = McpServer::new(&lab);
    tokio::spawn(async move {
        server.serve_http(listener).await.unwrap();
    });
    format!("http://{address}/mcp")
}

async fn connect_http(url: &str) -> RunningService<rmcp::RoleClient, ClientConfig> {
    let mut last = String::new();
    for _ in 0..50 {
        match raw_client()
            .serve(StreamableHttpClientTransport::from_uri(url))
            .await
        {
            Ok(client) => return client,
            Err(error) => last = error.to_string(),
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("mcp not ready: {last}");
}

async fn connect_stdio(lab: Arc<dyn A2aLabApi>) -> RunningService<rmcp::RoleClient, ClientConfig> {
    let (server_read, client_write) = tokio::io::duplex(1024 * 1024);
    let (client_read, server_write) = tokio::io::duplex(1024 * 1024);
    let server = McpServer::new(&lab);
    tokio::spawn(async move {
        let running = server.serve((server_read, server_write)).await.unwrap();
        running.waiting().await.unwrap();
    });
    raw_client()
        .serve((client_read, client_write))
        .await
        .expect("stdio client")
}

async fn assert_inline_json(client: &RunningService<rmcp::RoleClient, ClientConfig>, bytes: &[u8]) {
    let listed = call_tool(
        client,
        "list_images",
        json!({"source_id": "alpha", "page": {"limit": 10}}),
    )
    .await;
    assert_json_envelope(&listed);
    let page = structured(&listed);
    assert!(
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| { item.get("data").is_none() && item.get("uri").is_none() })
    );

    let fetched = call_tool(client, "get_image", json!({"id": "a"})).await;
    assert_json_envelope(&fetched);
    let image = structured(&fetched);
    assert!(image.get("uri").is_none());
    assert!(image["descriptor"].get("data").is_none());
    let encoded = image["data"].as_str().expect("base64 data");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    assert_eq!(decoded, bytes);
}

async fn assert_malformed(client: &RunningService<rmcp::RoleClient, ClientConfig>) {
    for (name, args, marker) in [
        (
            "list_image_sources",
            json!({"page": {"limit": 0}}),
            "invalid",
        ),
        ("search_images", json!({"page": {"limit": 10}}), "invalid"),
        (
            "search_images",
            json!({"text": " ", "page": {"limit": 10}}),
            "invalid",
        ),
        ("get_current_image", json!({"id": "a"}), "source_id"),
    ] {
        let result = call_tool(client, name, args).await;
        assert_eq!(result.is_error, Some(true), "{name}");
        let message = result
            .content
            .first()
            .and_then(|block| block.as_text())
            .map(|text| text.text.clone())
            .unwrap_or_default();
        assert!(
            message.to_ascii_lowercase().contains(marker),
            "{name} {message}"
        );
    }
}

async fn exercise(lab: &McpLab, bytes: &[u8]) {
    assert_eq!(lab.max_image_bytes(), DEFAULT_MAX_IMAGE_BYTES);
    page_sources(lab).await;
    page_images(lab).await;
    search(lab).await;
    retrieve(lab, bytes).await;
}

async fn page_sources(lab: &McpLab) {
    let sources = outcome_page(
        lab,
        A2aLabCommand::ListImageSources(ListImageSourcesRequest::new(page(2)).unwrap()),
    )
    .await;
    let A2aLabResult::ListImageSources(sources) = sources else {
        panic!("list_image_sources");
    };
    let source_ids: Vec<_> = sources
        .items()
        .iter()
        .map(|source| source.id.as_str())
        .collect();
    assert_eq!(source_ids, ["alpha", "mid"]);
    let cursor = sources.next_cursor().unwrap().to_owned();
    let rest = outcome_page(
        lab,
        A2aLabCommand::ListImageSources(
            ListImageSourcesRequest::new(PageRequest::new(Some(cursor), 2).unwrap()).unwrap(),
        ),
    )
    .await;
    let A2aLabResult::ListImageSources(rest) = rest else {
        panic!("list_image_sources page");
    };
    assert_eq!(rest.items()[0].id.as_str(), "zeta");
    assert!(rest.next_cursor().is_none());
}

async fn page_images(lab: &McpLab) {
    let listed = outcome_page(
        lab,
        A2aLabCommand::ListImages(
            ListImagesRequest::new(ImageSourceId::new("alpha").unwrap(), page(2)).unwrap(),
        ),
    )
    .await;
    let A2aLabResult::ListImages(listed) = listed else {
        panic!("list_images");
    };
    assert_eq!(ids(listed.items()), ["a", "b"]);
    let image_cursor = listed.next_cursor().unwrap().to_owned();
    let listed_rest = outcome_page(
        lab,
        A2aLabCommand::ListImages(
            ListImagesRequest::new(
                ImageSourceId::new("alpha").unwrap(),
                PageRequest::new(Some(image_cursor), 2).unwrap(),
            )
            .unwrap(),
        ),
    )
    .await;
    let A2aLabResult::ListImages(listed_rest) = listed_rest else {
        panic!("list_images page");
    };
    assert_eq!(ids(listed_rest.items()), ["c"]);
}

async fn search(lab: &McpLab) {
    let by_range = outcome_page(
        lab,
        A2aLabCommand::SearchImages(
            SearchImagesRequest::new(
                Some(ImageSourceId::new("alpha").unwrap()),
                Some(
                    TimeRange::new(
                        timestamp("2024-01-01T00:00:00Z"),
                        timestamp("2024-01-01T01:00:00Z"),
                    )
                    .unwrap(),
                ),
                None,
                page(10),
            )
            .unwrap(),
        ),
    )
    .await;
    let A2aLabResult::SearchImages(by_range) = by_range else {
        panic!("search range");
    };
    assert_eq!(ids(by_range.items()), ["a", "b"]);
    let by_text = outcome_page(
        lab,
        A2aLabCommand::SearchImages(
            SearchImagesRequest::new(None, None, Some("WELL".to_owned()), page(10)).unwrap(),
        ),
    )
    .await;
    let A2aLabResult::SearchImages(by_text) = by_text else {
        panic!("search text");
    };
    assert_eq!(ids(by_text.items()), ["b", "c"]);
}

async fn retrieve(lab: &McpLab, bytes: &[u8]) {
    let image = lab
        .execute(A2aLabCommand::GetImage(GetImageRequest::new(
            ImageId::new("a").unwrap(),
        )))
        .await
        .unwrap();
    let A2aLabResult::GetImage(image) = image.task.result else {
        panic!("get_image");
    };
    assert_eq!(image.data(), bytes);
    assert_eq!(image.descriptor().source_id().as_str(), "alpha");

    let current = lab
        .execute(A2aLabCommand::GetCurrentImage(GetCurrentImageRequest::new(
            ImageSourceId::new("alpha").unwrap(),
        )))
        .await
        .unwrap();
    let A2aLabResult::GetCurrentImage(current) = current.task.result else {
        panic!("get_current_image");
    };
    assert_eq!(current.descriptor().id().as_str(), "a");
    assert_eq!(current.data(), bytes);
    let latest = lab
        .execute(A2aLabCommand::GetCurrentImage(GetCurrentImageRequest::new(
            ImageSourceId::new("zeta").unwrap(),
        )))
        .await
        .unwrap();
    let A2aLabResult::GetCurrentImage(latest) = latest.task.result else {
        panic!("latest current image");
    };
    assert_eq!(latest.descriptor().id().as_str(), "z");
}

async fn outcome_page(lab: &McpLab, command: A2aLabCommand) -> A2aLabResult {
    lab.execute(command).await.unwrap().task.result
}

#[tokio::test]
async fn image_operations_round_trip_on_http_and_stdio() {
    let (images, bytes) = catalog().await;
    let url = serve_http(service(images.clone(), None)).await;
    let http = connect_http(&url).await;
    assert_inline_json(&http, &bytes).await;
    assert_malformed(&http).await;
    exercise(&McpLab::from_session(http), &bytes).await;

    let stdio = connect_stdio(service(images, None)).await;
    assert_inline_json(&stdio, &bytes).await;
    assert_malformed(&stdio).await;
    exercise(&McpLab::from_session(stdio), &bytes).await;
}

#[tokio::test]
async fn image_operations_reject_missing_and_unavailable_input() {
    let (images, _) = catalog().await;
    let http = McpLab::connect(&serve_http(service(images.clone(), None)).await)
        .await
        .unwrap();
    let stdio = McpLab::from_session(connect_stdio(service(images.clone(), None)).await);
    for lab in [&http, &stdio] {
        let missing_image = lab
            .execute(A2aLabCommand::GetImage(GetImageRequest::new(
                ImageId::new("missing").unwrap(),
            )))
            .await
            .unwrap_err();
        assert_eq!(missing_image.code(), "not_found");
        let missing_source = lab
            .execute(A2aLabCommand::ListImages(
                ListImagesRequest::new(ImageSourceId::new("missing").unwrap(), page(10)).unwrap(),
            ))
            .await
            .unwrap_err();
        assert_eq!(missing_source.code(), "not_found");
        let missing_current = lab
            .execute(A2aLabCommand::GetCurrentImage(GetCurrentImageRequest::new(
                ImageSourceId::new("missing").unwrap(),
            )))
            .await
            .unwrap_err();
        assert_eq!(missing_current.code(), "not_found");
    }

    images.set_unavailable("images offline").await;
    let unavailable = McpLab::connect(&serve_http(service(images, None)).await)
        .await
        .unwrap()
        .execute(A2aLabCommand::ListImageSources(
            ListImageSourcesRequest::new(page(10)).unwrap(),
        ))
        .await
        .unwrap_err();
    assert_eq!(unavailable.code(), "unavailable");
    assert!(unavailable.to_string().contains("images offline"));

    let empty = McpLab::connect(
        &serve_http(
            A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share(),
        )
        .await,
    )
    .await
    .unwrap();
    let missing_provider = empty
        .execute(A2aLabCommand::SearchImages(
            SearchImagesRequest::new(None, None, Some("plate".to_owned()), page(10)).unwrap(),
        ))
        .await
        .unwrap_err();
    assert_eq!(missing_provider.code(), "unavailable");
}

#[tokio::test]
async fn four_k_rgba_round_trip_honors_client_and_server_limits() {
    let mut payload = vec![0_u8; 3840 * 2160 * 4];
    payload[0] = 0x11;
    payload[1] = 0x22;
    let last = payload.len() - 1;
    payload[last] = 0x33;
    let decoded_len = u64::try_from(payload.len()).unwrap();
    let images = MemoryImages::new();
    images.insert_source(source("cam")).await.unwrap();
    images
        .insert_image(
            Image::new(
                ImageDescriptor::new(
                    ImageId::new("frame").unwrap(),
                    ImageSourceId::new("cam").unwrap(),
                    timestamp("2024-01-01T00:00:00Z"),
                    "image/png",
                    3840,
                    2160,
                    None,
                    JsonObject::empty(),
                )
                .unwrap(),
                payload.clone(),
            )
            .unwrap(),
        )
        .await
        .unwrap();

    let url = serve_http(service(images.clone(), None)).await;
    let image = McpLab::connect(&url)
        .await
        .unwrap()
        .execute(A2aLabCommand::GetImage(GetImageRequest::new(
            ImageId::new("frame").unwrap(),
        )))
        .await
        .unwrap();
    let A2aLabResult::GetImage(image) = image.task.result else {
        panic!("get_image");
    };
    assert!(
        image.data() == payload.as_slice(),
        "default limit dropped image bytes"
    );

    let below = ImageTransportConfig::new(decoded_len - 1).unwrap();
    let client_limited = McpLab::connect(&url)
        .await
        .unwrap()
        .with_image_transport(below)
        .execute(A2aLabCommand::GetImage(GetImageRequest::new(
            ImageId::new("frame").unwrap(),
        )))
        .await
        .unwrap_err();
    assert_eq!(client_limited.code(), "invalid");
    assert!(client_limited.to_string().contains("maximum"));

    let tight = serve_http(service(images.clone(), Some(below))).await;
    let server_limited = McpLab::connect(&tight)
        .await
        .unwrap()
        .execute(A2aLabCommand::GetImage(GetImageRequest::new(
            ImageId::new("frame").unwrap(),
        )))
        .await
        .unwrap_err();
    assert_eq!(server_limited.code(), "invalid");
    assert!(server_limited.to_string().contains("maximum"));

    let raised = ImageTransportConfig::new(decoded_len).unwrap();
    let raised_url = serve_http(service(images, Some(raised))).await;
    let raised_image = McpLab::connect_with(&raised_url, raised)
        .await
        .unwrap()
        .execute(A2aLabCommand::GetImage(GetImageRequest::new(
            ImageId::new("frame").unwrap(),
        )))
        .await
        .unwrap();
    let A2aLabResult::GetImage(raised_image) = raised_image.task.result else {
        panic!("raised get_image");
    };
    assert!(
        raised_image.data() == payload.as_slice(),
        "raised limit dropped image bytes"
    );
}
