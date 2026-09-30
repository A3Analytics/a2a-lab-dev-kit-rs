//! Opaque identifiers for lab resources.

use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::SdkError;

const MAX_LEN: usize = 128;

pub(crate) fn validate(value: &str) -> Result<String, SdkError> {
    let allowed = value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
    });
    if value.is_empty() || value.len() > MAX_LEN || !allowed {
        return Err(SdkError::invalid(
            "id",
            format!("must be 1..={MAX_LEN} ASCII letters, digits, or . _ : -"),
        ));
    }
    Ok(value.to_owned())
}

macro_rules! id_type {
    ($name:ident, $label:literal) => {
        #[doc = concat!("Identifier for a ", $label, ".")]
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            /// Validates and creates an identifier.
            pub fn new(value: impl AsRef<str>) -> Result<Self, SdkError> {
                validate(value.as_ref()).map(Self)
            }

            /// Returns the identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_LEN
                })
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    };
}

id_type!(SourceId, "log source");
id_type!(MetricId, "metric");
id_type!(WorkflowId, "workflow definition");
id_type!(RunId, "workflow run");
