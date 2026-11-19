//! HTTP, MCP, and A2A coverage for one synchronous generic image decision.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use a2a_client::A2AClient;
use a2a_grpc::GrpcTransport;
use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabCommand, A2aLabError, A2aLabResult, A2aLabService, A2aServer,
    FakeFrameDecisionLedgerProvider, FrameDecisionClientRole, FrameDecisionEvaluatorLimits,
    FrameDecisionEvaluatorProviderConfig, FrameDecisionLedgerProvider,
    GetFrameDecisionCapabilitiesRequest, IMAGE_DECISION_DOWNSTREAM_DEPENDENCY,
    IMAGE_DECISION_EXCLUDED_SCOPE, IMAGE_DECISION_TRANSPORTS, Image, ImageDecisionExecutor,
    ImageDecisionModelAnswer, ImageDecisionModelInput, ImageDecisionProviderError,
    ImageDecisionRequest, ImageDecisionResult, ImageDecisionRoi, ImageDescriptor, ImageId,
    ImageSourceId, JsonObject, McpLab, McpServer, MemoryLogs, MemoryMetrics, MemoryTasks,
    MemoryVideos, Sha256Digest, UtcTimestamp, execute_generic_image_decision,
};
use a2a_types::{Message, Part, Role, SendMessageRequest, SendMessageResponse, error_code};
use async_trait::async_trait;
use rmcp::ServiceExt;
use rmcp::handler::server::common::schema_for_input;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
};
use rmcp::service::ServiceError;
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::{Value, json};

#[derive(Clone)]
enum Behavior {
    Answers(Vec<ImageDecisionModelAnswer>),
    Error(ImageDecisionProviderError),
}

#[derive(Clone)]
struct Scripted {
    behavior: Arc<std::sync::Mutex<Behavior>>,
}

impl Scripted {
    fn answers(answers: Vec<ImageDecisionModelAnswer>) -> Self {
        Self {
            behavior: Arc::new(std::sync::Mutex::new(Behavior::Answers(answers))),
        }
    }

    fn set(&self, behavior: Behavior) {
        *self.behavior.lock().expect("script") = behavior;
    }
}

#[async_trait]
impl ImageDecisionExecutor for Scripted {
    async fn decide(
        &self,
        request: &ImageDecisionRequest,
    ) -> Result<ImageDecisionResult, ImageDecisionProviderError> {
        let behavior = self.behavior.lock().expect("script").clone();
        match behavior {
            Behavior::Answers(answers) => execute_generic_image_decision(request, &answers),
            Behavior::Error(error) => Err(error),
        }
    }
}

struct CountingLedger {
    inner: FakeFrameDecisionLedgerProvider,
    appends: Arc<AtomicUsize>,
}

#[async_trait]
impl FrameDecisionLedgerProvider for CountingLedger {
    fn client_role(&self) -> FrameDecisionClientRole {
        self.inner.client_role()
    }

    async fn get_frame_decision_capabilities(
        &self,
        request: GetFrameDecisionCapabilitiesRequest,
    ) -> Result<a2a_lab_dev_kit::FrameDecisionCapabilities, A2aLabError> {
        self.inner.get_frame_decision_capabilities(request).await
    }

    async fn append_frame_decision(
        &self,
        request: a2a_lab_dev_kit::AppendFrameDecisionRequest,
    ) -> Result<a2a_lab_dev_kit::AppendFrameDecisionResult, A2aLabError> {
        self.appends.fetch_add(1, Ordering::SeqCst);
        self.inner.append_frame_decision(request).await
    }

    async fn read_frame_decision_history(
        &self,
        request: a2a_lab_dev_kit::ReadFrameDecisionHistoryRequest,
    ) -> Result<a2a_lab_dev_kit::FrameDecisionHistoryResult, A2aLabError> {
        self.inner.read_frame_decision_history(request).await
    }
}

fn image() -> Image {
    let descriptor = ImageDescriptor::new(
        ImageId::new("frame").unwrap(),
        ImageSourceId::new("cam").unwrap(),
        UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
        "image/png",
        640,
        480,
        None,
        JsonObject::empty(),
    )
    .unwrap();
    Image::new(descriptor, vec![0xff, 0x00, 0x10, b'a']).unwrap()
}

