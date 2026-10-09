use std::sync::Arc;
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabCommand, A2aLabError, A2aLabResult, A2aLabService, A2aServer,
    GetCurrentImageRequest, GetImageRequest, Image, ImageDescriptor, ImageId, ImageSource,
    ImageSourceId, ImageTransportConfig, JsonObject, LAB_MEDIA_TYPE, ListImageSourcesRequest,
    ListImagesRequest, McpLab, McpServer, MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks,
    Page, PageRequest, SearchImagesRequest, StreamResponse, TimeRange, UtcTimestamp, bind_local,
};
use a2a_types::{error_code, reason_to_error_code};
use base64::Engine;
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation,
    ProtocolVersion,
};
use rmcp::service::{RunningService, ServiceError};
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::{Value, json};

const IMAGE_SKILLS: [&str; 5] = [
    "list-image-sources",
    "list-images",
    "search-images",
    "get-image",
    "get-current-image",
];

const IMAGE_TOOLS: [&str; 5] = [
    "list_image_sources",
    "list_images",
    "search_images",
    "get_image",
    "get_current_image",
];

type McpSession = RunningService<rmcp::RoleClient, ClientConfig>;

struct Fixture {
    images: MemoryImages,
    alpha: Vec<u8>,
    mid: Vec<u8>,
}

struct Endpoints {
    a2a: A2aClient,
    mcp: McpLab,
    a2a_base: String,
    mcp_url: String,
}

enum Op {
    ListSources(ListImageSourcesRequest),
    ListImages(ListImagesRequest),
    Search(SearchImagesRequest),
    Get(GetImageRequest),
    Current(GetCurrentImageRequest),
}

