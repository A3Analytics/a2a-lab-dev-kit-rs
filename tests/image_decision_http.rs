//! Synchronous JEV-style HTTP client for generic image decisions.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aLabConfig, A2aLabError, FakeFrameDecisionEvaluator, FakeJevBehavior, FrameDecisionEvaluator,
    FrameDecisionEvaluatorLimits, FrameDecisionEvaluatorProviderConfig, FrameDecisionLedger,
    FrameDecisionRecord, IMAGE_DECISION_DOWNSTREAM_DEPENDENCY, IMAGE_DECISION_EXCLUDED_SCOPE,
    IMAGE_DECISION_HTTP_DEADLINE, IMAGE_DECISION_HTTP_MAX_IMAGE_BYTES,
    IMAGE_DECISION_HTTP_MAX_REQUEST_BYTES, IMAGE_DECISION_HTTP_MAX_RESPONSE_BYTES,
    IMAGE_DECISION_TRANSPORTS, Image, ImageDecisionClass, ImageDecisionExecutor,
    ImageDecisionHttpClient, ImageDecisionHttpLimits, ImageDecisionModelAnswer,
    ImageDecisionModelInput, ImageDecisionProviderError, ImageDecisionRequest, ImageDecisionResult,
    ImageDecisionRoi, ImageDescriptor, ImageId, ImageSourceId, JsonObject, LabPredicate,
    PREDICATE_FOREIGN_OBJECT_PRESENT, PREDICATE_PIPETTE_TIP_ATTACHED, PREDICATE_PLATE_MISALIGNED,
    ResolvedImageDecisionClient, Sha256Digest, UtcTimestamp, bind_local,
    execute_generic_image_decision,
};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::Response;
use axum::routing::post;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn successful_response_returns_structured_answers_and_normalized_boxes() {
    let request = decision(None);
    let answers = vec![
        model_answer(Some("yes"), Some(0.5), Some([0.25, 0.25, 0.75, 0.75])),
        model_answer(Some("no"), Some(0.25), None),
    ];
    let result = post_answers(&request, &answers, "application/json; charset=utf-8").await;
    assert_eq!(result, expected(&request, &answers));
    assert_eq!(result.answers()[0].text(), Some("yes"));
    assert_eq!(result.answers()[0].probability(), Some(0.5));
    assert_eq!(
        bits(result.answers()[0].source_box().unwrap().bounds()),
        bits([0.25, 0.25, 0.75, 0.75])
    );
    assert_eq!(result.answers()[1].text(), Some("no"));
    assert!(result.answers()[1].source_box().is_none());
    assert!(
        serde_json::to_value(&result)
            .unwrap()
            .get("predicate")
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn omitted_roi_reaches_the_model_as_the_full_image() {
    let request = decision(None);
    let answers = vec![model_answer(
        Some("yes"),
        Some(0.5),
        Some([0.25, 0.5, 0.75, 1.0]),
    )];
    let (result, sent) = post_answers_captured(&request, &answers, "application/json").await;
    assert!(sent["model_input"].get("roi").is_none());
    assert_eq!(sent, serde_json::to_value(&request).unwrap());
    assert!(request.model_input().omits_roi());
    assert_eq!(result, expected(&request, &answers));
    assert_eq!(
        bits(result.answers()[0].source_box().unwrap().bounds()),
        bits([0.25, 0.5, 0.75, 1.0])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cropped_boxes_map_back_to_the_original_source_image() {
    let crop = ImageDecisionRoi::new([0.25, 0.25, 0.75, 0.75]).unwrap();
    let request = decision(Some(crop));
    let answers = vec![model_answer(
        Some("yes"),
        Some(1.0),
        Some([0.0, 0.0, 0.5, 1.0]),
    )];
    let (result, sent) = post_answers_captured(&request, &answers, "application/json").await;
    assert_eq!(
        sent["model_input"]["roi"]["bounds"],
        json!([0.25, 0.25, 0.75, 0.75])
    );
    assert_eq!(
        sent["image"],
        serde_json::to_value(request.image()).unwrap()
    );
    assert_eq!(
        bits(result.answers()[0].source_box().unwrap().bounds()),
        bits([0.25, 0.25, 0.5, 0.75])
    );

    let edge_crop = ImageDecisionRoi::new([0.5, 0.5, 1.0, 1.0]).unwrap();
    let edge_request = decision(Some(edge_crop));
    let edge_answers = vec![model_answer(
        Some("edge"),
        Some(0.0),
        Some([0.0, 0.0, 1.0, 1.0]),
    )];
    let edge = post_answers(&edge_request, &edge_answers, "application/json").await;
    assert_eq!(
        bits(edge.answers()[0].source_box().unwrap().bounds()),
        bits([0.5, 0.5, 1.0, 1.0])
    );

    let origin = ImageDecisionRoi::new([0.0, 0.0, 0.5, 0.5]).unwrap();
    let origin_request = decision(Some(origin));
    let origin_answers = vec![model_answer(
        Some("origin"),
        None,
        Some([0.0, 0.0, 1.0, 1.0]),
    )];
    let origin_result = post_answers(&origin_request, &origin_answers, "application/json").await;
    assert_eq!(
        bits(origin_result.answers()[0].source_box().unwrap().bounds()),
        bits([0.0, 0.0, 0.5, 0.5])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn out_of_range_boxes_are_stable_invalid_answers() {
    let request = decision(Some(ImageDecisionRoi::new([0.0, 0.0, 1.0, 1.0]).unwrap()));
    for body in [
        r#"{"answers":[{"text":"yes","model_box":{"bounds":[0.0,0.0,1.2,1.0]}}]}"#,
        r#"{"answers":[{"text":"yes","model_box":{"bounds":[-0.1,0.0,0.5,0.5]}}]}"#,
        r#"{"answers":[{"text":"yes","model_box":{"bounds":[0.4,0.2,0.4,0.8]}}]}"#,
        r#"{"answers":[{"text":"yes","source_box":{"bounds":[0.0,0.0,1.0,1.0]}}]}"#,
    ] {
        let error = post_raw(
            &request,
            StatusCode::OK,
            Some("application/json"),
            body.as_bytes(),
            false,
        )
        .await;
        assert_eq!(error.code(), "invalid_answer");
        assert!(matches!(
            error,
            ImageDecisionProviderError::InvalidAnswer { ref message }
                if message == "response answers are invalid"
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_success_malformed_and_content_type_errors_are_stable() {
    let request = decision(None);
    let unavailable = post_raw(
        &request,
        StatusCode::SERVICE_UNAVAILABLE,
        Some("text/plain"),
        b"down",
        false,
    )
    .await;
    assert!(matches!(
        unavailable,
        ImageDecisionProviderError::Unavailable { ref message } if message == "http status 503"
    ));

    let provided = post_raw(
        &request,
        StatusCode::BAD_REQUEST,
        Some("application/json"),
        br#"{"reason":"invalid_answer","message":"schema mismatch"}"#,
        false,
    )
    .await;
    assert!(matches!(
        provided,
        ImageDecisionProviderError::InvalidAnswer { ref message } if message == "schema mismatch"
    ));

    let timeout_status = post_raw(
        &request,
        StatusCode::GATEWAY_TIMEOUT,
        Some("application/json"),
        br#"{"reason":"unavailable","message":"late"}"#,
        false,
    )
    .await;
    assert_eq!(timeout_status.code(), "deadline_exceeded");

    let too_large = post_raw(&request, StatusCode::PAYLOAD_TOO_LARGE, None, b"", false).await;
    assert!(matches!(
        too_large,
        ImageDecisionProviderError::LimitExceeded { ref limit, .. } if limit == "request"
    ));

    let malformed = post_raw(
        &request,
        StatusCode::OK,
        Some("application/json"),
        b"{\"answers\":",
        false,
    )
    .await;
    assert!(matches!(
        malformed,
        ImageDecisionProviderError::InvalidAnswer { ref message }
            if message == "response json is malformed"
    ));

    let plain = post_raw(
        &request,
        StatusCode::OK,
        Some("text/plain"),
        br#"{"answers":[]}"#,
        false,
    )
    .await;
    assert!(matches!(
        plain,
        ImageDecisionProviderError::InvalidAnswer { ref message }
            if message == "response content type is not application/json"
    ));

    let missing_type = post_raw(&request, StatusCode::OK, None, br#"{"answers":[]}"#, false).await;
    assert_eq!(missing_type.code(), "invalid_answer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn image_request_and_response_bounds_use_transport_errors() {
    let image_request = decision_bytes(vec![1; 8]);
    let (image_url, image_seen) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        b"{}",
        false,
    ))
    .await;
    let image_client = ImageDecisionHttpClient::new(
        &image_url,
        limits(4, 1024 * 1024, 1024, Duration::from_secs(2)),
    )
    .unwrap();
    let image_error = call(&image_client, &image_request).await.unwrap_err();
    assert!(matches!(
        image_error,
        ImageDecisionProviderError::LimitExceeded { ref limit, .. } if limit == "image"
    ));
    assert!(image_seen.0.lock().expect("seen").is_none());

    let (request_url, request_seen) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        b"{}",
        false,
    ))
    .await;
    let request_client = ImageDecisionHttpClient::new(
        &request_url,
        limits(1024 * 1024, 8, 1024, Duration::from_secs(2)),
    )
    .unwrap();
    let request_error = call(&request_client, &decision(None)).await.unwrap_err();
    assert!(matches!(
        request_error,
        ImageDecisionProviderError::LimitExceeded { ref limit, .. } if limit == "request"
    ));
    assert!(request_seen.0.lock().expect("seen").is_none());

    let (response_url, _) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        &[b' '; 128],
        false,
    ))
    .await;
    let response_client = ImageDecisionHttpClient::new(
        &response_url,
        limits(1024 * 1024, 1024 * 1024, 32, Duration::from_secs(2)),
    )
    .unwrap();
    let response_error = call(&response_client, &decision(None)).await.unwrap_err();
    assert!(matches!(
        response_error,
        ImageDecisionProviderError::LimitExceeded { ref limit, .. } if limit == "response"
    ));

    let (chunk_url, _) = spawn(reply(StatusCode::OK, Some("application/json"), b"", true)).await;
    let chunk_client = ImageDecisionHttpClient::new(
        &chunk_url,
        limits(1024 * 1024, 1024 * 1024, 50, Duration::from_secs(2)),
    )
    .unwrap();
    let chunk_error = call(&chunk_client, &decision(None)).await.unwrap_err();
    assert!(matches!(
        chunk_error,
        ImageDecisionProviderError::LimitExceeded { ref limit, .. } if limit == "response"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_is_deadline_exceeded_without_a_ledger_append() {
    let request = decision(None);
    let ledger = FrameDecisionLedger::new();
    let before = ledger.history().to_vec();
    let (url, _) = spawn(reply_delayed(
        StatusCode::OK,
        Some("application/json"),
        br#"{"answers":[{"text":"yes"}]}"#,
        Duration::from_secs(2),
    ))
    .await;
    let client = ImageDecisionHttpClient::new(
        &url,
        limits(1024 * 1024, 1024 * 1024, 1024, Duration::from_millis(200)),
    )
    .unwrap();
    let error = call(&client, &request).await.unwrap_err();
    assert_eq!(error.code(), "deadline_exceeded");
    assert_eq!(ledger.history(), before.as_slice());
    assert_ne!(
        std::any::TypeId::of::<ImageDecisionResult>(),
        std::any::TypeId::of::<FrameDecisionRecord>()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generic_success_does_not_append_typed_ledger_records() {
    let request = decision(None);
    let ledger = FrameDecisionLedger::new();
    let before = ledger.history().to_vec();
    let result = post_answers(
        &request,
        &[model_answer(
            Some("yes"),
            Some(0.5),
            Some([0.1, 0.1, 0.2, 0.2]),
        )],
        "application/json",
    )
    .await;
    assert_eq!(ledger.history(), before.as_slice());
    assert_eq!(request.class(), ImageDecisionClass::Generic);
    assert!(result.answers()[0].source_box().is_some());
    let encoded = serde_json::to_value(&result).unwrap();
    assert!(encoded.get("decision_id").is_none());
    assert!(encoded.get("idempotency_key").is_none());
}

#[test]
#[allow(clippy::cognitive_complexity)]
fn typed_evaluator_and_published_revision_boundary_stay_in_place() {
    let config = FrameDecisionEvaluatorProviderConfig::pilot(
        Sha256Digest::hash_bytes(b"image-decision-http"),
        Sha256Digest::hash_bytes(b"threshold"),
    )
    .unwrap();
    let evaluator = FakeFrameDecisionEvaluator::new(
        config,
        FrameDecisionEvaluatorLimits::pilot(),
        FakeJevBehavior::OkPresent,
    );
    let capabilities = evaluator.capabilities();
    let predicates = capabilities.supported_predicates();
    assert_eq!(predicates.len(), 3);
    assert_eq!(predicates[0].id(), PREDICATE_PIPETTE_TIP_ATTACHED);
    assert_eq!(predicates[1].id(), PREDICATE_FOREIGN_OBJECT_PRESENT);
    assert_eq!(predicates[2].id(), PREDICATE_PLATE_MISALIGNED);
    assert!(predicates.iter().all(|predicate| predicate.version() == 1));
    assert!(
        LabPredicate::pipette_tip_attached_v1()
            .id()
            .starts_with("lab.")
    );

    assert_eq!(
        IMAGE_DECISION_DOWNSTREAM_DEPENDENCY,
        "published-git-revision"
    );
    assert_ne!(IMAGE_DECISION_DOWNSTREAM_DEPENDENCY, "path");
    assert_eq!(IMAGE_DECISION_TRANSPORTS, ["http", "mcp", "a2a"].as_slice());
    for excluded in [
        "settings-pages",
        "whole-video-indexing",
        "whole-video-backfill",
        "strands",
        "aws-sdk",
        "profile-1.3.0",
    ] {
        assert!(IMAGE_DECISION_EXCLUDED_SCOPE.contains(&excluded));
    }
    let style = ImageDecisionHttpLimits::jev_style();
    assert_eq!(
        style.image_transport().max_image_bytes(),
        IMAGE_DECISION_HTTP_MAX_IMAGE_BYTES
    );
    assert_eq!(
        style.max_request_bytes(),
        IMAGE_DECISION_HTTP_MAX_REQUEST_BYTES
    );
    assert_eq!(
        style.max_response_bytes(),
        IMAGE_DECISION_HTTP_MAX_RESPONSE_BYTES
    );
    assert_eq!(style.deadline(), IMAGE_DECISION_HTTP_DEADLINE);
    assert_eq!(
        ImageDecisionHttpClient::new("ftp://example.com/decision", style)
            .unwrap_err()
            .code(),
        "invalid"
    );
    assert!(
        ImageDecisionHttpLimits::new(style.image_transport(), 0, 1, Duration::from_secs(1),)
            .is_err()
    );
    assert_eq!(
        A2aLabError::invalid("deadline", "must be positive").code(),
        ImageDecisionHttpLimits::new(style.image_transport(), 1, 1, Duration::ZERO)
            .unwrap_err()
            .code()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn configured_preamble_is_prepended_before_provider_limits() {
    let request = decision(None);
    let bare = serde_json::to_vec(&request).unwrap();
    let answers = br#"{"answers":[{"text":"yes","probability":1.0}]}"#;

    let (url, seen) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        answers,
        false,
    ))
    .await;
    let executor = configured_executor(&url, Some(" \t\n"), None, None);
    let result = executor.decide(&request).await.unwrap();
    assert_eq!(result.answers()[0].text(), Some("yes"));
    assert_eq!(seen.0.lock().expect("seen").clone().unwrap(), bare);

    let (url, seen) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        answers,
        false,
    ))
    .await;
    let executor = configured_executor(&url, None, None, None);
    executor.decide(&request).await.unwrap();
    assert_eq!(seen.0.lock().expect("seen").clone().unwrap(), bare);

    let (url, seen) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        answers,
        false,
    ))
    .await;
    let executor = configured_executor(&url, Some("  Guide"), None, None);
    executor.decide(&request).await.unwrap();
    let sent: serde_json::Value =
        serde_json::from_slice(&seen.0.lock().expect("seen").clone().unwrap()).unwrap();
    let expected: serde_json::Value = serde_json::from_slice(&bare).unwrap();
    assert_eq!(sent["image"], expected["image"]);
    assert_eq!(sent["model_input"], expected["model_input"]);
    assert_eq!(sent["question"], format!("  Guide\n{}", request.question()));

    let (url, seen) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        answers,
        false,
    ))
    .await;
    let too_long = "x".repeat(a2a_lab_dev_kit::MAX_IMAGE_DECISION_QUESTION_CHARS);
    let executor = configured_executor(&url, Some(&too_long), None, None);
    let error = executor.decide(&request).await.unwrap_err();
    assert!(matches!(
        error,
        ImageDecisionProviderError::LimitExceeded { ref limit, .. } if limit == "question"
    ));
    assert!(seen.0.lock().expect("seen").is_none());

    let (url, seen) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        answers,
        false,
    ))
    .await;
    let executor = configured_executor(
        &url,
        Some("Guide"),
        Some(u64::try_from(bare.len()).unwrap()),
        Some(64),
    );
    let error = executor.decide(&request).await.unwrap_err();
    assert!(matches!(
        error,
        ImageDecisionProviderError::LimitExceeded { ref limit, .. } if limit == "request"
    ));
    assert!(seen.0.lock().expect("seen").is_none());
}

fn configured_executor(
    url: &str,
    preamble: Option<&str>,
    max_request_bytes: Option<u64>,
    max_image_bytes: Option<u64>,
) -> std::sync::Arc<dyn ImageDecisionExecutor> {
    let mut client = serde_json::json!({
        "endpoint": url,
        "model": "jev-1.14.0"
    });
    if let Some(preamble) = preamble {
        client["preamble"] = serde_json::json!(preamble);
    }
    if let Some(max_request_bytes) = max_request_bytes {
        client["max_request_bytes"] = serde_json::json!(max_request_bytes);
    }
    if let Some(max_image_bytes) = max_image_bytes {
        client["max_image_bytes"] = serde_json::json!(max_image_bytes);
    }
    let config = A2aLabConfig::parse(
        &serde_json::to_string(&serde_json::json!({
            "schema_version": 1,
            "image_decision": client
        }))
        .unwrap(),
    )
    .unwrap();
    ResolvedImageDecisionClient::from_config(config.image_decision.unwrap())
        .executor()
        .unwrap()
}

struct Reply {
    status: StatusCode,
    content_type: Option<&'static str>,
    body: Vec<u8>,
    chunked: bool,
    delay: Duration,
}

fn reply(
    status: StatusCode,
    content_type: Option<&'static str>,
    body: &[u8],
    chunked: bool,
) -> Reply {
    Reply {
        status,
        content_type,
        body: body.to_vec(),
        chunked,
        delay: Duration::ZERO,
    }
}

fn reply_delayed(
    status: StatusCode,
    content_type: Option<&'static str>,
    body: &[u8],
    delay: Duration,
) -> Reply {
    Reply {
        delay,
        ..reply(status, content_type, body, false)
    }
}

struct Seen(Mutex<Option<Vec<u8>>>);

async fn spawn(reply: Reply) -> (String, Arc<Seen>) {
    let seen = Arc::new(Seen(Mutex::new(None)));
    let (listener, address) = bind_local().await.unwrap();
    let state = AppState {
        reply: Arc::new(reply),
        seen: Arc::clone(&seen),
    };
    tokio::spawn(async move {
        let app = Router::new().route("/", post(received)).with_state(state);
        let _ = axum::serve(listener, app).await;
    });
    wait_ready(address).await;
    (format!("http://{address}/"), seen)
}

#[derive(Clone)]
struct AppState {
    reply: Arc<Reply>,
    seen: Arc<Seen>,
}

async fn received(State(state): State<AppState>, body: Bytes) -> Response {
    {
        let mut seen = state.seen.0.lock().expect("seen");
        *seen = Some(body.to_vec());
    }
    if !state.reply.delay.is_zero() {
        tokio::time::sleep(state.reply.delay).await;
    }
    let mut builder = Response::builder().status(state.reply.status);
    if let Some(content_type) = state.reply.content_type {
        builder = builder.header(CONTENT_TYPE, content_type);
    }
    builder.body(reply_body(&state.reply)).expect("response")
}

fn reply_body(reply: &Reply) -> Body {
    if reply.chunked {
        let chunks = [Bytes::from(vec![b'{'; 40]), Bytes::from(vec![b'}'; 40])];
        Body::from_stream(futures_util::stream::iter(
            chunks.map(Ok::<_, std::io::Error>),
        ))
    } else {
        Body::from(reply.body.clone())
    }
}

async fn wait_ready(address: std::net::SocketAddr) {
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(address).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("decision server did not accept connections");
}

fn limits(
    image_bytes: u64,
    max_request_bytes: usize,
    max_response_bytes: usize,
    deadline: Duration,
) -> ImageDecisionHttpLimits {
    ImageDecisionHttpLimits::new(
        a2a_lab_dev_kit::ImageTransportConfig::new(image_bytes).unwrap(),
        max_request_bytes,
        max_response_bytes,
        deadline,
    )
    .unwrap()
}

fn decision(roi: Option<ImageDecisionRoi>) -> ImageDecisionRequest {
    decision_bytes_roi(vec![0xff, 0x00, 0x10, b'a'], roi)
}

fn decision_bytes(bytes: Vec<u8>) -> ImageDecisionRequest {
    decision_bytes_roi(bytes, None)
}

fn decision_bytes_roi(bytes: Vec<u8>, roi: Option<ImageDecisionRoi>) -> ImageDecisionRequest {
    ImageDecisionRequest::new(
        Image::new(descriptor(), bytes).unwrap(),
        "Is the tip attached?",
        ImageDecisionModelInput::new("jev-1.14.0", roi).unwrap(),
    )
    .unwrap()
}

fn descriptor() -> ImageDescriptor {
    ImageDescriptor::new(
        ImageId::new("frame").unwrap(),
        ImageSourceId::new("cam").unwrap(),
        UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
        "image/png",
        640,
        480,
        None,
        JsonObject::empty(),
    )
    .unwrap()
}

fn model_answer(
    text: Option<&str>,
    probability: Option<f32>,
    bounds: Option<[f32; 4]>,
) -> ImageDecisionModelAnswer {
    ImageDecisionModelAnswer::new(
        text,
        probability,
        bounds.map(|bounds| ImageDecisionRoi::new(bounds).unwrap()),
    )
    .unwrap()
}

fn expected(
    request: &ImageDecisionRequest,
    answers: &[ImageDecisionModelAnswer],
) -> ImageDecisionResult {
    let body = serde_json::to_vec(&json!({ "answers": answers })).unwrap();
    let wire: Value = serde_json::from_slice(&body).unwrap();
    let decoded: Vec<ImageDecisionModelAnswer> =
        serde_json::from_value(wire["answers"].clone()).unwrap();
    execute_generic_image_decision(request, &decoded).unwrap()
}

async fn post_answers(
    request: &ImageDecisionRequest,
    answers: &[ImageDecisionModelAnswer],
    content_type: &'static str,
) -> ImageDecisionResult {
    post_answers_captured(request, answers, content_type)
        .await
        .0
}

async fn post_answers_captured(
    request: &ImageDecisionRequest,
    answers: &[ImageDecisionModelAnswer],
    content_type: &'static str,
) -> (ImageDecisionResult, Value) {
    let body = serde_json::to_vec(&json!({ "answers": answers })).unwrap();
    let (url, seen) = spawn(reply(StatusCode::OK, Some(content_type), &body, false)).await;
    let client = ImageDecisionHttpClient::new(&url, generous()).unwrap();
    let result = call(&client, request).await.unwrap();
    let sent =
        serde_json::from_slice(&seen.0.lock().expect("seen").clone().expect("body")).unwrap();
    (result, sent)
}

async fn post_raw(
    request: &ImageDecisionRequest,
    status: StatusCode,
    content_type: Option<&'static str>,
    body: &[u8],
    chunked: bool,
) -> ImageDecisionProviderError {
    let (url, _) = spawn(reply(status, content_type, body, chunked)).await;
    let client = ImageDecisionHttpClient::new(&url, generous()).unwrap();
    call(&client, request).await.unwrap_err()
}

async fn call(
    client: &ImageDecisionHttpClient,
    request: &ImageDecisionRequest,
) -> Result<ImageDecisionResult, ImageDecisionProviderError> {
    tokio::time::timeout(Duration::from_secs(5), client.decide(request))
        .await
        .unwrap_or_else(|_| panic!("decision http call exceeded 5s"))
}

fn generous() -> ImageDecisionHttpLimits {
    limits(
        IMAGE_DECISION_HTTP_MAX_IMAGE_BYTES,
        IMAGE_DECISION_HTTP_MAX_REQUEST_BYTES,
        IMAGE_DECISION_HTTP_MAX_RESPONSE_BYTES,
        Duration::from_secs(2),
    )
}

fn bits(bounds: [f32; 4]) -> [u32; 4] {
    bounds.map(f32::to_bits)
}
