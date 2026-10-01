//! A2A HTTP+JSON server.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use a2a_server::{
    DefaultRequestHandler, HttpPushSender, HttpPushSenderConfig, InMemoryPushConfigStore,
    InMemoryTaskStore, RequestAuthorizer, ServiceParams, StaticAgentCard,
};
use a2a_types::{A2AError, AgentCapabilities, AgentCard, SecurityRequirement, SecurityScheme};
use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::http::header::{CONTENT_TYPE, HeaderValue};
use axum::middleware::{Next, from_fn};
use axum::response::Response;
use tokio::net::TcpListener;

use crate::error::A2aLabError;
use crate::service::LabApi;

use super::card::agent_card;
use super::executor::LabExecutor;

const DEFAULT_ADDRESS: &str = "127.0.0.1:31000";

/// HTTP+JSON A2A server for a lab service.
pub struct A2aServer {
    lab: Arc<dyn LabApi>,
    public_url: Option<String>,
    push_notifications: bool,
    loopback_push: bool,
    extended_card: Option<AgentCard>,
    security_schemes: Option<HashMap<String, SecurityScheme>>,
    security_requirements: Option<Vec<SecurityRequirement>>,
}

impl A2aServer {
    /// Creates a server that dispatches to `lab`.
    #[must_use]
    pub fn new(lab: &Arc<dyn LabApi>) -> Self {
        Self {
            lab: Arc::clone(lab),
            public_url: None,
            push_notifications: false,
            loopback_push: false,
            extended_card: None,
            security_schemes: None,
            security_requirements: None,
        }
    }

    /// Overrides the Agent Card interface URL.
    #[must_use]
    pub fn with_public_url(mut self, url: impl Into<String>) -> Self {
        self.public_url = Some(url.into());
        self
    }

    /// Advertises and serves push notification config routes.
    #[must_use]
    pub fn with_push_notifications(mut self) -> Self {
        self.push_notifications = true;
        self
    }

    /// Allows push webhooks on loopback addresses. For tests.
    #[must_use]
    pub fn with_loopback_push(mut self) -> Self {
        self.push_notifications = true;
        self.loopback_push = true;
        self
    }

    /// Serves `card` at `GET /extendedAgentCard`.
    #[must_use]
    pub fn with_extended_card(mut self, card: AgentCard) -> Self {
        self.extended_card = Some(card);
        self
    }

    /// Declares Agent Card security schemes and requirements.
    #[must_use]
    pub fn with_security(
        mut self,
        schemes: HashMap<String, SecurityScheme>,
        requirements: Vec<SecurityRequirement>,
    ) -> Self {
        self.security_schemes = Some(schemes);
        self.security_requirements = Some(requirements);
        self
    }

    /// Serves the Agent Card and the A2A 1.0 HTTP+JSON routes.
    ///
    /// `listener` defaults to `127.0.0.1:31000` when it is `None`.
    pub async fn listen(self, listener: impl Into<Option<TcpListener>>) -> Result<(), A2aLabError> {
        let listener = match listener.into() {
            Some(listener) => listener,
            None => TcpListener::bind(DEFAULT_ADDRESS)
                .await
                .map_err(|error| A2aLabError::transport(error.to_string()))?,
        };
        let address = listener
            .local_addr()
            .map_err(|error| A2aLabError::transport(error.to_string()))?;
        let public_url = self
            .public_url
            .clone()
            .unwrap_or_else(|| format!("http://{address}"));
        axum::serve(listener, router(&self, &public_url))
            .await
            .map_err(|error| A2aLabError::transport(error.to_string()))
    }
}

/// Returns the socket address selected for `127.0.0.1:0`.
pub async fn bind_local() -> Result<(TcpListener, SocketAddr), A2aLabError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| A2aLabError::transport(error.to_string()))?;
    let address = listener
        .local_addr()
        .map_err(|error| A2aLabError::transport(error.to_string()))?;
    Ok((listener, address))
}

