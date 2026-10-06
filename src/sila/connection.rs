//! Persistent `ConnectionConfigurationService` and server-initiated sessions.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tonic::{Request, Response, Status};

use crate::sila::cancel::CancelFeature;
use crate::sila::cloud;
use crate::sila::constraints::bounded_characters;
use crate::sila::core::CoreService;
use crate::sila::core::CONNECTION_CONFIGURATION;
use crate::sila::errors::{defined, reject_metadata, validation};
use crate::sila::lab::LabFeature;
use crate::sila::values::{sila_bool, sila_string};
use crate::sila::wire::sila2::org::silastandard::core::connectionconfigurationservice::v1::connection_configuration_service_server::ConnectionConfigurationService;
use crate::sila::wire::sila2::org::silastandard::core::connectionconfigurationservice::v1::{
    ConnectSiLaClientParameters, ConnectSiLaClientResponses, DisconnectSiLaClientParameters,
    DisconnectSiLaClientResponses, EnableServerInitiatedConnectionModeParameters,
    EnableServerInitiatedConnectionModeResponses, DisableServerInitiatedConnectionModeParameters,
    DisableServerInitiatedConnectionModeResponses, GetConfiguredSiLaClientsParameters,
    GetConfiguredSiLaClientsResponses, GetServerInitiatedConnectionModeStatusParameters,
    GetServerInitiatedConnectionModeStatusResponses,
    get_configured_si_la_clients_responses::ConfiguredSiLaClientsStruct,
};
use crate::sila::wire::sila2::org::silastandard::Integer;

const CONNECT: &str =
    "org.silastandard/core/ConnectionConfigurationService/v1/Command/ConnectSiLAClient/Parameter";
const DISCONNECT: &str = "org.silastandard/core/ConnectionConfigurationService/v1/Command/DisconnectSiLAClient/Parameter";

#[derive(Clone, Serialize, Deserialize)]
struct ClientRecord {
    name: String,
    host: String,
    port: i64,
    persist: bool,
    #[serde(default)]
    suspended: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    enabled: bool,
    clients: Vec<ClientRecord>,
}

pub(crate) struct Hub {
    pub core: CoreService,
    pub lab: LabFeature,
    pub cancel: CancelFeature,
    path: PathBuf,
    store: Mutex<Store>,
    sessions: Mutex<BTreeMap<String, watch::Sender<bool>>>,
    pub plaintext: bool,
    pub ca_pem: Option<String>,
}

#[derive(Clone)]
pub(crate) struct ConnectionFeature {
    hub: Arc<Hub>,
}

