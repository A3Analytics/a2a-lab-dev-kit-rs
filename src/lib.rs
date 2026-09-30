//! Rust SDK for lab logs, metrics, and workflows.
//!
//! Provider traits are the source of truth. The A2A HTTP+JSON adapter and the
//! MCP adapter expose the same seven operations over those traits.

#![allow(clippy::doc_markdown)]

pub mod a2a;
pub mod error;
pub mod id;
pub mod json_object;
pub mod logs;
pub mod mcp;
pub mod memory;
pub mod metrics;
pub mod page;
pub mod service;
pub mod time;
pub mod workflows;

pub use a2a::{
    A2A_PROTOCOL_VERSION, A2aClient, A2aServer, LAB_MEDIA_TYPE, StreamEvent, bind_local,
};
pub use error::SdkError;
pub use id::{MetricId, RunId, SourceId, WorkflowId};
pub use json_object::JsonObject;
pub use logs::{
    ListLogSourcesRequest, LogLevel, LogProvider, LogRecord, LogSource, QueryLogsRequest,
};
pub use mcp::McpServer;
pub use memory::{MemoryLogs, MemoryMetrics, MemoryWorkflows};
pub use metrics::{
    ListMetricsRequest, MetricDescriptor, MetricPoint, MetricProvider, QueryMetricRequest,
};
pub use page::{MAX_PAGE_LIMIT, Page, PageRequest};
pub use service::{LabApi, LabCommand, LabOutcome, LabResult, LabService, TaskSnapshot, TaskState};
pub use time::{TimeRange, UtcTimestamp};
pub use workflows::{
    GetWorkflowStatusRequest, ListWorkflowsRequest, RunState, StartWorkflowRequest,
    WorkflowDefinition, WorkflowProvider, WorkflowRun,
};

/// Returns the SDK package version.
#[must_use]
pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod version_tests {
    #[test]
    fn reports_package_version() {
        assert_eq!(super::version(), env!("CARGO_PKG_VERSION"));
    }
}
