//! Hardware-free checks for `/config/settings`.

#![cfg(feature = "web-ui")]

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use a2a_lab_dev_kit::videos::SENSOR_30FPS_PERIOD_NS;
use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, CameraSelector, ConfigDuration, HttpAuthSecurityScheme,
    LabConfigStore, MemoryLogs, MemoryMetrics, MemoryTasks, SaveIntervalDuration, SecurityScheme,
    VideoCaptureSettings, VideoCaptureSettingsHandle, VideoConfig, VideoDvrConfig,
    VideoInputFormat, VideoSourceConfig, VideoSourceId, bind_local,
};
use a2a_types::SecurityRequirement;
use serde_json::{Value, json};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

const ORIGINAL: &str = r#"{"schema_version":1,"chat":{"provider":"openai","model":"keep-me"},"videos":{"sources":[{"id":"camera","format":"yuyv","device_index":6}],"fps":5,"dvr":{"storage_path":"./dvr"}}}"#;

const CADENCE: &str = r#"{"schema_version":1,"videos":{"sources":[{"id":"camera","format":"yuyv","device_index":6}],"fps":5,"live_retention":"30s","dvr":{"save_interval":"1s","retention":"1h","storage_path":"./dvr"}}}"#;

const CREDENTIALS: &str = r#"{"schema_version":2,"semantic_indexing":{"description":{"credentials":{"env":"A2ALAB_SETTINGS_HTTP_SECRET"}},"embeddings":{"provider":"p","model":"m","space":{"dimensions":4,"normalization":"l2_unit","metric":"cosine"}},"limits":{"worker":{"max_batch_items":1,"max_concurrency":1,"max_queue_backlog":1,"max_retry_attempts":1,"job_deadline":"30s","retry_backoff":"5s"},"search":{"top_k":1,"candidate_limit":1,"max_neighbor_frames":1,"query_window":"1h"},"roi":{"min_edge_pixels":280,"max_edge_pixels":1568,"max_rois_per_frame":1},"storage":{"vector_index_path":"./x","max_disk_bytes":1024},"decisions":{"max_batch_items":1,"max_concurrency":1,"job_deadline":"45s","coverage_policy":"on_demand"}}}}"#;

const SECRET: &str = "super-secret-credential-value";

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "a2a-lab-settings-http-{}-{}-{}",
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

