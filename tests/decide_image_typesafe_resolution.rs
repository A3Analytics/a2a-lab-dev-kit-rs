//! Loaded-config resolution for `decide_image`.
//!
//! Fixtures are local axum servers. These tests do not contact api.typesafe.ai.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabCommand, A2aLabError, A2aLabResult, A2aLabService, A2aServer,
    FakeFrameDecisionLedgerProvider, FrameDecisionClientRole, FrameDecisionEvaluatorLimits,
    FrameDecisionEvaluatorProviderConfig, FrameDecisionLedgerProvider,
    GetFrameDecisionCapabilitiesRequest, Image, ImageDecisionExecutor,
    ImageDecisionPrimitiveAnswer, ImageDecisionProviderError, ImageDecisionQuestion,
    ImageDecisionQuestions, ImageDescriptor, ImageId, ImageSource, ImageSourceId, JsonObject,
    LoadedLabConfig, McpLab, McpServer, MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks,
    MemoryVideos, Sha256Digest, SourceImageDecisionRequest, SourceImageDecisionResult,
    UtcTimestamp,
};
use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::post;
use base64::Engine;
use serde_json::{Value, json};

const FILE_KEY: &str = "file-typesafe-key";
const FALLBACK_ENV: &str = "IMAGE_DECISION_TASK217_KEY";
const FALLBACK_KEY: &str = "image-decision-fallback-key";
const MODEL: &str = "jev-1.14.0";

#[derive(Clone)]
struct Captured {
    authorization: Option<String>,
    danger: Option<String>,
    body: Vec<u8>,
}

#[derive(Clone)]
struct Fixture {
    body: Vec<u8>,
    seen: Arc<std::sync::Mutex<Option<Captured>>>,
}

async fn spawn_fixture(body: Vec<u8>) -> (String, Arc<std::sync::Mutex<Option<Captured>>>) {
    let seen = Arc::new(std::sync::Mutex::new(None));
    let (listener, address) = a2a_lab_dev_kit::bind_local().await.unwrap();
    let state = Fixture {
        body,
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
    panic!("decision fixture did not accept connections");
}

async fn received(State(state): State<Fixture>, headers: HeaderMap, body: Bytes) -> Response {
    *state.seen.lock().expect("seen") = Some(Captured {
        authorization: header(&headers, AUTHORIZATION),
        danger: header(&headers, "typesafe-danger-zone-images"),
        body: body.to_vec(),
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(state.body))
        .expect("response")
}

fn header(headers: &HeaderMap, name: impl axum::http::header::AsHeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn typesafe_reply() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "answers": {"tip_attached": {"type": "noul", "noul": 0.42}}
    }))
    .unwrap()
}

fn http_reply() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "answers": {"tip_attached": {"noul": 0.17}}
    }))
    .unwrap()
}

