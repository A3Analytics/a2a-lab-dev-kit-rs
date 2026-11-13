//! Public config/settings routes for the web shell.

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::Value;

use crate::config::{VideoCaptureSettings, VideoCaptureSettingsWatch};
use crate::error::A2aLabError;

/// Path served by the settings route and its HTTP handlers.
pub const CAPTURE_SETTINGS_PATH: &str = "/config/settings";

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
}

#[derive(Clone)]
struct SettingsState {
    settings: VideoCaptureSettingsHandle,
}

/// Registers `GET` and `PATCH` handlers for [`CAPTURE_SETTINGS_PATH`].
pub fn router(handle: VideoCaptureSettingsHandle) -> Router {
    Router::new()
        .route(
            CAPTURE_SETTINGS_PATH,
            get(read_settings).patch(patch_settings),
        )
        .with_state(SettingsState { settings: handle })
}

/// Returns true when `method` and `path` are the public settings route.
#[must_use]
pub fn is_public_route(method: &axum::http::Method, path: &str) -> bool {
    path == CAPTURE_SETTINGS_PATH
        && (*method == axum::http::Method::GET || *method == axum::http::Method::PATCH)
}

async fn read_settings(State(state): State<SettingsState>) -> impl IntoResponse {
    Json(state.settings.snapshot().view())
}

async fn patch_settings(State(state): State<SettingsState>, Json(body): Json<Value>) -> Response {
    match state.settings.apply_patch(&body) {
        Ok(()) => Json(serde_json::json!({
            "view": state.settings.snapshot().view(),
        }))
        .into_response(),
        Err(error) => settings_error(&error),
    }
}

fn settings_error(error: &A2aLabError) -> Response {
    let status = match &error {
        A2aLabError::Invalid { .. } => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let body = serde_json::json!({
        "error": {
            "code": error.code(),
            "message": error.to_string(),
        }
    });
    (status, Json(body)).into_response()
}
