//! Workflow provider interface.

use crate::error::SdkError;
use crate::page::Page;
use crate::workflows::{
    GetWorkflowStatusRequest, ListWorkflowsRequest, StartWorkflowRequest, WorkflowDefinition,
    WorkflowRun,
};

/// Lists workflows, starts runs, and reads run status.
pub trait WorkflowProvider: Send + Sync {
    /// Returns the workflows this agent can start.
    fn list_workflows(
        &self,
        request: ListWorkflowsRequest,
    ) -> impl Future<Output = Result<Page<WorkflowDefinition>, SdkError>> + Send;

    /// Starts a workflow and returns the new run.
    fn start(
        &self,
        request: StartWorkflowRequest,
    ) -> impl Future<Output = Result<WorkflowRun, SdkError>> + Send;

    /// Returns the current status of a run.
    fn status(
        &self,
        request: GetWorkflowStatusRequest,
    ) -> impl Future<Output = Result<WorkflowRun, SdkError>> + Send;
}
