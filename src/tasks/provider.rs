//! Task provider interface.

use crate::error::SdkError;
use crate::page::Page;
use crate::tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskRun,
};

/// Lists startable tasks, starts them, and reads A2A task status.
pub trait TaskProvider: Send + Sync {
    /// Returns the tasks this agent can start.
    fn list_tasks(
        &self,
        request: ListTasksRequest,
    ) -> impl Future<Output = Result<Page<TaskDefinition>, SdkError>> + Send;

    /// Starts a task and returns the A2A task (run id is the task id).
    fn start(
        &self,
        request: StartTaskRequest,
    ) -> impl Future<Output = Result<TaskRun, SdkError>> + Send;

    /// Returns the current A2A task status.
    fn status(
        &self,
        request: GetTaskStatusRequest,
    ) -> impl Future<Output = Result<TaskRun, SdkError>> + Send;
}
