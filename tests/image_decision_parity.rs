//! Parity for one synchronous generic image decision across HTTP, MCP, and A2A.
//!
//! Downstream repositories consume the devkit from a published git revision.
//! Settings pages, whole-video indexing, backfill, deployment runtimes, cloud
//! SDKs, and profile 1.3.0 stay outside this suite.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use a2a_client::A2AClient;
use a2a_grpc::GrpcTransport;
use a2a_lab_dev_kit::{
    A2aClient, A2aLabCommand, A2aLabError, A2aLabService, A2aServer,
    FakeFrameDecisionLedgerProvider, FrameDecisionClientRole, FrameDecisionEvaluatorLimits,
    FrameDecisionEvaluatorProviderConfig, FrameDecisionLedgerProvider,
    GetFrameDecisionCapabilitiesRequest, IMAGE_DECISION_DOWNSTREAM_DEPENDENCY,
    IMAGE_DECISION_EXCLUDED_SCOPE, IMAGE_DECISION_TRANSPORTS, Image, ImageDecisionExecutor,
    ImageDecisionModelAnswer, ImageDecisionModelInput, ImageDecisionProviderError,
    ImageDecisionRequest, ImageDecisionResult, ImageDecisionRoi, ImageDescriptor, ImageId,
    ImageSourceId, JsonObject, McpServer, MemoryLogs, MemoryMetrics, MemoryTasks, MemoryVideos,
    Sha256Digest, UtcTimestamp, execute_generic_image_decision,
};
use a2a_types::{Message, Part, Role, SendMessageRequest, SendMessageResponse, error_code};
use async_trait::async_trait;
use rmcp::ServiceExt;
use rmcp::handler::server::common::{schema_for_input, schema_for_output};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
};
use rmcp::service::{RunningService, ServiceError};
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::{Value, json};

type McpSession = RunningService<rmcp::RoleClient, ClientConfig>;

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

#[derive(Debug)]
enum Observed {
    Decision(ImageDecisionResult),
    Error(String),
}

struct Endpoints {
    http: String,
    a2a: A2aClient,
    grpc: A2AClient<GrpcTransport>,
    mcp: McpSession,
    appends: Arc<AtomicUsize>,
}

