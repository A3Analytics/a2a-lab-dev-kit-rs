//! Typed A2A client for the seven lab operations.

use std::sync::Arc;

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
use crate::service::{A2aLabCommand, A2aLabResult, TaskSnapshot};
use crate::tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskRun,
};

use super::wire::{self, LAB_MEDIA_TYPE};

/// Plain-text answer from an `agent-message` turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMessageResponse {
    /// Assistant text.
    pub text: String,
    /// Protocol task for this turn, when the agent returned a task.
    pub task_id: Option<String>,
    /// Conversation to send on the next turn.
    pub context_id: String,
}

/// Client for a lab agent speaking A2A HTTP+JSON.
pub struct A2aClient {
    inner: A2AClient<RestTransport>,
    base: String,
}

impl A2aClient {
    /// Creates a client for an agent origin such as `http://127.0.0.1:8080`.
    pub fn new(base_url: &str) -> Result<Self, A2aLabError> {
        let http = a2a_client::default_reqwest_client(None).map_err(wire::sdk_error)?;
        Ok(Self {
            inner: A2AClient::new(RestTransport::new(http, base_url.to_owned())),
            base: base_url.trim_end_matches('/').to_owned(),
        })
    }

    /// Sends `Authorization: Bearer <token>` on later protocol calls.
    #[must_use]
    pub fn with_bearer_token(self, token: impl Into<String>) -> Self {
        let Self { inner, base } = self;
        Self {
            inner: inner.with_interceptors(vec![Arc::new(
                a2a_client::auth::AuthInterceptor::bearer(token),
            )]),
            base,
        }
    }

    /// Lists log sources.
    pub async fn list_log_sources(
        &self,
        request: ListLogSourcesRequest,
    ) -> Result<Page<LogSource>, A2aLabError> {
        self.result(
            A2aLabCommand::ListLogSources(request),
            |result| match result {
                A2aLabResult::ListLogSources(page) => Some(page),
                _ => None,
            },
        )
        .await
    }

    /// Queries logs.
    pub async fn query_logs(
        &self,
        request: QueryLogsRequest,
    ) -> Result<Page<LogRecord>, A2aLabError> {
        self.result(A2aLabCommand::QueryLogs(request), |result| match result {
            A2aLabResult::QueryLogs(page) => Some(page),
            _ => None,
        })
        .await
    }

    /// Lists metrics.
    pub async fn list_metrics(
        &self,
        request: ListMetricsRequest,
    ) -> Result<Page<MetricDescriptor>, A2aLabError> {
        self.result(A2aLabCommand::ListMetrics(request), |result| match result {
            A2aLabResult::ListMetrics(page) => Some(page),
            _ => None,
        })
        .await
    }

    /// Queries one metric.
    pub async fn query_metric(
        &self,
        request: QueryMetricRequest,
    ) -> Result<Page<MetricPoint>, A2aLabError> {
        self.result(A2aLabCommand::QueryMetric(request), |result| match result {
            A2aLabResult::QueryMetric(page) => Some(page),
            _ => None,
        })
        .await
    }

    /// Lists startable lab tasks.
    pub async fn list_tasks(
        &self,
        request: ListTasksRequest,
    ) -> Result<Page<TaskDefinition>, A2aLabError> {
        self.result(A2aLabCommand::ListTasks(request), |result| match result {
            A2aLabResult::ListTasks(page) => Some(page),
            _ => None,
        })
        .await
    }

    /// Starts a lab task and returns the A2A task that wraps the run.
    pub async fn start_task(&self, request: StartTaskRequest) -> Result<TaskSnapshot, A2aLabError> {
        let wait = request.wait;
        let snapshot = self
            .invoke(
                A2aLabCommand::StartTask(request),
                Some(SendMessageConfiguration {
                    accepted_output_modes: None,
                    task_push_notification_config: None,
                    history_length: None,
                    return_immediately: Some(!wait),
                }),
            )
            .await?;
        expect_variant(snapshot, |result| {
            matches!(result, A2aLabResult::StartTask(_))
        })
    }

