//! Deterministic live source for tests.

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::catalog::Endpoint;
use crate::error::SdkError;
use crate::industrial::LiveRun;
use crate::industrial::LiveSource;
use crate::json_object::JsonObject;
use crate::logs::LogRecord;
use crate::metrics::MetricPoint;
use crate::page::{Page, PageRequest, slice_page};
use crate::tasks::TaskState;
use crate::time::TimeRange;

#[derive(Default)]
struct Script {
    logs: Vec<LogRecord>,
    metrics: Vec<MetricPoint>,
    runs: BTreeMap<String, LiveRun>,
    next: u64,
}

/// Live source whose responses are supplied by the test.
#[derive(Clone, Default)]
pub struct ScriptedLive {
    inner: Arc<Mutex<Script>>,
}

impl ScriptedLive {
    /// Creates an empty script.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a log record.
    pub async fn insert_log(&self, record: LogRecord) {
        self.inner.lock().await.logs.push(record);
    }

    /// Adds a metric sample.
    pub async fn insert_metric(&self, point: MetricPoint) {
        self.inner.lock().await.metrics.push(point);
    }

    /// Changes a run state.
    pub async fn transition(&self, run_id: &str, state: TaskState) -> Result<(), SdkError> {
        let mut script = self.inner.lock().await;
        let run = script
            .runs
            .get_mut(run_id)
            .ok_or_else(|| SdkError::not_found("task run", run_id))?;
        run.state = state;
        Ok(())
    }
}

impl LiveSource for ScriptedLive {
    async fn query_logs(
        &self,
        _endpoint: &Endpoint,
        range: TimeRange,
        page: PageRequest,
    ) -> Result<Page<LogRecord>, SdkError> {
        let script = self.inner.lock().await;
        let records: Vec<_> = script
            .logs
            .iter()
            .filter(|record| range.contains(record.timestamp))
            .cloned()
            .collect();
        slice_page(&records, &page)
    }

    async fn query_metrics(
        &self,
        _endpoint: &Endpoint,
        range: TimeRange,
        page: PageRequest,
    ) -> Result<Page<MetricPoint>, SdkError> {
        let script = self.inner.lock().await;
        let points: Vec<_> = script
            .metrics
            .iter()
            .filter(|point| range.contains(point.timestamp))
            .copied()
            .collect();
        slice_page(&points, &page)
    }

    async fn start(
        &self,
        _endpoint: &Endpoint,
        task_id: &str,
        _input: JsonObject,
    ) -> Result<LiveRun, SdkError> {
        let mut script = self.inner.lock().await;
        script.next += 1;
        let run = LiveRun {
            id: format!("run-{}", script.next),
            task_id: task_id.to_owned(),
            state: TaskState::Submitted,
            message: None,
        };
        script.runs.insert(run.id.clone(), run.clone());
        Ok(run)
    }

    async fn status(&self, run_id: &str) -> Result<LiveRun, SdkError> {
        self.inner
            .lock()
            .await
            .runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| SdkError::not_found("task run", run_id))
    }
}
