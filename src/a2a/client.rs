//! Typed A2A client for the seven lab operations.

use std::sync::atomic::{AtomicU64, Ordering};

use reqwest::Url;

use crate::error::SdkError;
use crate::logs::{ListLogSourcesRequest, LogRecord, LogSource, QueryLogsRequest};
use crate::metrics::{ListMetricsRequest, MetricDescriptor, MetricPoint, QueryMetricRequest};
use crate::page::Page;
use crate::service::{LabCommand, LabResult, TaskSnapshot};
use crate::tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskRun,
};

use super::wire::{self, ClientTask, StreamEvent};

/// Client for a lab agent speaking A2A HTTP+JSON.
pub struct A2aClient {
    http: reqwest::Client,
    base: Url,
    messages: AtomicU64,
}

impl A2aClient {
    /// Creates a client for an agent origin such as `http://127.0.0.1:8080`.
    pub fn new(base_url: &str) -> Result<Self, SdkError> {
        let base =
            Url::parse(base_url).map_err(|error| SdkError::invalid("url", error.to_string()))?;
        let http = reqwest::Client::builder()
            .build()
            .map_err(|error| SdkError::transport(error.to_string()))?;
        Ok(Self {
            http,
            base,
            messages: AtomicU64::new(0),
        })
    }

    /// Lists log sources.
    pub async fn list_log_sources(
        &self,
        request: ListLogSourcesRequest,
    ) -> Result<Page<LogSource>, SdkError> {
        self.result(LabCommand::ListLogSources(request), |result| match result {
            LabResult::ListLogSources(page) => Some(page),
            _ => None,
        })
        .await
    }

    /// Queries logs.
    pub async fn query_logs(&self, request: QueryLogsRequest) -> Result<Page<LogRecord>, SdkError> {
        self.result(LabCommand::QueryLogs(request), |result| match result {
            LabResult::QueryLogs(page) => Some(page),
            _ => None,
        })
        .await
    }

    /// Lists metrics.
    pub async fn list_metrics(
        &self,
        request: ListMetricsRequest,
    ) -> Result<Page<MetricDescriptor>, SdkError> {
        self.result(LabCommand::ListMetrics(request), |result| match result {
            LabResult::ListMetrics(page) => Some(page),
            _ => None,
        })
        .await
    }

    /// Queries one metric.
    pub async fn query_metric(
        &self,
        request: QueryMetricRequest,
    ) -> Result<Page<MetricPoint>, SdkError> {
        self.result(LabCommand::QueryMetric(request), |result| match result {
            LabResult::QueryMetric(page) => Some(page),
            _ => None,
        })
        .await
    }

    /// Lists tasks.
    pub async fn list_tasks(
        &self,
        request: ListTasksRequest,
    ) -> Result<Page<TaskDefinition>, SdkError> {
        self.result(LabCommand::ListTasks(request), |result| match result {
            LabResult::ListTasks(page) => Some(page),
            _ => None,
        })
        .await
    }

    /// Starts a task and returns the A2A task (`id` is the run id).
    pub async fn start_task(&self, request: StartTaskRequest) -> Result<TaskSnapshot, SdkError> {
        let snapshot = self.invoke(LabCommand::StartTask(request)).await?;
        expect_variant(snapshot, |result| matches!(result, LabResult::StartTask(_)))
    }

    /// A2A Get Task analog used by MCP: reads a started task through a completed status task.
    ///
    /// `task` is `GET /tasks/{id}` and reloads the original A2A task.
    pub async fn task_status(&self, request: GetTaskStatusRequest) -> Result<TaskRun, SdkError> {
        self.result(LabCommand::GetTaskStatus(request), |result| match result {
            LabResult::GetTaskStatus(run) => Some(run),
            _ => None,
        })
        .await
    }

    /// Fetches an existing task.
    pub async fn task(&self, task_id: &str) -> Result<TaskSnapshot, SdkError> {
        let response = self
            .http
            .get(self.url(&format!("tasks/{task_id}"))?)
            .send()
            .await
            .map_err(|error| SdkError::transport(error.to_string()))?;
        self.read_task(response).await
    }

    /// Collects SSE events until the task reaches a terminal state.
    pub async fn subscribe(&self, task_id: &str) -> Result<Vec<StreamEvent>, SdkError> {
        let response = self
            .http
            .get(self.url(&format!("tasks/{task_id}/subscribe"))?)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .await
            .map_err(|error| SdkError::transport(error.to_string()))?;
        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(wire::error_from_body(&body));
        }
        let body = response
            .text()
            .await
            .map_err(|error| SdkError::transport(error.to_string()))?;
        wire::parse_events(&body)
    }

    async fn result<T>(
        &self,
        command: LabCommand,
        pick: impl FnOnce(LabResult) -> Option<T>,
    ) -> Result<T, SdkError> {
        let snapshot = self.invoke(command).await?;
        pick(snapshot.result).ok_or_else(|| SdkError::protocol("unexpected result variant"))
    }

    async fn invoke(&self, command: LabCommand) -> Result<TaskSnapshot, SdkError> {
        let number = self.messages.fetch_add(1, Ordering::Relaxed) + 1;
        let body = wire::client_send(&format!("msg-{number}"), &command)?;
        let response = self
            .http
            .post(self.url("message:send")?)
            .json(&body)
            .send()
            .await
            .map_err(|error| SdkError::transport(error.to_string()))?;
        self.read_task(response).await
    }

    async fn read_task(&self, response: reqwest::Response) -> Result<TaskSnapshot, SdkError> {
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| SdkError::transport(error.to_string()))?;
        if !status.is_success() {
            return Err(wire::error_from_body(&body));
        }
        serde_json::from_str::<ClientTask>(&body)
            .map_err(|error| SdkError::protocol(error.to_string()))?
            .snapshot()
    }

    fn url(&self, path: &str) -> Result<Url, SdkError> {
        let mut base = self.base.as_str().trim_end_matches('/').to_owned();
        base.push('/');
        base.push_str(path.trim_start_matches('/'));
        Url::parse(&base).map_err(|error| SdkError::invalid("url", error.to_string()))
    }
}

fn expect_variant(
    snapshot: TaskSnapshot,
    matches_result: impl FnOnce(&LabResult) -> bool,
) -> Result<TaskSnapshot, SdkError> {
    if matches_result(&snapshot.result) {
        Ok(snapshot)
    } else {
        Err(SdkError::protocol("unexpected result variant"))
    }
}
