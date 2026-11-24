//! Persistent config/settings routes for the web shell.

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Map, Value};

use crate::config::{
    ConfigStoreError, LabConfigSnapshot, LabConfigStore, VideoCaptureSettings,
    VideoCaptureSettingsView, VideoCaptureSettingsWatch, VideoConfig, pending_restart_and_reindex,
    strip_typesafe_secret,
};
use crate::error::A2aLabError;
#[cfg(feature = "nokhwa")]
use crate::videos::{CameraEnumerator, NokhwaEnumerator};

/// Path served by the settings route and its HTTP handlers.
pub const CAPTURE_SETTINGS_PATH: &str = "/config/settings";
/// Path used by the settings UI to discover cameras through Nokhwa.
pub const CAMERA_DISCOVERY_PATH: &str = "/config/cameras";

/// Handle shared between tests and [`super::A2aServer`].
#[derive(Clone)]
pub struct VideoCaptureSettingsHandle {
    watch: VideoCaptureSettingsWatch,
}

impl VideoCaptureSettingsHandle {
    /// Wraps mutable capture settings for HTTP handlers.
    #[must_use]
    pub fn new(settings: VideoCaptureSettings) -> Self {
        Self {
            watch: VideoCaptureSettingsWatch::new(settings),
        }
    }

    /// Current snapshot for tests.
    #[must_use]
    pub fn snapshot(&self) -> VideoCaptureSettings {
        self.watch.snapshot()
    }

    /// Subscribes to live settings without blocking the capture thread on writes.
    #[must_use]
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<VideoCaptureSettings> {
        self.watch.subscribe()
    }

    /// Replaces settings and notifies live subscribers (tests and shell).
    pub fn replace(&self, settings: VideoCaptureSettings) {
        self.watch.replace(settings);
    }

    /// Applies a shell patch and publishes the merged snapshot.
    pub fn apply_patch(&self, document: &Value) -> Result<(), A2aLabError> {
        self.watch.apply_patch(document)
    }

    /// Applies persisted fps/save-interval cadence after a successful file write.
    pub fn apply_persisted_cadence(&self, videos: Option<&VideoConfig>) -> Result<(), A2aLabError> {
        self.watch.apply_persisted_cadence(videos)
    }
}

#[derive(Clone)]
struct SettingsState {
    store: LabConfigStore,
    settings: Option<VideoCaptureSettingsHandle>,
    baseline: Value,
}

/// Registers `GET` and `PATCH` handlers for [`CAPTURE_SETTINGS_PATH`].
pub fn router(store: LabConfigStore, settings: Option<VideoCaptureSettingsHandle>) -> Router {
    let baseline = store
        .snapshot()
        .map_or_else(|_| Value::Object(Map::new()), |snapshot| snapshot.document);
    Router::new()
        .route(
            CAPTURE_SETTINGS_PATH,
            get(read_settings).patch(patch_settings),
        )
        .route(CAMERA_DISCOVERY_PATH, get(discover_cameras))
        .with_state(SettingsState {
            store,
            settings,
            baseline,
        })
}

async fn discover_cameras() -> Response {
    #[cfg(feature = "nokhwa")]
    {
        match NokhwaEnumerator.discover() {
            Ok(snapshot) => Json(serde_json::json!({
                "cameras": snapshot.capable().iter().map(|camera| {
                    serde_json::json!({
                        "index": camera.index(),
                        "name": camera.human_name(),
                    })
                }).collect::<Vec<_>>(),
            }))
            .into_response(),
            Err(error) => api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                error.code(),
                &error.to_string(),
            ),
        }
    }
    #[cfg(not(feature = "nokhwa"))]
    {
        api_error(
            StatusCode::NOT_IMPLEMENTED,
            "unsupported",
            "Camera discovery requires the nokhwa feature.",
        )
    }
}

async fn read_settings(State(state): State<SettingsState>) -> Response {
    match state.store.snapshot() {
        Ok(snapshot) => Json(settings_payload(
            &state.baseline,
            &snapshot,
            &live_view(&state),
        ))
        .into_response(),
        Err(error) => store_error(&error),
    }
}

