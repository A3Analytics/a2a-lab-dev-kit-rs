//! A2A HTTP+JSON server.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::stream;
use tokio::net::TcpListener;

use crate::error::SdkError;
use crate::service::{LabApi, LabOutcome, TaskSnapshot};

use super::card::agent_card;
use super::wire::{self, ApiErrorBody, SendRequest};

const POLL: Duration = Duration::from_millis(25);
const DEFAULT_ADDRESS: &str = "127.0.0.1:31000";

struct App {
    lab: Arc<dyn LabApi>,
    public_url: String,
}

/// HTTP+JSON A2A server for a lab service.
pub struct A2aServer {
    lab: Arc<dyn LabApi>,
}

impl A2aServer {
    /// Creates a server that dispatches to `lab`.
    ///
    /// The server stores its own handle to the same service.
    #[must_use]
    pub fn new(lab: &Arc<dyn LabApi>) -> Self {
        Self {
            lab: Arc::clone(lab),
        }
    }

    /// Serves the Agent Card, message send, task lookup, and task subscription routes.
    ///
    /// `listener` defaults to `127.0.0.1:31000` when it is `None`.
    pub async fn listen(self, listener: impl Into<Option<TcpListener>>) -> Result<(), SdkError> {
        let listener = match listener.into() {
            Some(listener) => listener,
            None => TcpListener::bind(DEFAULT_ADDRESS)
                .await
                .map_err(|error| SdkError::transport(error.to_string()))?,
        };
        let address = listener
            .local_addr()
            .map_err(|error| SdkError::transport(error.to_string()))?;
        let app = router(App {
            lab: self.lab,
            public_url: format!("http://{address}"),
        });
        axum::serve(listener, app)
            .await
            .map_err(|error| SdkError::transport(error.to_string()))
    }
}

/// Returns the socket address selected for `127.0.0.1:0`.
pub async fn bind_local() -> Result<(TcpListener, SocketAddr), SdkError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| SdkError::transport(error.to_string()))?;
    let address = listener
        .local_addr()
        .map_err(|error| SdkError::transport(error.to_string()))?;
    Ok((listener, address))
}

fn router(app: App) -> Router {
    let state = Arc::new(app);
    Router::new()
        .route("/.well-known/agent-card.json", get(card))
        .route("/message:send", post(send))
        .route("/message/send", post(send))
        .route("/tasks/{id}", get(task))
        .route("/tasks/{id}/subscribe", get(subscribe))
        .with_state(state)
}

async fn card(State(app): State<Arc<App>>) -> Json<serde_json::Value> {
    Json(agent_card(&app.public_url))
}

async fn send(
    State(app): State<Arc<App>>,
    body: Result<Json<SendRequest>, JsonRejection>,
) -> Response {
    let request = match body {
        Ok(Json(request)) => request,
        Err(error) => return error_response(&SdkError::protocol(error.to_string())),
    };
    let command = match wire::command_from_request(request) {
        Ok(command) => command,
        Err(error) => return error_response(&error),
    };
    match app.lab.execute(command).await {
        Ok(outcome) => task_response(&outcome),
        Err(error) => error_response(&error),
    }
}

async fn task(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    if let Some(task_id) = id.strip_suffix(":subscribe") {
        return subscribe(State(app), Path(task_id.to_owned())).await;
    }
    match app.lab.task(&id).await {
        Ok(snapshot) => task_response(&LabOutcome { task: snapshot }),
        Err(error) => error_response(&error),
    }
}

async fn subscribe(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    let snapshot = match app.lab.task(&id).await {
        Ok(snapshot) => snapshot,
        Err(error) => return error_response(&error),
    };
    let lab = Arc::clone(&app.lab);
    let stream = stream::unfold(Loop::Start(snapshot), move |state| {
        let lab = Arc::clone(&lab);
        async move { next_event(lab, state).await }
    });
    Sse::new(stream).into_response()
}

enum Loop {
    Start(TaskSnapshot),
    Status(TaskSnapshot),
    Artifacts(TaskSnapshot, usize),
    Poll(TaskSnapshot),
}

async fn next_event(
    lab: Arc<dyn LabApi>,
    state: Loop,
) -> Option<(Result<Event, Infallible>, Loop)> {
    // Axum's SSE stream expects each item to be a result. Serialization here cannot fail.
    match state {
        Loop::Start(snapshot) => Some((
            Ok(sse("task", &wire::task_response(&snapshot).ok()?)),
            Loop::Status(snapshot),
        )),
        Loop::Status(snapshot) => Some((
            Ok(sse("statusUpdate", &wire::status_event(&snapshot))),
            Loop::Artifacts(snapshot, 0),
        )),
        Loop::Artifacts(snapshot, index) => {
            let events = wire::artifact_events(&snapshot).ok()?;
            if index >= events.len() {
                return if snapshot.state.is_terminal() {
                    None
                } else {
                    Some((
                        Ok(sse("statusUpdate", &wire::status_event(&snapshot))),
                        Loop::Poll(snapshot),
                    ))
                };
            }
            let event = Ok(sse("artifactUpdate", &events[index]));
            Some((event, Loop::Artifacts(snapshot, index + 1)))
        }
        Loop::Poll(previous) => {
            tokio::time::sleep(POLL).await;
            let snapshot = lab.task(&previous.id).await.ok()?;
            if snapshot.state == previous.state {
                return Some((
                    Ok(sse("statusUpdate", &wire::status_event(&snapshot))),
                    Loop::Poll(snapshot),
                ));
            }
            Some((
                Ok(sse("statusUpdate", &wire::status_event(&snapshot))),
                Loop::Artifacts(snapshot, 0),
            ))
        }
    }
}

fn sse(event: &str, value: &impl serde::Serialize) -> Event {
    let data = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_owned());
    Event::default().event(event).data(data)
}

fn task_response(outcome: &LabOutcome) -> Response {
    match wire::task_response(&outcome.task) {
        Ok(body) => Json(body).into_response(),
        Err(error) => error_response(&error),
    }
}

fn error_response(error: &SdkError) -> Response {
    let status = match error {
        SdkError::Invalid { .. } | SdkError::Protocol { .. } => StatusCode::BAD_REQUEST,
        SdkError::NotFound { .. } => StatusCode::NOT_FOUND,
        SdkError::Unavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
        SdkError::Transport { .. } => StatusCode::BAD_GATEWAY,
    };
    (status, Json(ApiErrorBody::new(error))).into_response()
}
