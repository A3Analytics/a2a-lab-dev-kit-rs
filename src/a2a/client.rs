//! Typed A2A client for lab operations.

use std::sync::Arc;

use a2a_client::A2AClient;
use a2a_client::rest::RestTransport;
use a2a_types::{
    Message, Part, Role, SendMessageConfiguration, SendMessageRequest, SendMessageResponse, Task,
};

use crate::error::A2aLabError;
use crate::images::{
    GetCurrentImageRequest, GetImageRequest, Image, ImageDecisionRequest, ImageDecisionResult,
    ImageDescriptor, ImageSource, ImageTransportConfig, ListImageSourcesRequest, ListImagesRequest,
    SearchImagesRequest,
};
use crate::logs::{ListLogSourcesRequest, LogRecord, LogSource, QueryLogsRequest};
use crate::metrics::{ListMetricsRequest, MetricDescriptor, MetricPoint, QueryMetricRequest};
use crate::page::Page;
use crate::service::{A2aLabCommand, A2aLabResult, TaskSnapshot};
use crate::tasks::{ListTasksRequest, StartTaskRequest, TaskDefinition};

use super::wire::{self, LAB_MEDIA_TYPE};

#[path = "client_a2a.rs"]
mod client_a2a;
#[path = "client_videos.rs"]
mod client_videos;

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
    pub(super) inner: A2AClient<RestTransport>,
    pub(super) base: String,
    image_transport: ImageTransportConfig,
    pub(super) bearer: Option<String>,
}

impl A2aClient {
    /// Creates a client for an agent origin such as `http://127.0.0.1:8080`.
    pub fn new(base_url: &str) -> Result<Self, A2aLabError> {
        let http = a2a_client::default_reqwest_client(None).map_err(wire::sdk_error)?;
        Ok(Self {
            inner: A2AClient::new(RestTransport::new(http, base_url.to_owned())),
            base: base_url.trim_end_matches('/').to_owned(),
            image_transport: ImageTransportConfig::default(),
            bearer: None,
        })
    }

    /// Sends `Authorization: Bearer <token>` on later protocol calls.
    #[must_use]
    pub fn with_bearer_token(self, token: impl Into<String>) -> Self {
        let Self {
            inner,
            base,
            image_transport,
            ..
        } = self;
        let token = token.into();
        Self {
            inner: inner.with_interceptors(vec![Arc::new(
                a2a_client::auth::AuthInterceptor::bearer(token.clone()),
            )]),
            base,
            image_transport,
            bearer: Some(token),
        }
    }

    /// Sets the decoded-byte maximum for image results.
    ///
    /// The default is 64 MiB, matching [`ImageTransportConfig::default`].
    #[must_use]
    pub fn with_image_transport(mut self, image_transport: ImageTransportConfig) -> Self {
        self.image_transport = image_transport;
        self
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

    /// Lists image sources.
    pub async fn list_image_sources(
        &self,
        request: ListImageSourcesRequest,
    ) -> Result<Page<ImageSource>, A2aLabError> {
        self.result(
            A2aLabCommand::ListImageSources(request),
            |result| match result {
                A2aLabResult::ListImageSources(page) => Some(page),
                _ => None,
            },
        )
        .await
    }

    /// Lists image metadata for one source.
    pub async fn list_images(
        &self,
        request: ListImagesRequest,
    ) -> Result<Page<ImageDescriptor>, A2aLabError> {
        self.result(A2aLabCommand::ListImages(request), |result| match result {
            A2aLabResult::ListImages(page) => Some(page),
            _ => None,
        })
        .await
    }

    /// Searches image metadata.
    pub async fn search_images(
        &self,
        request: SearchImagesRequest,
    ) -> Result<Page<ImageDescriptor>, A2aLabError> {
        self.result(
            A2aLabCommand::SearchImages(request),
            |result| match result {
                A2aLabResult::SearchImages(page) => Some(page),
                _ => None,
            },
        )
        .await
    }

    /// Reads one image, including its inline bytes.
    pub async fn get_image(&self, request: GetImageRequest) -> Result<Image, A2aLabError> {
        self.result(A2aLabCommand::GetImage(request), |result| match result {
            A2aLabResult::GetImage(image) => Some(image),
            _ => None,
        })
        .await
    }

    /// Reads the current image for one source, including its inline bytes.
    pub async fn get_current_image(
        &self,
        request: GetCurrentImageRequest,
    ) -> Result<Image, A2aLabError> {
        self.result(
            A2aLabCommand::GetCurrentImage(request),
            |result| match result {
                A2aLabResult::GetCurrentImage(image) => Some(image),
                _ => None,
            },
        )
        .await
    }

    /// Runs one synchronous generic image decision.
    ///
    /// An omitted ROI is the full source image. Returned boxes are normalized
    /// to that image. This call does not append a typed ledger record.
    pub async fn decide_image(
        &self,
        request: ImageDecisionRequest,
    ) -> Result<ImageDecisionResult, A2aLabError> {
        self.result(A2aLabCommand::DecideImage(request), |result| match result {
            A2aLabResult::DecideImage(decision) => Some(decision),
            _ => None,
        })
        .await
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
            SendMessageResponse::Task(task) => {
                wire::snapshot_from_task(&task, self.image_transport)
            }
            SendMessageResponse::Message(_) => Err(A2aLabError::protocol(
                "send returned a message instead of a task",
            )),
        }
    }

    pub(super) fn send_request(
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

pub(super) fn agent_message_response(
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

pub(super) fn text_from_task(task: &Task) -> Result<String, A2aLabError> {
    text_from_parts(
        task.artifacts
            .iter()
            .flatten()
            .flat_map(|artifact| artifact.parts.iter()),
    )
}

pub(super) fn text_from_parts<'a>(
    parts: impl Iterator<Item = &'a Part>,
) -> Result<String, A2aLabError> {
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

pub(super) fn expect_variant(
    snapshot: TaskSnapshot,
    matches_result: impl FnOnce(&A2aLabResult) -> bool,
) -> Result<TaskSnapshot, A2aLabError> {
    if matches_result(&snapshot.result) {
        Ok(snapshot)
    } else {
        Err(A2aLabError::protocol("unexpected result variant"))
    }
}

pub(super) fn clip_http_error(status: reqwest::StatusCode, body: &[u8]) -> A2aLabError {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body)
        && let Some(code) = value
            .pointer("/error/code")
            .and_then(serde_json::Value::as_str)
    {
        let message = value
            .pointer("/error/message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(code);
        return A2aLabError::from_code(code, message);
    }
    match status.as_u16() {
        401 | 403 => A2aLabError::invalid("authorization", status.to_string()),
        404 => A2aLabError::not_found("video clip", status.to_string()),
        400 => A2aLabError::invalid("id", status.to_string()),
        _ => A2aLabError::transport(status.to_string()),
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
