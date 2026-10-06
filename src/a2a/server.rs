//! A2A HTTP+JSON, JSON-RPC, and gRPC server.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use a2a_grpc::server::GrpcHandler;
use a2a_pb::proto::a2a_service_server::A2aServiceServer;
use a2a_server::{
    DefaultRequestHandler, HttpPushSender, HttpPushSenderConfig, InMemoryPushConfigStore,
    StaticAgentCard,
};
use a2a_types::{AgentCapabilities, AgentCard, SecurityRequirement, SecurityScheme};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::Router;
use axum::extract::Request;
use axum::http::header::HeaderMap;
use axum::http::header::{CONTENT_TYPE, HeaderValue};
use axum::http::{Request as HttpRequest, Response as HttpResponse, StatusCode};
use axum::middleware::{Next, from_fn};
use axum::response::Response;
use futures_util::stream::{Stream, unfold};
use http_body::Body;
use tokio::net::{TcpListener, TcpStream};
use tower_layer::Layer;
use tower_service::Service;

use crate::error::A2aLabError;
use crate::service::LabApi;

use super::access::{
    AccessError, AccessGate, OwnedTaskStore, auth_response, grpc_auth_status,
    security_configuration_error, with_caller,
};
use super::auth::Authenticator;
use super::card::{accepted_modes, agent_card};
use super::executor::LabExecutor;
use super::message::AgentMessageHandler;

const DEFAULT_ADDRESS: &str = "127.0.0.1:31000";

/// A2A server for a lab service.
///
/// HTTP+JSON and JSON-RPC share one listener. gRPC listens on a second socket.
pub struct A2aServer {
    lab: Arc<dyn LabApi>,
    public_url: Option<String>,
    grpc_host: Option<String>,
    push_notifications: bool,
    loopback_push: bool,
    extended_card: Option<AgentCard>,
    security_schemes: Option<HashMap<String, SecurityScheme>>,
    security_requirements: Option<Vec<SecurityRequirement>>,
    authenticator: Option<Arc<dyn Authenticator>>,
    messages: Option<Arc<dyn AgentMessageHandler>>,
}

impl A2aServer {
    /// Creates a server that dispatches to `lab`.
    #[must_use]
    pub fn new(lab: &Arc<dyn LabApi>) -> Self {
        Self {
            lab: Arc::clone(lab),
            public_url: None,
            grpc_host: None,
            push_notifications: false,
            loopback_push: false,
            extended_card: None,
            security_schemes: None,
            security_requirements: None,
            authenticator: None,
            messages: None,
        }
    }

    /// Answers plain-text messages with `handler` and advertises `agent-message`.
    #[must_use]
    pub fn with_message_handler(mut self, handler: Arc<dyn AgentMessageHandler>) -> Self {
        self.messages = Some(handler);
        self
    }

    /// Overrides the Agent Card interface URL.
    #[must_use]
    pub fn with_public_url(mut self, url: impl Into<String>) -> Self {
        self.public_url = Some(url.into());
        self
    }

