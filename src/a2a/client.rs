//! Typed A2A client for the seven lab operations.

use std::sync::atomic::{AtomicU64, Ordering};

use a2a_client::A2AClient;
use a2a_client::rest::RestTransport;
use a2a_types::{
    AgentCard, CancelTaskRequest, DeleteTaskPushNotificationConfigRequest,
    GetTaskPushNotificationConfigRequest, GetTaskRequest, ListTaskPushNotificationConfigsRequest,
    ListTasksResponse, Message, Part, Role, SendMessageConfiguration, SendMessageRequest,
    SendMessageResponse, StreamResponse, SubscribeToTaskRequest, Task, TaskPushNotificationConfig,
};
use futures_util::StreamExt;

use crate::error::A2aLabError;
use crate::logs::{ListLogSourcesRequest, LogRecord, LogSource, QueryLogsRequest};
use crate::metrics::{ListMetricsRequest, MetricDescriptor, MetricPoint, QueryMetricRequest};
use crate::page::Page;
use crate::service::{LabCommand, LabResult, TaskSnapshot};
use crate::tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskRun,
};

use super::wire::{self, LAB_MEDIA_TYPE};

/// Client for a lab agent speaking A2A HTTP+JSON.
pub struct A2aClient {
    inner: A2AClient<RestTransport>,
    base: String,
    messages: AtomicU64,
}

impl A2aClient {
    /// Creates a client for an agent origin such as `http://127.0.0.1:8080`.
    pub fn new(base_url: &str) -> Result<Self, A2aLabError> {
        let http = a2a_client::default_reqwest_client(None).map_err(wire::sdk_error)?;
        Ok(Self {
            inner: A2AClient::new(RestTransport::new(http, base_url.to_owned())),
            base: base_url.trim_end_matches('/').to_owned(),
            messages: AtomicU64::new(0),
        })
    }

    /// Lists log sources.
    pub async fn list_log_sources(
        &self,
        request: ListLogSourcesRequest,
    ) -> Result<Page<LogSource>, A2aLabError> {
        self.result(LabCommand::ListLogSources(request), |result| match result {
            LabResult::ListLogSources(page) => Some(page),
            _ => None,
        })
        .await
    }

    /// Queries logs.
    pub async fn query_logs(
        &self,
        request: QueryLogsRequest,
    ) -> Result<Page<LogRecord>, A2aLabError> {
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
    ) -> Result<Page<MetricDescriptor>, A2aLabError> {
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
    ) -> Result<Page<MetricPoint>, A2aLabError> {
        self.result(LabCommand::QueryMetric(request), |result| match result {
            LabResult::QueryMetric(page) => Some(page),
            _ => None,
        })
        .await
    }

    /// Lists startable lab tasks.
    pub async fn list_tasks(
        &self,
        request: ListTasksRequest,
    ) -> Result<Page<TaskDefinition>, A2aLabError> {
        self.result(LabCommand::ListTasks(request), |result| match result {
            LabResult::ListTasks(page) => Some(page),
            _ => None,
        })
        .await
    }

    /// Starts a lab task and returns the A2A task that wraps the run.
    pub async fn start_task(&self, request: StartTaskRequest) -> Result<TaskSnapshot, A2aLabError> {
        let wait = request.wait;
        let snapshot = self
            .invoke(
                LabCommand::StartTask(request),
                Some(SendMessageConfiguration {
                    accepted_output_modes: None,
                    task_push_notification_config: None,
                    history_length: None,
                    return_immediately: Some(!wait),
                }),
            )
            .await?;
        expect_variant(snapshot, |result| matches!(result, LabResult::StartTask(_)))
    }

    /// Reads a started lab run through the `get_task_status` skill.
    pub async fn task_status(&self, request: GetTaskStatusRequest) -> Result<TaskRun, A2aLabError> {
        self.result(LabCommand::GetTaskStatus(request), |result| match result {
            LabResult::GetTaskStatus(run) => Some(run),
            _ => None,
        })
        .await
    }

    /// Fetches an A2A task.
    pub async fn task(&self, task_id: &str) -> Result<TaskSnapshot, A2aLabError> {
        let task = self
            .inner
            .get_task(&GetTaskRequest {
                id: task_id.to_owned(),
                history_length: None,
                tenant: None,
            })
            .await
            .map_err(wire::sdk_error)?;
        wire::snapshot_from_task(&task)
    }

    /// Fetches the raw A2A task resource.
    pub async fn get_a2a_task(&self, task_id: &str) -> Result<Task, A2aLabError> {
        self.inner
            .get_task(&GetTaskRequest {
                id: task_id.to_owned(),
                history_length: None,
                tenant: None,
            })
            .await
            .map_err(wire::sdk_error)
    }

    /// Lists A2A tasks, optionally filtered by context.
    pub async fn list_a2a_tasks(
        &self,
        context_id: Option<&str>,
    ) -> Result<ListTasksResponse, A2aLabError> {
        self.inner
            .list_tasks(&a2a_types::ListTasksRequest {
                context_id: context_id.map(ToOwned::to_owned),
                status: None,
                page_size: None,
                page_token: None,
                history_length: Some(10),
                status_timestamp_after: None,
                include_artifacts: Some(true),
                tenant: None,
            })
            .await
            .map_err(wire::sdk_error)
    }

