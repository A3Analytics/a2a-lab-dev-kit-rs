//! Server-initiated SiLA connection through `CloudClientEndpoint`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Context, Poll};
use std::time::Duration;

use prost::Message;
use tokio::sync::{mpsc, watch};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};
use tonic::{Request, Status, Streaming};

use crate::sila::connection::Hub;
use crate::sila::wire::sila2::com::a3analytics::lab::laboperations::v1::lab_operations_server::LabOperations;
use crate::sila::wire::sila2::org::silastandard::core::commands::cancelcontroller::v1::cancel_controller_server::CancelController;
use crate::sila::wire::sila2::org::silastandard::core::connectionconfigurationservice::v1::connection_configuration_service_server::ConnectionConfigurationService;
use crate::sila::wire::sila2::org::silastandard::core::silaservice::v1::si_la_service_server::SiLaService;
use crate::sila::errors::{error_of, framework};
use crate::sila::wire::sila2::org::silastandard::cloud_client_endpoint_client::CloudClientEndpointClient;
use crate::sila::wire::sila2::org::silastandard::framework_error::ErrorType;
use crate::sila::wire::sila2::org::silastandard::si_la_client_message::Message as ClientMessage;
use crate::sila::wire::sila2::org::silastandard::si_la_server_message::Message as ServerMessage;
use crate::sila::wire::sila2::org::silastandard::{
    CommandConfirmation, CommandExecutionUuid, CommandParameter, ObservableCommandExecutionInfo,
    ObservableCommandGetResponse, ObservableCommandInitiation, SiLaClientMessage,
    SiLaServerMessage, UnobservableCommandExecution, UnobservablePropertyRead,
};

pub(crate) async fn maintain(
    hub: Arc<Hub>,
    name: String,
    host: String,
    port: i64,
    mut stop: watch::Receiver<bool>,
) {
    let mut delay = Duration::from_secs(1);
    loop {
        if *stop.borrow() {
            return;
        }
        let started = std::time::Instant::now();
        let ended = run_stream(Arc::clone(&hub), &host, port, &mut stop).await;
        if *stop.borrow() || ended == StreamEnd::Stopped {
            return;
        }
        if started.elapsed() > Duration::from_secs(1) {
            delay = Duration::from_secs(1);
        }
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return;
                }
            }
            () = tokio::time::sleep(delay) => {}
        }
        delay = (delay * 2).min(Duration::from_secs(30));
        let _ = name;
    }
}

#[derive(PartialEq, Eq)]
enum StreamEnd {
    Dropped,
    Stopped,
}

async fn run_stream(
    hub: Arc<Hub>,
    host: &str,
    port: i64,
    stop: &mut watch::Receiver<bool>,
) -> StreamEnd {
    let Ok(channel) = open_channel(host, port, hub.plaintext, hub.ca_pem.as_deref()).await else {
        return StreamEnd::Dropped;
    };
    let (outbound, inbound) = mpsc::channel(32);
    let mut client = CloudClientEndpointClient::new(channel);
    let Ok(response) = client
        .connect_si_la_server(Request::new(Outbound { inner: inbound }))
        .await
    else {
        return StreamEnd::Dropped;
    };
    read_client(hub, response.into_inner(), outbound, stop).await
}

async fn open_channel(
    host: &str,
    port: i64,
    plaintext: bool,
    ca_pem: Option<&str>,
) -> Result<Channel, Status> {
    let scheme = if plaintext { "http" } else { "https" };
    let mut endpoint = Endpoint::from_shared(format!("{scheme}://{host}:{port}"))
        .map_err(|error| Status::unavailable(error.to_string()))?;
    if !plaintext {
        let mut tls = ClientTlsConfig::new().domain_name("SiLA2");
        if let Some(ca_pem) = ca_pem {
            tls = tls.ca_certificate(Certificate::from_pem(ca_pem));
        }
        endpoint = endpoint
            .tls_config(tls)
            .map_err(|error| Status::unavailable(error.to_string()))?;
    }
    endpoint
        .connect()
        .await
        .map_err(|error| Status::unavailable(error.to_string()))
}

