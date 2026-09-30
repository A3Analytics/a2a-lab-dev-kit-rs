//! In-memory workflow provider.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::Mutex;

use crate::error::SdkError;
use crate::id::RunId;
use crate::page::{Page, slice_page};
use crate::workflows::{
    GetWorkflowStatusRequest, ListWorkflowsRequest, RunState, StartWorkflowRequest,
    WorkflowDefinition, WorkflowProvider, WorkflowRun,
};

#[derive(Default)]
struct WorkflowState {
    definitions: Vec<WorkflowDefinition>,
    runs: Vec<WorkflowRun>,
    unavailable: Option<String>,
}

/// In-memory [`WorkflowProvider`] for examples and tests.
#[derive(Clone, Default)]
pub struct MemoryWorkflows {
    inner: Arc<Mutex<WorkflowState>>,
    ids: Arc<AtomicU64>,
}

impl MemoryWorkflows {
    /// Creates an empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a workflow definition.
    pub async fn insert(&self, definition: WorkflowDefinition) {
        self.inner.lock().await.definitions.push(definition);
    }

    /// Changes the state of an existing run.
    pub async fn transition(
        &self,
        run_id: &RunId,
        state: RunState,
        message: Option<String>,
    ) -> Result<(), SdkError> {
        let mut data = self.inner.lock().await;
        let run = data
            .runs
            .iter_mut()
            .find(|run| &run.id == run_id)
            .ok_or_else(|| SdkError::not_found("workflow run", run_id.to_string()))?;
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

impl WorkflowProvider for MemoryWorkflows {
    async fn list_workflows(
        &self,
        request: ListWorkflowsRequest,
    ) -> Result<Page<WorkflowDefinition>, SdkError> {
        let state = self.inner.lock().await;
        if let Some(message) = &state.unavailable {
            return Err(SdkError::unavailable(message.clone()));
        }
        let mut definitions = state.definitions.clone();
        definitions.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        slice_page(&definitions, &request.page)
    }

    async fn start(&self, request: StartWorkflowRequest) -> Result<WorkflowRun, SdkError> {
        let mut state = self.inner.lock().await;
        if let Some(message) = &state.unavailable {
            return Err(SdkError::unavailable(message.clone()));
        }
        if !state
            .definitions
            .iter()
            .any(|definition| definition.id == request.workflow_id)
        {
            return Err(SdkError::not_found(
                "workflow",
                request.workflow_id.to_string(),
            ));
        }
        let number = self.ids.fetch_add(1, Ordering::Relaxed) + 1;
        let run = WorkflowRun {
            id: RunId::new(format!("run-{number}"))?,
            workflow_id: request.workflow_id,
            state: RunState::Submitted,
            input: request.input,
            message: None,
        };
        state.runs.push(run.clone());
        Ok(run)
    }

    async fn status(&self, request: GetWorkflowStatusRequest) -> Result<WorkflowRun, SdkError> {
        let state = self.inner.lock().await;
        if let Some(message) = &state.unavailable {
            return Err(SdkError::unavailable(message.clone()));
        }
        state
            .runs
            .iter()
            .find(|run| run.id == request.run_id)
            .cloned()
            .ok_or_else(|| SdkError::not_found("workflow run", request.run_id.to_string()))
    }
}