enum Expect {
    Sources(&'static [&'static str], Option<&'static str>),
    Images(&'static [&'static str], Option<&'static str>),
    Frame(&'static str, Sample),
    Code(&'static str),
}

enum Sample {
    Alpha,
    Mid,
}

struct Case {
    name: &'static str,
    op: Op,
    expect: Expect,
}

enum Outcome {
    Sources(Page<ImageSource>),
    Images(Page<ImageDescriptor>),
    Image(Image),
    Error(A2aLabError),
}

struct RawCase {
    name: &'static str,
    tool: &'static str,
    params: Value,
}

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

fn page(cursor: Option<&str>, limit: u32) -> PageRequest {
    PageRequest::new(cursor.map(str::to_owned), limit).unwrap()
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

fn sized_frame(id: &str, source_id: &str, width: u32, height: u32, bytes: Vec<u8>) -> Image {
    Image::new(
        ImageDescriptor::new(
            ImageId::new(id).unwrap(),
            ImageSourceId::new(source_id).unwrap(),
            timestamp("2024-01-01T00:00:00Z"),
            "image/png",
            width,
            height,
            None,
            JsonObject::empty(),
        )
        .unwrap(),
        bytes,
    )
    .unwrap()
}

fn search(
    source_id: Option<&str>,
    start: Option<&str>,
    end: Option<&str>,
    text: Option<&str>,
) -> SearchImagesRequest {
    let range = match (start, end) {
        (Some(start), Some(end)) => Some(TimeRange::new(timestamp(start), timestamp(end)).unwrap()),
        (None, None) => None,
        _ => panic!("a range needs both ends"),
    };
    SearchImagesRequest::new(
        source_id.map(|id| ImageSourceId::new(id).unwrap()),
        range,
        text.map(str::to_owned),
        page(None, 10),
    )
    .unwrap()
}

async fn catalog() -> Fixture {
    let images = MemoryImages::new();
    images.insert_source(source("alpha")).await.unwrap();
    images.insert_source(source("empty")).await.unwrap();
    images.insert_source(source("mid")).await.unwrap();
    let alpha = vec![9, 8, 7, 6];
    let mid = vec![5, 4, 3, 2];
    let other = vec![1, 2, 3, 4];
    for (id, source_id, stamp, caption, payload) in [
        (
            "a",
            "alpha",
            "2024-01-01T00:00:00Z",
            Some("Plate"),
            alpha.clone(),
        ),
        (
            "b",
            "alpha",
            "2024-01-01T00:30:00Z",
            Some("plate well"),
            other.clone(),
        ),
        (
            "c",
            "alpha",
            "2024-01-01T01:00:00Z",
            Some("Well"),
            other.clone(),
        ),
        (
            "m",
            "mid",
            "2024-01-01T00:15:00Z",
            Some("other"),
            mid.clone(),
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
    Fixture { images, alpha, mid }
}

fn lab(images: MemoryImages, transport: ImageTransportConfig) -> Arc<dyn A2aLabApi> {
    A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_images(images)
        .with_image_transport(transport)
        .share()
}

async fn serve_a2a(service: Arc<dyn A2aLabApi>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service);
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

async fn serve_mcp(service: Arc<dyn A2aLabApi>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    let server = McpServer::new(&service);
    tokio::spawn(async move {
        server.serve_http(listener).await.unwrap();
    });
    format!("http://{address}/mcp")
}

async fn open(service: Arc<dyn A2aLabApi>, transport: ImageTransportConfig) -> Endpoints {
    let a2a_base = serve_a2a(Arc::clone(&service)).await;
    let mcp_url = serve_mcp(service).await;
    let a2a = A2aClient::new(&a2a_base)
        .unwrap()
        .with_image_transport(transport);
    let mcp = McpLab::connect_with(&mcp_url, transport).await.unwrap();
    Endpoints {
        a2a,
        mcp,
        a2a_base,
        mcp_url,
    }
}

fn list_sources(
    name: &'static str,
    cursor: Option<&str>,
    ids: &'static [&'static str],
    next: Option<&'static str>,
) -> Case {
    Case {
        name,
        op: Op::ListSources(ListImageSourcesRequest::new(page(cursor, 2)).unwrap()),
        expect: Expect::Sources(ids, next),
    }
}

fn list_images(
    name: &'static str,
    source_id: &str,
    cursor: Option<&str>,
    limit: u32,
    expect: Expect,
) -> Case {
    Case {
        name,
        op: Op::ListImages(
            ListImagesRequest::new(ImageSourceId::new(source_id).unwrap(), page(cursor, limit))
                .unwrap(),
        ),
        expect,
    }
}

fn searched(name: &'static str, request: SearchImagesRequest, expect: Expect) -> Case {
    Case {
        name,
        op: Op::Search(request),
        expect,
    }
}

fn page_cases() -> Vec<Case> {
    vec![
        list_sources(
            "source listing first page",
            None,
            &["alpha", "empty"],
            Some("2"),
        ),
        list_sources("source listing final page", Some("2"), &["mid"], None),
        list_sources("source listing empty page", Some("3"), &[], None),
        list_images(
            "source-scoped image listing first page",
            "alpha",
            None,
            2,
            Expect::Images(&["a", "b"], Some("2")),
        ),
        list_images(
            "source-scoped image listing final page",
            "alpha",
            Some("2"),
            2,
            Expect::Images(&["c"], None),
        ),
        list_images(
            "source-scoped image listing empty page",
            "empty",
            None,
            10,
            Expect::Images(&[], None),
        ),
    ]
}

fn search_cases() -> Vec<Case> {
    vec![
        searched(
            "range-only search",
            search(
                Some("alpha"),
                Some("2024-01-01T00:00:00Z"),
                Some("2024-01-01T01:00:00Z"),
                None,
            ),
            Expect::Images(&["a", "b"], None),
        ),
        searched(
            "half-open timestamp boundaries",
            search(
                None,
                Some("2024-01-01T00:30:00Z"),
                Some("2024-01-01T01:00:00Z"),
                None,
            ),
            Expect::Images(&["b"], None),
        ),
        searched(
            "text-only search",
            search(None, None, None, Some("WELL")),
            Expect::Images(&["b", "c"], None),
        ),
        searched(
            "combined search",
            search(
                Some("alpha"),
                Some("2024-01-01T00:00:00Z"),
                Some("2024-01-01T01:00:00Z"),
                Some("well"),
            ),
            Expect::Images(&["b"], None),
        ),
        searched(
            "search empty page",
            search(None, None, None, Some("absent")),
            Expect::Images(&[], None),
        ),
    ]
}

fn retrieval_cases() -> Vec<Case> {
    vec![
        Case {
            name: "specific retrieval",
            op: Op::Get(GetImageRequest::new(ImageId::new("a").unwrap())),
            expect: Expect::Frame("a", Sample::Alpha),
        },
        Case {
            name: "current retrieval",
            op: Op::Current(GetCurrentImageRequest::new(
                ImageSourceId::new("alpha").unwrap(),
            )),
            expect: Expect::Frame("a", Sample::Alpha),
        },
        Case {
            name: "current retrieval without a selection",
            op: Op::Current(GetCurrentImageRequest::new(
                ImageSourceId::new("mid").unwrap(),
            )),
            expect: Expect::Frame("m", Sample::Mid),
        },
        Case {
            name: "unknown image id",
            op: Op::Get(GetImageRequest::new(ImageId::new("missing").unwrap())),
            expect: Expect::Code("not_found"),
        },
        list_images(
            "unknown source id",
            "missing",
            None,
            10,
            Expect::Code("not_found"),
        ),
        Case {
            name: "unknown source for current retrieval",
            op: Op::Current(GetCurrentImageRequest::new(
                ImageSourceId::new("missing").unwrap(),
            )),
            expect: Expect::Code("not_found"),
        },
        searched(
            "unknown source for search",
            search(Some("missing"), None, None, Some("plate")),
            Expect::Code("not_found"),
        ),
        Case {
            name: "current retrieval for a source with no frames",
            op: Op::Current(GetCurrentImageRequest::new(
                ImageSourceId::new("empty").unwrap(),
            )),
            expect: Expect::Code("not_found"),
        },
    ]
}

fn catalog_cases() -> Vec<Case> {
    let mut cases = page_cases();
    cases.extend(search_cases());
    cases.extend(retrieval_cases());
    cases
}

fn command(op: &Op) -> A2aLabCommand {
    match op {
        Op::ListSources(request) => A2aLabCommand::ListImageSources(request.clone()),
        Op::ListImages(request) => A2aLabCommand::ListImages(request.clone()),
        Op::Search(request) => A2aLabCommand::SearchImages(request.clone()),
        Op::Get(request) => A2aLabCommand::GetImage(request.clone()),
        Op::Current(request) => A2aLabCommand::GetCurrentImage(request.clone()),
    }
}

async fn observe_a2a(client: &A2aClient, op: &Op) -> Outcome {
    let result = match op {
        Op::ListSources(request) => client
            .list_image_sources(request.clone())
            .await
            .map(Outcome::Sources),
        Op::ListImages(request) => client
            .list_images(request.clone())
            .await
            .map(Outcome::Images),
        Op::Search(request) => client
            .search_images(request.clone())
            .await
            .map(Outcome::Images),
        Op::Get(request) => client.get_image(request.clone()).await.map(Outcome::Image),
        Op::Current(request) => client
            .get_current_image(request.clone())
            .await
            .map(Outcome::Image),
    };
    result.unwrap_or_else(Outcome::Error)
}

async fn observe_mcp(lab: &McpLab, op: &Op) -> Outcome {
    match lab.execute(command(op)).await {
        Ok(outcome) => match (op, outcome.task.result) {
            (Op::ListSources(_), A2aLabResult::ListImageSources(page)) => Outcome::Sources(page),
            (Op::ListImages(_), A2aLabResult::ListImages(page))
            | (Op::Search(_), A2aLabResult::SearchImages(page)) => Outcome::Images(page),
            (Op::Get(_), A2aLabResult::GetImage(image))
            | (Op::Current(_), A2aLabResult::GetCurrentImage(image)) => Outcome::Image(image),
            (op, result) => Outcome::Error(A2aLabError::protocol(format!(
                "unexpected {} result for {}",
                result_name(&result),
                op_name(op)
            ))),
        },
        Err(error) => Outcome::Error(error),
    }
}

fn op_name(op: &Op) -> &'static str {
    match op {
        Op::ListSources(_) => "list_image_sources",
        Op::ListImages(_) => "list_images",
        Op::Search(_) => "search_images",
        Op::Get(_) => "get_image",
        Op::Current(_) => "get_current_image",
    }
}

fn result_name(result: &A2aLabResult) -> &'static str {
    match result {
        A2aLabResult::ListImageSources(_) => "list_image_sources",
        A2aLabResult::ListImages(_) => "list_images",
        A2aLabResult::SearchImages(_) => "search_images",
        A2aLabResult::GetImage(_) => "get_image",
        A2aLabResult::GetCurrentImage(_) => "get_current_image",
        A2aLabResult::ListLogSources(_) => "list_log_sources",
        A2aLabResult::QueryLogs(_) => "query_logs",
        A2aLabResult::ListMetrics(_) => "list_metrics",
        A2aLabResult::QueryMetric(_) => "query_metric",
        A2aLabResult::ListTasks(_) => "list_tasks",
        A2aLabResult::StartTask(_) => "start_task",
        A2aLabResult::GetTaskStatus(_) => "get_task_status",
    }
}

fn same(left: &Outcome, right: &Outcome) -> bool {
    match (left, right) {
        (Outcome::Sources(left), Outcome::Sources(right)) => left == right,
        (Outcome::Images(left), Outcome::Images(right)) => left == right,
        (Outcome::Image(left), Outcome::Image(right)) => {
            left.descriptor() == right.descriptor() && left.data() == right.data()
        }
        (Outcome::Error(left), Outcome::Error(right)) => left.code() == right.code(),
        _ => false,
    }
}

fn render(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Sources(page) => format!(
            "sources ids={:?} next={:?}",
            page.items()
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            page.next_cursor()
        ),
        Outcome::Images(page) => format!(
            "images ids={:?} next={:?}",
            page.items()
                .iter()
                .map(|item| item.id().as_str())
                .collect::<Vec<_>>(),
            page.next_cursor()
        ),
        Outcome::Image(image) => format!(
            "image id={} source={} bytes={} prefix={:02x?}",
            image.descriptor().id(),
            image.descriptor().source_id(),
            image.data().len(),
            image.data().iter().take(4).copied().collect::<Vec<_>>()
        ),
        Outcome::Error(error) => format!("error code={} message={error}", error.code()),
    }
}

fn diverge(case: &str, left: &Outcome, right: &Outcome) -> ! {
    let bytes = match (left, right) {
        (Outcome::Image(left), Outcome::Image(right)) if left.data() != right.data() => {
            format!("\n{}", byte_diff(left.data(), right.data()))
        }
        _ => String::new(),
    };
    panic!(
        "case {case}\nA2A: {}\nMCP: {}{bytes}",
        render(left),
        render(right)
    );
}

fn byte_diff(left: &[u8], right: &[u8]) -> String {
    let shared = left.len().min(right.len());
    for index in 0..shared {
        if left[index] != right[index] {
            return format!(
                "first mismatch at {index}: {:02x} vs {:02x}; lengths {} and {}",
                left[index],
                right[index],
                left.len(),
                right.len()
            );
        }
    }
    format!("lengths {} and {}", left.len(), right.len())
}

fn sample_bytes<'a>(sample: &Sample, fixture: &'a Fixture) -> &'a [u8] {
    match sample {
        Sample::Alpha => &fixture.alpha,
        Sample::Mid => &fixture.mid,
    }
}

