//! Encrypted SiLA 2 client session over a dynamic Feature catalogue.

use std::collections::BTreeMap;
use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceEvent};

use crate::error::A2aLabError;
use crate::json_object::JsonObject;
use crate::sila::consumer::codec::{input_schema, output_schema, property_input_schema};
use crate::sila::consumer::model::FeatureModel;
use crate::sila::consumer::rpc::{Consumer, Invoke, Poll};

/// Fully qualified identifier of the mandatory SiLAService feature.
pub const SILA_SERVICE: &str = "org.silastandard/core/SiLAService/v1";

/// Explicit SiLA server address and expected server UUID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SilaEndpoint {
    /// Server host or address.
    pub host: String,
    /// gRPC port.
    pub port: u16,
    /// UUID published by SiLAService.
    pub server_uuid: String,
}

/// PEM certificate authority used to verify a SiLA server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SilaTrust {
    authority_pem: Vec<u8>,
}

impl SilaTrust {
    /// Trusts one PEM-encoded certificate authority.
    #[must_use]
    pub fn authority_pem(pem: impl Into<Vec<u8>>) -> Self {
        Self {
            authority_pem: pem.into(),
        }
    }
}

/// A discovered SiLA server and the Feature Definitions it published.
pub struct SilaSession {
    consumer: Consumer,
    name: String,
    uuid: String,
}

impl SilaSession {
    /// Connects to `endpoint` and rejects a certificate or server UUID that does not match.
    pub async fn connect(endpoint: &SilaEndpoint, trust: &SilaTrust) -> Result<Self, A2aLabError> {
        if endpoint.host.is_empty() || endpoint.port == 0 {
            return Err(A2aLabError::invalid(
                "sila_endpoint",
                "host and port are required",
            ));
        }
        let consumer = Consumer::connect(
            &endpoint.host,
            endpoint.port,
            &endpoint.server_uuid,
            &trust.authority_pem,
        )
        .await?;
        Self::from_consumer(consumer).await
    }

    /// Accepts one server-initiated connection and returns the reported server name.
    pub async fn serve_cloud_endpoint(port: u16) -> Result<String, A2aLabError> {
        crate::sila::consumer::cloud::serve_until_server_name(port).await
    }

    /// Connects without TLS. The only intended peer is the local Caddy h2c gateway.
    pub async fn connect_unencrypted(endpoint: &SilaEndpoint) -> Result<Self, A2aLabError> {
        if endpoint.host.is_empty() || endpoint.port == 0 {
            return Err(A2aLabError::invalid(
                "sila_endpoint",
                "host and port are required",
            ));
        }
        let consumer =
            Consumer::connect_unencrypted(&endpoint.host, endpoint.port, &endpoint.server_uuid)
                .await?;
        Self::from_consumer(consumer).await
    }