async fn read_client(
    hub: Arc<Hub>,
    mut inbound: Streaming<SiLaClientMessage>,
    outbound: mpsc::Sender<SiLaServerMessage>,
    stop: &mut watch::Receiver<bool>,
) -> StreamEnd {
    let subscriptions = Arc::new(Mutex::new(HashMap::<String, Arc<AtomicBool>>::new()));
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return StreamEnd::Stopped;
                }
            }
            incoming = inbound.message() => {
                match incoming {
                    Ok(Some(message)) => dispatch(
                        Arc::clone(&hub),
                        message,
                        outbound.clone(),
                        &subscriptions,
                    ),
                    _ => return StreamEnd::Dropped,
                }
            }
        }
    }
}

fn dispatch(
    hub: Arc<Hub>,
    message: SiLaClientMessage,
    outbound: mpsc::Sender<SiLaServerMessage>,
    subscriptions: &Mutex<HashMap<String, Arc<AtomicBool>>>,
) {
    let request_id = message.request_uuid;
    match message.message {
        Some(ClientMessage::UnobservableCommandExecution(call)) => {
            spawn_bytes(request_id, outbound, async move {
                unobservable_command(hub, &call).await
            });
        }
        Some(ClientMessage::ObservableCommandInitiation(call)) => {
            spawn_message(request_id, outbound, async move {
                observable_confirmation(&hub, &call).await
            });
        }
        Some(ClientMessage::ObservableCommandExecutionInfoSubscription(call)) => {
            subscribe_info(
                hub,
                request_id,
                call.command_execution_uuid,
                outbound,
                subscriptions,
            );
        }
        Some(ClientMessage::ObservableCommandGetResponse(call)) => {
            spawn_message(request_id, outbound, async move {
                observable_result(&hub, &call).await
            });
        }
        Some(ClientMessage::UnobservablePropertyRead(call)) => {
            spawn_property(
                request_id,
                outbound,
                async move { property(&hub, &call).await },
            );
        }
        Some(ClientMessage::CancelObservableCommandExecutionInfoSubscription(_)) => {
            stop_subscription(subscriptions, &request_id);
        }
        Some(ClientMessage::MetadataRequest(_)) => {
            let _ = outbound.try_send(server(
                request_id,
                ServerMessage::GetFcpAffectedByMetadataResponse(
                    crate::sila::wire::sila2::org::silastandard::GetFcpAffectedByMetadataResponse::default(),
                ),
            ));
        }
        Some(_) => {
            let failure = framework(
                ErrorType::CommandExecutionNotAccepted,
                "this server does not implement that cloud message",
            );
            let _ = outbound.try_send(command_failure(request_id, &failure));
        }
        None => {}
    }
}

fn subscribe_info(
    hub: Arc<Hub>,
    request_id: String,
    execution: Option<CommandExecutionUuid>,
    outbound: mpsc::Sender<SiLaServerMessage>,
    subscriptions: &Mutex<HashMap<String, Arc<AtomicBool>>>,
) {
    let stop = Arc::new(AtomicBool::new(false));
    if let Ok(mut slots) = subscriptions.lock() {
        slots.insert(request_id.clone(), Arc::clone(&stop));
    }
    tokio::spawn(async move {
        let Some(execution) = execution else {
            let failure = framework(ErrorType::InvalidCommandExecutionUuid, "is required");
            let _ = outbound.send(command_failure(request_id, &failure)).await;
            return;
        };
        let uuid = execution.value.clone();
        let response = match hub.lab.start_task_info(Request::new(execution)).await {
            Ok(response) => response,
            Err(status) => {
                let _ = outbound.send(command_failure(request_id, &status)).await;
                return;
            }
        };
        let mut stream = response.into_inner();
        while !stop.load(Ordering::Relaxed) {
            let next = futures_util::StreamExt::next(&mut stream).await;
            let Some(Ok(info)) = next else {
                break;
            };
            let terminal = info.command_status >= 2;
            let _ = outbound
                .send(server(
                    request_id.clone(),
                    ServerMessage::ObservableCommandExecutionInfo(ObservableCommandExecutionInfo {
                        command_execution_uuid: Some(CommandExecutionUuid {
                            value: uuid.clone(),
                        }),
                        execution_info: Some(info),
                    }),
                ))
                .await;
            if terminal {
                break;
            }
        }
    });
}

