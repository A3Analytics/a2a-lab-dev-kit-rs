//! Public config/settings routes for the web shell.

use std::sync::{Arc, RwLock};

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::Value;

use crate::config::VideoCaptureSettings;
use crate::error::A2aLabError;

/// Path served by the settings route and its HTTP handlers.
pub const CAPTURE_SETTINGS_PATH: &str = "/config/settings";

/// Handle shared between tests and [`super::A2aServer`].
#[derive(Clone)]
pub struct VideoCaptureSettingsHandle {
    inner: Arc<RwLock<VideoCaptureSettings>>,
}

impl VideoCaptureSettingsHandle {
    /// Wraps mutable capture settings for HTTP handlers.
    #[must_use]
    pub fn new(settings: VideoCaptureSettings) -> Self {
        Self {
            inner: Arc::new(RwLock::new(settings)),
        }
    }

    /// Current snapshot for tests.
    pub fn snapshot(&self) -> VideoCaptureSettings {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
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
    let settings = state
        .settings
        .inner
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Json(settings.view())
}

async fn patch_settings(State(state): State<SettingsState>, Json(body): Json<Value>) -> Response {
    let mut settings = state
        .settings
        .inner
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match settings.apply_patch(&body) {
        Ok(()) => Json(serde_json::json!({
            "view": settings.view(),
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
