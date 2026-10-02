//! Rust dev kit for lab logs, metrics, and tasks.
//!
//! Provider traits are the source of truth. The A2A adapter (HTTP+JSON, JSON-RPC,
//! and gRPC) and the MCP adapter expose the same seven operations over those traits.

#![allow(clippy::doc_markdown)]

pub mod a2a;
#[cfg(feature = "aas")]
pub mod aas;
pub mod catalog;
pub mod error;
pub mod id;
pub mod industrial;
pub mod json_object;
pub mod logs;
pub mod mcp;
pub mod memory;
pub mod metrics;
#[cfg(feature = "opcua")]
pub mod opcua;
pub mod page;
pub mod ros2;
pub mod service;
#[cfg(feature = "sila2")]
pub mod sila;
pub mod tasks;
pub mod time;

pub use a2a::{
    A2A_PROTOCOL_VERSION, A2aClient, A2aServer, AgentCard, HttpAuthSecurityScheme, LAB_MEDIA_TYPE,
    SecurityScheme, StreamResponse, Task, TaskPushNotificationConfig, bind_local,
};
pub use catalog::{
    Asset, AssetCatalogProvider, AssetKey, Binding, BindingRole, Endpoint, ListAssetsRequest,
    ListBindingsRequest, OpcUaIdentityKind, ProtocolKind, SecurityMode, SemanticId, SemanticKind,
};
pub use error::A2aLabError;
pub use id::{MetricId, RunId, SourceId, TaskId};
pub use industrial::{
    IndustrialLabBuilder, IndustrialLogs, IndustrialMetrics, IndustrialTasks, ScriptedLive,
};
pub use json_object::JsonObject;
pub use logs::{
    ListLogSourcesRequest, LogLevel, LogProvider, LogRecord, LogSource, QueryLogsRequest,
};
pub use mcp::{DEFAULT_MCP_URL, McpLab, McpServer};
pub use memory::{MemoryCatalog, MemoryLogs, MemoryMetrics, MemoryTasks};
pub use metrics::{
    ListMetricsRequest, MetricDescriptor, MetricPoint, MetricProvider, QueryMetricRequest,
};
pub use page::{MAX_PAGE_LIMIT, Page, PageRequest};
pub use ros2::{MemoryRos2, Ros2Action, Ros2Goal, Ros2GoalStatus, Ros2Graph, Ros2Tasks};
pub use service::{LabApi, LabCommand, LabOutcome, LabResult, LabService, TaskSnapshot};
pub use tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskProvider,
    TaskRun, TaskState, start_run,
};
pub use time::{TimeRange, UtcTimestamp};

/// Returns the crate version.
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