fn router(server: &A2aServer, public_url: &str) -> Router {
    let extended = server.extended_card.is_some();
    let card = agent_card(
        public_url,
        server.push_notifications,
        extended,
        server.security_schemes.clone(),
        server.security_requirements.clone(),
    );
    let capabilities = AgentCapabilities {
        streaming: Some(true),
        push_notifications: Some(server.push_notifications),
        extensions: None,
        extended_agent_card: Some(extended),
    };
    let mut handler = DefaultRequestHandler::new(
        LabExecutor::new(Arc::clone(&server.lab)),
        InMemoryTaskStore::new(),
    )
    .with_authorizer(VersionAuthorizer);
    if server.push_notifications {
        handler = if server.loopback_push {
            handler.with_push_notifications(
                InMemoryPushConfigStore::new(),
                HttpPushSender::new(Some(HttpPushSenderConfig {
                    validate_urls: false,
                    ..HttpPushSenderConfig::default()
                })),
            )
        } else {
            handler.with_push_config_store(InMemoryPushConfigStore::new())
        };
    }
    if let Some(extended_card) = server.extended_card.clone() {
        handler = handler.with_extended_agent_card(extended_card);
    }
    handler = handler.with_capabilities(capabilities);
    Router::new()
        .merge(a2a_server::agent_card::agent_card_router(Arc::new(
            StaticAgentCard::new(card),
        )))
        .merge(a2a_server::rest::rest_router(Arc::new(handler)))
        .layer(from_fn(normalize_a2a_json))
}

struct VersionAuthorizer;

impl RequestAuthorizer for VersionAuthorizer {
    fn authorize(&self, params: &ServiceParams, _task_id: Option<&str>) -> Result<(), A2AError> {
        let requested = params
            .get("a2a-version")
            .and_then(|values| values.first())
            .map(String::as_str);
        match requested {
            None => Ok(()),
            Some(version)
                if version
                    .trim()
                    .split('.')
                    .next()
                    .and_then(|part| part.parse::<u32>().ok())
                    == Some(1) =>
            {
                Ok(())
            }
            Some(version) => Err(A2AError::version_not_supported(version)),
        }
    }
}

async fn normalize_a2a_json(mut request: Request, next: Next) -> Response {
    if let Some(value) = request.headers().get(CONTENT_TYPE)
        && value
            .to_str()
            .is_ok_and(|content_type| content_type.starts_with("application/a2a+json"))
    {
        request
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }
    if request.headers().get("a2a-version").is_none()
        && let Some(version) = query_version(request.uri().query())
        && let Ok(header) = HeaderValue::from_str(&version)
    {
        request.headers_mut().insert("a2a-version", header);
    }
    remap_rest_error(next.run(request).await).await
}

async fn remap_rest_error(response: Response) -> Response {
    if response.status().is_success() {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, 1_048_576).await else {
        return Response::from_parts(parts, axum::body::Body::empty());
    };
    let mut bytes = bytes.to_vec();
    if let Some((status, patched)) = rest_error_patch(&bytes) {
        parts.status = status;
        bytes = patched;
    }
    if parts
        .headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| content_type.starts_with("application/problem+json"))
    {
        parts
            .headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }
    Response::from_parts(parts, axum::body::Body::from(bytes))
}

fn rest_error_patch(bytes: &[u8]) -> Option<(StatusCode, Vec<u8>)> {
    let mut body: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let reason = body
        .pointer("/error/details")
        .and_then(serde_json::Value::as_array)
        .and_then(|details| details.last())
        .and_then(|detail| detail.get("reason"))
        .and_then(serde_json::Value::as_str)?;
    let status = match reason {
        "TASK_NOT_CANCELABLE" => StatusCode::CONFLICT,
        "CONTENT_TYPE_NOT_SUPPORTED" => StatusCode::UNSUPPORTED_MEDIA_TYPE,
        _ => return None,
    };
    if let Some(code) = body.pointer_mut("/error/code") {
        *code = serde_json::json!(status.as_u16());
    }
    Some((status, serde_json::to_vec(&body).ok()?))
}

fn query_version(query: Option<&str>) -> Option<String> {
    query.and_then(|query| {
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            key.eq_ignore_ascii_case("A2A-Version")
                .then(|| value.replace("%2E", "."))
        })
    })
}
