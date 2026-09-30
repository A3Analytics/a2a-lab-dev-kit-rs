//! Task definition and A2A task types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::SdkError;
use crate::id::{RunId, TaskId};
use crate::json_object::JsonObject;
use crate::page::PageRequest;

/// Lifecycle of an A2A task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// The task has been accepted.
    Submitted,
    /// The task is running.
    Working,
    /// The task finished successfully.
    Completed,
    /// The task finished with an error.
    Failed,
    /// The task was canceled.
    Canceled,
}

impl TaskState {
    /// Reports whether no further state changes are expected.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Canceled)
    }

    /// Returns the A2A 1.0 protocol state name.
    #[must_use]
    pub const fn as_protocol(&self) -> &'static str {
        match self {
            Self::Submitted => "TASK_STATE_SUBMITTED",
            Self::Working => "TASK_STATE_WORKING",
            Self::Completed => "TASK_STATE_COMPLETED",
            Self::Failed => "TASK_STATE_FAILED",
            Self::Canceled => "TASK_STATE_CANCELED",
        }
    }

    /// Parses an A2A 1.0 protocol state name.
    pub fn from_protocol(value: &str) -> Result<Self, SdkError> {
        match value {
            "TASK_STATE_SUBMITTED" => Ok(Self::Submitted),
            "TASK_STATE_WORKING" => Ok(Self::Working),
            "TASK_STATE_COMPLETED" => Ok(Self::Completed),
            "TASK_STATE_FAILED" => Ok(Self::Failed),
            "TASK_STATE_CANCELED" => Ok(Self::Canceled),
            _ => Err(SdkError::protocol(format!("unknown task state `{value}`"))),
        }
    }
}

/// A task the agent can start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskDefinition {
    /// Stable task identifier.
    pub id: TaskId,
    /// Human-readable name.
    pub name: String,
    /// What the task does.
    pub description: String,
    /// Asset identifier from the industrial catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    /// Semantic identifier from the industrial catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_id: Option<String>,
}

/// A started task. Its `id` is the A2A task id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskRun {
    /// A2A task identifier assigned at start.
    pub id: RunId,
    /// Definition that was started.
    pub task_id: TaskId,
    /// A2A task state.
    pub state: TaskState,
    /// Object supplied at start.
    pub input: JsonObject,
    /// Optional status detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Request for a page of task definitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListTasksRequest {
    /// Page bounds.
    pub page: PageRequest,
}

/// Request to start a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StartTaskRequest {
    /// Definition to start.
    pub task_id: TaskId,
    /// JSON object passed to the task.
    pub input: JsonObject,
    /// When true, wait until the run is terminal before returning.
    /// When false, return as soon as the run is accepted. Defaults to true.
    #[serde(default = "default_wait", skip_serializing_if = "is_true")]
    pub wait: bool,
    /// Seconds to wait when [`Self::wait`] is true. Default 60.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
}

const fn default_wait() -> bool {
    true
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_true(value: &bool) -> bool {
    *value
}

impl StartTaskRequest {
    /// Starts a task and waits until the run is terminal.
    #[must_use]
    pub fn new(task_id: TaskId, input: JsonObject) -> Self {
        Self {
            task_id,
            input,
            wait: true,
            timeout_seconds: None,
        }
    }

    /// Return as soon as the run is accepted.
    #[must_use]
    pub fn immediate(mut self) -> Self {
        self.wait = false;
        self
    }
}

/// A2A Get Task request for a previously started task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetTaskStatusRequest {
    /// A2A task identifier (the run id assigned at start).
    pub id: RunId,
}
