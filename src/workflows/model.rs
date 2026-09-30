//! Workflow definition and run types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::id::{RunId, WorkflowId};
use crate::json_object::JsonObject;
use crate::page::PageRequest;

/// Lifecycle of a workflow run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Accepted and waiting to run.
    Submitted,
    /// Currently executing.
    Working,
    /// Finished successfully.
    Completed,
    /// Finished with an error.
    Failed,
    /// Stopped before completion.
    Canceled,
}

impl RunState {
    /// Reports whether the run will not change on its own.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Canceled)
    }
}

/// A workflow the agent can start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowDefinition {
    /// Stable workflow identifier.
    pub id: WorkflowId,
    /// Human-readable name.
    pub name: String,
    /// What the workflow does.
    pub description: String,
}

/// A started workflow and its current status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowRun {
    /// Stable run identifier.
    pub id: RunId,
    /// Workflow that was started.
    pub workflow_id: WorkflowId,
    /// Current lifecycle state.
    pub state: RunState,
    /// Object supplied at start.
    pub input: JsonObject,
    /// Optional status detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Request for a page of workflow definitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListWorkflowsRequest {
    /// Page bounds.
    pub page: PageRequest,
}

/// Request to start a workflow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StartWorkflowRequest {
    /// Workflow to start.
    pub workflow_id: WorkflowId,
    /// JSON object passed to the workflow.
    pub input: JsonObject,
}

/// Request for the status of an existing run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetWorkflowStatusRequest {
    /// Run to inspect.
    pub run_id: RunId,
}