async fn unobservable_command(
    hub: Arc<Hub>,
    call: &UnobservableCommandExecution,
) -> Result<Vec<u8>, Status> {
    reject_cloud_metadata(call.command_parameter.as_ref())?;
    let bytes = parameters(call.command_parameter.as_ref());
    if call
        .fully_qualified_command_id
        .starts_with("org.silastandard/core/ConnectionConfigurationService/v1/Command/")
    {
        return connection_command(&hub, &call.fully_qualified_command_id, bytes).await;
    }
    match call.fully_qualified_command_id.as_str() {
        "org.silastandard/core/SiLAService/v1/Command/GetFeatureDefinition" => {
            unary(bytes, |request| hub.core.get_feature_definition(request)).await
        }
        "org.silastandard/core/SiLAService/v1/Command/SetServerName" => {
            unary(bytes, |request| hub.core.set_server_name(request)).await
        }
        "org.silastandard/core/commands/CancelController/v1/Command/CancelCommand" => {
            unary(bytes, |request| hub.cancel.cancel_command(request)).await
        }
        "org.silastandard/core/commands/CancelController/v1/Command/CancelAll" => {
            unary(bytes, |request| hub.cancel.cancel_all(request)).await
        }
        "com.a3analytics/lab/LabOperations/v1/Command/ListLogSources" => {
            unary(bytes, |request| hub.lab.list_log_sources(request)).await
        }
        "com.a3analytics/lab/LabOperations/v1/Command/QueryLogs" => {
            unary(bytes, |request| hub.lab.query_logs(request)).await
        }
        "com.a3analytics/lab/LabOperations/v1/Command/ListMetrics" => {
            unary(bytes, |request| hub.lab.list_metrics(request)).await
        }
        "com.a3analytics/lab/LabOperations/v1/Command/QueryMetric" => {
            unary(bytes, |request| hub.lab.query_metric(request)).await
        }
        "com.a3analytics/lab/LabOperations/v1/Command/ListTasks" => {
            unary(bytes, |request| hub.lab.list_tasks(request)).await
        }
        "com.a3analytics/lab/LabOperations/v1/Command/GetTaskStatus" => {
            unary(bytes, |request| hub.lab.get_task_status(request)).await
        }
        id => Err(framework(
            ErrorType::CommandExecutionNotAccepted,
            format!("unknown command `{id}`"),
        )),
    }
}

async fn observable_confirmation(
    hub: &Hub,
    call: &ObservableCommandInitiation,
) -> Result<ServerMessage, Status> {
    reject_cloud_metadata(call.command_parameter.as_ref())?;
    let bytes = parameters(call.command_parameter.as_ref());
    if call.fully_qualified_command_id != "com.a3analytics/lab/LabOperations/v1/Command/StartTask" {
        return Err(framework(
            ErrorType::CommandExecutionNotAccepted,
            "unknown observable command",
        ));
    }
    let confirmation = unary_message::<
        crate::sila::wire::sila2::com::a3analytics::lab::laboperations::v1::StartTaskParameters,
        CommandConfirmation,
        _,
        _,
    >(bytes, |request| hub.lab.start_task(request))
    .await?;
    Ok(ServerMessage::ObservableCommandConfirmation(
        crate::sila::wire::sila2::org::silastandard::ObservableCommandConfirmation {
            command_confirmation: Some(confirmation),
        },
    ))
}

async fn observable_result(
    hub: &Hub,
    call: &ObservableCommandGetResponse,
) -> Result<ServerMessage, Status> {
    let execution = call
        .command_execution_uuid
        .clone()
        .ok_or_else(|| framework(ErrorType::InvalidCommandExecutionUuid, "is required"))?;
    let uuid = execution.value.clone();
    let response = hub.lab.start_task_result(Request::new(execution)).await?;
    Ok(ServerMessage::ObservableCommandResponse(
        crate::sila::wire::sila2::org::silastandard::ObservableCommandResponse {
            command_execution_uuid: Some(CommandExecutionUuid { value: uuid }),
            response: encode(&response.into_inner())?,
        },
    ))
}

