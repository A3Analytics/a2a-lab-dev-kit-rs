//! Shared execution of the seven lab operations.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::SdkError;
use crate::id::RunId;
use crate::logs::{ListLogSourcesRequest, LogProvider, LogRecord, LogSource, QueryLogsRequest};
use crate::metrics::{
    ListMetricsRequest, MetricDescriptor, MetricPoint, MetricProvider, QueryMetricRequest,
};
use crate::page::{Page, PageRequest};
use crate::tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskProvider,
    TaskRun, TaskState,
};

/// Future returned by [`LabApi`].
pub type LabFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Tagged request envelope shared by A2A and MCP.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub enum LabCommand {
    /// List log sources.
    ListLogSources(ListLogSourcesRequest),
    /// Query logs.
    QueryLogs(QueryLogsRequest),
    /// List metrics.
    ListMetrics(ListMetricsRequest),
    /// Query one metric.
    QueryMetric(QueryMetricRequest),
    /// List tasks.
    ListTasks(ListTasksRequest),
    /// Start a task.
    StartTask(StartTaskRequest),
    /// Read task status.
    GetTaskStatus(GetTaskStatusRequest),
}

/// Tagged result envelope shared by A2A and MCP.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "result", rename_all = "snake_case")]
pub enum LabResult {
    /// A page of log sources.
    ListLogSources(Page<LogSource>),
    /// A page of log records.
    QueryLogs(Page<LogRecord>),
    /// A page of metric descriptors.
    ListMetrics(Page<MetricDescriptor>),
    /// A page of metric samples.
    QueryMetric(Page<MetricPoint>),
    /// A page of task definitions.
    ListTasks(Page<TaskDefinition>),
    /// The run created by a start request.
    StartTask(TaskRun),
    /// The current run status.
    GetTaskStatus(TaskRun),
}

/// Stored view of an A2A task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskSnapshot {
    /// A2A task identifier.
    pub id: String,
    /// A2A context identifier.
    pub context_id: String,
    /// A2A task state.
    pub state: TaskState,
    /// Typed lab result carried by the task artifact.
    pub result: LabResult,
}

/// Outcome of executing one lab command.
#[derive(Debug, Clone, PartialEq)]
pub struct LabOutcome {
    /// Task created or updated by the command.
    pub task: TaskSnapshot,
}

/// Object-safe facade used by the protocol adapters.
pub trait LabApi: Send + Sync {
    /// Executes a lab command and records its task.
    fn execute(&self, command: LabCommand) -> LabFuture<'_, Result<LabOutcome, SdkError>>;

    /// Returns the current snapshot of a previously created task.
    fn task<'a>(&'a self, task_id: &str) -> LabFuture<'a, Result<TaskSnapshot, SdkError>>;
}

#[derive(Clone)]
struct StoredTask {
    context_id: String,
    body: StoredBody,
}

#[derive(Clone)]
enum StoredBody {
    Snapshot { state: TaskState, result: LabResult },
    Run(RunId),
}

/// Routes lab commands to the three provider traits.
pub struct LabService<L, M, W> {
    logs: L,
    metrics: M,
    tasks: W,
    task_store: Mutex<BTreeMap<String, StoredTask>>,
    ids: AtomicU64,
}