    /// Binds gRPC on `0.0.0.0` and advertises `host` on the Agent Card.
    ///
    /// Used when the TCK runs in a container and must dial the host.
    #[must_use]
    pub fn with_grpc_host(mut self, host: impl Into<String>) -> Self {
        self.grpc_host = Some(host.into());
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

    /// Validates bearer tokens for OAuth2, OpenID Connect, and HTTP bearer schemes.
    #[must_use]
    pub fn with_authenticator(mut self, authenticator: Arc<dyn Authenticator>) -> Self {
        self.authenticator = Some(authenticator);
        self
    }

    /// Serves the Agent Card, HTTP+JSON routes, JSON-RPC `POST /`, and gRPC.
    ///
    /// `listener` defaults to `127.0.0.1:31000` when it is `None`. gRPC binds
    /// `127.0.0.1:0`. The Agent Card `GRPC` interface URL is that socket.
    pub async fn listen(self, listener: impl Into<Option<TcpListener>>) -> Result<(), A2aLabError> {
        security_configuration_error(
            self.security_schemes.as_ref().unwrap_or(&HashMap::new()),
            self.security_requirements.as_deref().unwrap_or(&[]),
            self.authenticator.is_some(),
        )
        .map_err(|message| A2aLabError::invalid("security", message))?;
        if let Some(authenticator) = &self.authenticator {
            authenticator
                .warm()
                .await
                .map_err(|error| A2aLabError::unavailable(error.to_string()))?;
        }
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
        let (grpc_listener, grpc_address) = bind_grpc(self.grpc_host.as_deref()).await?;
        let grpc_url = format!("http://{grpc_address}");
        let (app, handler, gate) = router(&self, &public_url, &grpc_url);
        let grpc = tokio::spawn(serve_grpc(grpc_listener, handler, gate));
        tokio::pin!(grpc);
        tokio::select! {
            result = axum::serve(listener, app) => {
                grpc.abort();
                result.map_err(|error| A2aLabError::transport(error.to_string()))
            }
            result = &mut grpc => match result {
                Ok(result) => result,
                Err(error) => Err(A2aLabError::transport(error.to_string())),
            },
        }
    }
}

async fn bind_grpc(host: Option<&str>) -> Result<(TcpListener, String), A2aLabError> {
    let Some(host) = host else {
        let (listener, address) = bind_local().await?;
        return Ok((listener, address.to_string()));
    };
    let listener = TcpListener::bind("0.0.0.0:0")
        .await
        .map_err(|error| A2aLabError::transport(error.to_string()))?;
    let port = listener
        .local_addr()
        .map_err(|error| A2aLabError::transport(error.to_string()))?
        .port();
    Ok((listener, format!("{host}:{port}")))
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

fn router(
    server: &A2aServer,
    public_url: &str,
    grpc_url: &str,
) -> (Router, Arc<DefaultRequestHandler>, AccessGate) {
    let extended = server.extended_card.is_some();
    let gate = AccessGate::new(
        server.security_schemes.clone(),
        server.security_requirements.clone(),
        server.authenticator.clone(),
    );
    let card = agent_card(
        public_url,
        grpc_url,
        server.push_notifications,
        extended,
        server.security_schemes.clone(),
        server.security_requirements.clone(),
        server.messages.is_some(),
    );
    let capabilities = AgentCapabilities {
        streaming: Some(true),
        push_notifications: Some(server.push_notifications),
        extensions: None,
        extended_agent_card: Some(extended),
    };
    let mut handler = DefaultRequestHandler::new(
        LabExecutor::new(Arc::clone(&server.lab), server.messages.clone()),
        OwnedTaskStore::new(gate.enforced()),
    )
    .with_authorizer(gate.clone());
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
    handler = handler
        .with_capabilities(capabilities)
        .with_default_input_modes(accepted_modes());
    let handler = Arc::new(handler);
    let http_gate = gate.clone();
    let app = Router::new()
        .merge(a2a_server::agent_card::agent_card_router(Arc::new(
            StaticAgentCard::new(card),
        )))
        .merge(a2a_server::rest::rest_router(Arc::clone(&handler)))
        .merge(a2a_server::jsonrpc::jsonrpc_router(Arc::clone(&handler)))
        .layer(from_fn(move |request: Request, next: Next| {
            let gate = http_gate.clone();
            async move {
                if request.uri().path() == "/.well-known/agent-card.json" {
                    return normalize_a2a_json(request, next).await;
                }
                match gate
                    .authenticate(request.headers(), request.uri().query())
                    .await
                {
                    Ok(caller) => with_caller(caller, normalize_a2a_json(request, next)).await,
                    Err(error) => auth_response(&error),
                }
            }
        }));
    (app, handler, gate)
}

fn grpc_incoming(listener: TcpListener) -> impl Stream<Item = std::io::Result<TcpStream>> + Send {
    unfold(listener, |listener| async move {
        let accepted = listener.accept().await.map(|(stream, _)| stream);
        Some((accepted, listener))
    })
}

async fn serve_grpc(
    listener: TcpListener,
    handler: Arc<DefaultRequestHandler>,
    gate: AccessGate,
) -> Result<(), A2aLabError> {
    tonic::transport::Server::builder()
        .layer(GrpcStatusLayer { gate })
        .add_service(A2aServiceServer::new(GrpcHandler::new(handler)))
        .serve_with_incoming(grpc_incoming(listener))
        .await
        .map_err(|error| A2aLabError::transport(error.to_string()))
}

/// The pinned TCK maps these A2A errors to gRPC `UNIMPLEMENTED`. `a2a-grpc`
/// emits `FAILED_PRECONDITION` for the same reasons.
fn unimplemented_grpc_message(message: &str) -> bool {
    message.contains("push notification not supported")
        || message.contains("version not supported")
        || message.contains("terminal state")
        || message.contains("does not declare streaming")
}

fn remap_grpc_status(headers: &mut HeaderMap) {
    let Some(status) = headers
        .get("grpc-status")
        .and_then(|value| value.to_str().ok())
    else {
        return;
    };
    if status != "9" {
        return;
    }
    let message = headers
        .get("grpc-message")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if unimplemented_grpc_message(&super::access::percent_decode(message)) {
        headers.insert("grpc-status", HeaderValue::from_static("12"));
    }
}

#[derive(Clone)]
struct GrpcStatusLayer {
    gate: AccessGate,
}

impl<S> Layer<S> for GrpcStatusLayer {
    type Service = GrpcStatusService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GrpcStatusService {
            inner,
            gate: self.gate.clone(),
        }
    }
}

#[derive(Clone)]
struct GrpcStatusService<S> {
    inner: S,
    gate: AccessGate,
}

impl<S, ReqBody, ResBody> Service<HttpRequest<ReqBody>> for GrpcStatusService<S>
where
    S: Service<HttpRequest<ReqBody>, Response = HttpResponse<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Body + Unpin + Send + 'static,
{
    type Response = HttpResponse<GatedBody<ResBody>>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: HttpRequest<ReqBody>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let gate = self.gate.clone();
        Box::pin(async move {
            match gate
                .authenticate(request.headers(), request.uri().query())
                .await
            {
                Ok(caller) => {
                    let response = with_caller(caller, inner.call(request)).await?;
                    let (mut parts, body) = response.into_parts();
                    remap_grpc_status(&mut parts.headers);
                    Ok(HttpResponse::from_parts(
                        parts,
                        GatedBody::Inner(RemapBody { inner: body }),
                    ))
                }
                Err(error) => Ok(grpc_auth_response(&error)),
            }
        })
    }
}