fn frame() -> Image {
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

async fn images() -> MemoryImages {
    let store = MemoryImages::new();
    store
        .insert_source(ImageSource {
            id: ImageSourceId::new("cam").unwrap(),
            name: "Deck".to_owned(),
            description: "deck camera".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await
        .unwrap();
    store.insert_image(frame()).await.unwrap();
    store
}

fn request() -> SourceImageDecisionRequest {
    let mut questions = BTreeMap::new();
    questions.insert(
        "tip_attached".to_owned(),
        ImageDecisionQuestion::Noul {
            instructions: "Is a pipette tip attached?".to_owned(),
            criteria: None,
        },
    );
    SourceImageDecisionRequest::new(
        ImageSourceId::new("cam").unwrap(),
        UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
        questions,
    )
    .unwrap()
}

fn noul(result: &SourceImageDecisionResult) -> f32 {
    let ImageDecisionPrimitiveAnswer::Noul { noul } = result.answers().get("tip_attached").unwrap()
    else {
        panic!("expected noul");
    };
    *noul
}

fn assert_noul(result: &SourceImageDecisionResult, expected: f32) {
    assert_eq!(noul(result).to_bits(), expected.to_bits());
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

fn ledger() -> (Arc<CountingLedger>, Arc<AtomicUsize>) {
    let appends = Arc::new(AtomicUsize::new(0));
    let config = FrameDecisionEvaluatorProviderConfig::pilot(
        Sha256Digest::hash_bytes(b"decide-image-typesafe"),
        Sha256Digest::hash_bytes(b"threshold"),
    )
    .unwrap();
    let provider = CountingLedger {
        inner: FakeFrameDecisionLedgerProvider::authorized_producer(
            &config,
            FrameDecisionEvaluatorLimits::pilot(),
        )
        .unwrap(),
        appends: Arc::clone(&appends),
    };
    (Arc::new(provider), appends)
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "a2a-lab-decide-image-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_config(
    path: &Path,
    endpoint: &str,
    typesafe_key: Option<&str>,
    credentials_env: Option<&str>,
) {
    let mut image_decision = json!({
        "endpoint": endpoint,
        "model": MODEL,
    });
    if let Some(name) = credentials_env {
        image_decision["credentials"] = json!({"env": name});
    }
    let mut document = json!({
        "schema_version": 1,
        "image_decision": image_decision,
    });
    if let Some(key) = typesafe_key {
        document["typesafe"] = json!({"api_key": key});
    }
    fs::write(path, serde_json::to_vec(&document).unwrap()).unwrap();
}

struct EnvGuard {
    previous_key: Option<String>,
    previous_fallback: Option<String>,
}

impl EnvGuard {
    fn set(fallback: Option<&str>) -> Self {
        let previous_key = std::env::var("TYPESAFE_API_KEY").ok();
        let previous_fallback = std::env::var(FALLBACK_ENV).ok();
        unsafe {
            std::env::remove_var("TYPESAFE_API_KEY");
            match fallback {
                Some(value) => std::env::set_var(FALLBACK_ENV, value),
                None => std::env::remove_var(FALLBACK_ENV),
            }
        }
        Self {
            previous_key,
            previous_fallback,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        restore("TYPESAFE_API_KEY", self.previous_key.as_deref());
        restore(FALLBACK_ENV, self.previous_fallback.as_deref());
    }
}

fn restore(name: &str, value: Option<&str>) {
    unsafe {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}

fn captured(seen: &std::sync::Mutex<Option<Captured>>) -> Captured {
    seen.lock().expect("seen").clone().expect("request")
}

fn body_of(seen: &std::sync::Mutex<Option<Captured>>) -> Value {
    serde_json::from_slice(&captured(seen).body).unwrap()
}

type Lab = A2aLabService<MemoryLogs, MemoryMetrics, MemoryTasks, MemoryImages, MemoryVideos>;

async fn decide(service: &Lab) -> SourceImageDecisionResult {
    match service
        .execute(A2aLabCommand::DecideImage(request()))
        .await
        .unwrap()
        .task
        .result
    {
        A2aLabResult::DecideImage(result) => result,
        other => panic!("decide_image result {other:?}"),
    }
}

fn service(images: MemoryImages, ledger: Arc<CountingLedger>) -> Lab {
    A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(MemoryVideos::default())
        .with_images(images)
        .with_frame_decisions(ledger)
}

struct Scripted;

#[async_trait]
impl ImageDecisionExecutor for Scripted {
    async fn decide(
        &self,
        _request: &a2a_lab_dev_kit::ImageDecisionRequest,
    ) -> Result<a2a_lab_dev_kit::ImageDecisionResult, ImageDecisionProviderError> {
        Err(ImageDecisionProviderError::unavailable("generic path is unused").unwrap())
    }

    async fn decide_questions(
        &self,
        _image: &Image,
        questions: &ImageDecisionQuestions,
    ) -> Result<SourceImageDecisionResult, ImageDecisionProviderError> {
        let mut answers = BTreeMap::new();
        answers.insert(
            "tip_attached".to_owned(),
            ImageDecisionPrimitiveAnswer::Noul { noul: 0.94 },
        );
        SourceImageDecisionResult::from_questions(questions, answers)
    }
}

fn assert_typesafe_request(seen: &std::sync::Mutex<Option<Captured>>, key: &str) {
    let request = captured(seen);
    assert_eq!(
        request.authorization.as_deref(),
        Some(format!("Bearer {key}").as_str())
    );
    assert_eq!(request.danger.as_deref(), Some("true"));
    let body = body_of(seen);
    assert_eq!(body["model"], MODEL);
    let reference = body["state"]["image"].as_str().expect("asset reference");
    let asset = &body["assets"][reference];
    assert_eq!(asset["type"], "image");
    assert_eq!(asset["format"], "png");
    assert_eq!(
        asset["data_base64"],
        base64::engine::general_purpose::STANDARD.encode(frame().data())
    );
    assert!(body["state"]["image"].get("media_type").is_none());
    assert_eq!(body["questions"]["tip_attached"]["type"], "noul");
    assert!(
        !request
            .body
            .windows(key.len())
            .any(|window| window == key.as_bytes())
    );
}

fn assert_http_request(seen: &std::sync::Mutex<Option<Captured>>) {
    let request = captured(seen);
    assert!(request.authorization.is_none());
    assert!(request.danger.is_none());
    let body = body_of(seen);
    assert_eq!(body["model"], MODEL);
    assert_eq!(body["state"]["image"]["media_type"], "image/png");
    assert_eq!(
        body["state"]["image"]["data"],
        base64::engine::general_purpose::STANDARD.encode(frame().data())
    );
    assert!(body.get("assets").is_none());
    assert!(body["state"].get("roi").is_none());
    assert_eq!(body["questions"]["tip_attached"]["type"], "noul");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loaded_config_sends_decide_image_through_typesafe() {
    let (url, seen) = spawn_fixture(typesafe_reply()).await;
    let root = TempRoot::new("typesafe");
    let path = root.0.join("a2a-lab.json");
    write_config(&path, &url, Some(FILE_KEY), None);
    let loaded = LoadedLabConfig::load(&path).unwrap();
    let key = loaded.image_decision_api_key().unwrap().unwrap();
    let (ledger, appends) = ledger();
    let service = service(images().await, Arc::clone(&ledger))
        .resolve_image_decision_from_loaded(&loaded)
        .unwrap();
    assert_eq!(
        service.image_decision_client().unwrap().config().model(),
        Some(MODEL)
    );
    assert_eq!(
        service.image_decision_client().unwrap().config().endpoint(),
        Some(url.as_str())
    );

    let result = decide(&service).await;
    assert_noul(&result, 0.42);
    assert_typesafe_request(&seen, &key);
    assert_eq!(appends.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credentials_env_fallback_selects_the_typesafe_client() {
    let (url, seen) = spawn_fixture(typesafe_reply()).await;
    let root = TempRoot::new("fallback");
    let path = root.0.join("a2a-lab.json");
    write_config(&path, &url, None, Some(FALLBACK_ENV));
    let images = images().await;
    let (ledger, appends) = ledger();
    let _env = EnvGuard::set(Some(FALLBACK_KEY));
    let loaded = LoadedLabConfig::load(&path).unwrap();
    assert_eq!(
        loaded.image_decision_api_key().unwrap().as_deref(),
        Some(FALLBACK_KEY)
    );
    let service = service(images, ledger)
        .resolve_image_decision_from_loaded(&loaded)
        .unwrap();

    let result = decide(&service).await;
    assert_noul(&result, 0.42);
    assert_typesafe_request(&seen, FALLBACK_KEY);
    assert_eq!(appends.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn injected_executor_is_not_replaced_by_typesafe_resolution() {
    let (url, seen) = spawn_fixture(typesafe_reply()).await;
    let root = TempRoot::new("injected");
    let path = root.0.join("a2a-lab.json");
    write_config(&path, &url, Some(FILE_KEY), None);
    let loaded = LoadedLabConfig::load(&path).unwrap();
    let (ledger, appends) = ledger();
    let service = service(images().await, ledger)
        .with_image_decision_executor(Arc::new(Scripted))
        .resolve_image_decision_from_loaded(&loaded)
        .unwrap();

    let result = decide(&service).await;
    assert_noul(&result, 0.94);
    assert!(seen.lock().expect("seen").is_none());
    assert_eq!(appends.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_key_keeps_the_unauthenticated_http_client() {
    let (url, seen) = spawn_fixture(http_reply()).await;
    let root = TempRoot::new("http");
    let path = root.0.join("a2a-lab.json");
    write_config(&path, &url, None, None);
    let catalog = images().await;
    let (provider, appends) = ledger();
    let _env = EnvGuard::set(None);
    let loaded = LoadedLabConfig::load(&path).unwrap();
    assert!(loaded.image_decision_api_key().unwrap().is_none());
    let lab = service(catalog, Arc::clone(&provider))
        .resolve_image_decision_from_loaded(&loaded)
        .unwrap();

    let result = decide(&lab).await;
    assert_noul(&result, 0.17);
    assert_http_request(&seen);
    assert_eq!(appends.load(Ordering::SeqCst), 0);

    let (direct_url, direct_seen) = spawn_fixture(http_reply()).await;
    let direct_root = TempRoot::new("direct");
    let direct_path = direct_root.0.join("a2a-lab.json");
    write_config(&direct_path, &direct_url, Some(FILE_KEY), None);
    let direct = LoadedLabConfig::load(&direct_path).unwrap();
    assert!(direct.image_decision_api_key().unwrap().is_some());
    let (direct_ledger, direct_appends) = ledger();
    let direct_service =
        service(images().await, direct_ledger).resolve_image_decision_client(&direct.config);
    let direct_result = decide(&direct_service).await;
    assert_noul(&direct_result, 0.17);
    assert_http_request(&direct_seen);
    assert_eq!(direct_appends.load(Ordering::SeqCst), 0);
}

async fn open_transports(service: Lab) -> (A2aClient, McpLab) {
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
    (
        A2aClient::new(&http).unwrap(),
        McpLab::connect(&mcp_url).await.unwrap(),
    )
}

async fn decide_transports(a2a: &A2aClient, mcp: &McpLab) -> [SourceImageDecisionResult; 2] {
    let http = a2a.decide_image(request()).await.unwrap();
    let mcp_result = match mcp
        .execute(A2aLabCommand::DecideImage(request()))
        .await
        .unwrap()
        .task
        .result
    {
        A2aLabResult::DecideImage(result) => result,
        other => panic!("mcp result {other:?}"),
    };
    [http, mcp_result]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transports_use_typesafe_resolution_without_a_ledger_append() {
    let (url, seen) = spawn_fixture(typesafe_reply()).await;
    let root = TempRoot::new("transport-typesafe");
    let path = root.0.join("a2a-lab.json");
    write_config(&path, &url, Some(FILE_KEY), None);
    let loaded = LoadedLabConfig::load(&path).unwrap();
    let key = loaded.image_decision_api_key().unwrap().unwrap();
    let (ledger, appends) = ledger();
    let service = service(images().await, ledger)
        .resolve_image_decision_from_loaded(&loaded)
        .unwrap();
    let (a2a, mcp) = open_transports(service).await;

    for result in decide_transports(&a2a, &mcp).await {
        assert_noul(&result, 0.42);
    }
    assert_typesafe_request(&seen, &key);
    assert_eq!(appends.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transports_keep_an_injected_executor_and_skip_the_ledger() {
    let (url, seen) = spawn_fixture(typesafe_reply()).await;
    let root = TempRoot::new("transport-injected");
    let path = root.0.join("a2a-lab.json");
    write_config(&path, &url, Some(FILE_KEY), None);
    let loaded = LoadedLabConfig::load(&path).unwrap();
    let (ledger, appends) = ledger();
    let service = service(images().await, ledger)
        .with_image_decision_executor(Arc::new(Scripted))
        .resolve_image_decision_from_loaded(&loaded)
        .unwrap();
    let (a2a, mcp) = open_transports(service).await;

    for result in decide_transports(&a2a, &mcp).await {
        assert_noul(&result, 0.94);
    }
    assert!(seen.lock().expect("seen").is_none());
    assert_eq!(appends.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transports_keep_the_unauthenticated_http_shape() {
    let (url, seen) = spawn_fixture(http_reply()).await;
    let root = TempRoot::new("transport-http");
    let path = root.0.join("a2a-lab.json");
    write_config(&path, &url, None, None);
    let images = images().await;
    let (ledger, appends) = ledger();
    let _env = EnvGuard::set(None);
    let loaded = LoadedLabConfig::load(&path).unwrap();
    let service = service(images, ledger)
        .resolve_image_decision_from_loaded(&loaded)
        .unwrap();
    let (a2a, mcp) = open_transports(service).await;

    for result in decide_transports(&a2a, &mcp).await {
        assert_noul(&result, 0.17);
    }
    assert_http_request(&seen);
    assert_eq!(appends.load(Ordering::SeqCst), 0);
}