impl<L, M, W> LabService<L, M, W>
where
    L: LogProvider,
    M: MetricProvider,
    W: TaskProvider,
{
    /// Creates a service over the three providers.
    #[must_use]
    pub const fn new(logs: L, metrics: M, tasks: W) -> Self {
        Self {
            logs,
            metrics,
            tasks,
            task_store: Mutex::const_new(BTreeMap::new()),
            ids: AtomicU64::new(0),
        }
    }

    /// Shares the service with protocol adapters.
    #[must_use]
    pub fn share(self) -> Arc<dyn LabApi>
    where
        L: 'static,
        M: 'static,
        W: 'static,
    {
        Arc::new(self)
    }

    /// Executes a command and stores the resulting task.
    pub async fn execute(&self, command: LabCommand) -> Result<LabOutcome, SdkError> {
        let task = match command {
            LabCommand::ListLogSources(request) => self.list_sources(request).await?,
            LabCommand::QueryLogs(request) => self.query_logs(request).await?,
            LabCommand::ListMetrics(request) => self.list_metrics(request).await?,
            LabCommand::QueryMetric(request) => self.query_metric(request).await?,
            LabCommand::ListTasks(request) => self.list_tasks(request).await?,
            LabCommand::StartTask(request) => self.start_task(request).await?,
            LabCommand::GetTaskStatus(request) => self.task_status(request).await?,
        };
        Ok(LabOutcome { task })
    }

    /// Returns a task, refreshing task runs from the provider.
    pub async fn task(&self, task_id: &str) -> Result<TaskSnapshot, SdkError> {
        let stored = self
            .task_store
            .lock()
            .await
            .get(task_id)
            .cloned()
            .ok_or_else(|| SdkError::not_found("task", task_id))?;
        self.materialize(task_id, stored).await
    }

    async fn list_sources(&self, request: ListLogSourcesRequest) -> Result<TaskSnapshot, SdkError> {
        check_page(&request.page)?;
        let page = self.logs.list_sources(request).await?;
        self.store_snapshot(LabResult::ListLogSources(page)).await
    }

    async fn query_logs(&self, request: QueryLogsRequest) -> Result<TaskSnapshot, SdkError> {
        check_page(&request.page)?;
        request.range.check()?;
        let page = self.logs.query(request).await?;
        self.store_snapshot(LabResult::QueryLogs(page)).await
    }

    async fn list_metrics(&self, request: ListMetricsRequest) -> Result<TaskSnapshot, SdkError> {
        check_page(&request.page)?;
        let page = self.metrics.list_metrics(request).await?;
        self.store_snapshot(LabResult::ListMetrics(page)).await
    }

    async fn query_metric(&self, request: QueryMetricRequest) -> Result<TaskSnapshot, SdkError> {
        check_page(&request.page)?;
        request.range.check()?;
        let page = self.metrics.query(request).await?;
        self.store_snapshot(LabResult::QueryMetric(page)).await
    }

    async fn list_tasks(&self, request: ListTasksRequest) -> Result<TaskSnapshot, SdkError> {
        check_page(&request.page)?;
        let page = self.tasks.list_tasks(request).await?;
        self.store_snapshot(LabResult::ListTasks(page)).await
    }

    async fn start_task(&self, request: StartTaskRequest) -> Result<TaskSnapshot, SdkError> {
        let run = self.tasks.start(request).await?;
        let id = run.id.as_str().to_owned();
        let stored = StoredTask {
            context_id: context_id(&id),
            body: StoredBody::Run(run.id.clone()),
        };
        self.task_store
            .lock()
            .await
            .insert(id.clone(), stored.clone());
        self.materialize(&id, stored).await
    }

    async fn task_status(&self, request: GetTaskStatusRequest) -> Result<TaskSnapshot, SdkError> {
        let run = self.tasks.status(request).await?;
        self.store_snapshot(LabResult::GetTaskStatus(run)).await
    }

    async fn store_snapshot(&self, result: LabResult) -> Result<TaskSnapshot, SdkError> {
        let id = self.allocate("task");
        let stored = StoredTask {
            context_id: context_id(&id),
            body: StoredBody::Snapshot {
                state: TaskState::Completed,
                result,
            },
        };
        self.task_store
            .lock()
            .await
            .insert(id.clone(), stored.clone());
        self.materialize(&id, stored).await
    }

    async fn materialize(&self, id: &str, stored: StoredTask) -> Result<TaskSnapshot, SdkError> {
        let (state, result) = match stored.body {
            StoredBody::Snapshot { state, result } => (state, result),
            StoredBody::Run(id) => {
                let run = self.tasks.status(GetTaskStatusRequest { id }).await?;
                (run.state, LabResult::StartTask(run))
            }
        };
        Ok(TaskSnapshot {
            id: id.to_owned(),
            context_id: stored.context_id,
            state,
            result,
        })
    }

    fn allocate(&self, prefix: &str) -> String {
        let number = self.ids.fetch_add(1, Ordering::Relaxed) + 1;
        format!("{prefix}-{number}")
    }
}

impl<L, M, W> LabApi for LabService<L, M, W>
where
    L: LogProvider + 'static,
    M: MetricProvider + 'static,
    W: TaskProvider + 'static,
{
    fn execute(&self, command: LabCommand) -> LabFuture<'_, Result<LabOutcome, SdkError>> {
        Box::pin(LabService::execute(self, command))
    }

    fn task<'a>(&'a self, task_id: &str) -> LabFuture<'a, Result<TaskSnapshot, SdkError>> {
        let task_id = task_id.to_owned();
        Box::pin(async move { LabService::task(self, &task_id).await })
    }
}

fn check_page(page: &PageRequest) -> Result<(), SdkError> {
    page.check()
}

fn context_id(task_id: &str) -> String {
    format!("ctx-{task_id}")
}