fn grpc_auth_response<B>(error: &AccessError) -> HttpResponse<GatedBody<B>> {
    let (code, message) = grpc_auth_status(error);
    let mut response = HttpResponse::new(GatedBody::Empty);
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert("content-type", HeaderValue::from_static("application/grpc"));
    if let Ok(status) = HeaderValue::from_str(code) {
        response.headers_mut().insert("grpc-status", status);
    }
    if let Ok(message) = HeaderValue::from_str(&message) {
        response.headers_mut().insert("grpc-message", message);
    }
    response
}

enum GatedBody<B> {
    Empty,
    Inner(RemapBody<B>),
}

struct RemapBody<B> {
    inner: B,
}

impl<B> Body for GatedBody<B>
where
    B: Body + Unpin,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            GatedBody::Empty => Poll::Ready(None),
            GatedBody::Inner(body) => Pin::new(body).poll_frame(cx),
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            GatedBody::Empty => true,
            GatedBody::Inner(body) => body.is_end_stream(),
        }
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match self {
            GatedBody::Empty => http_body::SizeHint::with_exact(0),
            GatedBody::Inner(body) => body.size_hint(),
        }
    }
}

impl<B> Body for RemapBody<B>
where
    B: Body + Unpin,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let body = self.get_mut();
        match Pin::new(&mut body.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => Poll::Ready(Some(Ok(remap_frame(frame)))),
            other => other,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}

fn remap_frame<T>(frame: http_body::Frame<T>) -> http_body::Frame<T> {
    match frame.into_trailers() {
        Ok(mut trailers) => {
            remap_grpc_status(&mut trailers);
            http_body::Frame::trailers(trailers)
        }
        Err(frame) => frame,
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
    if let Some(response) = reject_content_type(&request) {
        return response;
    }
    if request.headers().get("a2a-version").is_none()
        && let Some(version) = query_version(request.uri().query())
        && let Ok(header) = HeaderValue::from_str(&version)
    {
        request.headers_mut().insert("a2a-version", header);
    }
    let path = request.uri().path().to_owned();
    let mut response = remap_rest_error(next.run(request).await).await;
    if path == "/.well-known/agent-card.json"
        && response.headers().get("last-modified").is_none()
        && let Ok(value) = HeaderValue::from_str(&http_date())
    {
        response.headers_mut().insert("last-modified", value);
    }
    response
}

fn http_date() -> String {
    jiff::Timestamp::now()
        .strftime("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
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

fn reject_content_type(request: &Request) -> Option<Response> {
    let value = request.headers().get(CONTENT_TYPE)?;
    let content_type = value.to_str().ok()?;
    if content_type.starts_with("application/json") {
        return None;
    }
    if request.uri().path() == "/" {
        return Some(jsonrpc_content_type_error());
    }
    Some(rest_content_type_error())
}

fn rest_content_type_error() -> Response {
    json_response(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        &serde_json::json!({
            "error": {
                "code": 415,
                "status": "INVALID_ARGUMENT",
                "message": "content type not supported",
                "details": [{
                    "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                    "reason": "CONTENT_TYPE_NOT_SUPPORTED",
                    "domain": "a2a-protocol.org"
                }]
            }
        }),
    )
}

fn jsonrpc_content_type_error() -> Response {
    json_response(
        StatusCode::OK,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": -32005,
                "message": "content type not supported",
                "data": [{
                    "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                    "reason": "CONTENT_TYPE_NOT_SUPPORTED",
                    "domain": "a2a-protocol.org"
                }]
            }
        }),
    )
}

fn json_response(status: StatusCode, body: &serde_json::Value) -> Response {
    let mut response = Response::new(axum::body::Body::from(body.to_string()));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
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