fn matches_expect(outcome: &Outcome, expect: &Expect, fixture: &Fixture) -> Result<(), String> {
    match (outcome, expect) {
        (Outcome::Sources(page), Expect::Sources(ids, next)) => {
            let got = source_ids(page);
            page_expect(&got, page.next_cursor(), ids, *next)
        }
        (Outcome::Images(page), Expect::Images(ids, next)) => {
            let got = image_ids(page);
            page_expect(&got, page.next_cursor(), ids, *next)
        }
        (Outcome::Image(image), Expect::Frame(id, sample)) => {
            if image.descriptor().id().as_str() != *id {
                return Err(format!("expected image {id}"));
            }
            if image.data() != sample_bytes(sample, fixture) {
                return Err(format!(
                    "expected {} image bytes, got {}",
                    sample_bytes(sample, fixture).len(),
                    image.data().len()
                ));
            }
            Ok(())
        }
        (Outcome::Error(error), Expect::Code(code)) => {
            if error.code() == *code {
                Ok(())
            } else {
                Err(format!("expected code {code}"))
            }
        }
        _ => Err("outcome kind differed from the fixture".to_owned()),
    }
}

fn source_ids(page: &Page<ImageSource>) -> Vec<&str> {
    page.items().iter().map(|item| item.id.as_str()).collect()
}

