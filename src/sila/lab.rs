//! `LabOperations` commands over [`LabApi`](crate::service::LabApi).

use std::pin::Pin;
use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::error::A2aLabError;
use crate::service::{LabApi, LabCommand, LabResult};
use crate::sila::constraints::EXECUTION_LIFETIME_SECONDS;
use crate::sila::errors::{framework, lab_error, reject_metadata, undefined, validation};
use crate::sila::executions::{Executions, View};
use crate::sila::values::{
    metric_message, parameter, point_message, read_json, read_metric_id, read_range, read_run_id,
    read_source_id, read_task_id, record_message, run_message, sila_bool, sila_string,
    source_message, task_message,
};
use crate::sila::wire::sila2::com::a3analytics::lab::laboperations::v1::lab_operations_server::LabOperations;
use crate::sila::wire::sila2::com::a3analytics::lab::laboperations::v1::{
    DataTypePageRequest, GetTaskStatusParameters, GetTaskStatusResponses, ListLogSourcesParameters,
    ListLogSourcesResponses, ListMetricsParameters, ListMetricsResponses, ListTasksParameters,
    ListTasksResponses, QueryLogsParameters, QueryLogsResponses, QueryMetricParameters,
    QueryMetricResponses, StartTaskParameters, StartTaskResponses,
};
use crate::sila::wire::sila2::org::silastandard::execution_info::CommandStatus;
use crate::sila::wire::sila2::org::silastandard::framework_error::ErrorType;
use crate::sila::wire::sila2::org::silastandard::{
    CommandConfirmation, CommandExecutionUuid, Duration, ExecutionInfo, Real,
};
use crate::tasks::{StartTaskRequest, TaskState};

#[derive(Clone)]
pub(crate) struct LabFeature {
    pub lab: Arc<dyn LabApi>,
    pub executions: Executions,
}

#[tonic::async_trait]
impl LabOperations for LabFeature {
    async fn list_log_sources(
        &self,
        request: Request<ListLogSourcesParameters>,
    ) -> Result<Response<ListLogSourcesResponses>, Status> {
        reject_metadata(&request)?;
        let page = page_from(request.into_inner().page.as_ref(), "ListLogSources")?;
        let page = self
            .page_result(
                LabCommand::ListLogSources(crate::logs::ListLogSourcesRequest { page }),
                "ProviderUnavailable",
            )
            .await?;
        let LabResult::ListLogSources(page) = page else {
            return Err(undefined("unexpected list log sources result"));
        };
        Ok(Response::new(ListLogSourcesResponses {
            sources: page.items().iter().map(source_message).collect(),
            has_next_cursor: Some(sila_bool(page.next_cursor().is_some())),
            next_cursor: Some(sila_string(page.next_cursor().unwrap_or_default())),
        }))
    }

