//! Local System One HTTP fixtures. These tests do not contact the network.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use a2a_lab_dev_kit::{
    Image, ImageDecisionAnswer, ImageDecisionHttpClient, ImageDecisionHttpLimits,
    ImageDecisionModelInput, ImageDecisionPrimitiveAnswer, ImageDecisionProviderError,
    ImageDecisionQuestion, ImageDecisionQuestions, ImageDecisionRequest, ImageDescriptor, ImageId,
    ImageSourceId, ImageTransportConfig, JsonObject, TypeSafeSystemOneClient, UtcTimestamp,
};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::post;
use base64::Engine;
use serde_json::{Value, json};

const API_KEY: &str = "super-secret-typesafe-key";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fixture_receives_bearer_danger_header_and_named_question_body() {
    let image = image_with("image/png", vec![0xff, 0x00, 0x10, b'a']);
    let questions = named_questions();
    let (url, seen) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        &answer_body(),
        false,
        Duration::ZERO,
    ))
    .await;
    let client = TypeSafeSystemOneClient::new(&url, API_KEY, generous()).unwrap();
    let result = client
        .decide_questions("jev-1.14.0", &image, &questions)
        .await
        .unwrap();
    let captured = seen.lock().expect("seen").clone().expect("request");
    assert_eq!(
        captured.authorization.as_deref(),
        Some("Bearer super-secret-typesafe-key")
    );
    assert_eq!(captured.danger.as_deref(), Some("true"));
    assert_eq!(captured.content_type.as_deref(), Some("application/json"));
    let body: Value = serde_json::from_slice(&captured.body).unwrap();
    let reference = body["state"]["image"].as_str().unwrap();
    let asset = &body["assets"][reference];
    assert_eq!(body["model"], "jev-1.14.0");
    assert_eq!(asset["type"], "image");
    assert_eq!(asset["format"], "png");
    assert_eq!(
        asset["data_base64"],
        base64::engine::general_purpose::STANDARD.encode(image.data())
    );
    assert!(body.get("image").is_none());
    assert_eq!(body["questions"]["billing"]["type"], "noul");
    assert_eq!(body["questions"]["tone"]["type"], "choice");
    assert_eq!(body["questions"]["urgency"]["type"], "score");
    assert_eq!(
        body["questions"]["urgency"]["criteria"],
        json!(["low", "high"])
    );
    assert!(!captured.body_text().contains(API_KEY));
    assert_eq!(
        result.answers().get("billing"),
        Some(&ImageDecisionPrimitiveAnswer::Noul { noul: 0.5 })
    );
    assert_eq!(
        result.answers().get("tone"),
        Some(&ImageDecisionPrimitiveAnswer::Choice {
            choice: "calm".to_owned(),
            probabilities: choice_probabilities(),
            confidence: 0.25,
        })
    );
    assert_eq!(
        result.answers().get("urgency"),
        Some(&ImageDecisionPrimitiveAnswer::Score {
            score: 1.5,
            legend: vec!["low".to_owned(), "high".to_owned()],
            probabilities: vec![0.25, 0.75],
            confidence: 0.25,
        })
    );
    assert!(!format!("{result:?}").contains(API_KEY));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn asset_format_follows_png_jpg_and_jpeg_media_types() {
    for (media_type, format) in [
        ("image/png", "png"),
        ("image/jpg", "jpg"),
        ("image/jpeg", "jpeg"),
    ] {
        let image = image_with(media_type, vec![1, 2, 3, 4]);
        let (url, seen) = spawn(reply(
            StatusCode::OK,
            Some("application/json"),
            br#"{"answers":{"billing":{"type":"noul","noul":0.5}},"usage":{"input_tokens":1}}"#,
            false,
            Duration::ZERO,
        ))
        .await;
        let client = TypeSafeSystemOneClient::new(&url, API_KEY, generous()).unwrap();
        client
            .decide_questions("jev-1.14.0", &image, &noul_only())
            .await
            .unwrap();
        let captured = seen.lock().expect("seen").clone().expect("request");
        let body: Value = serde_json::from_slice(&captured.body).unwrap();
        let reference = body["state"]["image"].as_str().unwrap();
        assert_eq!(body["assets"][reference]["format"], format);
        assert_eq!(captured.danger.as_deref(), Some("true"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn box_edges_convert_to_normalized_roi_bounds() {
    let mut questions = BTreeMap::new();
    questions.insert("red".to_owned(), "Locate the red rectangle.".to_owned());
    questions.insert("tip".to_owned(), "Locate the tip.".to_owned());
    let body = json!({
        "model": "jev-1.14.0",
        "answers": {
            "red": {
                "type": "box",
                "box": {"top": 0.25, "bottom": 0.75, "left": 0.125, "right": 0.5},
                "confidence": null
            },
            "tip": {
                "type": "box",
                "box": {"top": 0.0, "bottom": 1.0, "left": 0.0, "right": 0.5},
                "confidence": 0.5
            }
        },
        "usage": {"input_tokens": 3, "output_tokens": 1}
    });
    let (url, seen) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        &serde_json::to_vec(&body).unwrap(),
        false,
        Duration::ZERO,
    ))
    .await;
    let client = TypeSafeSystemOneClient::new(&url, API_KEY, generous()).unwrap();
    let result = client
        .decide_boxes(
            "jev-1.14.0",
            &image_with("image/png", vec![9, 8, 7]),
            &questions,
        )
        .await
        .unwrap();
    let captured = seen.lock().expect("seen").clone().expect("request");
    let sent: Value = serde_json::from_slice(&captured.body).unwrap();
    assert_eq!(captured.danger.as_deref(), Some("true"));
    assert_eq!(sent["questions"]["red"]["type"], "box");
    assert_eq!(
        sent["questions"]["red"]["instructions"],
        "Locate the red rectangle."
    );
    assert_eq!(sent["questions"]["tip"]["instructions"], "Locate the tip.");
    assert_answer(&result["red"], None, [0.125, 0.25, 0.5, 0.75]);
    assert_answer(&result["tip"], Some(0.5), [0.0, 0.0, 0.5, 1.0]);

    let invalid = json!({
        "answers": {
            "red": {
                "type": "box",
                "box": {"top": 0.8, "bottom": 0.2, "left": 0.1, "right": 0.4},
                "confidence": 0.5
            }
        }
    });
    let (invalid_url, _) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        &serde_json::to_vec(&invalid).unwrap(),
        false,
        Duration::ZERO,
    ))
    .await;
    let mut one = BTreeMap::new();
    one.insert("red".to_owned(), "Locate the red rectangle.".to_owned());
    let error = TypeSafeSystemOneClient::new(&invalid_url, API_KEY, generous())
        .unwrap()
        .decide_boxes("jev-1.14.0", &image_with("image/png", vec![9]), &one)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "invalid_answer");
    assert!(!error.to_string().contains(API_KEY));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_statuses_timeouts_and_bad_bodies_hide_the_api_key() {
    let image = image_with("image/png", vec![1, 2, 3, 4]);
    let secret_body = format!(r#"{{"detail":"{API_KEY}"}}"#);
    let cases = [
        (StatusCode::BAD_REQUEST, "invalid_answer", "http status 400"),
        (StatusCode::UNAUTHORIZED, "unavailable", "http status 401"),
        (StatusCode::PAYLOAD_TOO_LARGE, "limit_exceeded", "request"),
        (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_answer",
            "http status 415",
        ),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_answer",
            "http status 422",
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            "unavailable",
            "http status 429",
        ),
        (
            StatusCode::from_u16(529).unwrap(),
            "unavailable",
            "http status 529",
        ),
    ];
    for (status, code, marker) in cases {
        let error = post_status(
            &image,
            status,
            Some("application/json"),
            secret_body.as_bytes(),
        )
        .await;
        assert_eq!(error.code(), code);
        let rendered = format!("{error} {error:?}");
        assert!(rendered.contains(marker), "{rendered}");
        assert!(!rendered.contains(API_KEY), "{rendered}");
        assert!(!rendered.contains("detail"), "{rendered}");
    }

    let malformed = post_status(
        &image,
        StatusCode::OK,
        Some("application/json"),
        br#"{"answers":"#,
    )
    .await;
    assert!(matches!(
        malformed,
        ImageDecisionProviderError::InvalidAnswer { ref message }
            if message == "response json is malformed"
    ));
    assert!(!malformed.to_string().contains(API_KEY));

    let plain = post_status(&image, StatusCode::OK, Some("text/plain"), b"not-json").await;
    assert!(matches!(
        plain,
        ImageDecisionProviderError::InvalidAnswer { ref message }
            if message == "response content type is not application/json"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeouts_and_oversized_responses_hide_the_api_key() {
    let image = image_with("image/png", vec![1, 2, 3, 4]);
    let (timeout_url, _) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        br#"{"answers":{"billing":{"type":"noul","noul":0.5}}}"#,
        false,
        Duration::from_secs(1),
    ))
    .await;
    let timeout = TypeSafeSystemOneClient::new(
        &timeout_url,
        API_KEY,
        limits(1024, 1024 * 1024, 1024, Duration::from_millis(150)),
    )
    .unwrap()
    .decide_questions("jev-1.14.0", &image, &noul_only())
    .await
    .unwrap_err();
    assert_eq!(timeout.code(), "deadline_exceeded");
    assert!(!format!("{timeout:?}").contains(API_KEY));

    let oversized = post_status(
        &image,
        StatusCode::OK,
        Some("application/json"),
        &[b' '; 128],
    )
    .await;
    assert!(matches!(
        oversized,
        ImageDecisionProviderError::LimitExceeded { ref limit, .. } if limit == "response"
    ));
    let (chunk_url, _) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        b"",
        true,
        Duration::ZERO,
    ))
    .await;
    let chunked = TypeSafeSystemOneClient::new(
        &chunk_url,
        API_KEY,
        limits(1024, 1024 * 1024, 50, Duration::from_secs(2)),
    )
    .unwrap()
    .decide_questions("jev-1.14.0", &image, &noul_only())
    .await
    .unwrap_err();
    assert!(matches!(
        chunked,
        ImageDecisionProviderError::LimitExceeded { ref limit, .. } if limit == "response"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn image_request_and_unsupported_format_are_rejected_before_send() {
    let (url, seen) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        br#"{"answers":{"billing":{"type":"noul","noul":0.5}}}"#,
        false,
        Duration::ZERO,
    ))
    .await;
    let image_client = TypeSafeSystemOneClient::new(
        &url,
        API_KEY,
        limits(4, 1024 * 1024, 1024, Duration::from_secs(2)),
    )
    .unwrap();
    let image_error = image_client
        .decide_questions(
            "jev-1.14.0",
            &image_with("image/png", vec![1, 2, 3, 4, 5]),
            &noul_only(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        image_error,
        ImageDecisionProviderError::LimitExceeded { ref limit, .. } if limit == "image"
    ));
    assert!(seen.lock().expect("seen").is_none());

    let (request_url, request_seen) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        b"{}",
        false,
        Duration::ZERO,
    ))
    .await;
    let request_error = TypeSafeSystemOneClient::new(
        &request_url,
        API_KEY,
        limits(1024, 8, 1024, Duration::from_secs(2)),
    )
    .unwrap()
    .decide_questions(
        "jev-1.14.0",
        &image_with("image/png", vec![1, 2, 3, 4]),
        &noul_only(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        request_error,
        ImageDecisionProviderError::LimitExceeded { ref limit, .. } if limit == "request"
    ));
    assert!(request_seen.lock().expect("seen").is_none());

    let (format_url, format_seen) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        b"{}",
        false,
        Duration::ZERO,
    ))
    .await;
    let format_error = TypeSafeSystemOneClient::new(&format_url, API_KEY, generous())
        .unwrap()
        .decide_questions(
            "jev-1.14.0",
            &image_with("image/gif", vec![1, 2, 3, 4]),
            &noul_only(),
        )
        .await
        .unwrap_err();
    assert_eq!(format_error.code(), "invalid_answer");
    assert!(format_seen.lock().expect("seen").is_none());
    let rendered = format!("{image_client:?}");
    assert!(rendered.contains("<redacted>"));
    assert!(!rendered.contains(API_KEY));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generic_client_keeps_the_unauthenticated_request_shape() {
    let request = ImageDecisionRequest::new(
        image_with("image/png", vec![0xff, 0x00, 0x10, b'a']),
        "Is the tip attached?",
        ImageDecisionModelInput::new("jev-1.14.0", None).unwrap(),
    )
    .unwrap();
    let (url, seen) = spawn(reply(
        StatusCode::OK,
        Some("application/json"),
        br#"{"answers":[{"text":"yes","probability":0.5}]}"#,
        false,
        Duration::ZERO,
    ))
    .await;
    let result = ImageDecisionHttpClient::new(&url, generous())
        .unwrap()
        .decide(&request)
        .await
        .unwrap();
    assert_eq!(result.answers()[0].text(), Some("yes"));
    let captured = seen.lock().expect("seen").clone().expect("request");
    assert!(captured.authorization.is_none());
    assert!(captured.danger.is_none());
    assert_eq!(
        serde_json::from_slice::<Value>(&captured.body).unwrap(),
        serde_json::to_value(&request).unwrap()
    );
}