fn image_ids(page: &Page<ImageDescriptor>) -> Vec<&str> {
    page.items().iter().map(|item| item.id().as_str()).collect()
}

fn page_expect(
    got: &[&str],
    cursor: Option<&str>,
    ids: &[&str],
    next: Option<&str>,
) -> Result<(), String> {
    if got == ids && cursor == next {
        Ok(())
    } else {
        Err(format!("expected ids={ids:?} next={next:?}"))
    }
}

async fn compare(endpoints: &Endpoints, case: &Case, fixture: &Fixture) {
    let a2a = observe_a2a(&endpoints.a2a, &case.op).await;
    let mcp = observe_mcp(&endpoints.mcp, &case.op).await;
    if !same(&a2a, &mcp) {
        diverge(case.name, &a2a, &mcp);
    }
    if let Err(why) = matches_expect(&a2a, &case.expect, fixture) {
        panic!(
            "case {} shared result diverged from the fixture: {why}\nA2A: {}\nMCP: {}",
            case.name,
            render(&a2a),
            render(&mcp)
        );
    }
}

fn sdk_category(code: i32) -> &'static str {
    match code {
        error_code::TASK_NOT_FOUND => "not_found",
        error_code::INVALID_PARAMS
        | error_code::INVALID_REQUEST
        | error_code::PARSE_ERROR
        | error_code::CONTENT_TYPE_NOT_SUPPORTED => "invalid",
        error_code::INTERNAL_ERROR => "unavailable",
        _ => "protocol",
    }
}

