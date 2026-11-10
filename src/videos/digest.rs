//! SHA-256 digests for saved media payloads and indexing metadata.

use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::error::A2aLabError;

const HEX_LEN: usize = 64;

/// A lowercase SHA-256 digest in `sha256:<hex>` form.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    /// Validates and stores a digest string.
    pub fn parse(value: &str) -> Result<Self, A2aLabError> {
        let hex = value
            .strip_prefix("sha256:")
            .unwrap_or(value)
            .to_ascii_lowercase();
        if hex.len() != HEX_LEN || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(A2aLabError::invalid(
                "digest",
                "must be sha256: followed by 64 lowercase hex digits",
            ));
        }
        Ok(Self(format!("sha256:{hex}")))
    }

    /// Digests `bytes` and returns the canonical form.
    #[must_use]
    pub fn hash_bytes(bytes: &[u8]) -> Self {
        let digest = Sha256::digest(bytes);
        let mut hex = String::with_capacity(HEX_LEN);
        for byte in digest {
            use std::fmt::Write;
            let _ = write!(hex, "{byte:02x}");
        }
        Self(format!("sha256:{hex}"))
    }

    /// Full `sha256:<hex>` value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Hex digits without the prefix.
    #[must_use]
    pub fn hex(&self) -> &str {
        self.0.strip_prefix("sha256:").unwrap_or(&self.0)
    }
}

impl Serialize for Sha256Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Sha256Digest {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Sha256Digest".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^sha256:[0-9a-f]{64}$"
        })
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn accepts_canonical_prefix() {
        let hex = "a".repeat(64);
        let digest = Sha256Digest::parse(&format!("sha256:{hex}")).unwrap();
        assert_eq!(digest.as_str(), format!("sha256:{hex}"));
    }

    #[test]
    fn hash_bytes_is_stable() {
        let first = Sha256Digest::hash_bytes(b"fixture");
        let second = Sha256Digest::hash_bytes(b"fixture");
        assert_eq!(first, second);
        assert_ne!(first, Sha256Digest::hash_bytes(b"other"));
    }
}