fn request(roi: Option<ImageDecisionRoi>) -> ImageDecisionRequest {
    ImageDecisionRequest::new(
        image(),
        "Is the tip attached?",
        ImageDecisionModelInput::new("jev-1.14.0", roi).unwrap(),
    )
    .unwrap()
}

fn answer(bounds: [f32; 4]) -> ImageDecisionModelAnswer {
    ImageDecisionModelAnswer::new(
        Some("yes"),
        Some(0.91),
        Some(ImageDecisionRoi::new(bounds).unwrap()),
    )
    .unwrap()
}

fn bits(bounds: [f32; 4]) -> [u32; 4] {
    bounds.map(f32::to_bits)
}

struct Endpoints {
    http: String,
    mcp_url: String,
    a2a: A2aClient,
    mcp: McpLab,
    grpc: A2AClient<GrpcTransport>,
    appends: Arc<AtomicUsize>,
}

async fn open(script: Scripted) -> Endpoints {
    let appends = Arc::new(AtomicUsize::new(0));
    let config = FrameDecisionEvaluatorProviderConfig::pilot(
        Sha256Digest::hash_bytes(b"eval-config"),
        Sha256Digest::hash_bytes(b"threshold"),
    )
    .unwrap();
    let ledger = CountingLedger {
        inner: FakeFrameDecisionLedgerProvider::authorized_producer(
            &config,
            FrameDecisionEvaluatorLimits::pilot(),
        )
        .unwrap(),
        appends: Arc::clone(&appends),
    };
    let lab = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(MemoryVideos::default())
        .with_frame_decisions(Arc::new(ledger))
        .with_image_decision_executor(Arc::new(script))
        .share();
    let a2a_lab = Arc::clone(&lab);
    let (a2a_listener, a2a_address) = a2a_lab_dev_kit::bind_local().await.unwrap();
    tokio::spawn(async move {
        A2aServer::new(&a2a_lab).listen(a2a_listener).await.unwrap();
    });
    let (mcp_listener, mcp_address) = a2a_lab_dev_kit::bind_local().await.unwrap();
    tokio::spawn(async move {
        McpServer::new(&lab).serve_http(mcp_listener).await.unwrap();
    });
    let http = format!("http://{a2a_address}");
    let mcp_url = format!("http://{mcp_address}/mcp");
    let grpc = grpc_client(&http).await;
    Endpoints {
        a2a: A2aClient::new(&http).unwrap(),
        mcp: McpLab::connect(&mcp_url).await.unwrap(),
        grpc,
        http,
        mcp_url,
        appends,
    }
}