    /// Reads a started lab run through the `get_task_status` skill.
    pub async fn task_status(&self, request: GetTaskStatusRequest) -> Result<TaskRun, A2aLabError> {
        self.result(
            A2aLabCommand::GetTaskStatus(request),
            |result| match result {
                A2aLabResult::GetTaskStatus(run) => Some(run),
                _ => None,
            },
        )
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
        command: A2aLabCommand,
    ) -> Result<Vec<StreamResponse>, A2aLabError> {
        let request = Self::send_request(&command, None)?;
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

    /// Sends plain text and returns the agent reply for that conversation.
    pub async fn agent_message(
        &self,
        text: &str,
        context_id: Option<&str>,
    ) -> Result<AgentMessageResponse, A2aLabError> {
        self.agent_message_with_references(text, context_id, &[] as &[&str])
            .await
    }

    /// Sends plain text that refines the tasks in `reference_task_ids`.
    pub async fn agent_message_with_references(
        &self,
        text: &str,
        context_id: Option<&str>,
        reference_task_ids: &[impl AsRef<str>],
    ) -> Result<AgentMessageResponse, A2aLabError> {
        let mut message = Message::new(
            Role::User,
            vec![Part::text(text).with_media_type("text/plain")],
        );
        message.context_id = context_id.map(ToOwned::to_owned);
        if !reference_task_ids.is_empty() {
            message.reference_task_ids = Some(
                reference_task_ids
                    .iter()
                    .map(|id| id.as_ref().to_owned())
                    .collect(),
            );
        }
        let response = self
            .inner
            .send_message(&SendMessageRequest {
                message,
                configuration: None,
                metadata: None,
                tenant: None,
            })
            .await
            .map_err(wire::sdk_error)?;
        agent_message_response(response)
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
        command: A2aLabCommand,
        pick: impl FnOnce(A2aLabResult) -> Option<T>,
    ) -> Result<T, A2aLabError> {
        let snapshot = self.invoke(command, None).await?;
        pick(snapshot.result).ok_or_else(|| A2aLabError::protocol("unexpected result variant"))
    }

    async fn invoke(
        &self,
        command: A2aLabCommand,
        configuration: Option<SendMessageConfiguration>,
    ) -> Result<TaskSnapshot, A2aLabError> {
        let request = Self::send_request(&command, configuration)?;
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
        command: &A2aLabCommand,
        configuration: Option<SendMessageConfiguration>,
    ) -> Result<SendMessageRequest, A2aLabError> {
        let data = serde_json::to_value(command)
            .map_err(|error| A2aLabError::protocol(error.to_string()))?;
        let message = Message::new(
            Role::User,
            vec![Part::data(data).with_media_type(LAB_MEDIA_TYPE)],
        );
        Ok(SendMessageRequest {
            message,
            configuration,
            metadata: None,
            tenant: None,
        })
    }
}

fn agent_message_response(
    response: SendMessageResponse,
) -> Result<AgentMessageResponse, A2aLabError> {
    match response {
        SendMessageResponse::Task(task) => {
            if task.context_id.is_empty() {
                return Err(A2aLabError::protocol("server task is missing contextId"));
            }
            Ok(AgentMessageResponse {
                text: text_from_task(&task)?,
                task_id: Some(task.id),
                context_id: task.context_id,
            })
        }
        SendMessageResponse::Message(message) => {
            if message.role != Role::Agent {
                return Err(A2aLabError::protocol(
                    "server message role must be ROLE_AGENT",
                ));
            }
            let Some(context_id) = message.context_id.filter(|id| !id.is_empty()) else {
                return Err(A2aLabError::protocol("server message is missing contextId"));
            };
            Ok(AgentMessageResponse {
                text: text_from_parts(message.parts.iter())?,
                task_id: message.task_id,
                context_id,
            })
        }
    }
}

fn text_from_task(task: &Task) -> Result<String, A2aLabError> {
    text_from_parts(
        task.artifacts
            .iter()
            .flatten()
            .flat_map(|artifact| artifact.parts.iter()),
    )
}

fn text_from_parts<'a>(parts: impl Iterator<Item = &'a Part>) -> Result<String, A2aLabError> {
    let text = parts
        .filter_map(Part::as_text)
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        Err(A2aLabError::protocol(
            "agent message is missing text".to_owned(),
        ))
    } else {
        Ok(text)
    }
}

fn expect_variant(
    snapshot: TaskSnapshot,
    matches_result: impl FnOnce(&A2aLabResult) -> bool,
) -> Result<TaskSnapshot, A2aLabError> {
    if matches_result(&snapshot.result) {
        Ok(snapshot)
    } else {
        Err(A2aLabError::protocol("unexpected result variant"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_message_requires_context_id() {
        let message = Message::new(Role::Agent, vec![Part::text("hi")]);
        let error = agent_message_response(SendMessageResponse::Message(message)).unwrap_err();
        assert_eq!(error.code(), "protocol");
    }

    #[test]
    fn server_message_keeps_its_context_id() {
        let mut message = Message::new(Role::Agent, vec![Part::text("hi")]);
        message.context_id = Some("ctx-1".to_owned());
        let response = agent_message_response(SendMessageResponse::Message(message)).unwrap();
        assert_eq!(response.text, "hi");
        assert_eq!(response.context_id, "ctx-1");
        assert!(response.task_id.is_none());
    }
}
