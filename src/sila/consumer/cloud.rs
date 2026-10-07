//! SiLA cloud client endpoint. A server dials this service and we read one property.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::StreamExt;
use prost_reflect::{DynamicMessage, ReflectMessage};
use tokio::sync::{Mutex, mpsc};
use tonic::body::Body;
use tonic::codec::Streaming;
use tonic::server::{Grpc, NamedService, StreamingService};
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use tower_service::Service;

use crate::error::A2aLabError;
use crate::sila::consumer::compile::framework_pool;
use crate::sila::consumer::rpc::DynamicCodec;

const PROPERTY: &str = "org.silastandard/core/SiLAService/v1/Property/ServerName";

/// Listens for one server-initiated connection and returns the server name it reports.
pub(crate) async fn serve_until_server_name(port: u16) -> Result<String, A2aLabError> {
    let result = Arc::new(Mutex::new(None));
    let service = CloudService {
        result: Arc::clone(&result),
    };
    let address = format!("127.0.0.1:{port}")
        .parse()
        .map_err(|error| A2aLabError::protocol(format!("{error}")))?;
    let server = Server::builder().add_service(service).serve(address);
    tokio::pin!(server);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(name) = result.lock().await.clone() {
            return Ok(name);
        }
        if tokio::time::Instant::now() > deadline {
            return Err(A2aLabError::unavailable(
                "server-initiated connection did not return a server name",
            ));
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(200)) => {}
            result = &mut server => {
                result.map_err(|error| A2aLabError::transport(error.to_string()))?;
                break;
            }
        }
    }
    result.lock().await.clone().ok_or_else(|| {
        A2aLabError::unavailable("server-initiated connection closed before a server name")
    })
}

#[derive(Clone)]
struct CloudService {
    result: Arc<Mutex<Option<String>>>,
}

impl NamedService for CloudService {
    const NAME: &'static str = "sila2.org.silastandard.CloudClientEndpoint";
}

impl Service<http::Request<Body>> for CloudService {
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        let result = Arc::clone(&self.result);
        Box::pin(async move {
            let Ok(pool) = framework_pool() else {
                return Ok(
                    Status::internal("SiLA framework descriptors are unavailable").into_http(),
                );
            };
            let pool = pool.clone();
            let Some(request_type) =
                pool.get_message_by_name("sila2.org.silastandard.SiLAServerMessage")
            else {
                return Ok(Status::internal("missing SiLAServerMessage").into_http());
            };
            let mut grpc = Grpc::new(DynamicCodec::new(request_type));
            Ok(grpc.streaming(CloudCall { pool, result }, request).await)
        })
    }
}

struct CloudCall {
    pool: prost_reflect::DescriptorPool,
    result: Arc<Mutex<Option<String>>>,
}

impl StreamingService<DynamicMessage> for CloudCall {
    type Response = DynamicMessage;
    type ResponseStream =
        Pin<Box<dyn futures_util::Stream<Item = Result<DynamicMessage, Status>> + Send>>;
    type Future =
        Pin<Box<dyn Future<Output = Result<Response<Self::ResponseStream>, Status>> + Send>>;

    fn call(&mut self, request: Request<Streaming<DynamicMessage>>) -> Self::Future {
        let pool = self.pool.clone();
        let result = Arc::clone(&self.result);
        let incoming = request.into_inner();
        Box::pin(async move {
            let (sender, receiver) = mpsc::channel(4);
            let outbound =
                property_read(&pool).map_err(|error| Status::internal(error.to_string()))?;
            let open = sender.clone();
            sender
                .send(Ok(outbound))
                .await
                .map_err(|_| Status::internal("cloud stream closed"))?;
            drop(sender);
            tokio::spawn(async move {
                let _open = open;
                let mut incoming = incoming;
                while let Some(item) = incoming.next().await {
                    let Ok(message) = item else {
                        continue;
                    };
                    if let Some(name) = property_text(&message) {
                        *result.lock().await = Some(name);
                        break;
                    }
                }
            });
            let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
                let item = receiver.recv().await?;
                Some((item, receiver))
            });
            let boxed: Pin<
                Box<dyn futures_util::Stream<Item = Result<DynamicMessage, Status>> + Send>,
            > = Box::pin(stream);
            Ok(Response::new(boxed))
        })
    }
}

fn property_read(pool: &prost_reflect::DescriptorPool) -> Result<DynamicMessage, A2aLabError> {
    let read_type = pool
        .get_message_by_name("sila2.org.silastandard.UnobservablePropertyRead")
        .ok_or_else(|| A2aLabError::protocol("missing UnobservablePropertyRead"))?;
    let mut read = DynamicMessage::new(read_type);
    set(&mut read, "fullyQualifiedPropertyId", PROPERTY)?;
    let client_type = pool
        .get_message_by_name("sila2.org.silastandard.SiLAClientMessage")
        .ok_or_else(|| A2aLabError::protocol("missing SiLAClientMessage"))?;
    let mut message = DynamicMessage::new(client_type);
    set(&mut message, "requestUUID", "property-read")?;
    let field = message
        .descriptor()
        .get_field_by_name("unobservablePropertyRead")
        .ok_or_else(|| A2aLabError::protocol("missing unobservablePropertyRead"))?;
    message.set_field(&field, prost_reflect::Value::Message(read));
    Ok(message)
}

fn property_text(message: &DynamicMessage) -> Option<String> {
    let field = message
        .descriptor()
        .get_field_by_name("unobservablePropertyValue")?;
    if !message.has_field(&field) {
        return None;
    }
    let value = message.get_field(&field).as_message().cloned()?;
    let bytes = value
        .descriptor()
        .get_field_by_name("value")
        .and_then(|field| value.get_field(&field).as_bytes().cloned())?;
    let string_type = message
        .descriptor()
        .parent_pool()
        .get_message_by_name("sila2.org.silastandard.String")?;
    let decoded = DynamicMessage::decode(string_type, bytes.as_ref()).ok()?;
    let text = decoded
        .descriptor()
        .get_field_by_name("value")
        .and_then(|field| decoded.get_field(&field).as_str().map(str::to_owned))?;
    if text.is_empty() { None } else { Some(text) }
}

fn set(message: &mut DynamicMessage, name: &str, value: &str) -> Result<(), A2aLabError> {
    let field = message
        .descriptor()
        .get_field_by_name(name)
        .ok_or_else(|| A2aLabError::protocol(format!("missing {name}")))?;
    message.set_field(&field, prost_reflect::Value::String(value.to_owned()));
    Ok(())
}
