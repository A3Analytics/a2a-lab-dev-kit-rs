//! In-memory task provider.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::Mutex;

use crate::error::A2aLabError;
use crate::id::RunId;
use crate::page::{Page, slice_page};
use crate::tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskProvider,
    TaskRun, TaskState,
};

#[derive(Default)]
struct Store {
    definitions: Vec<TaskDefinition>,
    runs: Vec<TaskRun>,
    unavailable: Option<String>,
}

/// In-memory [`TaskProvider`] for examples and tests.
#[derive(Clone, Default)]
pub struct MemoryTasks {
    inner: Arc<Mutex<Store>>,
    ids: Arc<AtomicU64>,
}

impl MemoryTasks {
    /// Creates an empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a task definition.
    pub async fn insert(&self, definition: TaskDefinition) {
        self.inner.lock().await.definitions.push(definition);
    }

    /// Changes the state of an existing run.
    pub async fn transition(
        &self,
        run_id: &RunId,
        state: TaskState,
        message: Option<String>,
    ) -> Result<(), A2aLabError> {
        let mut data = self.inner.lock().await;
        let run = data
            .runs
            .iter_mut()
            .find(|run| &run.id == run_id)
            .ok_or_else(|| A2aLabError::not_found("task run", run_id.to_string()))?;
        run.state = state;
        run.message = message;
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

impl TaskProvider for MemoryTasks {
    async fn list_tasks(
        &self,
        request: ListTasksRequest,
    ) -> Result<Page<TaskDefinition>, A2aLabError> {
        let state = self.inner.lock().await;
        if let Some(message) = &state.unavailable {
            return Err(A2aLabError::unavailable(message.clone()));
        }
        let mut definitions = state.definitions.clone();
        definitions.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        slice_page(&definitions, &request.page)
    }

    async fn start(&self, request: StartTaskRequest) -> Result<TaskRun, A2aLabError> {
        let mut state = self.inner.lock().await;
        if let Some(message) = &state.unavailable {
            return Err(A2aLabError::unavailable(message.clone()));
        }
        if !state
            .definitions
            .iter()
            .any(|definition| definition.id == request.task_id)
        {
            return Err(A2aLabError::not_found("task", request.task_id.to_string()));
        }
        let number = self.ids.fetch_add(1, Ordering::Relaxed) + 1;
        let run = TaskRun {
            id: RunId::new(format!("run-{number}"))?,
            task_id: request.task_id,
            state: TaskState::Submitted,
            input: request.input,
            message: None,
            result: None,
            progress: None,
            error_kind: None,
            error_identifier: None,
        };
        state.runs.push(run.clone());
        Ok(run)
    }

    async fn status(&self, request: GetTaskStatusRequest) -> Result<TaskRun, A2aLabError> {
        let state = self.inner.lock().await;
        if let Some(message) = &state.unavailable {
            return Err(A2aLabError::unavailable(message.clone()));
        }
        state
            .runs
            .iter()
            .find(|run| run.id == request.id)
            .cloned()
            .ok_or_else(|| A2aLabError::not_found("task run", request.id.to_string()))
    }

    async fn cancel(&self, request: GetTaskStatusRequest) -> Result<TaskRun, A2aLabError> {
        let mut state = self.inner.lock().await;
        if let Some(message) = &state.unavailable {
            return Err(A2aLabError::unavailable(message.clone()));
        }
        let run = state
            .runs
            .iter_mut()
            .find(|run| run.id == request.id)
            .ok_or_else(|| A2aLabError::not_found("task run", request.id.to_string()))?;
        if run.state.is_terminal() {
            return Err(A2aLabError::unavailable("task is not cancelable"));
        }
        run.state = TaskState::Canceled;
        run.message = Some("canceled".to_owned());
        Ok(run.clone())
    }
}
