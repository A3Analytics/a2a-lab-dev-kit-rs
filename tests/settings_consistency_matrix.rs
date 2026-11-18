//! Hardware-free consistency matrix for file, API, runtime watch, and pending paths.

#![cfg(feature = "web-ui")]

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use a2a_lab_dev_kit::config::VideoCaptureSettingsWatch;
use a2a_lab_dev_kit::videos::SENSOR_30FPS_PERIOD_NS;
use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, CameraSelector, ConfigDuration, DEFAULT_LIVE_OUTPUT_FPS,
    HttpAuthSecurityScheme, LabConfigStore, MemoryLogs, MemoryMetrics, MemoryTasks,
    SaveIntervalDuration, SecurityScheme, VideoCaptureSettings, VideoCaptureSettingsHandle,
    VideoConfig, VideoDvrConfig, VideoInputFormat, VideoSourceConfig, VideoSourceId, bind_local,
};
use a2a_types::SecurityRequirement;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

const ORIGINAL: &str = r#"{"schema_version":1,"chat":{"provider":"openai","model":"keep-me"},"videos":{"sources":[{"id":"camera","format":"yuyv","device_index":6}],"fps":5,"dvr":{"storage_path":"./dvr"}}}"#;

const CADENCE: &str = r#"{"schema_version":1,"videos":{"sources":[{"id":"camera","format":"yuyv","device_index":6}],"fps":5,"live_retention":"30s","dvr":{"save_interval":"1s","retention":"1h","storage_path":"./dvr"}}}"#;

const CREDENTIALS: &str = r#"{"schema_version":2,"semantic_indexing":{"description":{"credentials":{"env":"A2ALAB_SETTINGS_HTTP_SECRET"}},"embeddings":{"provider":"p","model":"m","space":{"dimensions":4,"normalization":"l2_unit","metric":"cosine"}},"limits":{"worker":{"max_batch_items":1,"max_concurrency":1,"max_queue_backlog":1,"max_retry_attempts":1,"job_deadline":"30s","retry_backoff":"5s"},"search":{"top_k":1,"candidate_limit":1,"max_neighbor_frames":1,"query_window":"1h"},"roi":{"min_edge_pixels":280,"max_edge_pixels":1568,"max_rois_per_frame":1},"storage":{"vector_index_path":"./x","max_disk_bytes":1024},"decisions":{"max_batch_items":1,"max_concurrency":1,"job_deadline":"45s","coverage_policy":"on_demand"}}}}"#;

const SECRET: &str = "super-secret-credential-value";

const SEMANTIC_SOURCES: &str = include_str!("fixtures/a2a-lab-v2-semantic-valid.json");

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "a2a-lab-settings-matrix-{}-{}-{}",
            std::process::id(),
            unique,
            nanos
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn write_config(&self, body: &str) -> PathBuf {
        let path = self.0.join("a2a-lab.json");
        fs::write(&path, body).unwrap();
        path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn file_bytes(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
}

fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    format!("sha256:{hex}")
}

fn settings_handle() -> VideoCaptureSettingsHandle {
    let config = VideoConfig {
        sources: vec![VideoSourceConfig {
            id: VideoSourceId::new("bench").unwrap(),
            name: Some("Bench".to_owned()),
            description: None,
            format: VideoInputFormat::Mjpeg,
            camera_id: Some(CameraSelector::Index(4)),
            width: Some(640),
            height: Some(480),
            capture_rate: Some(30),
            semantic_indexing: None,
        }],
        fps: Some(5),
        live_retention: ConfigDuration::parse("videos.live_retention", "30s").ok(),
        dvr: Some(VideoDvrConfig {
            retention: ConfigDuration::parse("videos.dvr.retention", "1h").ok(),
            save_interval: SaveIntervalDuration::parse("videos.dvr.save_interval", "1s").ok(),
            storage_path: Some("./dvr".to_owned()),
        }),
    };
    let settings = VideoCaptureSettings::from_video_config(Some(&config))
        .with_negotiated_sensor_period_ns(SENSOR_30FPS_PERIOD_NS);
    VideoCaptureSettingsHandle::new(settings)
}

