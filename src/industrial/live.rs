//! Live reads and commands shared by OPC UA and SiLA clients.

use crate::catalog::Endpoint;
use crate::error::SdkError;
use crate::json_object::JsonObject;
use crate::logs::LogRecord;
use crate::metrics::MetricPoint;
use crate::page::{Page, PageRequest};
use crate::time::TimeRange;
use crate::workflows::RunState;

/// Result of starting a live command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRun {
    /// Run identifier.
    pub id: String,
    /// Workflow that was started.
    pub workflow_id: String,
    /// Current state.
    pub state: RunState,
    /// Optional detail.
    pub message: Option<String>,
}

/// Reads history and starts commands for one bound endpoint.
pub trait LiveSource: Send + Sync {
    /// Returns log records inside the half-open range.
    fn query_logs(
        &self,
        endpoint: &Endpoint,
        range: TimeRange,
        page: PageRequest,
    ) -> impl Future<Output = Result<Page<LogRecord>, SdkError>> + Send;

    /// Returns metric samples inside the half-open range.
    fn query_metrics(
        &self,
        endpoint: &Endpoint,
        range: TimeRange,
        page: PageRequest,
    ) -> impl Future<Output = Result<Page<MetricPoint>, SdkError>> + Send;

    /// Starts a command.
    fn start(
        &self,
        endpoint: &Endpoint,
        workflow_id: &str,
        input: JsonObject,
    ) -> impl Future<Output = Result<LiveRun, SdkError>> + Send;

    /// Returns the status of a previously started command.
    fn status(&self, run_id: &str) -> impl Future<Output = Result<LiveRun, SdkError>> + Send;
}
