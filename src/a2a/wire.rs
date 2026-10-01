//! Lab DataPart codec over official A2A 1.0 types.

use a2a_types::{
    A2AError, Artifact, Message, Part, PartContent, StreamResponse, Task, TaskArtifactUpdateEvent,
    TaskStatus, TaskStatusUpdateEvent, error_code,
};

use crate::error::A2aLabError;
use crate::page::Page;
use crate::service::{LabCommand, LabResult, TaskSnapshot};
use crate::tasks::TaskState;

/// Media type of lab command and result data parts.
pub const LAB_MEDIA_TYPE: &str = "application/vnd.a2a-lab.v1+json";

/// A2A protocol version advertised by this dev kit.
pub const A2A_PROTOCOL_VERSION: &str = a2a_types::VERSION;

pub(crate) fn command_from_message(message: &Message) -> Result<Option<LabCommand>, A2AError> {
    for part in &message.parts {
        let PartContent::Data(data) = &part.content else {
            continue;
        };
        if let Some(media_type) = part.media_type.as_deref()
            && !media_type.is_empty()
            && media_type != LAB_MEDIA_TYPE
        {
            return Err(A2AError::content_type_not_supported());
        }
        return serde_json::from_value(normalize_json(data.clone()))
            .map(Some)
            .map_err(|error| A2AError::invalid_params(error.to_string()));
    }
    Ok(None)
}

fn normalize_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Number(number) => whole_number(number),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(normalize_json).collect())
        }
        serde_json::Value::Object(object) => serde_json::Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, normalize_json(value)))
                .collect(),
        ),
        other => other,
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn whole_number(number: serde_json::Number) -> serde_json::Value {
    if number.as_u64().is_some() || number.as_i64().is_some() {
        return serde_json::Value::Number(number);
    }
    let Some(float) = number.as_f64() else {
        return serde_json::Value::Number(number);
    };
    if !(0.0..=f64::from(u32::MAX)).contains(&float) || float.fract() != 0.0 {
        return serde_json::Value::Number(number);
    }
    serde_json::Value::Number(serde_json::Number::from(float.round() as u32))
}

pub(crate) fn lab_part(result: &LabResult) -> Result<Part, A2AError> {
    let data =
        serde_json::to_value(result).map_err(|error| A2AError::internal(error.to_string()))?;
    Ok(Part::data(data).with_media_type(LAB_MEDIA_TYPE))
}

pub(crate) fn artifact(result: &LabResult, artifact_id: String) -> Result<Artifact, A2AError> {
    Ok(Artifact {
        artifact_id,
        name: None,
        description: None,
        parts: vec![lab_part(result)?],
        metadata: None,
        extensions: None,
    })
}

pub(crate) fn chunks(result: &LabResult) -> Vec<LabResult> {
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

fn chunk_page<T>(next_cursor: Option<&str>, len: usize, index: usize, item: T) -> Page<T> {
    let next = (index + 1 == len)
        .then(|| next_cursor.map(ToOwned::to_owned))
        .flatten();
    Page::new(vec![item], next)
}

pub(crate) fn protocol_state(state: TaskState) -> a2a_types::TaskState {
    match state {
        TaskState::Submitted => a2a_types::TaskState::Submitted,
        TaskState::Working => a2a_types::TaskState::Working,
        TaskState::Completed => a2a_types::TaskState::Completed,
        TaskState::Failed => a2a_types::TaskState::Failed,
        TaskState::Canceled => a2a_types::TaskState::Canceled,
    }
}

pub(crate) fn lab_state(state: &a2a_types::TaskState) -> TaskState {
    match state {
        a2a_types::TaskState::Working
        | a2a_types::TaskState::InputRequired
        | a2a_types::TaskState::AuthRequired => TaskState::Working,
        a2a_types::TaskState::Completed => TaskState::Completed,
        a2a_types::TaskState::Failed | a2a_types::TaskState::Rejected => TaskState::Failed,
        a2a_types::TaskState::Canceled => TaskState::Canceled,
        a2a_types::TaskState::Submitted | a2a_types::TaskState::Unspecified => TaskState::Submitted,
    }
}

pub(crate) fn status(state: TaskState) -> TaskStatus {
    TaskStatus {
        state: protocol_state(state),
        message: None,
        timestamp: None,
    }
}

pub(crate) fn status_update(task_id: &str, context_id: &str, state: TaskState) -> StreamResponse {
    StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
        task_id: task_id.to_owned(),
        context_id: context_id.to_owned(),
        status: status(state),
        metadata: None,
    })
}