    /// Cancels an A2A task.
    pub async fn cancel(&self, task_id: &str) -> Result<Task, A2aLabError> {
        self.inner
            .cancel_task(&CancelTaskRequest {
                id: task_id.to_owned(),
                metadata: None,
                tenant: None,
            })
            .await
            .map_err(wire::sdk_error)
    }

    /// Collects SSE events until the stream ends.
    pub async fn subscribe(&self, task_id: &str) -> Result<Vec<StreamResponse>, A2aLabError> {
        let mut stream = self
            .inner
            .subscribe_to_task(&SubscribeToTaskRequest {
                id: task_id.to_owned(),
                tenant: None,
            })
            .await
            .map_err(wire::sdk_error)?;
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.map_err(wire::sdk_error)?);
        }
        Ok(events)
    }

    /// Sends a streaming message.
    pub async fn send_stream(
        &self,
        command: LabCommand,
    ) -> Result<Vec<StreamResponse>, A2aLabError> {
        let request = self.send_request(&command, None)?;
        let mut stream = self
            .inner
            .send_streaming_message(&request)
            .await
            .map_err(wire::sdk_error)?;
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.map_err(wire::sdk_error)?);
        }
        Ok(events)
    }

    /// Creates a push notification config.
    pub async fn create_push_config(
        &self,
        config: TaskPushNotificationConfig,
    ) -> Result<TaskPushNotificationConfig, A2aLabError> {
        self.inner
            .create_push_config(&config)
            .await
            .map_err(wire::sdk_error)
    }

    /// Deletes a push notification config.
    pub async fn delete_push_config(
        &self,
        task_id: &str,
        config_id: &str,
    ) -> Result<(), A2aLabError> {
        self.inner
            .delete_push_config(&DeleteTaskPushNotificationConfigRequest {
                task_id: task_id.to_owned(),
                id: config_id.to_owned(),
                tenant: None,
            })
            .await
            .map_err(wire::sdk_error)
    }

    /// Fetches a push notification config.
    pub async fn get_push_config(
        &self,
        task_id: &str,
        config_id: &str,
    ) -> Result<TaskPushNotificationConfig, A2aLabError> {
        self.inner
            .get_push_config(&GetTaskPushNotificationConfigRequest {
                task_id: task_id.to_owned(),
                id: config_id.to_owned(),
                tenant: None,
            })
            .await
            .map_err(wire::sdk_error)
    }

    /// Lists push notification configs for a task.
    pub async fn list_push_configs(
        &self,
        task_id: &str,
    ) -> Result<a2a_types::ListTaskPushNotificationConfigsResponse, A2aLabError> {
        self.inner
            .list_push_configs(&ListTaskPushNotificationConfigsRequest {
                task_id: task_id.to_owned(),
                page_size: None,
                page_token: None,
                tenant: None,
            })
            .await
            .map_err(wire::sdk_error)
    }

    /// Fetches the well-known Agent Card.
    pub async fn agent_card(&self) -> Result<AgentCard, A2aLabError> {
        a2a_client::agent_card::AgentCardResolver::new(None)
            .resolve(&self.base)
            .await
            .map_err(wire::sdk_error)
    }

    /// Fetches the extended Agent Card.
    pub async fn extended_agent_card(&self) -> Result<AgentCard, A2aLabError> {
        self.inner
            .get_extended_agent_card(&a2a_types::GetExtendedAgentCardRequest { tenant: None })
            .await
            .map_err(wire::sdk_error)
    }

    async fn result<T>(
        &self,
        command: LabCommand,
        pick: impl FnOnce(LabResult) -> Option<T>,
    ) -> Result<T, A2aLabError> {
        let snapshot = self.invoke(command, None).await?;
        pick(snapshot.result).ok_or_else(|| A2aLabError::protocol("unexpected result variant"))
    }

    async fn invoke(
        &self,
        command: LabCommand,
        configuration: Option<SendMessageConfiguration>,
    ) -> Result<TaskSnapshot, A2aLabError> {
        let request = self.send_request(&command, configuration)?;
        match self
            .inner
            .send_message(&request)
            .await
            .map_err(wire::sdk_error)?
        {
            SendMessageResponse::Task(task) => wire::snapshot_from_task(&task),
            SendMessageResponse::Message(_) => Err(A2aLabError::protocol(
                "send returned a message instead of a task",
            )),
        }
    }

    fn send_request(
        &self,
        command: &LabCommand,
        configuration: Option<SendMessageConfiguration>,
    ) -> Result<SendMessageRequest, A2aLabError> {
        let number = self.messages.fetch_add(1, Ordering::Relaxed) + 1;
        let data = serde_json::to_value(command)
            .map_err(|error| A2aLabError::protocol(error.to_string()))?;
        let mut message = Message::new(
            Role::User,
            vec![Part::data(data).with_media_type(LAB_MEDIA_TYPE)],
        );
        message.message_id = format!("msg-{number}");
        Ok(SendMessageRequest {
            message,
            configuration,
            metadata: None,
            tenant: None,
        })
    }
}

fn expect_variant(
    snapshot: TaskSnapshot,
    matches_result: impl FnOnce(&LabResult) -> bool,
) -> Result<TaskSnapshot, A2aLabError> {
    if matches_result(&snapshot.result) {
        Ok(snapshot)
    } else {
        Err(A2aLabError::protocol("unexpected result variant"))
    }
}