async fn connection_command(hub: &Arc<Hub>, id: &str, bytes: &[u8]) -> Result<Vec<u8>, Status> {
    let feature = hub.feature();
    let prefix = "org.silastandard/core/ConnectionConfigurationService/v1/Command/";
    match id.strip_prefix(prefix) {
        Some("EnableServerInitiatedConnectionMode") => {
            unary(bytes, |request| {
                feature.enable_server_initiated_connection_mode(request)
            })
            .await
        }
        Some("DisableServerInitiatedConnectionMode") => {
            unary(bytes, |request| {
                feature.disable_server_initiated_connection_mode(request)
            })
            .await
        }
        Some("ConnectSiLAClient") => {
            unary(bytes, |request| feature.connect_si_la_client(request)).await
        }
        Some("DisconnectSiLAClient") => {
            unary(bytes, |request| feature.disconnect_si_la_client(request)).await
        }
        Some(_) | None => Err(framework(
            ErrorType::CommandExecutionNotAccepted,
            format!("unknown command `{id}`"),
        )),
    }
}

async fn property(hub: &Arc<Hub>, call: &UnobservablePropertyRead) -> Result<Vec<u8>, Status> {
    if !call.metadata.is_empty() {
        return Err(framework(
            ErrorType::NoMetadataAllowed,
            "this server does not accept SiLA client metadata",
        ));
    }
    let empty = Vec::new();
    match call.fully_qualified_property_id.as_str() {
        "org.silastandard/core/SiLAService/v1/Property/ServerName" => {
            unary(&empty, |request| hub.core.get_server_name(request)).await
        }
        "org.silastandard/core/SiLAService/v1/Property/ServerType" => {
            unary(&empty, |request| hub.core.get_server_type(request)).await
        }
        "org.silastandard/core/SiLAService/v1/Property/ServerUUID" => {
            unary(&empty, |request| hub.core.get_server_uuid(request)).await
        }
        "org.silastandard/core/SiLAService/v1/Property/ServerDescription" => {
            unary(&empty, |request| hub.core.get_server_description(request)).await
        }
        "org.silastandard/core/SiLAService/v1/Property/ServerVersion" => {
            unary(&empty, |request| hub.core.get_server_version(request)).await
        }
        "org.silastandard/core/SiLAService/v1/Property/ServerVendorURL" => {
            unary(&empty, |request| hub.core.get_server_vendor_url(request)).await
        }
        "org.silastandard/core/SiLAService/v1/Property/ImplementedFeatures" => {
            unary(&empty, |request| hub.core.get_implemented_features(request)).await
        }
        "org.silastandard/core/ConnectionConfigurationService/v1/Property/ServerInitiatedConnectionModeStatus" =>
        {
            let feature = hub.feature();
            unary(&empty, |request| {
                feature.get_server_initiated_connection_mode_status(request)
            })
            .await
        }
        "org.silastandard/core/ConnectionConfigurationService/v1/Property/ConfiguredSiLAClients" => {
            let feature = hub.feature();
            unary(&empty, |request| {
                feature.get_configured_si_la_clients(request)
            })
            .await
        }
        id => Err(framework(
            ErrorType::CommandExecutionNotAccepted,
            format!("unknown property `{id}`"),
        )),
    }
}

async fn unary<P, R, F, Fut>(bytes: &[u8], call: F) -> Result<Vec<u8>, Status>
where
    P: Message + Default,
    R: Message,
    F: FnOnce(Request<P>) -> Fut,
    Fut: Future<Output = Result<tonic::Response<R>, Status>>,
{
    let response = unary_message(bytes, call).await?;
    encode(&response)
}

async fn unary_message<P, R, F, Fut>(bytes: &[u8], call: F) -> Result<R, Status>
where
    P: Message + Default,
    F: FnOnce(Request<P>) -> Fut,
    Fut: Future<Output = Result<tonic::Response<R>, Status>>,
{
    let parameters = P::decode(bytes).unwrap_or_default();
    call(Request::new(parameters))
        .await
        .map(tonic::Response::into_inner)
}

fn parameters(value: Option<&CommandParameter>) -> &[u8] {
    value
        .map(|item| item.parameters.as_slice())
        .unwrap_or_default()
}

fn reject_cloud_metadata(value: Option<&CommandParameter>) -> Result<(), Status> {
    if value.is_some_and(|item| !item.metadata.is_empty()) {
        return Err(framework(
            ErrorType::NoMetadataAllowed,
            "this server does not accept SiLA client metadata",
        ));
    }
    Ok(())
}

