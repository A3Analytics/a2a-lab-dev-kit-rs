//! Read-only consumers versus authorized Jev producers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Caller role for frame decision transport APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FrameDecisionClientRole {
    /// May read history only.
    ReadOnlyConsumer,
    /// May append production records and read history.
    AuthorizedProducer,
}

/// Append/read operations exposed when a ledger provider is registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FrameDecisionOperation {
    /// Bounded history read.
    ReadFrameDecisionHistory,
    /// Authorized Jev production append.
    AppendFrameDecision,
}

/// JSON Schema type names advertised during capability discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDecisionSchemaRefs {
    append_request: String,
    append_result: String,
    read_request: String,
    read_result: String,
}

impl FrameDecisionSchemaRefs {
    /// Stable schema names for registered append/read APIs.
    #[must_use]
    pub fn registered() -> Self {
        Self {
            append_request: "AppendFrameDecisionRequest".to_owned(),
            append_result: "AppendFrameDecisionResult".to_owned(),
            read_request: "ReadFrameDecisionHistoryRequest".to_owned(),
            read_result: "FrameDecisionHistoryResult".to_owned(),
        }
    }

    /// Append request schema name.
    #[must_use]
    pub fn append_request(&self) -> &str {
        &self.append_request
    }

    /// Append result schema name.
    #[must_use]
    pub fn append_result(&self) -> &str {
        &self.append_result
    }

    /// Read request schema name.
    #[must_use]
    pub fn read_request(&self) -> &str {
        &self.read_request
    }

    /// Read result schema name.
    #[must_use]
    pub fn read_result(&self) -> &str {
        &self.read_result
    }
}
