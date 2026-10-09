//! SiLA gRPC listener.

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures_util::Stream;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tonic::transport::{Identity, Server, ServerTlsConfig};

use crate::error::A2aLabError;
use crate::service::A2aLabApi;
use crate::sila::binary::{BinaryStore, DownloadFeature};
use crate::sila::cancel::CancelFeature;
use crate::sila::cert::{install_crypto, SilaCertificate};
use crate::sila::connection::Hub;
use crate::sila::core::{CoreService, FeatureCatalog};
use crate::sila::discover::{Announcer, SharedAnnouncer};
use crate::sila::executions::Executions;
use crate::sila::identity::SilaIdentity;
use crate::sila::images::ImageFeature;
use crate::sila::lab::LabFeature;
use crate::sila::wire::sila2::com::a3analytics::lab::labimages::v1::lab_images_server::LabImagesServer;
use crate::sila::wire::sila2::com::a3analytics::lab::laboperations::v1::lab_operations_server::LabOperationsServer;
use crate::sila::wire::sila2::org::silastandard::binary_download_server::BinaryDownloadServer;
use crate::sila::wire::sila2::org::silastandard::core::commands::cancelcontroller::v1::cancel_controller_server::CancelControllerServer;
use crate::sila::wire::sila2::org::silastandard::core::connectionconfigurationservice::v1::connection_configuration_service_server::ConnectionConfigurationServiceServer;
use crate::sila::wire::sila2::org::silastandard::core::silaservice::v1::si_la_service_server::SiLaServiceServer;

/// Running SiLA server. Dropping it stops the listener and withdraws discovery.
pub struct SilaServerHandle {
    local_addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    announcer: SharedAnnouncer,
}

impl SilaServerHandle {
    /// Bound socket address.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

impl Drop for SilaServerHandle {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Ok(mut announcer) = self.announcer.lock() {
            announcer.take();
        }
    }
}

/// Feature Provider for one [`crate::service::A2aLabApi`].
pub struct SilaServer {
    identity: Arc<Mutex<SilaIdentity>>,
    lab: Arc<dyn A2aLabApi>,
    certificate: Option<SilaCertificate>,
    plaintext: bool,
    announce: bool,
    connection_store: Option<std::path::PathBuf>,
    cloud_ca: Option<String>,
    cloud_plaintext: bool,
}

impl SilaServer {
    /// Serves `lab` with the supplied identity. TLS is required unless [`Self::plaintext`] is set.
    #[must_use]
    pub fn new(identity: SilaIdentity, lab: Arc<dyn A2aLabApi>) -> Self {
        Self {
            identity: identity.share(),
            lab,
            certificate: None,
            plaintext: false,
            announce: false,
            connection_store: None,
            cloud_ca: None,
            cloud_plaintext: false,
        }
    }

    /// Uses this certificate for the encrypted listener and discovery CA records.
    #[must_use]
    pub fn certificate(mut self, certificate: SilaCertificate) -> Self {
        self.certificate = Some(certificate);
        self
    }

    /// Serves unencrypted HTTP/2. Encrypted serving stays the default.
    #[must_use]
    pub fn plaintext(mut self) -> Self {
        self.plaintext = true;
        self
    }

    /// Persists server-initiated clients at `path` and serves `ConnectionConfigurationService`.
    #[must_use]
    pub fn connection_store(mut self, path: std::path::PathBuf) -> Self {
        self.connection_store = Some(path);
        self
    }

    /// Trusts `ca_pem` when this server opens a server-initiated TLS connection.
    #[must_use]
    pub fn cloud_trust(mut self, ca_pem: impl Into<String>) -> Self {
        self.cloud_ca = Some(ca_pem.into());
        self
    }

    /// Opens server-initiated connections without TLS. Encrypted cloud connections stay the default.
    #[must_use]
    pub fn cloud_plaintext(mut self) -> Self {
        self.cloud_plaintext = true;
        self
    }

    /// Advertises `_sila._tcp.local.` after the listener is ready.
    #[must_use]
    pub fn announce(mut self) -> Self {
        self.announce = true;
        self
    }