impl Hub {
    pub(crate) fn open(
        path: PathBuf,
        core: CoreService,
        lab: LabFeature,
        cancel: CancelFeature,
        plaintext: bool,
        ca_pem: Option<String>,
    ) -> Result<Arc<Self>, crate::error::A2aLabError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)
                .map_err(|error| crate::error::A2aLabError::unavailable(error.to_string()))?;
        }
        let store = read_store(&path)?;
        let hub = Arc::new(Self {
            core,
            lab,
            cancel,
            path,
            store: Mutex::new(store),
            sessions: Mutex::new(BTreeMap::new()),
            plaintext,
            ca_pem,
        });
        if hub.store.lock().is_ok_and(|store| store.enabled) {
            hub.start_all();
        }
        Ok(hub)
    }

    pub(crate) fn feature(self: &Arc<Self>) -> ConnectionFeature {
        ConnectionFeature {
            hub: Arc::clone(self),
        }
    }

    fn start_all(self: &Arc<Self>) {
        let clients = self.snapshot();
        for client in clients {
            if !client.suspended {
                self.start_one(client);
            }
        }
    }

    fn start_one(self: &Arc<Self>, client: ClientRecord) {
        let (stop, receiver) = watch::channel(false);
        if let Ok(mut sessions) = self.sessions.lock()
            && let Some(previous) = sessions.insert(client.name.clone(), stop)
        {
            let _ = previous.send(true);
        }
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            cloud::maintain(hub, client.name, client.host, client.port, receiver).await;
        });
    }

    fn snapshot(&self) -> Vec<ClientRecord> {
        self.store
            .lock()
            .map(|store| store.clients.clone())
            .unwrap_or_default()
    }

    fn write(&self, store: &Store) -> Result<(), Status> {
        let persisted = Store {
            enabled: store.enabled,
            clients: store
                .clients
                .iter()
                .filter(|client| client.persist)
                .cloned()
                .map(|mut client| {
                    client.suspended = false;
                    client
                })
                .collect(),
        };
        let body = serde_json::to_vec_pretty(&persisted)
            .map_err(|error| crate::sila::errors::undefined(error.to_string()))?;
        fs::write(&self.path, body)
            .map_err(|error| crate::sila::errors::undefined(error.to_string()))
    }

    fn enable(self: &Arc<Self>) -> Result<(), Status> {
        let store = {
            let mut store = self
                .store
                .lock()
                .map_err(|_| crate::sila::errors::undefined("connection store is unavailable"))?;
            store.enabled = true;
            for client in &mut store.clients {
                client.suspended = false;
            }
            store.clone_state()
        };
        self.write(&store)?;
        self.start_all();
        Ok(())
    }

    fn disable(self: &Arc<Self>) -> Result<(), Status> {
        {
            let mut store = self
                .store
                .lock()
                .map_err(|_| crate::sila::errors::undefined("connection store is unavailable"))?;
            store.enabled = false;
            self.write(&store)?;
        }
        self.stop_all();
        Ok(())
    }

    pub(crate) fn shutdown(&self) {
        self.stop_all();
    }

    fn stop_all(&self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            for (_, stop) in std::mem::take(&mut *sessions) {
                let _ = stop.send(true);
            }
        }
    }

    fn connect(self: &Arc<Self>, request: ConnectSiLaClientParameters) -> Result<(), Status> {
        let name = required_text(request.client_name, &format!("{CONNECT}/ClientName"))?;
        let host = required_text(
            request.si_la_client_host,
            &format!("{CONNECT}/SiLAClientHost"),
        )?;
        let port = required_port(
            request.si_la_client_port,
            &format!("{CONNECT}/SiLAClientPort"),
        )?;
        let persist = request.persist.is_some_and(|value| value.value);
        if name.is_empty()
            || host.is_empty()
            || host.chars().any(char::is_whitespace)
            || port == 65536
        {
            return Err(invalid_client("the SiLA client host or name is not usable"));
        }
        let record = ClientRecord {
            name: name.clone(),
            host,
            port,
            persist,
            suspended: false,
        };
        let enabled = {
            let mut store = self
                .store
                .lock()
                .map_err(|_| crate::sila::errors::undefined("connection store is unavailable"))?;
            if store.clients.iter().any(|client| client.name == name) {
                return Err(invalid_client("the SiLA client name is already configured"));
            }
            store.clients.push(record.clone());
            self.write(&store)?;
            store.enabled
        };
        if enabled {
            self.start_one(record);
        }
        Ok(())
    }

    fn disconnect(self: &Arc<Self>, request: DisconnectSiLaClientParameters) -> Result<(), Status> {
        let name = required_text(request.client_name, &format!("{DISCONNECT}/ClientName"))?;
        let remove = request.remove.is_some_and(|value| value.value);
        {
            let mut store = self
                .store
                .lock()
                .map_err(|_| crate::sila::errors::undefined("connection store is unavailable"))?;
            let Some(position) = store.clients.iter().position(|client| client.name == name) else {
                return Err(invalid_client("the SiLA client is not configured"));
            };
            if remove {
                store.clients.remove(position);
            } else if let Some(client) = store.clients.get_mut(position) {
                client.suspended = true;
            }
            self.write(&store)?;
        }
        if let Ok(mut sessions) = self.sessions.lock()
            && let Some(stop) = sessions.remove(&name)
        {
            let _ = stop.send(true);
        }
        Ok(())
    }
}

impl Store {
    fn clone_state(&self) -> Self {
        Self {
            enabled: self.enabled,
            clients: self.clients.clone(),
        }
    }
}