async fn grpc_client(base: &str) -> A2AClient<GrpcTransport> {
    let card: Value = reqwest::get(format!("{base}/.well-known/agent-card.json"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let url = card["supportedInterfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|interface| interface["protocolBinding"] == "GRPC")
        .unwrap()["url"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut last = None;
    for _ in 0..20 {
        match GrpcTransport::connect(&url).await {
            Ok(transport) => return A2AClient::new(transport),
            Err(error) => {
                last = Some(error);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
    panic!("gRPC connect failed: {last:?}");
}

fn command_message(command: Value) -> SendMessageRequest {
    SendMessageRequest {
        message: Message::new(
            Role::User,
            vec![Part::data(command).with_media_type(a2a_lab_dev_kit::LAB_MEDIA_TYPE)],
        ),
        configuration: None,
        metadata: None,
        tenant: None,
    }
}

fn find_decision(value: &Value) -> Option<ImageDecisionResult> {
    if value.get("operation").and_then(Value::as_str) == Some("decide_image") {
        return serde_json::from_value(value.get("result")?.clone()).ok();
    }
    match value {
        Value::Array(items) => items.iter().find_map(find_decision),
        Value::Object(map) => map.values().find_map(find_decision),
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => None,
    }
}

fn metadata_code(details: Option<&[a2a_types::TypedDetail]>) -> Option<&str> {
    details?.iter().find_map(|detail| {
        detail
            .value
            .get("metadata")
            .and_then(|metadata| metadata.get("code"))
            .and_then(Value::as_str)
    })
}

fn category_from_protocol(
    code: i32,
    message: &str,
    details: Option<&[a2a_types::TypedDetail]>,
) -> String {
    if let Some(category) = metadata_code(details)
        && (category == "unavailable"
            || a2a_lab_dev_kit::ImageDecisionFailure::parse(category).is_some())
    {
        return category.to_owned();
    }
    for category in ["invalid_answer", "limit_exceeded", "deadline_exceeded"] {
        if message.starts_with(category) {
            return category.to_owned();
        }
    }
    if code == error_code::INVALID_PARAMS
        || code == error_code::INVALID_REQUEST
        || code == error_code::PARSE_ERROR
        || code == 400
    {
        return "invalid".to_owned();
    }
    if code == error_code::INTERNAL_ERROR {
        return "unavailable".to_owned();
    }
    format!("protocol-{code}")
}

async fn jsonrpc(base: &str, command: Value) -> Value {
    reqwest::Client::new()
        .post(format!("{base}/"))
        .header("Content-Type", "application/json")
        .header("A2A-Version", a2a_lab_dev_kit::A2A_PROTOCOL_VERSION)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "SendMessage",
            "params": command_message(command),
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn http_json(base: &str, command: Value) -> Value {
    reqwest::Client::new()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", a2a_lab_dev_kit::A2A_PROTOCOL_VERSION)
        .json(&command_message(command))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

fn observed_result(label: &str, value: &Value) -> ImageDecisionResult {
    find_decision(value).unwrap_or_else(|| panic!("{label} result missing from {value}"))
}

fn metadata_category(value: &Value) -> Option<String> {
    if let Some(code) = value
        .get("metadata")
        .and_then(|metadata| metadata.get("code"))
        .and_then(Value::as_str)
        && (code == "unavailable" || a2a_lab_dev_kit::ImageDecisionFailure::parse(code).is_some())
    {
        return Some(code.to_owned());
    }
    match value {
        Value::Array(items) => items.iter().find_map(metadata_category),
        Value::Object(map) => map.values().find_map(metadata_category),
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => None,
    }
}

fn observed_error(label: &str, value: &Value) -> String {
    if let Some(category) = metadata_category(value) {
        return category;
    }
    let code = value
        .pointer("/error/code")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| panic!("{label} error missing from {value}"));
    category_from_protocol(i32::try_from(code).unwrap_or(i32::MAX), "", None)
}

async fn success_codes(
    endpoints: &Endpoints,
    request: ImageDecisionRequest,
) -> [ImageDecisionResult; 4] {
    let command = serde_json::to_value(A2aLabCommand::DecideImage(request.clone())).unwrap();
    let http = endpoints.a2a.decide_image(request.clone()).await.unwrap();
    let rpc = observed_result("jsonrpc", &jsonrpc(&endpoints.http, command.clone()).await);
    let grpc = match endpoints
        .grpc
        .send_message(&command_message(command))
        .await
        .unwrap()
    {
        SendMessageResponse::Task(task) => {
            observed_result("grpc", &serde_json::to_value(task).unwrap())
        }
        SendMessageResponse::Message(_) => panic!("grpc returned a message"),
    };
    let mcp = match endpoints
        .mcp
        .execute(A2aLabCommand::DecideImage(request))
        .await
        .unwrap()
        .task
        .result
    {
        A2aLabResult::DecideImage(result) => result,
        other => panic!("mcp result {other:?}"),
    };
    [http, rpc, grpc, mcp]
}

fn source_bits(result: &ImageDecisionResult) -> [u32; 4] {
    bits(result.answers()[0].source_box().unwrap().bounds())
}

#[test]
fn decide_image_schema_is_object_rooted_and_shared() {
    let schema = schema_for_input::<Parameters<ImageDecisionRequest>>().expect("input schema");
    assert_eq!(schema.get("type").and_then(Value::as_str), Some("object"));
    let properties = schema.get("properties").and_then(Value::as_object).unwrap();
    assert!(properties.contains_key("image"));
    assert!(properties.contains_key("question"));
    assert!(properties.contains_key("model_input"));
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(required.contains(&"image"));
    assert!(required.contains(&"question"));
    assert!(required.contains(&"model_input"));
    assert!(!required.contains(&"roi"));
    let model = properties
        .get("model_input")
        .and_then(|value| value.get("required"))
        .and_then(Value::as_array);
    if let Some(model_required) = model {
        assert!(model_required.iter().all(|name| name != "roi"));
    }
    let wire = serde_json::to_value(A2aLabCommand::DecideImage(request(None))).unwrap();
    assert_eq!(wire["operation"], "decide_image");
    assert!(wire["params"]["model_input"].get("roi").is_none());
    assert_eq!(
        serde_json::to_value(A2aLabResult::DecideImage(
            execute_generic_image_decision(&request(None), &[answer([0.2, 0.2, 0.4, 0.4])])
                .unwrap()
        ))
        .unwrap()["operation"],
        "decide_image"
    );
}

#[tokio::test]
async fn generic_decision_matches_across_http_mcp_and_a2a() {
    assert_eq!(
        IMAGE_DECISION_DOWNSTREAM_DEPENDENCY,
        "published-git-revision"
    );
    assert_ne!(IMAGE_DECISION_DOWNSTREAM_DEPENDENCY, "path");
    assert_eq!(IMAGE_DECISION_TRANSPORTS, ["http", "mcp", "a2a"]);
    for excluded in [
        "settings-pages",
        "whole-video-indexing",
        "whole-video-backfill",
        "agentcore",
        "strands",
        "aws-sdk",
        "profile-1.3.0",
    ] {
        assert!(IMAGE_DECISION_EXCLUDED_SCOPE.contains(&excluded));
    }

    let script = Scripted::answers(vec![answer([0.2, 0.3, 0.4, 0.5])]);
    let endpoints = open(script.clone()).await;
    let card = endpoints.a2a.agent_card().await.unwrap();
    assert!(card.skills.iter().any(|skill| skill.id == "decide-image"));
    let bindings: Vec<_> = card
        .supported_interfaces
        .iter()
        .map(|interface| interface.protocol_binding.as_str())
        .collect();
    assert_eq!(bindings, ["HTTP+JSON", "JSONRPC", "GRPC"]);

    let full = success_codes(&endpoints, request(None)).await;
    let expected = bits([0.2, 0.3, 0.4, 0.5]);
    for result in &full {
        assert_eq!(source_bits(result), expected);
        assert_eq!(result.answers()[0].text(), Some("yes"));
        assert!(
            result.answers()[0]
                .source_box()
                .unwrap()
                .bounds()
                .iter()
                .all(|value| (0.0..=1.0).contains(value))
        );
    }
    assert_eq!(endpoints.appends.load(Ordering::SeqCst), 0);

    script.set(Behavior::Answers(vec![answer([0.0, 0.0, 1.0, 1.0])]));
    let crop = ImageDecisionRoi::new([0.25, 0.25, 0.75, 0.75]).unwrap();
    let cropped = success_codes(&endpoints, request(Some(crop))).await;
    let cropped_bits = bits([0.25, 0.25, 0.75, 0.75]);
    for result in &cropped {
        assert_eq!(source_bits(result), cropped_bits);
    }
    assert_eq!(endpoints.appends.load(Ordering::SeqCst), 0);

    let caps = endpoints
        .a2a
        .get_frame_decision_capabilities(GetFrameDecisionCapabilitiesRequest::new())
        .await
        .unwrap();
    assert!(caps.is_registered());
    assert_eq!(endpoints.appends.load(Ordering::SeqCst), 0);
    assert_provider_errors(&endpoints, &script).await;
    assert_malformed(&endpoints).await;
    assert_missing_client().await;
}

async fn assert_provider_errors(endpoints: &Endpoints, script: &Scripted) {
    let cases = [
        (
            "unavailable",
            ImageDecisionProviderError::unavailable("jev down").unwrap(),
        ),
        (
            "invalid_answer",
            ImageDecisionProviderError::invalid_answer("response answers are invalid").unwrap(),
        ),
        (
            "limit_exceeded",
            ImageDecisionProviderError::limit_exceeded(
                "image",
                "image exceeds the configured maximum",
            )
            .unwrap(),
        ),
        (
            "deadline_exceeded",
            ImageDecisionProviderError::deadline_exceeded("decision deadline elapsed").unwrap(),
        ),
    ];
    for (category, error) in cases {
        script.set(Behavior::Error(error));
        let command = serde_json::to_value(A2aLabCommand::DecideImage(request(None))).unwrap();
        let http = endpoints
            .a2a
            .decide_image(request(None))
            .await
            .expect_err(category);
        let mcp = endpoints
            .mcp
            .execute(A2aLabCommand::DecideImage(request(None)))
            .await
            .expect_err(category);
        let rpc_body = jsonrpc(&endpoints.http, command.clone()).await;
        let grpc = endpoints
            .grpc
            .send_message(&command_message(command))
            .await
            .expect_err(category);
        let rpc = observed_error(category, &rpc_body);
        let grpc_code = category_from_protocol(grpc.code, &grpc.message, grpc.details.as_deref());
        assert_eq!(http.code(), category, "{http}");
        assert_eq!(mcp.code(), category, "{mcp}");
        assert_eq!(rpc, category, "{rpc_body}");
        assert_eq!(grpc_code, category, "{grpc:?}");
    }
    assert_eq!(endpoints.appends.load(Ordering::SeqCst), 0);
}

async fn assert_malformed(endpoints: &Endpoints) {
    let malformed = {
        let mut command = serde_json::to_value(A2aLabCommand::DecideImage(request(None))).unwrap();
        command["params"]["question"] = json!("   ");
        command
    };
    let http_body = http_json(&endpoints.http, malformed.clone()).await;
    let rpc_body = jsonrpc(&endpoints.http, malformed.clone()).await;
    let grpc = endpoints
        .grpc
        .send_message(&command_message(malformed.clone()))
        .await
        .expect_err("malformed");
    let mcp_code = mcp_raw_error(&endpoints.mcp_url, malformed["params"].clone()).await;
    assert_eq!(observed_error("http", &http_body), "invalid");
    assert_eq!(observed_error("jsonrpc", &rpc_body), "invalid");
    assert_eq!(
        category_from_protocol(grpc.code, &grpc.message, grpc.details.as_deref()),
        "invalid"
    );
    assert_eq!(mcp_code, "invalid");
}

async fn assert_missing_client() {
    let bare = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(MemoryVideos::default())
        .share();
    let error = bare
        .execute(A2aLabCommand::DecideImage(request(None)))
        .await
        .expect_err("missing client");
    assert_eq!(error.code(), "unavailable");
    assert!(error.to_string().contains("not available"));
    let (listener, address) = a2a_lab_dev_kit::bind_local().await.unwrap();
    tokio::spawn(async move {
        A2aServer::new(&bare).listen(listener).await.unwrap();
    });
    let client = A2aClient::new(&format!("http://{address}")).unwrap();
    let missing = client
        .decide_image(request(None))
        .await
        .expect_err("missing client");
    assert_eq!(missing.code(), "unavailable");
}

async fn mcp_raw_error(url: &str, params: Value) -> String {
    let transport = StreamableHttpClientTransport::from_uri(url);
    let client = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("a2a-lab-test", "0.1.0"),
    )
    .with_protocol_version(ProtocolVersion::V_2026_07_28)
    .serve(transport)
    .await
    .unwrap();
    let arguments = params.as_object().cloned().unwrap();
    match client
        .call_tool(CallToolRequestParams::new("decide_image").with_arguments(arguments))
        .await
    {
        Ok(result) if result.is_error == Some(true) => "invalid".to_owned(),
        Err(ServiceError::McpError(data)) => match data
            .data
            .as_ref()
            .and_then(|value| value.get("code"))
            .and_then(Value::as_str)
        {
            Some(code) => code.to_owned(),
            None if data.code == rmcp::model::ErrorCode::INVALID_PARAMS => "invalid".to_owned(),
            None => "protocol".to_owned(),
        },
        other => panic!("malformed tool call: {other:?}"),
    }
}

#[test]
fn typed_ledger_type_stays_distinct_from_generic_result() {
    assert_ne!(
        std::any::TypeId::of::<ImageDecisionResult>(),
        std::any::TypeId::of::<a2a_lab_dev_kit::FrameDecisionRecord>()
    );
}