    /// Binds `address` and serves until the returned handle is dropped.
    pub async fn serve(self, address: SocketAddr) -> Result<SilaServerHandle, A2aLabError> {
        install_crypto();
        if !self.plaintext && self.certificate.is_none() {
            return Err(A2aLabError::invalid(
                "certificate",
                "encrypted SiLA serving requires a certificate",
            ));
        }
        let (listener, local_addr) = bind(address).await?;
        let executions = Executions::new();
        let announcer = self.start_discovery(local_addr)?;
        let certificate = (!self.plaintext)
            .then(|| self.certificate.clone())
            .flatten();
        let core = CoreService {
            identity: Arc::clone(&self.identity),
            on_rename: rename_callback(
                Arc::clone(&announcer),
                local_addr,
                certificate,
                Arc::clone(&self.identity),
            ),
            features: FeatureCatalog {
                connection: self.connection_store.is_some(),
            },
        };
        let binaries = BinaryStore::new();
        let images = ImageFeature {
            lab: Arc::clone(&self.lab),
            binaries: binaries.clone(),
        };
        let downloads = DownloadFeature { binaries };
        let lab = LabFeature {
            lab: self.lab,
            executions: executions.clone(),
        };
        let cancel = CancelFeature {
            lab: lab.lab.clone(),
            executions,
        };
        let connection = if let Some(path) = self.connection_store.clone() {
            Some(Hub::open(
                path,
                core.clone(),
                lab.clone(),
                cancel.clone(),
                self.cloud_plaintext,
                self.cloud_ca.clone(),
            )?)
        } else {
            None
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let builder = Server::builder();
        let mut router = if let Some(certificate) = &self.certificate {
            if self.plaintext {
                builder
            } else {
                let identity = Identity::from_pem(&certificate.cert_pem, &certificate.key_pem);
                builder
                    .tls_config(ServerTlsConfig::new().identity(identity))
                    .map_err(|error| A2aLabError::protocol(error.to_string()))?
            }
        } else {
            builder
        };
        let incoming = ListenerStream { listener };
        tokio::spawn(async move {
            let router = router
                .add_service(SiLaServiceServer::new(core))
                .add_service(LabOperationsServer::new(lab))
                .add_service(LabImagesServer::new(images))
                .add_service(BinaryDownloadServer::new(downloads))
                .add_service(CancelControllerServer::new(cancel));
            let served = if let Some(connection) = connection {
                let result = router
                    .add_service(ConnectionConfigurationServiceServer::new(
                        connection.feature(),
                    ))
                    .serve_with_incoming_shutdown(incoming, async move {
                        let _ = shutdown_rx.await;
                    })
                    .await;
                connection.shutdown();
                result
            } else {
                router
                    .serve_with_incoming_shutdown(incoming, async move {
                        let _ = shutdown_rx.await;
                    })
                    .await
            };
            let _ = served;
        });
        Ok(SilaServerHandle {
            local_addr,
            shutdown: Some(shutdown_tx),
            announcer,
        })
    }

    fn start_discovery(&self, local_addr: SocketAddr) -> Result<SharedAnnouncer, A2aLabError> {
        let announcer: SharedAnnouncer = Arc::new(Mutex::new(None));
        if self.announce {
            let identity = self
                .identity
                .lock()
                .map_err(|_| A2aLabError::unavailable("server identity is unavailable"))?;
            let certificate = (!self.plaintext)
                .then_some(self.certificate.as_ref())
                .flatten();
            let started = Announcer::start(local_addr, &identity, certificate)?;
            drop(identity);
            *announcer
                .lock()
                .map_err(|_| A2aLabError::unavailable("discovery lock"))? = Some(started);
        }
        Ok(announcer)
    }
}

async fn bind(address: SocketAddr) -> Result<(TcpListener, SocketAddr), A2aLabError> {
    let listener = TcpListener::bind(address)
        .await
        .map_err(|error| transport(&error))?;
    let local_addr = listener.local_addr().map_err(|error| transport(&error))?;
    Ok((listener, local_addr))
}

fn rename_callback(
    announcer: SharedAnnouncer,
    address: SocketAddr,
    certificate: Option<SilaCertificate>,
    identity: Arc<Mutex<SilaIdentity>>,
) -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(move || {
        let Ok(identity) = identity.lock() else {
            return;
        };
        let Ok(mut slot) = announcer.lock() else {
            return;
        };
        if let Some(announcer) = slot.as_mut() {
            let _ = announcer.refresh(address, &identity, certificate.as_ref());
        }
    })
}

fn transport(error: &std::io::Error) -> A2aLabError {
    A2aLabError::transport(error.to_string())
}

struct ListenerStream {
    listener: TcpListener,
}

impl Stream for ListenerStream {
    type Item = Result<TcpStream, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.listener.poll_accept(context) {
            Poll::Ready(Ok((stream, _))) => Poll::Ready(Some(Ok(stream))),
            Poll::Ready(Err(error)) => Poll::Ready(Some(Err(error))),
            Poll::Pending => Poll::Pending,
        }
    }
}