fn assert_answer(answer: &ImageDecisionAnswer, probability: Option<f32>, bounds: [f32; 4]) {
    assert_eq!(answer.text(), None);
    assert_eq!(
        answer.probability().map(f32::to_bits),
        probability.map(f32::to_bits)
    );
    assert_eq!(
        answer.source_box().unwrap().bounds().map(f32::to_bits),
        bounds.map(f32::to_bits)
    );
}

fn choice_probabilities() -> BTreeMap<String, f32> {
    BTreeMap::from([("angry".to_owned(), 0.25), ("calm".to_owned(), 0.75)])
}

fn named_questions() -> ImageDecisionQuestions {
    let mut tone = BTreeMap::new();
    tone.insert("angry".to_owned(), "An upset message".to_owned());
    tone.insert("calm".to_owned(), "A polite message".to_owned());
    let mut questions = ImageDecisionQuestions::new();
    questions.insert(
        "billing".to_owned(),
        ImageDecisionQuestion::Noul {
            instructions: "Is this message about billing?".to_owned(),
            criteria: None,
        },
    );
    questions.insert(
        "tone".to_owned(),
        ImageDecisionQuestion::Choice {
            instructions: "What is the tone?".to_owned(),
            criteria: tone,
        },
    );
    questions.insert(
        "urgency".to_owned(),
        ImageDecisionQuestion::Score {
            instructions: "How urgent is this message?".to_owned(),
            criteria: vec!["low".to_owned(), "high".to_owned()],
        },
    );
    questions
}