    /// Browses `_sila._tcp.local.` until a server implementing `feature` accepts `trust`.
    pub async fn discover(
        feature: &str,
        trust: &SilaTrust,
        timeout: Duration,
    ) -> Result<Self, A2aLabError> {
        let daemon =
            ServiceDaemon::new().map_err(|error| A2aLabError::transport(error.to_string()))?;
        let receiver = daemon
            .browse("_sila._tcp.local.")
            .map_err(|error| A2aLabError::transport(error.to_string()))?;
        let deadline = tokio::time::Instant::now() + timeout;
        let mut last = A2aLabError::transport("no SiLA server answered discovery");
        while tokio::time::Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let event = tokio::time::timeout(remaining, receiver.recv_async()).await;
            let Ok(Ok(event)) = event else {
                break;
            };
            let ServiceEvent::ServiceResolved(resolved) = event else {
                continue;
            };
            if resolved.get_property_val_str("version") != Some("1.1") {
                last = A2aLabError::transport(format!(
                    "RejectedServer(UnsupportedVersion({:?}))",
                    resolved.get_property_val_str("version")
                ));
                continue;
            }
            let Some(address) = service_address(&resolved) else {
                continue;
            };
            let uuid = resolved
                .get_fullname()
                .trim_end_matches("._sila._tcp.local.")
                .to_owned();
            match Consumer::connect(address.0.as_str(), address.1, &uuid, &trust.authority_pem)
                .await
            {
                Ok(consumer) if consumer.features().contains_key(feature) => {
                    let _ = daemon.shutdown();
                    return Self::from_consumer(consumer).await;
                }
                Ok(_) => {
                    last = A2aLabError::transport(format!(
                        "{uuid} at {} did not implement {feature}",
                        address.0
                    ));
                }
                Err(error) => {
                    last = A2aLabError::transport(format!(
                        "{uuid} at {}:{}: {error}",
                        address.0, address.1
                    ));
                }
            }
        }
        let _ = daemon.shutdown();
        Err(last)
    }

    /// Server name published by SiLAService.
    #[must_use]
    pub fn server_name(&self) -> &str {
        &self.name
    }

    /// Server UUID published by SiLAService.
    #[must_use]
    pub fn server_uuid(&self) -> &str {
        &self.uuid
    }

    /// Fully qualified identifiers of the implemented features.
    #[must_use]
    pub fn implemented_features(&self) -> Vec<String> {
        self.consumer.features().keys().cloned().collect()
    }

    /// Feature Definition XML published for `feature`.
    pub fn feature_definition(&self, feature: &str) -> Result<String, A2aLabError> {
        self.consumer
            .features()
            .get(feature)
            .map(|feature| feature.xml.clone())
            .ok_or_else(|| A2aLabError::not_found("feature", feature))
    }

    pub(crate) fn features(&self) -> &BTreeMap<String, FeatureModel> {
        self.consumer.features()
    }

    pub(crate) fn command_schemas(
        feature: &FeatureModel,
        command: &crate::sila::consumer::model::CommandModel,
    ) -> (String, String) {
        (
            input_schema(feature, &command.parameters),
            output_schema(feature, &command.responses),
        )
    }

    pub(crate) fn property_schemas(
        feature: &FeatureModel,
        property: &crate::sila::consumer::model::PropertyModel,
    ) -> (String, String) {
        (
            property_input_schema(),
            output_schema(
                feature,
                &[crate::sila::consumer::model::Element {
                    identifier: "value".to_owned(),
                    data_type: property.data_type.clone(),
                }],
            ),
        )
    }

    pub(crate) async fn call(
        &self,
        feature: &str,
        command: &str,
        input: &JsonObject,
    ) -> Result<Invoke, A2aLabError> {
        self.consumer.call(feature, command, input).await
    }

    pub(crate) async fn read_property(
        &self,
        feature: &str,
        name: &str,
        input: &JsonObject,
    ) -> Result<Invoke, A2aLabError> {
        self.consumer.read_property(feature, name, input).await
    }

    #[allow(dead_code)]
    pub(crate) async fn subscribe_property(
        &self,
        feature: &str,
        name: &str,
        count: usize,
        input: &JsonObject,
    ) -> Result<Invoke, A2aLabError> {
        self.consumer
            .subscribe_property(feature, name, count, input)
            .await
    }

    pub async fn affected_calls(
        &self,
        feature: &str,
        metadata: &str,
    ) -> Result<Vec<String>, A2aLabError> {
        self.consumer.affected_calls(feature, metadata).await
    }

    pub(crate) async fn start_observable(
        &self,
        feature: &str,
        command: &str,
        input: &JsonObject,
    ) -> Result<String, A2aLabError> {
        self.consumer
            .start_observable(feature, command, input)
            .await
    }

    pub(crate) async fn poll(
        &self,
        feature: &str,
        command: &str,
        execution: &str,
    ) -> Result<Poll, A2aLabError> {
        self.consumer.poll(feature, command, execution).await
    }

    pub(crate) async fn cancel(&self, execution: &str) -> Result<(), A2aLabError> {
        self.consumer.cancel(execution).await
    }

    async fn from_consumer(consumer: Consumer) -> Result<Self, A2aLabError> {
        let service = consumer
            .features()
            .keys()
            .find(|feature| feature.ends_with("/SiLAService/v1"))
            .cloned()
            .ok_or_else(|| A2aLabError::protocol("server did not publish SiLAService"))?;
        let name = consumer.string_property(&service, "ServerName").await?;
        let uuid = consumer.string_property(&service, "ServerUUID").await?;
        Ok(Self {
            consumer,
            name,
            uuid,
        })
    }
}

fn service_address(resolved: &mdns_sd::ResolvedService) -> Option<(String, u16)> {
    let port = resolved.get_port();
    let addresses = resolved.get_addresses_v4();
    let ip = addresses
        .iter()
        .find(|ip| ip.is_loopback())
        .or_else(|| addresses.iter().next())?;
    Some((ip.to_string(), port))
}
