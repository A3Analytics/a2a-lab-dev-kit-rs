//! Task provider interface.

use std::time::Duration;

use crate::error::SdkError;
use crate::id::RunId;
use crate::page::Page;
use crate::tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskRun,
};

const WAIT_POLL: Duration = Duration::from_millis(50);
const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(60);

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

    /// Cancels a started run. Providers that cannot cancel return an error.
    fn cancel(
        &self,
        _request: GetTaskStatusRequest,
    ) -> impl Future<Output = Result<TaskRun, SdkError>> + Send {
        async { Err(SdkError::unavailable("task is not cancelable")) }
    }
}

/// Starts a task. Waits until the run is terminal unless `request.wait` is false.
pub async fn start_run<P: TaskProvider>(
    provider: &P,
    request: StartTaskRequest,
) -> Result<TaskRun, SdkError> {
    let wait = request.wait;
    let timeout = wait_timeout(&request)?;
    let run = provider.start(request).await?;
    if wait {
        wait_for_run(provider, &run.id, timeout).await
    } else {
        Ok(run)
    }
}

fn wait_timeout(request: &StartTaskRequest) -> Result<Duration, SdkError> {
    if !request.wait {
        return Ok(DEFAULT_WAIT_TIMEOUT);
    }
    match request.timeout_seconds {
        None => Ok(DEFAULT_WAIT_TIMEOUT),
        Some(0) => Err(SdkError::invalid("timeout_seconds", "must be at least 1")),
        Some(seconds) => Ok(Duration::from_secs(u64::from(seconds))),
    }
}

async fn wait_for_run<P: TaskProvider>(
    provider: &P,
    id: &RunId,
    timeout: Duration,
) -> Result<TaskRun, SdkError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let run = provider
            .status(GetTaskStatusRequest { id: id.clone() })
            .await?;
        if run.state.is_terminal() {
            return Ok(run);
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(SdkError::unavailable("start_task wait timed out"));
        }
        tokio::time::sleep((deadline - now).min(WAIT_POLL)).await;
    }
}