async fn serve(
    store: LabConfigStore,
    settings: Option<VideoCaptureSettingsHandle>,
    secured: bool,
) -> String {
    let service =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share();
    let (listener, address) = bind_local().await.unwrap();
    let mut server = A2aServer::new(&service)
        .with_web_ui()
        .with_lab_config_store(store);
    if let Some(settings) = settings {
        server = server.with_video_capture_settings(settings);
    }
    if secured {
        let schemes: HashMap<String, SecurityScheme> = [(
            "bearerAuth".to_owned(),
            SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                scheme: "bearer".to_owned(),
                description: None,
                bearer_format: None,
            }),
        )]
        .into();
        let requirements: Vec<SecurityRequirement> =
            vec![[("bearerAuth".to_owned(), Vec::new())].into()];
        server = server.with_security(schemes, requirements);
    }
    tokio::spawn(async move { server.listen(Some(listener)).await.unwrap() });
    format!("http://{address}")
}

struct Matrix<'a> {
    client: reqwest::Client,
    base: String,
    path: Option<&'a Path>,
    handle: Option<&'a VideoCaptureSettingsHandle>,
    token: Option<&'a str>,
}

impl Matrix<'_> {
    async fn get(&self) -> Value {
        let mut request = self.client.get(format!("{}/config/settings", self.base));
        if let Some(token) = self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), 200);
        let body: Value = response.json().await.unwrap();
        self.assert_layers(&body);
        body
    }

    async fn patch(&self, revision: &str, changes: Value) -> reqwest::Response {
        let mut request = self.client.patch(format!("{}/config/settings", self.base));
        if let Some(token) = self.token {
            request = request.bearer_auth(token);
        }
        request
            .json(&json!({ "revision": revision, "changes": changes }))
            .send()
            .await
            .unwrap()
    }

    async fn patch_ok(&self, revision: &str, changes: Value) -> Value {
        let response = self.patch(revision, changes).await;
        assert_eq!(response.status(), 200);
        let body: Value = response.json().await.unwrap();
        self.assert_layers(&body);
        body
    }

    async fn patch_rejected(
        &self,
        revision: &str,
        changes: Value,
        status: u16,
        code: &str,
        before_bytes: Option<&[u8]>,
        before_watch: Option<&VideoCaptureSettings>,
    ) {
        let response = self.patch(revision, changes).await;
        assert_eq!(response.status().as_u16(), status);
        let err: Value = response.json().await.unwrap();
        assert_eq!(err["error"]["code"], code);
        if let (Some(path), Some(before)) = (self.path, before_bytes) {
            assert_eq!(file_bytes(path), before);
        }
        if let (Some(handle), Some(before)) = (self.handle, before_watch) {
            assert_eq!(&handle.snapshot(), before);
        }
        let _ = self.get().await;
        if let (Some(path), Some(before)) = (self.path, before_bytes) {
            assert_eq!(file_bytes(path), before);
        }
        if let (Some(handle), Some(before)) = (self.handle, before_watch) {
            assert_eq!(&handle.snapshot(), before);
        }
    }

    fn assert_layers(&self, body: &Value) {
        if let Some(path) = self.path {
            let bytes = file_bytes(path);
            let on_disk: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                body["document"], on_disk,
                "API document must match parsed file bytes"
            );
            assert_eq!(
                body["revision"].as_str().expect("revision"),
                content_hash(&bytes)
            );
        } else {
            assert!(body["revision"].is_null());
            assert_eq!(body["document"], json!({}));
        }

        let watch_view = match self.handle {
            Some(handle) => serde_json::to_value(handle.snapshot().view()).unwrap(),
            None => serde_json::to_value(
                VideoCaptureSettingsWatch::new(VideoCaptureSettings::from_video_config(None))
                    .snapshot()
                    .view(),
            )
            .unwrap(),
        };
        assert_eq!(
            body["live"], watch_view,
            "API live must match VideoCaptureSettingsWatch"
        );

        let restart = body["pending_restart"].as_array().expect("pending_restart");
        let reindex = body["pending_reindex"].as_array().expect("pending_reindex");
        for path in reindex {
            assert!(
                restart.contains(path),
                "pending_reindex {path} is missing from pending_restart {restart:?}"
            );
        }
        for forbidden in ["videos.fps", "videos.dvr.save_interval"] {
            assert!(
                !restart.iter().any(|value| value == forbidden),
                "{forbidden} must apply live and stay out of pending_restart"
            );
        }
        if let Ok(secret) = std::env::var("A2ALAB_SETTINGS_HTTP_SECRET") {
            assert!(
                !body.to_string().contains(&secret),
                "resolved credentials must not appear in the API envelope"
            );
        }
    }
}