async fn patch_settings(State(state): State<SettingsState>, Json(body): Json<Value>) -> Response {
    let (revision, changes) = match patch_parts(&body) {
        Ok(parts) => parts,
        Err(error) => return api_error(StatusCode::BAD_REQUEST, "invalid", error),
    };
    let proposed = match state.store.propose_patch(revision, changes) {
        Ok(proposed) => proposed,
        Err(error) => return store_error(&error),
    };
    let apply_live = proposed.live_cadence_changed();
    let videos = proposed
        .snapshot()
        .loaded
        .as_ref()
        .and_then(|loaded| loaded.config.videos.as_ref());
    if apply_live && let Some(settings) = state.settings.as_ref() {
        let mut probe = settings.snapshot();
        if let Err(error) = probe.apply_persisted_cadence(videos) {
            return api_error(StatusCode::BAD_REQUEST, error.code(), &error.to_string());
        }
    }
    match state.store.commit_patch(&proposed) {
        Ok(snapshot) => {
            if apply_live && let Some(settings) = state.settings.as_ref() {
                drop(settings.apply_persisted_cadence(videos));
            }
            Json(settings_payload(
                &state.baseline,
                &snapshot,
                &live_view(&state),
            ))
            .into_response()
        }
        Err(error) => store_error(&error),
    }
}

fn patch_parts(body: &Value) -> Result<(&str, &Value), &'static str> {
    let map = body.as_object().ok_or("must be a JSON object")?;
    for key in map.keys() {
        if key != "revision" && key != "changes" {
            return Err("unknown field");
        }
    }
    let revision = map
        .get("revision")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("revision is required")?;
    let changes = map.get("changes").ok_or("changes is required")?;
    if !changes.is_object() && !changes.is_null() {
        return Err("changes must be a JSON object");
    }
    Ok((revision, changes))
}

fn live_view(state: &SettingsState) -> VideoCaptureSettingsView {
    state.settings.as_ref().map_or_else(
        || VideoCaptureSettings::from_video_config(None).view(),
        |settings| settings.snapshot().view(),
    )
}

fn settings_payload(
    baseline: &Value,
    snapshot: &LabConfigSnapshot,
    live: &VideoCaptureSettingsView,
) -> Value {
    let mut document = snapshot.document.clone();
    strip_typesafe_secret(&mut document);
    keep_credential_env_names(&mut document);
    let (pending_restart, pending_reindex) = pending_restart_and_reindex(baseline, &document);
    serde_json::json!({
        "document": document,
        "revision": snapshot.revision,
        "persistence": snapshot.persistence.as_str(),
        "live": live,
        "pending_restart": pending_restart,
        "pending_reindex": pending_reindex,
    })
}

fn keep_credential_env_names(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(credentials) = map.get_mut("credentials") {
                retain_env_name(credentials);
            }
            for nested in map.values_mut() {
                keep_credential_env_names(nested);
            }
        }
        Value::Array(items) => {
            for item in items {
                keep_credential_env_names(item);
            }
        }
        _ => {}
    }
}

fn retain_env_name(credentials: &mut Value) {
    let Value::Object(map) = credentials else {
        *credentials = Value::Object(Map::new());
        return;
    };
    map.retain(|key, value| key == "env" && value.as_str().is_some());
}

fn store_error(error: &ConfigStoreError) -> Response {
    let status = match error {
        ConfigStoreError::StaleRevision => StatusCode::CONFLICT,
        ConfigStoreError::Unconfigured
        | ConfigStoreError::ReadOnly
        | ConfigStoreError::Permission { .. } => StatusCode::FORBIDDEN,
        ConfigStoreError::Invalid(_) | ConfigStoreError::Path { .. } => StatusCode::BAD_REQUEST,
    };
    let message = error.to_string();
    api_error(status, error.kind(), &message)
}

fn api_error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": {
                "code": code,
                "message": message,
            }
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigPersistence;

    #[test]
    fn settings_response_omits_typesafe_api_key() {
        let secret = "sk-settings-typesafe-key";
        let snapshot = LabConfigSnapshot {
            persistence: ConfigPersistence::Writable,
            revision: Some("sha256:ab".to_owned()),
            document: serde_json::json!({
                "schema_version": 1,
                "typesafe": { "api_key": secret }
            }),
            loaded: None,
        };
        let live = VideoCaptureSettings::from_video_config(None).view();
        let payload = settings_payload(&Value::Object(Map::new()), &snapshot, &live);
        let encoded = payload.to_string();
        assert!(!encoded.contains(secret));
        assert!(payload["document"].get("typesafe").is_none());
        assert!(!payload["error"].to_string().contains(secret));
    }
}