    async fn query_logs(
        &self,
        request: Request<QueryLogsParameters>,
    ) -> Result<Response<QueryLogsResponses>, Status> {
        reject_metadata(&request)?;
        let body = request.into_inner();
        let source_id = required(
            body.source_id.as_ref(),
            "QueryLogs",
            "SourceId",
            read_source_id,
        )?;
        let range = required_range(body.range.as_ref(), "QueryLogs")?;
        let page = page_from(body.page.as_ref(), "QueryLogs")?;
        let page = self
            .page_result(
                LabCommand::QueryLogs(crate::logs::QueryLogsRequest {
                    source_id,
                    range,
                    page,
                }),
                "SourceNotFound",
            )
            .await?;
        let LabResult::QueryLogs(page) = page else {
            return Err(undefined("unexpected query logs result"));
        };
        Ok(Response::new(QueryLogsResponses {
            records: page
                .items()
                .iter()
                .map(record_message)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| undefined(error.to_string()))?,
            has_next_cursor: Some(sila_bool(page.next_cursor().is_some())),
            next_cursor: Some(sila_string(page.next_cursor().unwrap_or_default())),
        }))
    }

    async fn list_metrics(
        &self,
        request: Request<ListMetricsParameters>,
    ) -> Result<Response<ListMetricsResponses>, Status> {
        reject_metadata(&request)?;
        let page = page_from(request.into_inner().page.as_ref(), "ListMetrics")?;
        let page = self
            .page_result(
                LabCommand::ListMetrics(crate::metrics::ListMetricsRequest { page }),
                "ProviderUnavailable",
            )
            .await?;
        let LabResult::ListMetrics(page) = page else {
            return Err(undefined("unexpected list metrics result"));
        };
        Ok(Response::new(ListMetricsResponses {
            metrics: page.items().iter().map(metric_message).collect(),
            has_next_cursor: Some(sila_bool(page.next_cursor().is_some())),
            next_cursor: Some(sila_string(page.next_cursor().unwrap_or_default())),
        }))
    }

    async fn query_metric(
        &self,
        request: Request<QueryMetricParameters>,
    ) -> Result<Response<QueryMetricResponses>, Status> {
        reject_metadata(&request)?;
        let body = request.into_inner();
        let metric_id = required(
            body.metric_id.as_ref(),
            "QueryMetric",
            "MetricId",
            read_metric_id,
        )?;
        let range = required_range(body.range.as_ref(), "QueryMetric")?;
        let page = page_from(body.page.as_ref(), "QueryMetric")?;
        let page = self
            .page_result(
                LabCommand::QueryMetric(crate::metrics::QueryMetricRequest {
                    metric_id,
                    range,
                    page,
                }),
                "MetricNotFound",
            )
            .await?;
        let LabResult::QueryMetric(page) = page else {
            return Err(undefined("unexpected query metric result"));
        };
        Ok(Response::new(QueryMetricResponses {
            points: page
                .items()
                .iter()
                .map(point_message)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| undefined(error.to_string()))?,
            has_next_cursor: Some(sila_bool(page.next_cursor().is_some())),
            next_cursor: Some(sila_string(page.next_cursor().unwrap_or_default())),
        }))
    }

    async fn list_tasks(
        &self,
        request: Request<ListTasksParameters>,
    ) -> Result<Response<ListTasksResponses>, Status> {
        reject_metadata(&request)?;
        let page = page_from(request.into_inner().page.as_ref(), "ListTasks")?;
        let page = self
            .page_result(
                LabCommand::ListTasks(crate::tasks::ListTasksRequest { page }),
                "ProviderUnavailable",
            )
            .await?;
        let LabResult::ListTasks(page) = page else {
            return Err(undefined("unexpected list tasks result"));
        };
        Ok(Response::new(ListTasksResponses {
            tasks: page.items().iter().map(task_message).collect(),
            has_next_cursor: Some(sila_bool(page.next_cursor().is_some())),
            next_cursor: Some(sila_string(page.next_cursor().unwrap_or_default())),
        }))
    }

    async fn get_task_status(
        &self,
        request: Request<GetTaskStatusParameters>,
    ) -> Result<Response<GetTaskStatusResponses>, Status> {
        reject_metadata(&request)?;
        let run_id = required(
            request.into_inner().run_id.as_ref(),
            "GetTaskStatus",
            "RunId",
            read_run_id,
        )?;
        let result = self
            .page_result(
                LabCommand::GetTaskStatus(crate::tasks::GetTaskStatusRequest { id: run_id }),
                "RunNotFound",
            )
            .await?;
        let LabResult::GetTaskStatus(run) = result else {
            return Err(undefined("unexpected task status result"));
        };
        Ok(Response::new(GetTaskStatusResponses {
            run: Some(run_message(&run).map_err(|error| undefined(error.to_string()))?),
        }))
    }

    async fn start_task(
        &self,
        request: Request<StartTaskParameters>,
    ) -> Result<Response<CommandConfirmation>, Status> {
        reject_metadata(&request)?;
        let body = request.into_inner();
        let task_id = required(body.task_id.as_ref(), "StartTask", "TaskId", read_task_id)?;
        let input = body
            .input
            .as_ref()
            .ok_or_else(|| validation(parameter("StartTask", "Input"), "is required"))
            .and_then(|value| {
                read_json(value, "input")
                    .map_err(|error| validation(parameter("StartTask", "Input"), error.to_string()))
            })?;
        let result = self
            .page_result(
                LabCommand::StartTask(StartTaskRequest {
                    task_id,
                    input,
                    wait: false,
                    timeout_seconds: None,
                }),
                "TaskNotFound",
            )
            .await?;
        let LabResult::StartTask(run) = result else {
            return Err(undefined("unexpected start task result"));
        };
        let execution = self.executions.start(Arc::clone(&self.lab), run).await;
        Ok(Response::new(CommandConfirmation {
            command_execution_uuid: Some(CommandExecutionUuid { value: execution }),
            lifetime_of_execution: Some(lifetime()),
        }))
    }

    type StartTask_InfoStream =
        Pin<Box<dyn futures_util::Stream<Item = Result<ExecutionInfo, Status>> + Send>>;

    async fn start_task_info(
        &self,
        request: Request<CommandExecutionUuid>,
    ) -> Result<Response<Self::StartTask_InfoStream>, Status> {
        reject_metadata(&request)?;
        let execution = request.into_inner().value;
        let mut updates = self.executions.subscribe(&execution).await.map_err(|_| {
            framework(
                ErrorType::InvalidCommandExecutionUuid,
                "unknown command execution",
            )
        })?;
        let (sender, receiver) = tokio::sync::mpsc::channel(4);
        tokio::spawn(async move {
            loop {
                let view = updates.borrow().clone();
                let terminal = view.state.is_terminal() || view.failure.is_some();
                if sender.send(Ok(info_message(&view))).await.is_err() {
                    return;
                }
                if terminal {
                    return;
                }
                tokio::select! {
                    changed = updates.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                    () = tokio::time::sleep(std::time::Duration::from_secs(15)) => {}
                }
            }
        });
        let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
            let item = receiver.recv().await?;
            Some((item, receiver))
        });
        Ok(Response::new(Box::pin(stream)))
    }

    async fn start_task_result(
        &self,
        request: Request<CommandExecutionUuid>,
    ) -> Result<Response<StartTaskResponses>, Status> {
        reject_metadata(&request)?;
        let execution = request.into_inner().value;
        let updates = self.executions.subscribe(&execution).await.map_err(|_| {
            framework(
                ErrorType::InvalidCommandExecutionUuid,
                "unknown command execution",
            )
        })?;
        let view = updates.borrow().clone();
        if view.state == TaskState::Canceled {
            return Err(undefined("canceled"));
        }
        if let Some(failure) = view.failure {
            return Err(undefined(failure));
        }
        if !view.state.is_terminal() {
            return Err(framework(
                ErrorType::CommandExecutionNotFinished,
                "the command is still running",
            ));
        }
        let run = view.run.ok_or_else(|| undefined("missing terminal run"))?;
        if view.state == TaskState::Failed {
            return Err(undefined(
                run.message.unwrap_or_else(|| "failed".to_owned()),
            ));
        }
        Ok(Response::new(StartTaskResponses {
            run: Some(run_message(&run).map_err(|error| undefined(error.to_string()))?),
        }))
    }
}