fn assert_runtime_not_file(body: &Value) {
    assert_ne!(
        body["document"]["videos"]["sources"][0]["id"], body["live"]["sources"][0]["id"],
        "runtime watch sources stay on the process baseline until restart"
    );
}

#[tokio::test]
async fn writable_updates_keep_file_api_and_watch_aligned() {
    let root = TempRoot::new();
    let path = root.write_config(CADENCE);
    let store = LabConfigStore::open(&path).unwrap();
    let handle = settings_handle();
    let watch_before = handle.snapshot();
    let matrix = Matrix {
        client: reqwest::Client::new(),
        base: serve(store, Some(handle.clone()), false).await,
        path: Some(&path),
        handle: Some(&handle),
        token: None,
    };
    let read = matrix.get().await;
    assert_eq!(read["persistence"], "writable");
    assert_eq!(read["live"]["live_output_fps"], 5);
    assert_eq!(read["live"]["dvr_save_interval"], "1s");
    assert_eq!(read["pending_restart"], json!([]));
    assert_eq!(read["pending_reindex"], json!([]));
    assert_runtime_not_file(&read);
    assert_eq!(read["document"]["videos"]["sources"][0]["id"], "camera");
    assert_eq!(read["live"]["sources"][0]["id"], "bench");
    assert_eq!(handle.snapshot(), watch_before);

    let fps = matrix
        .patch_ok(
            read["revision"].as_str().unwrap(),
            json!({ "videos": { "fps": 12 } }),
        )
        .await;
    assert_eq!(fps["document"]["videos"]["fps"], 12);
    assert_eq!(fps["live"]["live_output_fps"], 12);
    assert_eq!(fps["live"]["dvr_save_interval"], "1s");
    assert_eq!(fps["pending_restart"], json!([]));
    assert_eq!(handle.snapshot().live_output_fps(), 12);
    assert_eq!(
        handle.snapshot().view().dvr_storage_path.as_deref(),
        Some("./dvr")
    );
    assert_eq!(handle.snapshot().view().sources[0].id, "bench");

    let interval = matrix
        .patch_ok(
            fps["revision"].as_str().unwrap(),
            json!({ "videos": { "dvr": { "save_interval": "2s" } } }),
        )
        .await;
    assert_eq!(interval["document"]["videos"]["dvr"]["save_interval"], "2s");
    assert_eq!(interval["live"]["dvr_save_interval"], "2s");
    assert_eq!(interval["live"]["live_output_fps"], 12);
    assert_eq!(interval["pending_restart"], json!([]));
    assert_eq!(
        handle
            .snapshot()
            .dvr_save_interval()
            .map(SaveIntervalDuration::as_text)
            .as_deref(),
        Some("2s")
    );

    let chat = matrix
        .patch_ok(
            interval["revision"].as_str().unwrap(),
            json!({ "chat": { "model": "other" } }),
        )
        .await;
    assert_eq!(chat["document"]["chat"]["model"], "other");
    assert_eq!(chat["live"]["live_output_fps"], 12);
    assert!(
        chat["pending_restart"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "chat.model")
    );
    assert_eq!(chat["pending_reindex"], json!([]));
    assert_eq!(handle.snapshot().live_output_fps(), 12);
    assert_eq!(handle.snapshot().view().sources[0].id, "bench");

    let logging = matrix
        .patch_ok(
            chat["revision"].as_str().unwrap(),
            json!({ "logging": { "level": "debug" } }),
        )
        .await;
    assert_eq!(logging["document"]["logging"]["level"], "debug");
    assert!(
        logging["pending_restart"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "logging.level")
    );
    assert_eq!(handle.snapshot().live_output_fps(), 12);
}

