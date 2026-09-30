//! Log provider interface.

use crate::error::SdkError;
use crate::logs::{ListLogSourcesRequest, LogRecord, LogSource, QueryLogsRequest};
use crate::page::Page;

/// Lists log sources and reads their records.
pub trait LogProvider: Send + Sync {
    /// Returns the sources this agent can read.
    fn list_sources(
        &self,
        request: ListLogSourcesRequest,
    ) -> impl Future<Output = Result<Page<LogSource>, SdkError>> + Send;

    /// Returns records for one source in the requested interval.
    fn query(
        &self,
        request: QueryLogsRequest,
    ) -> impl Future<Output = Result<Page<LogRecord>, SdkError>> + Send;
}