async fn open(script: Option<Scripted>) -> Endpoints {
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
    let mut service =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_videos(MemoryVideos::default())
            .with_frame_decisions(Arc::new(ledger));
    if let Some(script) = script {
        service = service.with_image_decision_executor(Arc::new(script));
    }
    let lab = service.share();
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
    Endpoints {
        a2a: A2aClient::new(&http).unwrap(),
        grpc: grpc_client(&http).await,
        mcp: mcp_session(&mcp_url).await,
        http,
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

async fn mcp_session(url: &str) -> McpSession {
    let mut last = None;
    for _ in 0..20 {
        let transport = StreamableHttpClientTransport::from_uri(url);
        match client_config().serve(transport).await {
            Ok(session) => return session,
            Err(error) => {
                last = Some(error);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
    panic!("mcp connect failed: {last:?}");
}

fn client_config() -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("a2a-lab-parity", "0.1.0"),
    )
    .with_protocol_version(ProtocolVersion::V_2026_07_28)
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

fn command_value(request: &ImageDecisionRequest) -> Value {
    serde_json::to_value(A2aLabCommand::DecideImage(request.clone())).unwrap()
}

fn request_shape(request: &ImageDecisionRequest, omits_roi: bool) -> Value {
    let value = serde_json::to_value(request).unwrap();
    assert!(value.get("image").is_some() && value.get("question").is_some());
    assert!(value.get("model_input").is_some());
    assert_eq!(value["model_input"].get("roi").is_none(), omits_roi);
    let command = command_value(request);
    assert_eq!(command["operation"], "decide_image");
    assert_eq!(command["params"], value);
    value
}

fn result_shape(result: &ImageDecisionResult) -> Value {
    let value = serde_json::to_value(result).unwrap();
    assert!(value.get("answers").and_then(Value::as_array).is_some());
    let answer = &value["answers"][0];
    assert_eq!(answer["text"], "yes");
    assert!(answer.get("probability").is_some());
    assert!(answer["source_box"].get("bounds").is_some());
    assert!(answer.get("model_box").is_none());
    value
}

fn wrapped_result(value: &Value) -> Option<Value> {
    if value.get("operation").and_then(Value::as_str) == Some("decide_image")
        && let Some(result) = value.get("result")
    {
        return Some(result.clone());
    }
    match value {
        Value::Array(items) => items.iter().find_map(wrapped_result),
        Value::Object(map) => map.values().find_map(wrapped_result),
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
    if code == error_code::INTERNAL_ERROR || code == 500 || code == 503 {
        return "unavailable".to_owned();
    }
    format!("protocol-{code}")
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

fn error_category(value: &Value) -> String {
    if let Some(category) = metadata_category(value) {
        return category;
    }
    if let Some(code) = value.get("code").and_then(Value::as_str)
        && (code == "invalid"
            || code == "unavailable"
            || a2a_lab_dev_kit::ImageDecisionFailure::parse(code).is_some())
    {
        return code.to_owned();
    }
    let Some(code) = value.pointer("/error/code").and_then(Value::as_i64) else {
        return format!("unparsed:{value}");
    };
    let message = value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("");
    category_from_protocol(i32::try_from(code).unwrap_or(i32::MAX), message, None)
}

fn observe_body(value: &Value) -> Observed {
    if let Some(result) = wrapped_result(value) {
        return match serde_json::from_value(result.clone()) {
            Ok(decision) => Observed::Decision(decision),
            Err(error) => Observed::Error(format!("unparsed-result: {error}; {result}")),
        };
    }
    if value.get("answers").is_some() {
        return match serde_json::from_value(value.clone()) {
            Ok(decision) => Observed::Decision(decision),
            Err(error) => Observed::Error(format!("unparsed-result: {error}; {value}")),
        };
    }
    Observed::Error(error_category(value))
}

async fn post_json(url: &str, body: Value) -> Value {
    reqwest::Client::new()
        .post(url)
        .header("Content-Type", "application/json")
        .header("A2A-Version", a2a_lab_dev_kit::A2A_PROTOCOL_VERSION)
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn observe_http(endpoints: &Endpoints, command: Value) -> Observed {
    let body = post_json(
        &format!("{}/message:send", endpoints.http),
        serde_json::to_value(command_message(command)).unwrap(),
    )
    .await;
    observe_body(&body)
}

async fn observe_jsonrpc(endpoints: &Endpoints, command: Value) -> Observed {
    let body = post_json(
        &format!("{}/", endpoints.http),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "SendMessage",
            "params": command_message(command),
        }),
    )
    .await;
    observe_body(&body)
}

async fn observe_grpc(endpoints: &Endpoints, command: Value) -> Observed {
    match endpoints.grpc.send_message(&command_message(command)).await {
        Ok(SendMessageResponse::Task(task)) => observe_body(&serde_json::to_value(task).unwrap()),
        Ok(SendMessageResponse::Message(message)) => {
            Observed::Error(format!("grpc-message:{message:?}"))
        }
        Err(error) => Observed::Error(category_from_protocol(
            error.code,
            &error.message,
            error.details.as_deref(),
        )),
    }
}

async fn observe_mcp(endpoints: &Endpoints, params: Value) -> Observed {
    let arguments = params.as_object().cloned().unwrap();
    match endpoints
        .mcp
        .call_tool(CallToolRequestParams::new("decide_image").with_arguments(arguments))
        .await
    {
        Ok(result) if result.is_error == Some(true) => Observed::Error("invalid".to_owned()),
        Ok(result) => {
            if let Some(value) = result.structured_content {
                return observe_body(&value);
            }
            let text = result
                .content
                .first()
                .and_then(|block| block.as_text())
                .map(|text| text.text.clone())
                .unwrap_or_default();
            match serde_json::from_str::<Value>(&text) {
                Ok(value) => observe_body(&value),
                Err(error) => Observed::Error(format!("unparsed-mcp: {error}; {text}")),
            }
        }
        Err(ServiceError::McpError(data)) => {
            let coded = data
                .data
                .as_ref()
                .and_then(|value| value.get("code"))
                .and_then(Value::as_str);
            match coded {
                Some(code) => Observed::Error(code.to_owned()),
                None if data.code == rmcp::model::ErrorCode::INVALID_PARAMS => {
                    Observed::Error("invalid".to_owned())
                }
                None => Observed::Error(format!("protocol:{}", data.message)),
            }
        }
        Err(error) => Observed::Error(format!("transport:{error}")),
    }
}

fn same(left: &Observed, right: &Observed) -> bool {
    match (left, right) {
        (Observed::Decision(left), Observed::Decision(right)) => left == right,
        (Observed::Error(left), Observed::Error(right)) => left == right,
        _ => false,
    }
}

async fn compare(endpoints: &Endpoints, name: &str, command: Value, expected: &Observed) {
    let params = command
        .get("params")
        .cloned()
        .unwrap_or_else(|| command.clone());
    let http = observe_http(endpoints, command.clone()).await;
    let a2a = observe_jsonrpc(endpoints, command.clone()).await;
    let grpc = observe_grpc(endpoints, command).await;
    let mcp = observe_mcp(endpoints, params).await;
    let matched = [&http, &a2a, &grpc, &mcp]
        .iter()
        .all(|observed| same(observed, expected));
    assert!(
        matched,
        "case {name} diverged\nexpected: {expected:?}\nHTTP: {http:?}\nA2A: {a2a:?}\nGRPC: {grpc:?}\nMCP: {mcp:?}"
    );
}

fn decision_expected(request: &ImageDecisionRequest, model_box: [f32; 4]) -> Observed {
    let result =
        execute_generic_image_decision(request, &[answer(model_box)]).expect("local decision");
    let shape = result_shape(&result);
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        shape,
        "canonical result shape"
    );
    Observed::Decision(result)
}

fn source_bits(observed: &Observed) -> [u32; 4] {
    let Observed::Decision(result) = observed else {
        panic!("expected a decision, observed {observed:?}");
    };
    bits(result.answers()[0].source_box().unwrap().bounds())
}

fn assert_tool_schema(tool: &rmcp::model::Tool) {
    let input = schema_for_input::<Parameters<ImageDecisionRequest>>().expect("input schema");
    let output = schema_for_output::<ImageDecisionResult>();
    assert_eq!(tool.input_schema.as_ref(), input.as_ref());
    assert_eq!(tool.output_schema.as_deref(), Some(output.as_ref()));
    assert_eq!(input.get("type").and_then(Value::as_str), Some("object"));
    let required = input["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(required.contains(&"image"));
    assert!(required.contains(&"question"));
    assert!(required.contains(&"model_input"));
    assert!(!required.contains(&"roi"));
}

async fn assert_capability_parity(endpoints: &Endpoints) {
    let http = endpoints
        .a2a
        .get_frame_decision_capabilities(GetFrameDecisionCapabilitiesRequest::new())
        .await
        .unwrap();
    let mcp = endpoints
        .mcp
        .call_tool(
            CallToolRequestParams::new("get_frame_decision_capabilities")
                .with_arguments(serde_json::Map::new()),
        )
        .await
        .unwrap();
    let mcp_caps: a2a_lab_dev_kit::FrameDecisionCapabilities =
        serde_json::from_value(mcp.structured_content.expect("capability json")).unwrap();
    assert_eq!(
        serde_json::to_value(&http).unwrap(),
        serde_json::to_value(&mcp_caps).unwrap()
    );
    assert!(http.is_registered());
    assert_eq!(endpoints.appends.load(Ordering::SeqCst), 0);
}

async fn assert_discovery(endpoints: &Endpoints) {
    let card = endpoints.a2a.agent_card().await.unwrap();
    assert!(card.skills.iter().any(|skill| skill.id == "decide-image"));
    let bindings: Vec<_> = card
        .supported_interfaces
        .iter()
        .map(|interface| interface.protocol_binding.as_str())
        .collect();
    assert_eq!(bindings, ["HTTP+JSON", "JSONRPC", "GRPC"]);
    let listed = endpoints.mcp.list_tools(None).await.unwrap().tools;
    let tool = listed
        .iter()
        .find(|tool| tool.name == "decide_image")
        .expect("decide_image tool");
    assert_tool_schema(tool);
    assert_capability_parity(endpoints).await;
}

#[test]
fn published_boundary_and_local_normalization_stay_in_contract() {
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
        assert!(
            IMAGE_DECISION_EXCLUDED_SCOPE.contains(&excluded),
            "{excluded}"
        );
    }

    let full = request(None);
    let full_shape = request_shape(&full, true);
    assert!(full_shape["model_input"].get("roi").is_none());
    let full_result =
        execute_generic_image_decision(&full, &[answer([0.2, 0.3, 0.4, 0.5])]).expect("full image");
    assert_eq!(
        bits(full_result.answers()[0].source_box().unwrap().bounds()),
        bits([0.2, 0.3, 0.4, 0.5])
    );
    assert_eq!(full_result.answers()[0].text(), Some("yes"));

    let crop = ImageDecisionRoi::new([0.25, 0.25, 0.75, 0.75]).unwrap();
    let cropped = request(Some(crop));
    let cropped_shape = request_shape(&cropped, false);
    assert_eq!(
        cropped_shape["model_input"]["roi"]["bounds"],
        json!([0.25, 0.25, 0.75, 0.75])
    );
    let cropped_result =
        execute_generic_image_decision(&cropped, &[answer([0.0, 0.0, 0.5, 0.5])]).unwrap();
    assert_eq!(
        bits(cropped_result.answers()[0].source_box().unwrap().bounds()),
        bits([0.25, 0.25, 0.5, 0.5])
    );
    assert!(
        cropped_result.answers()[0]
            .source_box()
            .unwrap()
            .bounds()
            .iter()
            .all(|value| (0.0..=1.0).contains(value))
    );
}

#[tokio::test]
async fn generic_decision_parity_across_http_mcp_and_a2a() {
    let script = Scripted::answers(vec![answer([0.2, 0.3, 0.4, 0.5])]);
    let endpoints = open(Some(script.clone())).await;
    assert_discovery(&endpoints).await;

    let full = request(None);
    request_shape(&full, true);
    let expected = decision_expected(&full, [0.2, 0.3, 0.4, 0.5]);
    compare(
        &endpoints,
        "success-full-image",
        command_value(&full),
        &expected,
    )
    .await;
    assert_eq!(source_bits(&expected), bits([0.2, 0.3, 0.4, 0.5]));
    assert_eq!(endpoints.appends.load(Ordering::SeqCst), 0);

    let crop = ImageDecisionRoi::new([0.25, 0.25, 0.75, 0.75]).unwrap();
    script.set(Behavior::Answers(vec![answer([0.0, 0.0, 0.5, 0.5])]));
    let cropped = request(Some(crop));
    request_shape(&cropped, false);
    let cropped_expected = decision_expected(&cropped, [0.0, 0.0, 0.5, 0.5]);
    compare(
        &endpoints,
        "cropped-source-box",
        command_value(&cropped),
        &cropped_expected,
    )
    .await;
    assert_eq!(source_bits(&cropped_expected), bits([0.25, 0.25, 0.5, 0.5]));
    assert_eq!(endpoints.appends.load(Ordering::SeqCst), 0);

    let cases = [
        (
            "provider-unavailable",
            ImageDecisionProviderError::unavailable("jev down").unwrap(),
            "unavailable",
        ),
        (
            "provider-invalid-answer",
            ImageDecisionProviderError::invalid_answer("response answers are invalid").unwrap(),
            "invalid_answer",
        ),
        (
            "provider-limit",
            ImageDecisionProviderError::limit_exceeded(
                "image",
                "image exceeds the configured maximum",
            )
            .unwrap(),
            "limit_exceeded",
        ),
        (
            "provider-deadline",
            ImageDecisionProviderError::deadline_exceeded("decision deadline elapsed").unwrap(),
            "deadline_exceeded",
        ),
    ];
    for (name, error, category) in cases {
        script.set(Behavior::Error(error));
        compare(
            &endpoints,
            name,
            command_value(&request(None)),
            &Observed::Error(category.to_owned()),
        )
        .await;
    }
    assert_eq!(endpoints.appends.load(Ordering::SeqCst), 0);

    let mut malformed = command_value(&request(None));
    malformed["params"]["question"] = json!("   ");
    compare(
        &endpoints,
        "malformed-question",
        malformed,
        &Observed::Error("invalid".to_owned()),
    )
    .await;
    assert_eq!(endpoints.appends.load(Ordering::SeqCst), 0);

    let missing = open(None).await;
    compare(
        &missing,
        "client-unavailable",
        command_value(&request(None)),
        &Observed::Error("unavailable".to_owned()),
    )
    .await;
    assert_eq!(missing.appends.load(Ordering::SeqCst), 0);
    assert_eq!(endpoints.appends.load(Ordering::SeqCst), 0);
}
