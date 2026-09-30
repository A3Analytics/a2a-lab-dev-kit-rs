//! HTTP+JSON bodies for the lab profile of A2A 1.0.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::SdkError;
use crate::service::{LabCommand, LabResult, TaskSnapshot};
use crate::tasks::TaskState;

/// Media type of lab command and result data parts.
pub const LAB_MEDIA_TYPE: &str = "application/vnd.a2a-lab.v1+json";

/// A2A protocol version advertised by this SDK.
pub const A2A_PROTOCOL_VERSION: &str = "1.0";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SendRequest {
    message: IncomingMessage,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IncomingMessage {
    parts: Vec<IncomingPart>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IncomingPart {
    data: Option<Value>,
    media_type: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SendResponse {
    id: String,
    context_id: String,
    status: StatusBody,
    artifacts: Vec<ArtifactBody>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatusBody {
    state: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactBody {
    artifact_id: String,
    parts: Vec<OutgoingPart>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OutgoingPart {
    data: Value,
    media_type: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatusEvent {
    task_id: String,
    status: StatusBody,
    #[serde(rename = "final")]
    is_final: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactEvent {
    task_id: String,
    artifact: ArtifactBody,
    append: bool,
    last_chunk: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StreamEvent {
    /// SSE event name.
    pub event: String,
    /// Whether this artifact chunk continues a previous chunk.
    pub append: bool,
    /// Whether this is the last artifact chunk.
    pub last_chunk: bool,
    /// Task state when the event is a status update.
    pub state: Option<TaskState>,
    /// Lab result carried by an artifact chunk.
    pub result: Option<LabResult>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ApiErrorBody {
    code: &'static str,
    message: String,
}

impl ApiErrorBody {
    pub(crate) fn new(error: &SdkError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
        }
    }
}

pub(crate) fn command_from_request(request: SendRequest) -> Result<LabCommand, SdkError> {
    let part = request
        .message
        .parts
        .into_iter()
        .find(|part| part.data.is_some())
        .ok_or_else(|| SdkError::protocol("message requires a data part"))?;
    if let Some(media_type) = part.media_type.as_deref()
        && media_type != LAB_MEDIA_TYPE
    {
        return Err(SdkError::protocol(format!(
            "unsupported media type `{media_type}`"
        )));
    }
    let data = part
        .data
        .ok_or_else(|| SdkError::protocol("message requires a data part"))?;
    serde_json::from_value(data).map_err(|error| SdkError::protocol(error.to_string()))
}

pub(crate) fn task_response(snapshot: &TaskSnapshot) -> Result<SendResponse, SdkError> {
    Ok(SendResponse {
        id: snapshot.id.clone(),
        context_id: snapshot.context_id.clone(),
        status: StatusBody {
            state: snapshot.state.as_protocol().to_owned(),
        },
        artifacts: vec![artifact(&snapshot.id, 0, &snapshot.result)?],
    })
}

pub(crate) fn status_event(snapshot: &TaskSnapshot) -> StatusEvent {
    StatusEvent {
        task_id: snapshot.id.clone(),
        status: StatusBody {
            state: snapshot.state.as_protocol().to_owned(),
        },
        is_final: snapshot.state.is_terminal(),
    }
}

pub(crate) fn artifact_events(snapshot: &TaskSnapshot) -> Result<Vec<ArtifactEvent>, SdkError> {
    let chunks = chunks(&snapshot.result);
    let last = chunks.len().saturating_sub(1);
    chunks
        .iter()
        .enumerate()
        .map(|(index, result)| {
            Ok(ArtifactEvent {
                task_id: snapshot.id.clone(),
                artifact: artifact(&snapshot.id, index, result)?,
                append: index > 0,
                last_chunk: index == last,
            })
        })
        .collect()
}

fn artifact(task_id: &str, index: usize, result: &LabResult) -> Result<ArtifactBody, SdkError> {
    Ok(ArtifactBody {
        artifact_id: format!("artifact-{task_id}-{index}"),
        parts: vec![OutgoingPart {
            data: serde_json::to_value(result)
                .map_err(|error| SdkError::protocol(error.to_string()))?,
            media_type: LAB_MEDIA_TYPE.to_owned(),
        }],
    })
}

fn chunks(result: &LabResult) -> Vec<LabResult> {
    match result {
        LabResult::QueryLogs(page) if !page.items().is_empty() => page
            .items()
            .iter()
            .enumerate()
            .map(|(index, item)| {
                LabResult::QueryLogs(chunk_page(
                    page.next_cursor(),
                    page.items().len(),
                    index,
                    item.clone(),
                ))
            })
            .collect(),
        LabResult::QueryMetric(page) if !page.items().is_empty() => page
            .items()
            .iter()
            .enumerate()
            .map(|(index, item)| {
                LabResult::QueryMetric(chunk_page(
                    page.next_cursor(),
                    page.items().len(),
                    index,
                    *item,
                ))
            })
            .collect(),
        other => vec![other.clone()],
    }
}

fn chunk_page<T>(
    next_cursor: Option<&str>,
    len: usize,
    index: usize,
    item: T,
) -> crate::page::Page<T> {
    let next = (index + 1 == len)
        .then(|| next_cursor.map(ToOwned::to_owned))
        .flatten();
    crate::page::Page::new(vec![item], next)
}

pub(crate) fn encode_command(command: &LabCommand) -> Result<Value, SdkError> {
    serde_json::to_value(command).map_err(|error| SdkError::protocol(error.to_string()))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientMessage {
    message_id: String,
    role: &'static str,
    parts: Vec<OutgoingPart>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClientSend {
    message: ClientMessage,
}

pub(crate) fn client_send(message_id: &str, command: &LabCommand) -> Result<ClientSend, SdkError> {
    Ok(ClientSend {
        message: ClientMessage {
            message_id: message_id.to_owned(),
            role: "ROLE_USER",
            parts: vec![OutgoingPart {
                data: encode_command(command)?,
                media_type: LAB_MEDIA_TYPE.to_owned(),
            }],
        },
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientTask {
    id: String,
    context_id: String,
    status: StatusBody,
    #[serde(default)]
    artifacts: Vec<ArtifactBody>,
}

impl ClientTask {
    pub(crate) fn snapshot(&self) -> Result<TaskSnapshot, SdkError> {
        let state = TaskState::from_protocol(&self.status.state)?;
        let result = self
            .artifacts
            .first()
            .and_then(|artifact| artifact.parts.first())
            .map(|part| serde_json::from_value(part.data.clone()))
            .transpose()
            .map_err(|error| SdkError::protocol(error.to_string()))?
            .ok_or_else(|| SdkError::protocol("task artifact is missing a result"))?;
        Ok(TaskSnapshot {
            id: self.id.clone(),
            context_id: self.context_id.clone(),
            state,
            result,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    code: String,
    message: String,
}

pub(crate) fn parse_events(body: &str) -> Result<Vec<StreamEvent>, SdkError> {
    let mut events = Vec::new();
    for block in body.split("\n\n") {
        let mut name = String::from("message");
        let mut data = String::new();
        for line in block.lines() {
            if let Some(value) = line.strip_prefix("event:") {
                value.trim().clone_into(&mut name);
            } else if let Some(value) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value.trim());
            }
        }
        if !data.is_empty() {
            events.push(stream_event(&name, &data)?);
        }
    }
    Ok(events)
}

fn stream_event(name: &str, data: &str) -> Result<StreamEvent, SdkError> {
    if name == "artifactUpdate" {
        let event: ArtifactEvent =
            serde_json::from_str(data).map_err(|error| SdkError::protocol(error.to_string()))?;
        let result = event
            .artifact
            .parts
            .first()
            .map(|part| serde_json::from_value(part.data.clone()))
            .transpose()
            .map_err(|error| SdkError::protocol(error.to_string()))?
            .ok_or_else(|| SdkError::protocol("artifact chunk is missing a result"))?;
        return Ok(StreamEvent {
            event: name.to_owned(),
            append: event.append,
            last_chunk: event.last_chunk,
            state: None,
            result: Some(result),
        });
    }
    let state = if name == "statusUpdate" {
        let value: serde_json::Value =
            serde_json::from_str(data).map_err(|error| SdkError::protocol(error.to_string()))?;
        let raw = value
            .pointer("/status/state")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| SdkError::protocol("status event is missing a state"))?;
        Some(TaskState::from_protocol(raw)?)
    } else {
        None
    };
    Ok(StreamEvent {
        event: name.to_owned(),
        append: false,
        last_chunk: false,
        state,
        result: None,
    })
}

pub(crate) fn error_from_body(body: &str) -> SdkError {
    serde_json::from_str::<ErrorResponse>(body).map_or_else(
        |_| SdkError::protocol(body),
        |error| SdkError::from_code(&error.code, error.message),
    )
}
