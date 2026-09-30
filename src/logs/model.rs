//! Log catalog and record types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::id::SourceId;
use crate::json_object::JsonObject;
use crate::page::PageRequest;
use crate::time::{TimeRange, UtcTimestamp};

/// Severity recorded with a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Fine-grained diagnostic.
    Trace,
    /// Debugging detail.
    Debug,
    /// Normal operation.
    Info,
    /// Recoverable concern.
    Warn,
    /// Failed operation.
    Error,
}

/// A log source the agent can query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogSource {
    /// Stable source identifier.
    pub id: SourceId,
    /// Human-readable name.
    pub name: String,
    /// What the source contains.
    pub description: String,
    /// Asset identifier from the industrial catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    /// Semantic identifier from the industrial catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_id: Option<String>,
}

/// One structured log record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LogRecord {
    /// Source that emitted the record.
    pub source_id: SourceId,
    /// When the record was emitted.
    pub timestamp: UtcTimestamp,
    /// Severity.
    pub level: LogLevel,
    /// Rendered message.
    pub message: String,
    /// Structured attributes.
    pub attributes: JsonObject,
}

/// Request for a page of log sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListLogSourcesRequest {
    /// Page bounds.
    pub page: PageRequest,
}

/// Request for log records in a half-open time range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QueryLogsRequest {
    /// Source to read.
    pub source_id: SourceId,
    /// Half-open UTC interval.
    pub range: TimeRange,
    /// Page bounds.
    pub page: PageRequest,
}