#[tokio::test]
async fn rejected_writes_leave_file_bytes_and_watch_unchanged() {
    let root = TempRoot::new();
    let path = root.write_config(CADENCE);
    let store = LabConfigStore::open(&path).unwrap();
    let handle = settings_handle();
    let matrix = Matrix {
        client: reqwest::Client::new(),
        base: serve(store, Some(handle.clone()), false).await,
        path: Some(&path),
        handle: Some(&handle),
        token: None,
    };
    let read = matrix.get().await;
    let revision = read["revision"].as_str().unwrap().to_owned();
    let before = file_bytes(&path);
    let watch_before = handle.snapshot();

    matrix
        .patch_rejected(
            &revision,
            json!({ "videos": { "fps": 0 } }),
            400,
            "invalid",
            Some(&before),
            Some(&watch_before),
        )
        .await;
    matrix
        .patch_rejected(
            "sha256:dead",
            json!({ "videos": { "fps": 9 } }),
            409,
            "stale_revision",
            Some(&before),
            Some(&watch_before),
        )
        .await;
}

#[tokio::test]
async fn unconfigured_and_read_only_reject_without_mutating_watch_or_file() {
    let client = reqwest::Client::new();
    let handle = settings_handle();
    let watch_before = handle.snapshot();
    let unconfigured = Matrix {
        client: client.clone(),
        base: serve(LabConfigStore::unconfigured(), Some(handle.clone()), false).await,
        path: None,
        handle: Some(&handle),
        token: None,
    };
    let body = unconfigured.get().await;
    assert_eq!(body["persistence"], "unconfigured");
    assert_eq!(
        body["live"]["live_output_fps"],
        watch_before.live_output_fps()
    );
    unconfigured
        .patch_rejected(
            "sha256:never",
            json!({ "videos": { "fps": 3 } }),
            403,
            "unconfigured",
            None,
            Some(&watch_before),
        )
        .await;

    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let store = LabConfigStore::open(&path).unwrap();
    let handle = settings_handle();
    let watch_before = handle.snapshot();
    let read_only = Matrix {
        client,
        base: serve(store, Some(handle.clone()), false).await,
        path: Some(&path),
        handle: Some(&handle),
        token: None,
    };
    let read = read_only.get().await;
    assert_eq!(read["persistence"], "read_only");
    let before = file_bytes(&path);
    read_only
        .patch_rejected(
            read["revision"].as_str().unwrap(),
            json!({ "videos": { "fps": 30 } }),
            403,
            "read_only",
            Some(&before),
            Some(&watch_before),
        )
        .await;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
}