fn spawn_bytes<F>(request_id: String, outbound: mpsc::Sender<SiLaServerMessage>, future: F)
where
    F: Future<Output = Result<Vec<u8>, Status>> + Send + 'static,
{
    tokio::spawn(async move {
        let message = match future.await {
            Ok(response) => ServerMessage::UnobservableCommandResponse(
                crate::sila::wire::sila2::org::silastandard::UnobservableCommandResponse {
                    response,
                },
            ),
            Err(status) => ServerMessage::CommandError(error_of(&status)),
        };
        let _ = outbound.send(server(request_id, message)).await;
    });
}

fn spawn_property<F>(request_id: String, outbound: mpsc::Sender<SiLaServerMessage>, future: F)
where
    F: Future<Output = Result<Vec<u8>, Status>> + Send + 'static,
{
    tokio::spawn(async move {
        let message = match future.await {
            Ok(value) => ServerMessage::UnobservablePropertyValue(
                crate::sila::wire::sila2::org::silastandard::UnobservablePropertyValue { value },
            ),
            Err(status) => ServerMessage::PropertyError(error_of(&status)),
        };
        let _ = outbound.send(server(request_id, message)).await;
    });
}

fn spawn_message<F>(request_id: String, outbound: mpsc::Sender<SiLaServerMessage>, future: F)
where
    F: Future<Output = Result<ServerMessage, Status>> + Send + 'static,
{
    tokio::spawn(async move {
        let message = match future.await {
            Ok(message) => message,
            Err(status) => ServerMessage::CommandError(error_of(&status)),
        };
        let _ = outbound.send(server(request_id, message)).await;
    });
}

fn stop_subscription(subscriptions: &Mutex<HashMap<String, Arc<AtomicBool>>>, request_id: &str) {
    if let Ok(slots) = subscriptions.lock()
        && let Some(stop) = slots.get(request_id)
    {
        stop.store(true, Ordering::Relaxed);
    }
}

fn command_failure(request_id: String, status: &Status) -> SiLaServerMessage {
    server(request_id, ServerMessage::CommandError(error_of(status)))
}

fn server(request_uuid: String, message: ServerMessage) -> SiLaServerMessage {
    SiLaServerMessage {
        request_uuid,
        message: Some(message),
    }
}

fn encode(message: &impl Message) -> Result<Vec<u8>, Status> {
    let mut bytes = Vec::new();
    message
        .encode(&mut bytes)
        .map_err(|_| Status::internal("SiLA response encoding failed"))?;
    Ok(bytes)
}

struct Outbound {
    inner: mpsc::Receiver<SiLaServerMessage>,
}

impl futures_util::Stream for Outbound {
    type Item = SiLaServerMessage;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().inner.poll_recv(context)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use prost::Message;
    use tokio::net::TcpListener;
    use tokio::sync::mpsc::Receiver;
    use tonic::transport::Server;
    use tonic::{Request, Response, Status, Streaming};

    use super::mpsc;
    use super::{ClientMessage, ServerMessage, UnobservablePropertyRead};
    use crate::sila::api::{
        Boolean, ConnectSiLaClientParameters, ConnectionConfigurationServiceClient,
        EnableServerInitiatedConnectionModeParameters, Integer, SilaString,
    };
    use crate::sila::server::SilaServer;
    use crate::sila::wire::sila2::org::silastandard::cloud_client_endpoint_server::{
        CloudClientEndpoint, CloudClientEndpointServer,
    };
    use crate::sila::wire::sila2::org::silastandard::core::silaservice::v1::GetServerUuidResponses;
    use crate::sila::wire::sila2::org::silastandard::{SiLaClientMessage, SiLaServerMessage};
    use crate::sila::{SilaIdentity, SilaServerHandle};
    use crate::{LabService, MemoryLogs, MemoryMetrics, MemoryTasks};

    const SERVER_UUID: &str = "11111111-1111-1111-1111-111111111111";

    struct Probe {
        seen: mpsc::Sender<String>,
    }

    struct CommandStream {
        inner: Receiver<Result<SiLaClientMessage, Status>>,
    }