impl LabFeature {
    async fn page_result(&self, command: LabCommand, not_found: &str) -> Result<LabResult, Status> {
        self.lab
            .execute(command)
            .await
            .map(|outcome| outcome.task.result)
            .map_err(|error| lab_error(error, not_found))
    }
}

fn page_from(
    page: Option<&DataTypePageRequest>,
    command: &str,
) -> Result<crate::page::PageRequest, Status> {
    let page = page.ok_or_else(|| validation(parameter(command, "Page"), "is required"))?;
    crate::sila::values::read_page(page, command)
        .map_err(|error| validation(parameter(command, "Page"), error.to_string()))
}

fn required<T, U>(
    value: Option<&T>,
    command: &str,
    name: &str,
    read: impl FnOnce(&T) -> Result<U, A2aLabError>,
) -> Result<U, Status> {
    let value = value.ok_or_else(|| validation(parameter(command, name), "is required"))?;
    read(value).map_err(|error| validation(parameter(command, name), error.to_string()))
}

fn required_range(
    value: Option<
        &crate::sila::wire::sila2::com::a3analytics::lab::laboperations::v1::DataTypeTimeRange,
    >,
    command: &str,
) -> Result<crate::time::TimeRange, Status> {
    let value = value.ok_or_else(|| validation(parameter(command, "Range"), "is required"))?;
    read_range(value).map_err(|error| validation(parameter(command, "Range"), error.to_string()))
}

fn info_message(view: &View) -> ExecutionInfo {
    let status = match view.state {
        TaskState::Submitted => CommandStatus::Waiting,
        TaskState::Working => CommandStatus::Running,
        TaskState::Completed => CommandStatus::FinishedSuccessfully,
        TaskState::Failed | TaskState::Canceled => CommandStatus::FinishedWithError,
    };
    ExecutionInfo {
        command_status: status as i32,
        progress_info: view.progress.map(|value| Real { value }),
        estimated_remaining_time: None,
        updated_lifetime_of_execution: Some(lifetime()),
    }
}

fn lifetime() -> Duration {
    Duration {
        seconds: EXECUTION_LIFETIME_SECONDS,
        nanos: 0,
    }
}
