//! Asset and semantic identifiers used by industrial catalogs.

use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::SdkError;

fn required(field: &'static str, value: &str) -> Result<(), SdkError> {
    if value.is_empty() || value.len() > 2_048 {
        return Err(SdkError::invalid(field, "must be 1..=2048 characters"));
    }
    Ok(())
}

/// Identifier of an Asset Administration Shell or asset.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AssetKey(String);

impl AssetKey {
    /// Validates and creates an asset key.
    pub fn new(value: impl AsRef<str>) -> Result<Self, SdkError> {
        let value = value.as_ref();
        required("asset_key", value)?;
        Ok(Self(value.to_owned()))
    }

    /// Returns the identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for AssetKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AssetKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for AssetKey {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "AssetKey".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "minLength": 1, "maxLength": 2048 })
    }
}

impl fmt::Display for AssetKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Kind of semantic reference carried by a catalog binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SemanticKind {
    /// An absolute IRI.
    Iri,
    /// An IRDI such as an ECLASS code.
    Irdi,
    /// A custom identifier that is neither an IRI nor an IRDI.
    Custom,
}

/// Semantic identifier shared by AAS, OPC UA, and SiLA bindings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct SemanticId {
    kind: SemanticKind,
    value: String,
}

impl SemanticId {
    /// Creates a semantic identifier after checking its kind.
    pub fn new(kind: SemanticKind, value: impl Into<String>) -> Result<Self, SdkError> {
        let value = value.into();
        required("semantic_id", &value)?;
        let valid = match kind {
            SemanticKind::Iri => value.contains("://") || value.starts_with("urn:"),
            SemanticKind::Irdi => value.contains('#'),
            SemanticKind::Custom => !value.contains("://") && !value.contains('#'),
        };
        if !valid {
            return Err(SdkError::invalid(
                "semantic_id",
                "does not match its declared kind",
            ));
        }
        Ok(Self { kind, value })
    }

    /// Returns the identifier kind.
    #[must_use]
    pub const fn kind(&self) -> SemanticKind {
        self.kind
    }

    /// Returns the identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}
