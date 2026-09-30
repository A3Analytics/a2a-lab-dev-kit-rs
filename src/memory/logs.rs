//! In-memory log provider.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::error::SdkError;
use crate::logs::{ListLogSourcesRequest, LogProvider, LogRecord, LogSource, QueryLogsRequest};
use crate::page::{Page, slice_page};

#[derive(Default)]
struct LogState {
    sources: Vec<LogSource>,
    records: Vec<LogRecord>,
    unavailable: Option<String>,
}

/// In-memory [`LogProvider`] for examples and tests.
#[derive(Clone, Default)]
pub struct MemoryLogs {
    inner: Arc<Mutex<LogState>>,
}

impl MemoryLogs {
    /// Creates an empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a source.
    pub async fn insert_source(&self, source: LogSource) {
        self.inner.lock().await.sources.push(source);
    }

    /// Adds a record for an existing source.
    pub async fn insert_record(&self, record: LogRecord) -> Result<(), SdkError> {
        let mut state = self.inner.lock().await;
        if !state
            .sources
            .iter()
            .any(|source| source.id == record.source_id)
        {
            return Err(SdkError::not_found(
                "log source",
                record.source_id.to_string(),
            ));
        }
        state.records.push(record);
        Ok(())
    }

    /// Makes later calls fail until [`Self::clear_unavailable`].
    pub async fn set_unavailable(&self, message: impl Into<String>) {
        self.inner.lock().await.unavailable = Some(message.into());
    }

    /// Clears a simulated provider failure.
    pub async fn clear_unavailable(&self) {
        self.inner.lock().await.unavailable = None;
    }
}

impl LogProvider for MemoryLogs {
    async fn list_sources(
        &self,
        request: ListLogSourcesRequest,
    ) -> Result<Page<LogSource>, SdkError> {
        let state = self.inner.lock().await;
        fail_if_unavailable(state.unavailable.as_deref())?;
        let mut sources = state.sources.clone();
        sources.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        slice_page(&sources, &request.page)
    }

    async fn query(&self, request: QueryLogsRequest) -> Result<Page<LogRecord>, SdkError> {
        request.range.check()?;
        let state = self.inner.lock().await;
        fail_if_unavailable(state.unavailable.as_deref())?;
        if !state
            .sources
            .iter()
            .any(|source| source.id == request.source_id)
        {
            return Err(SdkError::not_found(
                "log source",
                request.source_id.to_string(),
            ));
        }
        let mut records: Vec<_> = state
            .records
            .iter()
            .filter(|record| {
                record.source_id == request.source_id && request.range.contains(record.timestamp)
            })
            .cloned()
            .collect();
        records.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.message.cmp(&right.message))
        });
        slice_page(&records, &request.page)
    }
}

fn fail_if_unavailable(message: Option<&str>) -> Result<(), SdkError> {
    match message {
        Some(message) => Err(SdkError::unavailable(message)),
        None => Ok(()),
    }
}
