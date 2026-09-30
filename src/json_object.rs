//! JSON objects accepted as workflow input.

use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::error::SdkError;

/// A JSON object. Arrays and scalars are rejected.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonObject(Map<String, Value>);

impl JsonObject {
    /// Creates an empty object.
    #[must_use]
    pub fn empty() -> Self {
        Self(Map::new())
    }

    /// Accepts a JSON object value.
    pub fn try_from_value(value: Value) -> Result<Self, SdkError> {
        match value {
            Value::Object(map) => Ok(Self(map)),
            _ => Err(SdkError::invalid("input", "must be a JSON object")),
        }
    }

    /// Parses a JSON object document.
    pub fn parse(document: &str) -> Result<Self, SdkError> {
        let value = serde_json::from_str(document)
            .map_err(|_| SdkError::invalid("input", "must be valid JSON"))?;
        Self::try_from_value(value)
    }

    /// Returns the underlying map.
    #[must_use]
    pub const fn as_map(&self) -> &Map<String, Value> {
        &self.0
    }

    /// Converts this object into a JSON value.
    #[must_use]
    pub fn into_value(self) -> Value {
        Value::Object(self.0)
    }
}

impl Serialize for JsonObject {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for JsonObject {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::try_from_value(value).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for JsonObject {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "JsonObject".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "object",
            "additionalProperties": true
        })
    }
}

impl fmt::Display for JsonObject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", Value::Object(self.0.clone()))
    }
}