fn read_store(path: &Path) -> Result<Store, crate::error::A2aLabError> {
    if !path.exists() {
        return Ok(Store::default());
    }
    let bytes = fs::read(path)
        .map_err(|error| crate::error::A2aLabError::unavailable(error.to_string()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| crate::error::A2aLabError::protocol(error.to_string()))
}

fn required_text(
    value: Option<crate::sila::wire::sila2::org::silastandard::String>,
    parameter: &str,
) -> Result<String, Status> {
    let text = value.map(|item| item.value).unwrap_or_default();
    if !bounded_characters(&text, 255) {
        return Err(validation(parameter, "must be at most 255 characters"));
    }
    Ok(text)
}

fn required_port(value: Option<Integer>, parameter: &str) -> Result<i64, Status> {
    let port = value
        .ok_or_else(|| validation(parameter, "is required"))?
        .value;
    if !(1..=65536).contains(&port) {
        return Err(validation(parameter, "must be from 1 to 65536"));
    }
    Ok(port)
}

fn invalid_client(message: &str) -> Status {
    defined(CONNECTION_CONFIGURATION, "InvalidSiLAClient", message)
}

#[tonic::async_trait]
impl ConnectionConfigurationService for ConnectionFeature {
    async fn enable_server_initiated_connection_mode(
        &self,
        request: Request<EnableServerInitiatedConnectionModeParameters>,
    ) -> Result<Response<EnableServerInitiatedConnectionModeResponses>, Status> {
        reject_metadata(&request)?;
        self.hub.enable()?;
        Ok(Response::new(
            EnableServerInitiatedConnectionModeResponses {},
        ))
    }

    async fn disable_server_initiated_connection_mode(
        &self,
        request: Request<DisableServerInitiatedConnectionModeParameters>,
    ) -> Result<Response<DisableServerInitiatedConnectionModeResponses>, Status> {
        reject_metadata(&request)?;
        self.hub.disable()?;
        Ok(Response::new(
            DisableServerInitiatedConnectionModeResponses {},
        ))
    }

    async fn connect_si_la_client(
        &self,
        request: Request<ConnectSiLaClientParameters>,
    ) -> Result<Response<ConnectSiLaClientResponses>, Status> {
        reject_metadata(&request)?;
        self.hub.connect(request.into_inner())?;
        Ok(Response::new(ConnectSiLaClientResponses {}))
    }

    async fn disconnect_si_la_client(
        &self,
        request: Request<DisconnectSiLaClientParameters>,
    ) -> Result<Response<DisconnectSiLaClientResponses>, Status> {
        reject_metadata(&request)?;
        self.hub.disconnect(request.into_inner())?;
        Ok(Response::new(DisconnectSiLaClientResponses {}))
    }

    async fn get_server_initiated_connection_mode_status(
        &self,
        request: Request<GetServerInitiatedConnectionModeStatusParameters>,
    ) -> Result<Response<GetServerInitiatedConnectionModeStatusResponses>, Status> {
        reject_metadata(&request)?;
        let enabled = self.hub.store.lock().is_ok_and(|store| store.enabled);
        Ok(Response::new(
            GetServerInitiatedConnectionModeStatusResponses {
                server_initiated_connection_mode_status: Some(sila_bool(enabled)),
            },
        ))
    }

    async fn get_configured_si_la_clients(
        &self,
        request: Request<GetConfiguredSiLaClientsParameters>,
    ) -> Result<Response<GetConfiguredSiLaClientsResponses>, Status> {
        reject_metadata(&request)?;
        let configured_si_la_clients = self
            .hub
            .snapshot()
            .into_iter()
            .map(|client| ConfiguredSiLaClientsStruct {
                client_name: Some(sila_string(client.name)),
                si_la_client_host: Some(sila_string(client.host)),
                si_la_client_port: Some(Integer { value: client.port }),
            })
            .collect();
        Ok(Response::new(GetConfiguredSiLaClientsResponses {
            configured_si_la_clients,
        }))
    }
}