fn noul_only() -> ImageDecisionQuestions {
    let mut questions = ImageDecisionQuestions::new();
    questions.insert(
        "billing".to_owned(),
        ImageDecisionQuestion::Noul {
            instructions: "Is this message about billing?".to_owned(),
            criteria: None,
        },
    );
    questions
}

fn answer_body() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "model": "jev-1.14.0",
        "answers": {
            "billing": {"type": "noul", "noul": 0.5},
            "tone": {
                "type": "choice",
                "choice": "calm",
                "confidence": 0.25,
                "probabilities": {"angry": 0.25, "calm": 0.75}
            },
            "urgency": {
                "type": "score",
                "score": 1.5,
                "confidence": 0.25,
                "legend": {"0": "low", "1": "high"},
                "probabilities": {"0": 0.25, "1": 0.75}
            }
        },
        "usage": {"input_tokens": 12, "output_tokens": 4}
    }))
    .unwrap()
}

fn image_with(media_type: &str, bytes: Vec<u8>) -> Image {
    Image::new(
        ImageDescriptor::new(
            ImageId::new("frame").unwrap(),
            ImageSourceId::new("cam").unwrap(),
            UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
            media_type,
            640,
            480,
            None,
            JsonObject::empty(),
        )
        .unwrap(),
        bytes,
    )
    .unwrap()
}