fn a2a_category(body: &Value) -> &'static str {
    let Some(reason) = body["error"]["details"].as_array().and_then(|details| {
        details.iter().find_map(|detail| {
            let kind = detail.get("@type").and_then(Value::as_str).unwrap_or("");
            if !kind.ends_with("ErrorInfo") {
                return None;
            }
            if detail.get("domain").and_then(Value::as_str) != Some("a2a-protocol.org") {
                return None;
            }
            detail.get("reason").and_then(Value::as_str)
        })
    }) else {
        return "protocol";
    };
    reason_to_error_code(reason).map_or("protocol", sdk_category)
}

fn mcp_category(error: ServiceError) -> A2aLabError {
    match error {
        ServiceError::McpError(data) => {
            let code = data
                .data
                .as_ref()
                .and_then(|value| value.get("code"))
                .and_then(Value::as_str)
                .unwrap_or("protocol");
            A2aLabError::from_code(code, data.message.to_string())
        }
        other => A2aLabError::transport(other.to_string()),
    }
}

async fn a2a_malformed(base: &str, message_id: &str, operation: &str, params: &Value) -> Outcome {
    let response = reqwest::Client::new()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", "1.0")
        .json(&json!({
            "message": {
                "messageId": message_id,
                "role": "ROLE_USER",
                "parts": [{
                    "mediaType": LAB_MEDIA_TYPE,
                    "data": {
                        "operation": operation,
                        "params": params
                    }
                }]
            }
        }))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    let parsed = serde_json::from_str::<Value>(&body).unwrap_or(Value::String(body.clone()));
    let category = if status.is_success() {
        "protocol"
    } else {
        a2a_category(&parsed)
    };
    Outcome::Error(A2aLabError::from_code(
        category,
        format!("status={status} body={body}"),
    ))
}