pub(crate) fn artifact_update(
    task_id: &str,
    context_id: &str,
    result: &LabResult,
    artifact_id: String,
    append: bool,
    last_chunk: bool,
) -> Result<StreamResponse, A2AError> {
    Ok(StreamResponse::ArtifactUpdate(TaskArtifactUpdateEvent {
        task_id: task_id.to_owned(),
        context_id: context_id.to_owned(),
        artifact: artifact(result, artifact_id)?,
        append: Some(append),
        last_chunk: Some(last_chunk),
        metadata: None,
    }))
}

pub(crate) fn result_from_task(task: &Task) -> Result<LabResult, A2aLabError> {
    let results: Vec<LabResult> = task
        .artifacts
        .as_ref()
        .into_iter()
        .flatten()
        .flat_map(|artifact| artifact.parts.iter())
        .filter_map(|part| match &part.content {
            PartContent::Data(data) => serde_json::from_value(normalize_json(data.clone())).ok(),
            _ => None,
        })
        .collect();
    merge_results(results)
}

fn merge_results(results: Vec<LabResult>) -> Result<LabResult, A2aLabError> {
    match results.as_slice() {
        [] => Err(A2aLabError::protocol(
            "task artifact is missing a lab result",
        )),
        [LabResult::QueryLogs(_), ..] => {
            let mut items = Vec::new();
            let mut next = None;
            for result in results {
                if let LabResult::QueryLogs(page) = result {
                    next = page.next_cursor().map(ToOwned::to_owned);
                    items.extend(page.items().iter().cloned());
                }
            }
            Ok(LabResult::QueryLogs(Page::new(items, next)))
        }
        [LabResult::QueryMetric(_), ..] => {
            let mut items = Vec::new();
            let mut next = None;
            for result in results {
                if let LabResult::QueryMetric(page) = result {
                    next = page.next_cursor().map(ToOwned::to_owned);
                    items.extend(page.items().iter().copied());
                }
            }
            Ok(LabResult::QueryMetric(Page::new(items, next)))
        }
        [first, ..] => Ok(first.clone()),
    }
}

pub(crate) fn snapshot_from_task(task: &Task) -> Result<TaskSnapshot, A2aLabError> {
    Ok(TaskSnapshot {
        id: task.id.clone(),
        context_id: task.context_id.clone(),
        state: lab_state(&task.status.state),
        result: result_from_task(task)?,
    })
}

pub(crate) fn sdk_error(error: A2AError) -> A2aLabError {
    match error.code {
        error_code::TASK_NOT_FOUND => A2aLabError::not_found("task", error.message),
        error_code::INVALID_PARAMS
        | error_code::INVALID_REQUEST
        | error_code::PARSE_ERROR
        | error_code::CONTENT_TYPE_NOT_SUPPORTED => A2aLabError::invalid("request", error.message),
        error_code::INTERNAL_ERROR => A2aLabError::unavailable(error.message),
        _ => A2aLabError::protocol(error.message),
    }
}

pub(crate) fn a2a_error(error: &A2aLabError) -> A2AError {
    match error {
        A2aLabError::Invalid { .. } => A2AError::invalid_params(error.to_string()),
        A2aLabError::NotFound { id, .. } => A2AError::task_not_found(id),
        A2aLabError::Unavailable { message } if message.contains("not cancelable") => {
            A2AError::task_not_cancelable("task")
        }
        A2aLabError::Unavailable { message } | A2aLabError::Transport { message } => {
            A2AError::internal(message.clone())
        }
        A2aLabError::Protocol { message } if message.contains("media type") => {
            A2AError::content_type_not_supported()
        }
        A2aLabError::Protocol { message } => A2AError::invalid_request(message.clone()),
    }
}