fn generous() -> ImageDecisionHttpLimits {
    limits(
        1024 * 1024,
        1024 * 1024,
        1024 * 1024,
        Duration::from_secs(2),
    )
}

fn limits(
    image_bytes: u64,
    max_request_bytes: usize,
    max_response_bytes: usize,
    deadline: Duration,
) -> ImageDecisionHttpLimits {
    ImageDecisionHttpLimits::new(
        ImageTransportConfig::new(image_bytes).unwrap(),
        max_request_bytes,
        max_response_bytes,
        deadline,
    )
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
    delay: Duration,
) -> Reply {
    Reply {
        status,
        content_type,
        body: body.to_vec(),
        chunked,
        delay,
    }
}

#[derive(Clone)]
struct Captured {
    authorization: Option<String>,
    danger: Option<String>,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Captured {
    fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[derive(Clone)]
struct AppState {
    reply: Arc<Reply>,
    seen: Arc<Mutex<Option<Captured>>>,
}

async fn spawn(reply: Reply) -> (String, Arc<Mutex<Option<Captured>>>) {
    let seen = Arc::new(Mutex::new(None));
    let (listener, address) = a2a_lab_dev_kit::bind_local().await.unwrap();
    let state = AppState {
        reply: Arc::new(reply),
        seen: Arc::clone(&seen),
    };
    tokio::spawn(async move {
        let app = Router::new().route("/", post(received)).with_state(state);
        let _ = axum::serve(listener, app).await;
    });
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(address).await.is_ok() {
            return (format!("http://{address}/"), seen);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("decision server did not accept connections");
}

async fn received(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let captured = Captured {
        authorization: header(&headers, AUTHORIZATION),
        danger: header(&headers, "typesafe-danger-zone-images"),
        content_type: header(&headers, CONTENT_TYPE),
        body: body.to_vec(),
    };
    *state.seen.lock().expect("seen") = Some(captured);
    if !state.reply.delay.is_zero() {
        tokio::time::sleep(state.reply.delay).await;
    }
    let mut builder = Response::builder().status(state.reply.status);
    if let Some(content_type) = state.reply.content_type {
        builder = builder.header(CONTENT_TYPE, content_type);
    }
    builder.body(reply_body(&state.reply)).expect("response")
}

fn header(headers: &HeaderMap, name: impl axum::http::header::AsHeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
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

async fn post_status(
    image: &Image,
    status: StatusCode,
    content_type: Option<&'static str>,
    body: &[u8],
) -> ImageDecisionProviderError {
    let max_response = if body.len() > 64 { 32 } else { 1024 };
    let (url, _) = spawn(reply(status, content_type, body, false, Duration::ZERO)).await;
    TypeSafeSystemOneClient::new(
        &url,
        API_KEY,
        limits(1024, 1024 * 1024, max_response, Duration::from_secs(2)),
    )
    .unwrap()
    .decide_questions("jev-1.14.0", image, &noul_only())
    .await
    .unwrap_err()
}