fn arguments(value: &Value) -> serde_json::Map<String, Value> {
    value.as_object().expect("object").clone()
}

fn raw_config() -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("a2a-lab-parity", "0.1.0"),
    )
    .with_protocol_version(ProtocolVersion::V_2026_07_28)
}

async fn connect_raw(url: &str) -> McpSession {
    let mut last = String::new();
    for _ in 0..50 {
        match raw_config()
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

async fn mcp_malformed(client: &McpSession, tool: &str, params: &Value) -> Outcome {
    match client
        .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments(params)))
        .await
    {
        Ok(result) if result.is_error == Some(true) => {
            let message = result
                .content
                .first()
                .and_then(|block| block.as_text())
                .map_or_else(|| format!("{result:?}"), |text| text.text.clone());
            Outcome::Error(A2aLabError::invalid("request", message))
        }
        Ok(result) => Outcome::Error(A2aLabError::protocol(format!(
            "unexpected success {}",
            summarize_tool(&result)
        ))),
        Err(error) => Outcome::Error(mcp_category(error)),
    }
}

fn summarize_tool(result: &CallToolResult) -> String {
    let text = result
        .content
        .first()
        .and_then(|block| block.as_text())
        .map(|text| text.text.chars().take(180).collect::<String>());
    format!("is_error={:?} text={text:?}", result.is_error)
}

fn malformed_cases() -> Vec<RawCase> {
    vec![
        RawCase {
            name: "list_image_sources limit",
            tool: "list_image_sources",
            params: json!({"page": {"limit": 0}}),
        },
        RawCase {
            name: "search_images empty",
            tool: "search_images",
            params: json!({"page": {"limit": 10}}),
        },
        RawCase {
            name: "search_images blank",
            tool: "search_images",
            params: json!({"text": " ", "page": {"limit": 10}}),
        },
        RawCase {
            name: "get_current_image field",
            tool: "get_current_image",
            params: json!({"id": "a"}),
        },
    ]
}

async fn compare_malformed(endpoints: &Endpoints) {
    let mcp = connect_raw(&endpoints.mcp_url).await;
    for case in malformed_cases() {
        let a2a = a2a_malformed(&endpoints.a2a_base, case.name, case.tool, &case.params).await;
        let mcp_outcome = mcp_malformed(&mcp, case.tool, &case.params).await;
        if !same(&a2a, &mcp_outcome) {
            diverge(case.name, &a2a, &mcp_outcome);
        }
        match &a2a {
            Outcome::Error(error) if error.code() == "invalid" => {}
            _ => panic!(
                "case {} expected invalid\nA2A: {}\nMCP: {}",
                case.name,
                render(&a2a),
                render(&mcp_outcome)
            ),
        }
    }
}

fn artifact_values(events: &[StreamResponse]) -> Vec<Value> {
    events
        .iter()
        .filter_map(|event| {
            let StreamResponse::ArtifactUpdate(update) = event else {
                return None;
            };
            Some(serde_json::to_value(&update.artifact).unwrap())
        })
        .collect()
}