    impl futures_util::Stream for CommandStream {
        type Item = Result<SiLaClientMessage, Status>;

        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            self.get_mut().inner.poll_recv(context)
        }
    }

    struct ProbeListener {
        listener: TcpListener,
    }

    impl futures_util::Stream for ProbeListener {
        type Item = Result<tokio::net::TcpStream, std::io::Error>;

        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            match self.listener.poll_accept(context) {
                std::task::Poll::Ready(Ok((stream, _))) => std::task::Poll::Ready(Some(Ok(stream))),
                std::task::Poll::Ready(Err(error)) => std::task::Poll::Ready(Some(Err(error))),
                std::task::Poll::Pending => std::task::Poll::Pending,
            }
        }
    }

    #[tonic::async_trait]
    impl CloudClientEndpoint for Probe {
        type ConnectSiLAServerStream = CommandStream;

        async fn connect_si_la_server(
            &self,
            request: Request<Streaming<SiLaServerMessage>>,
        ) -> Result<Response<Self::ConnectSiLAServerStream>, Status> {
            let mut inbound = request.into_inner();
            let seen = self.seen.clone();
            let (outbound, commands) = mpsc::channel(4);
            tokio::spawn(async move { read_uuid(&mut inbound, seen, outbound).await });
            Ok(Response::new(CommandStream { inner: commands }))
        }
    }

    async fn read_uuid(
        inbound: &mut Streaming<SiLaServerMessage>,
        seen: mpsc::Sender<String>,
        outbound: mpsc::Sender<Result<SiLaClientMessage, Status>>,
    ) {
        let request_id = "probe".to_owned();
        let _ = outbound
            .send(Ok(SiLaClientMessage {
                request_uuid: request_id.clone(),
                message: Some(ClientMessage::UnobservablePropertyRead(
                    UnobservablePropertyRead {
                        fully_qualified_property_id:
                            "org.silastandard/core/SiLAService/v1/Property/ServerUUID".to_owned(),
                        metadata: Vec::new(),
                    },
                )),
            }))
            .await;
        while let Ok(Some(message)) = inbound.message().await {
            let Some(uuid) = uuid_of(&request_id, message) else {
                continue;
            };
            let _ = seen.send(uuid).await;
        }
    }

    fn uuid_of(request_id: &str, message: SiLaServerMessage) -> Option<String> {
        if message.request_uuid != request_id {
            return None;
        }
        let ServerMessage::UnobservablePropertyValue(value) = message.message? else {
            return None;
        };
        GetServerUuidResponses::decode(value.value.as_slice())
            .ok()?
            .server_uuid
            .map(|item| item.value)
    }

    #[tokio::test]
    async fn server_initiated_connection_survives_restart() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen, mut uuids) = mpsc::channel(4);
        tokio::spawn(async move {
            let _ = Server::builder()
                .add_service(CloudClientEndpointServer::new(Probe { seen }))
                .serve_with_incoming(ProbeListener { listener })
                .await;
        });
        let path = std::env::temp_dir().join(format!("sila-connections-{port}.json"));
        let _ = std::fs::remove_file(&path);
        let first = provider(&path).await;
        configure(&first, port).await;
        let first_uuid = tokio::time::timeout(Duration::from_secs(8), uuids.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first_uuid, SERVER_UUID);
        drop(first);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let second = provider(&path).await;
        let second_uuid = tokio::time::timeout(Duration::from_secs(8), uuids.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second_uuid, SERVER_UUID);
        drop(second);
        let _ = std::fs::remove_file(&path);
    }

    async fn provider(path: &std::path::Path) -> SilaServerHandle {
        SilaServer::new(
            SilaIdentity::lab_dev_kit(SERVER_UUID).unwrap(),
            LabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share(),
        )
        .plaintext()
        .cloud_plaintext()
        .connection_store(path.to_path_buf())
        .serve("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap()
    }

    async fn configure(server: &SilaServerHandle, port: u16) {
        let channel =
            tonic::transport::Channel::from_shared(format!("http://{}", server.local_addr()))
                .unwrap()
                .connect()
                .await
                .unwrap();
        let mut client = ConnectionConfigurationServiceClient::new(channel);
        client
            .enable_server_initiated_connection_mode(
                EnableServerInitiatedConnectionModeParameters {},
            )
            .await
            .unwrap();
        client
            .connect_si_la_client(ConnectSiLaClientParameters {
                client_name: Some(SilaString {
                    value: "probe".to_owned(),
                }),
                si_la_client_host: Some(SilaString {
                    value: "127.0.0.1".to_owned(),
                }),
                si_la_client_port: Some(Integer {
                    value: i64::from(port),
                }),
                persist: Some(Boolean { value: true }),
            })
            .await
            .unwrap();
    }
}
