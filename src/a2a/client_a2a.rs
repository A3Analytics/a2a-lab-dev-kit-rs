//! Raw A2A task and agent-message calls on [`super::A2aClient`].

use a2a_types::{
    AgentCard, CancelTaskRequest, DeleteTaskPushNotificationConfigRequest,
    GetTaskPushNotificationConfigRequest, GetTaskRequest, ListTaskPushNotificationConfigsRequest,
    ListTasksResponse, Message, Part, Role, StreamResponse, SubscribeToTaskRequest, Task,
    TaskPushNotificationConfig,
};
use futures_util::StreamExt;

use crate::error::A2aLabError;
use crate::service::{A2aLabCommand, A2aLabResult, TaskSnapshot};
use crate::tasks::{GetTaskStatusRequest, TaskRun};

use super::agent_message_response;
use super::wire;
use super::{A2aClient, AgentMessageResponse};

impl A2aClient {
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
        wire::snapshot_from_task(&task, self.image_transport)
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
            .send_message(&a2a_types::SendMessageRequest {
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
}
