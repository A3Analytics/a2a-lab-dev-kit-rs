//! Server identity checked against the SiLAService constraints.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::error::A2aLabError;
use crate::sila::constraints::{bounded_characters, execution_uuid, server_version, vendor_url};

/// Identity published by [`SiLAService`](crate::sila::wire::sila2::org::silastandard::core::silaservice::v1::si_la_service_server::SiLaService).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SilaIdentity {
    /// Stable lowercase server UUID.
    pub server_uuid: String,
    /// Human-readable name. `SetServerName` may change it.
    pub server_name: String,
    /// Server type matching `[A-Z][a-zA-Z0-9]*`.
    pub server_type: String,
    /// Purpose of this server.
    pub description: String,
    /// Software version, such as `0.1.0`.
    pub version: String,
    /// Vendor or product URL.
    pub vendor_url: String,
    name_path: Option<PathBuf>,
}

impl SilaIdentity {
    /// Identity for this crate, with a caller-supplied stable UUID.
    pub fn lab_dev_kit(server_uuid: impl Into<String>) -> Result<Self, A2aLabError> {
        Self::new(
            server_uuid,
            "a2a-lab",
            "LabDevKit",
            "SiLA 2 Feature Provider for lab logs, metrics, and tasks.",
            env!("CARGO_PKG_VERSION"),
            "https://github.com/A3Analytics/a2a-lab-dev-kit-rs",
        )
    }

    /// Checks every SiLAService constraint used by this server.
    pub fn new(
        server_uuid: impl Into<String>,
        server_name: impl Into<String>,
        server_type: impl Into<String>,
        description: impl Into<String>,
        version: impl Into<String>,
        vendor_url: impl Into<String>,
    ) -> Result<Self, A2aLabError> {
        let identity = Self {
            server_uuid: server_uuid.into(),
            server_name: server_name.into(),
            server_type: server_type.into(),
            description: description.into(),
            version: version.into(),
            vendor_url: vendor_url.into(),
            name_path: None,
        };
        identity.check()?;
        Ok(identity)
    }

    /// Restores a previously set server name from `path` when that file exists.
    pub fn persist_name(mut self, path: PathBuf) -> Result<Self, A2aLabError> {
        if let Ok(name) = fs::read_to_string(&path) {
            let name = name.trim();
            if !name.is_empty() {
                self.server_name = name.to_string();
                self.check()?;
            }
        }
        self.name_path = Some(path);
        Ok(self)
    }

    pub(crate) fn share(self) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(self))
    }

    pub(crate) fn set_name(&mut self, name: String) -> Result<(), A2aLabError> {
        if !bounded_characters(&name, 255) {
            return Err(A2aLabError::invalid(
                "server_name",
                "must be at most 255 characters",
            ));
        }
        self.server_name = name;
        if let Some(path) = &self.name_path {
            fs::write(path, &self.server_name)
                .map_err(|error| A2aLabError::unavailable(error.to_string()))?;
        }
        Ok(())
    }

    fn check(&self) -> Result<(), A2aLabError> {
        if !execution_uuid(&self.server_uuid) {
            return Err(A2aLabError::invalid(
                "server_uuid",
                "must be a lowercase UUID",
            ));
        }
        if !bounded_characters(&self.server_name, 255) {
            return Err(A2aLabError::invalid(
                "server_name",
                "must be at most 255 characters",
            ));
        }
        if !valid_server_type(&self.server_type) {
            return Err(A2aLabError::invalid(
                "server_type",
                "must match [A-Z][a-zA-Z0-9]*",
            ));
        }
        if !server_version(&self.version) {
            return Err(A2aLabError::invalid(
                "version",
                "must be a major.minor[.patch][_suffix] version",
            ));
        }
        if !vendor_url(&self.vendor_url) {
            return Err(A2aLabError::invalid(
                "vendor_url",
                "must be an http or https URL",
            ));
        }
        Ok(())
    }
}

fn valid_server_type(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_uppercase())
        && chars.all(|character| character.is_ascii_alphanumeric())
}
