//! Live reads and commands shared by OPC UA and SiLA clients.

use crate::catalog::Endpoint;
use crate::error::A2aLabError;
use crate::json_object::JsonObject;
use crate::logs::LogRecord;
use crate::metrics::MetricPoint;
use crate::page::{Page, PageRequest};
use crate::tasks::TaskState;
use crate::time::TimeRange;

/// Result of starting a live command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRun {
    /// Run identifier.
    pub id: String,
    /// Task that was started.
    pub task_id: String,
    /// Current state.
    pub state: TaskState,
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
    ) -> impl Future<Output = Result<Page<LogRecord>, A2aLabError>> + Send;

    /// Returns metric samples inside the half-open range.
    fn query_metrics(
        &self,
        endpoint: &Endpoint,
        range: TimeRange,
        page: PageRequest,
    ) -> impl Future<Output = Result<Page<MetricPoint>, A2aLabError>> + Send;

    /// Starts a command.
    fn start(
        &self,
        endpoint: &Endpoint,
        task_id: &str,
        input: JsonObject,
    ) -> impl Future<Output = Result<LiveRun, A2aLabError>> + Send;

    /// Returns the status of a previously started command.
    fn status(&self, run_id: &str) -> impl Future<Output = Result<LiveRun, A2aLabError>> + Send;
}