#[tokio::test]
async fn public_shell_stays_open_while_settings_api_is_protected() {
    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let store = LabConfigStore::open(&path).unwrap();
    let handle = settings_handle();
    let base = serve(store, Some(handle.clone()), true).await;
    let client = reqwest::Client::new();
    let shell = client
        .get(format!("{base}/ui/config/settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(shell.status(), 200);
    let denied = client
        .get(format!("{base}/config/settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
    let matrix = Matrix {
        client,
        base,
        path: Some(&path),
        handle: Some(&handle),
        token: Some("settings-token"),
    };
    let body = matrix.get().await;
    assert_eq!(body["persistence"], "writable");
}

#[tokio::test]
async fn credential_env_names_round_trip_and_secrets_stay_unresolved() {
    unsafe {
        std::env::set_var("A2ALAB_SETTINGS_HTTP_SECRET", SECRET);
    }
    let root = TempRoot::new();
    let path = root.write_config(CREDENTIALS);
    let store = LabConfigStore::open(&path).unwrap();
    let handle = settings_handle();
    let matrix = Matrix {
        client: reqwest::Client::new(),
        base: serve(store, Some(handle.clone()), false).await,
        path: Some(&path),
        handle: Some(&handle),
        token: None,
    };
    let read = matrix.get().await;
    assert_eq!(
        read["document"]["semantic_indexing"]["description"]["credentials"]["env"],
        "A2ALAB_SETTINGS_HTTP_SECRET"
    );
    assert!(!read.to_string().contains(SECRET));
    assert!(
        !String::from_utf8(file_bytes(&path))
            .unwrap()
            .contains(SECRET)
    );

    let updated = matrix
        .patch_ok(
            read["revision"].as_str().unwrap(),
            json!({
                "semantic_indexing": {
                    "description": { "credentials": { "env": "A2ALAB_OTHER_SECRET" } }
                }
            }),
        )
        .await;
    assert_eq!(
        updated["document"]["semantic_indexing"]["description"]["credentials"]["env"],
        "A2ALAB_OTHER_SECRET"
    );
    assert!(
        updated["pending_restart"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "semantic_indexing.description.credentials.env")
    );
    assert_eq!(updated["pending_reindex"], json!([]));
    assert!(!updated.to_string().contains(SECRET));
    unsafe {
        std::env::remove_var("A2ALAB_SETTINGS_HTTP_SECRET");
    }
}

#[tokio::test]
async fn semantic_identity_is_restart_and_reindex_while_invalid_source_overrides_fail_closed() {
    let root = TempRoot::new();
    let path = root.write_config(SEMANTIC_SOURCES);
    let store = LabConfigStore::open(&path).unwrap();
    let handle = settings_handle();
    let watch_before = handle.snapshot();
    let matrix = Matrix {
        client: reqwest::Client::new(),
        base: serve(store, Some(handle.clone()), false).await,
        path: Some(&path),
        handle: Some(&handle),
        token: None,
    };
    let read = matrix.get().await;
    let updated = matrix
        .patch_ok(
            read["revision"].as_str().unwrap(),
            json!({
                "semantic_indexing": {
                    "description": { "model": "luna-2" }
                }
            }),
        )
        .await;
    assert!(
        updated["pending_restart"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "semantic_indexing.description.model")
    );
    assert!(
        updated["pending_reindex"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "semantic_indexing.description.model")
    );
    assert_eq!(
        handle.snapshot().live_output_fps(),
        watch_before.live_output_fps()
    );

    let before = file_bytes(&path);
    let watch_before = handle.snapshot();
    matrix
        .patch_rejected(
            updated["revision"].as_str().unwrap(),
            json!({
                "videos": {
                    "sources": [{
                        "semantic_indexing": { "embedding_model": "other-model" }
                    }]
                }
            }),
            400,
            "invalid",
            Some(&before),
            Some(&watch_before),
        )
        .await;
}

#[tokio::test]
async fn missing_handle_uses_default_live_view_not_the_file() {
    let root = TempRoot::new();
    let path = root.write_config(CADENCE);
    let store = LabConfigStore::open(&path).unwrap();
    let matrix = Matrix {
        client: reqwest::Client::new(),
        base: serve(store, None, false).await,
        path: Some(&path),
        handle: None,
        token: None,
    };
    let body = matrix.get().await;
    assert_eq!(body["document"]["videos"]["fps"], 5);
    assert_eq!(body["live"]["live_output_fps"], DEFAULT_LIVE_OUTPUT_FPS);
    assert!(body["live"]["sources"].as_array().unwrap().is_empty());
    assert_ne!(body["document"]["videos"]["sources"][0]["id"], json!(null));
}