fn bytes(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
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

async fn serve_store(
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

#[tokio::test]
async fn settings_shell_route_is_public() {
    let base = serve_store(LabConfigStore::unconfigured(), None, true).await;
    let response = reqwest::get(format!("{base}/ui/config/settings"))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn get_returns_original_document_revision_and_states() {
    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let store = LabConfigStore::open(&path).unwrap();
    let handle = settings_handle();
    let base = serve_store(store, Some(handle), false).await;
    let response = reqwest::Client::new()
        .get(format!("{base}/config/settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body["document"],
        serde_json::from_str::<Value>(ORIGINAL).unwrap()
    );
    assert!(body["document"].get("logging").is_none());
    assert_eq!(body["document"]["videos"]["sources"][0]["format"], "yuyv");
    let revision = body["revision"].as_str().expect("revision");
    assert!(revision.starts_with("sha256:"));
    assert_eq!(revision.len(), "sha256:".len() + 64);
    assert_eq!(body["persistence"], "writable");
    assert_eq!(body["live"]["live_output_fps"], 5);
    assert_eq!(body["live"]["dvr_save_interval"], "1s");
    assert_eq!(body["pending_restart"], json!([]));
    assert_eq!(body["pending_reindex"], json!([]));
}

#[tokio::test]
async fn patch_applies_surgical_revisioned_changes_and_rejects_stale() {
    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let store = LabConfigStore::open(&path).unwrap();
    let base = serve_store(store, None, false).await;
    let client = reqwest::Client::new();
    let read: Value = client
        .get(format!("{base}/config/settings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let revision = read["revision"].as_str().unwrap().to_owned();

    let ok = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({ "revision": revision, "changes": { "videos": { "fps": 12 } } }))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    let updated: Value = ok.json().await.unwrap();
    let on_disk: Value = serde_json::from_slice(&bytes(&path)).unwrap();
    assert_eq!(updated["document"], on_disk);
    assert_eq!(on_disk["videos"]["fps"], 12);
    assert_eq!(on_disk["videos"]["sources"][0]["format"], "yuyv");
    assert_eq!(on_disk["videos"]["sources"][0]["device_index"], 6);
    assert_eq!(on_disk["chat"]["model"], "keep-me");
    assert!(on_disk.get("logging").is_none());
    assert_ne!(updated["revision"].as_str().unwrap(), revision);

    let before = bytes(&path);
    let stale = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({ "revision": revision, "changes": { "videos": { "fps": 9 } } }))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 409);
    let err: Value = stale.json().await.unwrap();
    assert_eq!(err["error"]["code"], "stale_revision");
    assert_eq!(bytes(&path), before);
}

#[tokio::test]
async fn unconfigured_and_read_only_reject_patch_without_mutation() {
    let client = reqwest::Client::new();
    let unconfigured = serve_store(
        LabConfigStore::unconfigured(),
        Some(settings_handle()),
        false,
    )
    .await;
    let get = client
        .get(format!("{unconfigured}/config/settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), 200);
    let body: Value = get.json().await.unwrap();
    assert_eq!(body["persistence"], "unconfigured");
    assert_eq!(body["live"]["live_output_fps"], 5);
    assert!(body["revision"].is_null());
    let rejected = client
        .patch(format!("{unconfigured}/config/settings"))
        .json(&json!({ "revision": "sha256:never", "changes": { "videos": { "fps": 3 } } }))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 403);
    assert_eq!(
        rejected.json::<Value>().await.unwrap()["error"]["code"],
        "unconfigured"
    );

    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let store = LabConfigStore::open(&path).unwrap();
    let base = serve_store(store, None, false).await;
    let read: Value = client
        .get(format!("{base}/config/settings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(read["persistence"], "read_only");
    assert_eq!(read["live"]["live_output_fps"], 5);
    let before = bytes(&path);
    let rejected_write = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({
            "revision": read["revision"],
            "changes": { "videos": { "fps": 30 } }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected_write.status(), 403);
    assert_eq!(
        rejected_write.json::<Value>().await.unwrap()["error"]["code"],
        "read_only"
    );
    assert_eq!(bytes(&path), before);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
}

#[tokio::test]
async fn configured_security_protects_settings_api_not_shell() {
    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let store = LabConfigStore::open(&path).unwrap();
    let base = serve_store(store, None, true).await;
    let client = reqwest::Client::new();
    let shell = client
        .get(format!("{base}/ui/config/settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(shell.status(), 200);
    let denied_get = client
        .get(format!("{base}/config/settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied_get.status(), 401);
    let denied_patch = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({ "revision": "sha256:never", "changes": {} }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied_patch.status(), 401);
    let allowed = client
        .get(format!("{base}/config/settings"))
        .bearer_auth("settings-token")
        .send()
        .await
        .unwrap();
    assert_eq!(allowed.status(), 200);
}

#[tokio::test]
async fn null_removes_keys_and_forbidden_or_malformed_writes_do_not_mutate() {
    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let store = LabConfigStore::open(&path).unwrap();
    let base = serve_store(store, None, false).await;
    let client = reqwest::Client::new();
    let revision = client
        .get(format!("{base}/config/settings"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["revision"]
        .as_str()
        .unwrap()
        .to_owned();

    let removed = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({ "revision": revision, "changes": { "chat": Value::Null } }))
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), 200);
    let after_null: Value = removed.json().await.unwrap();
    assert!(after_null["document"].get("chat").is_none());
    let on_disk: Value = serde_json::from_slice(&bytes(&path)).unwrap();
    assert!(on_disk.get("chat").is_none());
    let revision = after_null["revision"].as_str().unwrap().to_owned();

    let before = bytes(&path);
    let malformed = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({ "videos": { "fps": 12 } }))
        .send()
        .await
        .unwrap();
    assert_eq!(malformed.status(), 400);
    assert_eq!(
        malformed.json::<Value>().await.unwrap()["error"]["code"],
        "invalid"
    );
    assert_eq!(bytes(&path), before);

    let invalid = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({
            "revision": revision,
            "changes": { "chat": { "provider": "not-a-provider" } }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), 400);
    assert_eq!(
        invalid.json::<Value>().await.unwrap()["error"]["code"],
        "invalid"
    );
    assert_eq!(bytes(&path), before);

    let forbidden = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({
            "revision": revision,
            "changes": { "semantic_indexing": { "fallback": true } }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden.status(), 400);
    let err: Value = forbidden.json().await.unwrap();
    assert_eq!(err["error"]["code"], "invalid");
    assert!(!err["error"]["message"].as_str().unwrap().is_empty());
    assert_eq!(bytes(&path), before);
}

#[tokio::test]
async fn credentials_expose_environment_variable_names_only() {
    unsafe {
        std::env::set_var("A2ALAB_SETTINGS_HTTP_SECRET", SECRET);
    }
    let root = TempRoot::new();
    let path = root.write_config(CREDENTIALS);
    let store = LabConfigStore::open(&path).unwrap();
    let base = serve_store(store, None, false).await;
    let body: Value = reqwest::Client::new()
        .get(format!("{base}/config/settings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = body.to_string();
    assert_eq!(
        body["document"]["semantic_indexing"]["description"]["credentials"]["env"],
        "A2ALAB_SETTINGS_HTTP_SECRET"
    );
    assert!(!text.contains(SECRET));
    unsafe {
        std::env::remove_var("A2ALAB_SETTINGS_HTTP_SECRET");
    }
}

async fn patch_ok(base: &str, revision: &str, changes: Value) -> Value {
    let response = reqwest::Client::new()
        .patch(format!("{base}/config/settings"))
        .json(&json!({ "revision": revision, "changes": changes }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.json().await.unwrap()
}

#[tokio::test]
async fn patch_fps_updates_live_watch_without_dvr() {
    let root = TempRoot::new();
    let path = root.write_config(CADENCE);
    let store = LabConfigStore::open(&path).unwrap();
    let handle = settings_handle();
    let base = serve_store(store, Some(handle.clone()), false).await;
    let client = reqwest::Client::new();
    let read: Value = client
        .get(format!("{base}/config/settings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let body = patch_ok(
        &base,
        read["revision"].as_str().unwrap(),
        json!({ "videos": { "fps": 12 } }),
    )
    .await;
    assert_eq!(body["live"]["live_output_fps"], 12);
    assert_eq!(body["live"]["dvr_save_interval"], "1s");
    assert_eq!(body["live"]["dvr_retention_secs"], 3600);
    assert_eq!(body["live"]["live_retention_secs"], 30);
    assert_eq!(body["pending_restart"], json!([]));
    assert_eq!(handle.snapshot().live_output_fps(), 12);
    assert_eq!(
        handle
            .snapshot()
            .dvr_save_interval()
            .map(SaveIntervalDuration::as_text)
            .as_deref(),
        Some("1s")
    );
}

#[tokio::test]
async fn patch_save_interval_updates_live_watch_without_fps() {
    let root = TempRoot::new();
    let path = root.write_config(CADENCE);
    let store = LabConfigStore::open(&path).unwrap();
    let handle = settings_handle();
    let base = serve_store(store, Some(handle.clone()), false).await;
    let client = reqwest::Client::new();
    let read: Value = client
        .get(format!("{base}/config/settings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let body = patch_ok(
        &base,
        read["revision"].as_str().unwrap(),
        json!({ "videos": { "dvr": { "save_interval": "2s" } } }),
    )
    .await;
    assert_eq!(body["live"]["dvr_save_interval"], "2s");
    assert_eq!(body["live"]["live_output_fps"], 5);
    assert_eq!(body["live"]["dvr_retention_secs"], 3600);
    assert_eq!(body["live"]["live_retention_secs"], 30);
    assert_eq!(body["pending_restart"], json!([]));
    assert_eq!(handle.snapshot().live_output_fps(), 5);
    assert_eq!(
        handle
            .snapshot()
            .dvr_save_interval()
            .map(SaveIntervalDuration::as_text)
            .as_deref(),
        Some("2s")
    );
}

#[tokio::test]
async fn other_fields_are_restart_required_and_identity_is_reindex() {
    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let store = LabConfigStore::open(&path).unwrap();
    let handle = settings_handle();
    let live_before = handle.snapshot();
    let base = serve_store(store, Some(handle.clone()), false).await;
    let client = reqwest::Client::new();
    let read: Value = client
        .get(format!("{base}/config/settings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let body = patch_ok(
        &base,
        read["revision"].as_str().unwrap(),
        json!({
            "chat": { "model": "other" },
            "videos": { "live_retention": "45s", "dvr": { "storage_path": "./other" } }
        }),
    )
    .await;
    assert_eq!(body["live"]["live_output_fps"], 5);
    assert_eq!(body["live"]["live_retention_secs"], 30);
    assert_eq!(body["live"]["dvr_storage_path"], "./dvr");
    assert_eq!(
        body["pending_restart"],
        json!([
            "chat.model",
            "videos.dvr.storage_path",
            "videos.live_retention"
        ])
    );
    assert_eq!(body["pending_reindex"], json!([]));
    assert_eq!(handle.snapshot(), live_before);

    let root = TempRoot::new();
    let path = root.write_config(CREDENTIALS);
    let store = LabConfigStore::open(&path).unwrap();
    let handle = settings_handle();
    let live_before = handle.snapshot();
    let base = serve_store(store, Some(handle.clone()), false).await;
    let read: Value = client
        .get(format!("{base}/config/settings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let body = patch_ok(
        &base,
        read["revision"].as_str().unwrap(),
        json!({
            "semantic_indexing": {
                "description": { "model": "luna-2" },
                "embeddings": {
                    "model": "m2",
                    "space": { "dimensions": 8 }
                },
                "limits": { "storage": { "vector_index_path": "./y", "sqlite_path": "./db2" } }
            }
        }),
    )
    .await;
    assert_eq!(
        body["live"]["live_output_fps"],
        live_before.live_output_fps()
    );
    let restart = body["pending_restart"].as_array().unwrap();
    for path in [
        "semantic_indexing.description.model",
        "semantic_indexing.embeddings.model",
        "semantic_indexing.embeddings.space.dimensions",
        "semantic_indexing.limits.storage.vector_index_path",
        "semantic_indexing.limits.storage.sqlite_path",
    ] {
        assert!(
            restart.iter().any(|value| value == path),
            "missing {path} in {restart:?}"
        );
        assert!(
            body["pending_reindex"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == path),
            "missing reindex {path}"
        );
    }
    assert_eq!(
        handle.snapshot().live_output_fps(),
        live_before.live_output_fps()
    );
    assert_eq!(
        handle.snapshot().view().dvr_storage_path,
        live_before.view().dvr_storage_path
    );
}

#[tokio::test]
async fn patch_source_name_is_surgical_and_does_not_go_live() {
    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let store = LabConfigStore::open(&path).unwrap();
    let handle = settings_handle();
    let live_before = handle.snapshot();
    let base = serve_store(store, Some(handle.clone()), false).await;
    let client = reqwest::Client::new();
    let read: Value = client
        .get(format!("{base}/config/settings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let body = patch_ok(
        &base,
        read["revision"].as_str().unwrap(),
        json!({ "videos": { "sources": [{ "name": "Overview" }] } }),
    )
    .await;
    let on_disk: Value = serde_json::from_slice(&bytes(&path)).unwrap();
    assert_eq!(on_disk["videos"]["sources"][0]["id"], "camera");
    assert_eq!(on_disk["videos"]["sources"][0]["format"], "yuyv");
    assert_eq!(on_disk["videos"]["sources"][0]["device_index"], 6);
    assert_eq!(on_disk["videos"]["sources"][0]["name"], "Overview");
    assert_eq!(on_disk["videos"]["dvr"]["storage_path"], "./dvr");
    assert_eq!(body["pending_restart"], json!(["videos.sources.0.name"]));
    assert_eq!(handle.snapshot(), live_before);
}

#[tokio::test]
async fn invalid_or_stale_cadence_leaves_bytes_and_watch_unchanged() {
    let root = TempRoot::new();
    let path = root.write_config(CADENCE);
    let store = LabConfigStore::open(&path).unwrap();
    let handle = settings_handle();
    let before_watch = handle.snapshot();
    let base = serve_store(store, Some(handle.clone()), false).await;
    let client = reqwest::Client::new();
    let read: Value = client
        .get(format!("{base}/config/settings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let revision = read["revision"].as_str().unwrap().to_owned();
    let before = bytes(&path);

    let invalid = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({ "revision": revision, "changes": { "videos": { "fps": 0 } } }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), 400);
    assert_eq!(bytes(&path), before);
    assert_eq!(handle.snapshot(), before_watch);

    let too_fast = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({
            "revision": revision,
            "changes": { "videos": { "dvr": { "save_interval": "10ms" } } }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(too_fast.status(), 400);
    assert_eq!(bytes(&path), before);
    assert_eq!(handle.snapshot(), before_watch);

    let stale = client
        .patch(format!("{base}/config/settings"))
        .json(&json!({ "revision": "sha256:dead", "changes": { "videos": { "fps": 9 } } }))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 409);
    assert_eq!(bytes(&path), before);
    assert_eq!(handle.snapshot(), before_watch);
}