fn assert_inline_artifact(events: &[StreamResponse], bytes: &[u8]) {
    let artifacts = artifact_values(events);
    assert_eq!(artifacts.len(), 1, "{artifacts:?}");
    let parts = artifacts[0]["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 1);
    let part = &parts[0];
    for key in ["text", "raw", "url", "file"] {
        assert!(part.get(key).is_none(), "{key} part on get-image");
    }
    assert_eq!(part["mediaType"], LAB_MEDIA_TYPE);
    assert_eq!(part["data"]["operation"], "get_image");
    let result = &part["data"]["result"];
    assert!(result.get("uri").is_none());
    assert!(result.get("file").is_none());
    let encoded = result["data"].as_str().expect("inline base64");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    assert_eq!(decoded, bytes);
}

fn assert_json_envelope(result: &CallToolResult, bytes: &[u8]) {
    assert!(
        result.content.iter().all(|block| block.as_text().is_some()),
        "image tools return JSON text"
    );
    assert!(result.content.iter().all(|block| {
        block.as_image().is_none()
            && block.as_resource().is_none()
            && block.as_resource_link().is_none()
    }));
    let structured = result.structured_content.clone().expect("json envelope");
    assert!(structured.get("uri").is_none());
    assert!(structured.get("file").is_none());
    let encoded = structured["data"].as_str().expect("inline base64");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    assert_eq!(decoded, bytes);
    let text = result
        .content
        .first()
        .and_then(|block| block.as_text())
        .map(|text| text.text.as_str())
        .unwrap();
    let envelope: Value = serde_json::from_str(text).unwrap();
    assert_eq!(envelope["data"], encoded);
    assert!(envelope.get("uri").is_none());
}

async fn confirm_framing(endpoints: &Endpoints, bytes: &[u8]) {
    let card = endpoints.a2a.agent_card().await.unwrap();
    let skills: Vec<_> = card.skills.iter().map(|skill| skill.id.as_str()).collect();
    let session = connect_raw(&endpoints.mcp_url).await;
    let listed = session.list_tools(None).await.unwrap().tools;
    let tools: Vec<_> = listed.iter().map(|tool| tool.name.to_string()).collect();
    for (skill, tool) in IMAGE_SKILLS.iter().zip(IMAGE_TOOLS) {
        assert!(skills.contains(skill), "{skill} missing from {skills:?}");
        assert!(tools.iter().any(|name| name == tool), "{tool}");
        assert_ne!(*skill, tool);
        assert_eq!(skill.replace('-', "_"), tool);
        let advertised = card.skills.iter().find(|item| item.id == *skill).unwrap();
        assert_eq!(
            advertised.output_modes.as_deref(),
            Some([LAB_MEDIA_TYPE.to_owned()].as_slice())
        );
    }

    let events = endpoints
        .a2a
        .send_stream(A2aLabCommand::GetImage(GetImageRequest::new(
            ImageId::new("a").unwrap(),
        )))
        .await
        .unwrap();
    assert_inline_artifact(&events, bytes);

    let fetched = session
        .call_tool(
            CallToolRequestParams::new("get_image".to_owned())
                .with_arguments(arguments(&json!({"id": "a"}))),
        )
        .await
        .unwrap();
    assert_json_envelope(&fetched, bytes);
}

async fn compare_limit(endpoints: &Endpoints, name: &str, id: &str, bytes: Option<&[u8]>) {
    let op = Op::Get(GetImageRequest::new(ImageId::new(id).unwrap()));
    let a2a = observe_a2a(&endpoints.a2a, &op).await;
    let mcp = observe_mcp(&endpoints.mcp, &op).await;
    if !same(&a2a, &mcp) {
        diverge(name, &a2a, &mcp);
    }
    match (&a2a, bytes) {
        (Outcome::Image(image), Some(bytes))
            if image.descriptor().id().as_str() == id && image.data() == bytes => {}
        (Outcome::Error(error), None) if error.code() == "invalid" => {}
        _ => panic!(
            "case {name} shared result diverged from the fixture\nA2A: {}\nMCP: {}",
            render(&a2a),
            render(&mcp)
        ),
    }
}

async fn insert_sized(images: &MemoryImages, id: &str, bytes: Vec<u8>, width: u32, height: u32) {
    images
        .insert_image(sized_frame(id, "cam", width, height, bytes))
        .await
        .unwrap();
}

#[tokio::test]
async fn image_parity_matches_a2a_and_mcp() {
    let fixture = catalog().await;
    let endpoints = open(
        lab(fixture.images.clone(), ImageTransportConfig::default()),
        ImageTransportConfig::default(),
    )
    .await;
    for case in catalog_cases() {
        compare(&endpoints, &case, &fixture).await;
    }
    compare_malformed(&endpoints).await;
    confirm_framing(&endpoints, &fixture.alpha).await;

    let offline = catalog().await;
    offline.images.set_unavailable("images offline").await;
    let down = open(
        lab(offline.images, ImageTransportConfig::default()),
        ImageTransportConfig::default(),
    )
    .await;
    compare(
        &down,
        &Case {
            name: "unavailable provider",
            op: Op::ListSources(ListImageSourcesRequest::new(page(None, 10)).unwrap()),
            expect: Expect::Code("unavailable"),
        },
        &fixture,
    )
    .await;

    let missing = open(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share(),
        ImageTransportConfig::default(),
    )
    .await;
    compare(
        &missing,
        &Case {
            name: "unconfigured provider",
            op: Op::Search(search(None, None, None, Some("plate"))),
            expect: Expect::Code("unavailable"),
        },
        &fixture,
    )
    .await;

    let maximum = 8_u64;
    let transport = ImageTransportConfig::new(maximum).unwrap();
    let images = MemoryImages::new();
    images.insert_source(source("cam")).await.unwrap();
    let below = vec![1, 2, 3, 4, 5, 6, 7];
    let at = vec![1, 2, 3, 4, 5, 6, 7, 8];
    let above = vec![1, 2, 3, 4, 5, 6, 7, 8, 9];
    insert_sized(&images, "below", below.clone(), 1, 1).await;
    insert_sized(&images, "at", at.clone(), 1, 1).await;
    insert_sized(&images, "above", above, 1, 1).await;
    let limited = open(lab(images, transport), transport).await;
    compare_limit(
        &limited,
        "payload immediately below maximum",
        "below",
        Some(&below),
    )
    .await;
    compare_limit(&limited, "payload at maximum", "at", Some(&at)).await;
    compare_limit(&limited, "payload immediately above maximum", "above", None).await;

    let mut payload = vec![0_u8; 3840 * 2160 * 4];
    payload[0] = 0x11;
    payload[1] = 0x22;
    let last = payload.len() - 1;
    payload[last] = 0x33;
    let wide = MemoryImages::new();
    wide.insert_source(source("cam")).await.unwrap();
    insert_sized(&wide, "frame", payload.clone(), 3840, 2160).await;
    let four_k = open(
        lab(wide, ImageTransportConfig::default()),
        ImageTransportConfig::default(),
    )
    .await;
    let op = Op::Get(GetImageRequest::new(ImageId::new("frame").unwrap()));
    let a2a = observe_a2a(&four_k.a2a, &op).await;
    let mcp = observe_mcp(&four_k.mcp, &op).await;
    if !same(&a2a, &mcp) {
        diverge("3840x2160 rgba payload", &a2a, &mcp);
    }
    match &a2a {
        Outcome::Image(image)
            if image.descriptor().width() == 3840
                && image.descriptor().height() == 2160
                && image.data() == payload.as_slice() => {}
        _ => panic!(
            "case 3840x2160 rgba payload shared result diverged from the fixture\nA2A: {}\nMCP: {}",
            render(&a2a),
            render(&mcp)
        ),
    }
}
