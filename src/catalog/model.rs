//! Assets, endpoints, and bindings discovered from an AAS catalog.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::id::{AssetKey, SemanticId};
use crate::error::A2aLabError;
use crate::id;
use crate::page::PageRequest;

/// Live protocol selected by a catalog binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolKind {
    /// OPC UA session.
    OpcUa,
    /// Generated SiLA 2 client.
    Sila2,
}

/// Role a binding plays in the lab API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BindingRole {
    /// Historical events or alarms.
    LogSource,
    /// A numeric time series.
    Metric,
    /// A startable command.
    Task,
}

/// OPC UA security mode. Unsecured sessions are rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecurityMode {
    /// Sign messages.
    Sign,
    /// Sign and encrypt messages.
    SignAndEncrypt,
}

/// How the OPC UA session authenticates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OpcUaIdentityKind {
    /// Username and password supplied outside the catalog.
    Username,
    /// X.509 certificate supplied outside the catalog.
    Certificate,
}

/// Protocol endpoint stored on a binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "protocol", rename_all = "snake_case")]
pub enum Endpoint {
    /// OPC UA node that supplies live or historical values.
    OpcUa {
        /// Server endpoint URL.
        url: String,
        /// Security policy URI.
        security_policy: String,
        /// Message security mode.
        security_mode: SecurityMode,
        /// Identity kind. Secrets stay outside the catalog.
        identity: OpcUaIdentityKind,
        /// Server-independent node identifier.
        node_id: String,
        /// Namespace URI used to resolve the namespace index.
        namespace_uri: String,
        /// Optional browse path.
        browse_path: String,
    },
    /// SiLA 2 feature member served by a generated client.
    Sila2 {
        /// Server host.
        host: String,
        /// gRPC port.
        port: u16,
        /// Fully qualified feature identifier.
        feature: String,
        /// Command or property name.
        member: String,
        /// Feature major version.
        version: String,
    },
}

impl Endpoint {
    /// Returns the protocol used by this endpoint.
    #[must_use]
    pub const fn protocol(&self) -> ProtocolKind {
        match self {
            Self::OpcUa { .. } => ProtocolKind::OpcUa,
            Self::Sila2 { .. } => ProtocolKind::Sila2,
        }
    }

    /// Rejects empty addresses and unsecured OPC UA sessions.
    pub fn check(&self) -> Result<(), A2aLabError> {
        match self {
            Self::OpcUa {
                url,
                security_policy,
                node_id,
                namespace_uri,
                ..
            } => {
                required("opcua_url", url)?;
                required("security_policy", security_policy)?;
                required("node_id", node_id)?;
                required("namespace_uri", namespace_uri)?;
                if security_policy.contains("None") {
                    return Err(A2aLabError::invalid(
                        "security_policy",
                        "unsecured OPC UA sessions are rejected",
                    ));
                }
            }
            Self::Sila2 {
                host,
                port,
                feature,
                member,
                version,
            } => {
                required("sila_host", host)?;
                if *port == 0 {
                    return Err(A2aLabError::invalid("sila_port", "must be non-zero"));
                }
                required("sila_feature", feature)?;
                required("sila_member", member)?;
                required("sila_version", version)?;
            }
        }
        Ok(())
    }
}

fn required(field: &'static str, value: &str) -> Result<(), A2aLabError> {
    if value.is_empty() {
        return Err(A2aLabError::invalid(field, "must not be empty"));
    }
    Ok(())
}

/// Link from an agent-facing lab identifier to a protocol endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Binding {
    lab_id: String,
    asset_key: AssetKey,
    semantic_id: SemanticId,
    role: BindingRole,
    endpoint: Endpoint,
}

impl Binding {
    /// Creates a binding after validating the lab token and endpoint.
    pub fn new(
        lab_id: impl Into<String>,
        asset_key: AssetKey,
        semantic_id: SemanticId,
        role: BindingRole,
        endpoint: Endpoint,
    ) -> Result<Self, A2aLabError> {
        let lab_id = lab_id.into();
        id::parse_public(&lab_id)?;
        endpoint.check()?;
        Ok(Self {
            lab_id,
            asset_key,
            semantic_id,
            role,
            endpoint,
        })
    }

    /// Agent-facing identifier.
    #[must_use]
    pub fn lab_id(&self) -> &str {
        &self.lab_id
    }

    /// Asset that owns the binding.
    #[must_use]
    pub const fn asset_key(&self) -> &AssetKey {
        &self.asset_key
    }

    /// Semantic identifier.
    #[must_use]
    pub const fn semantic_id(&self) -> &SemanticId {
        &self.semantic_id
    }

    /// Lab role.
    #[must_use]
    pub const fn role(&self) -> BindingRole {
        self.role
    }

    /// Protocol endpoint.
    #[must_use]
    pub const fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

/// Asset record returned by the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Asset {
    key: AssetKey,
    global_asset_id: Option<String>,
    bindings: Vec<Binding>,
}

impl Asset {
    /// Creates an asset.
    #[must_use]
    pub const fn new(
        key: AssetKey,
        global_asset_id: Option<String>,
        bindings: Vec<Binding>,
    ) -> Self {
        Self {
            key,
            global_asset_id,
            bindings,
        }
    }

    /// Asset key.
    #[must_use]
    pub const fn key(&self) -> &AssetKey {
        &self.key
    }

    /// Optional global asset identifier.
    #[must_use]
    pub fn global_asset_id(&self) -> Option<&str> {
        self.global_asset_id.as_deref()
    }

    /// Bindings owned by the asset.
    #[must_use]
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }
}

/// Page request for catalog listings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListAssetsRequest {
    /// Page bounds.
    pub page: PageRequest,
}

/// Page request for bindings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListBindingsRequest {
    /// Page bounds.
    pub page: PageRequest,
}
