//! Lab DataPart codec over official A2A 1.0 types.

use a2a_types::{
    A2AError, Artifact, Message, Part, PartContent, StreamResponse, Task, TaskArtifactUpdateEvent,
    TaskStatus, TaskStatusUpdateEvent, error_code,
};

use crate::error::A2aLabError;
use crate::images::{Image, ImageTransportConfig};
use crate::page::Page;
use crate::service::{A2aLabCommand, A2aLabResult, TaskSnapshot};
use crate::tasks::TaskState;

/// Media type of lab command and result data parts.
pub const LAB_MEDIA_TYPE: &str = "application/json";

/// A2A protocol version advertised by this dev kit.
pub const A2A_PROTOCOL_VERSION: &str = a2a_types::VERSION;

pub(crate) fn command_from_message(message: &Message) -> Result<Option<A2aLabCommand>, A2AError> {
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

pub(crate) fn lab_part(result: &A2aLabResult) -> Result<Part, A2AError> {
    let data =
        serde_json::to_value(result).map_err(|error| A2AError::internal(error.to_string()))?;
    Ok(Part::data(data).with_media_type(LAB_MEDIA_TYPE))
}

pub(crate) fn artifact(result: &A2aLabResult, artifact_id: String) -> Result<Artifact, A2AError> {
    Ok(Artifact {
        artifact_id,
        name: None,
        description: None,
        parts: vec![lab_part(result)?],
        metadata: None,
        extensions: None,
    })
}

pub(crate) fn chunks(result: &A2aLabResult) -> Vec<A2aLabResult> {
    match result {
        A2aLabResult::QueryLogs(page) if !page.items().is_empty() => {
            chunk_items(page, A2aLabResult::QueryLogs)
        }
        A2aLabResult::QueryMetric(page) if !page.items().is_empty() => {
            chunk_items(page, A2aLabResult::QueryMetric)
        }
        A2aLabResult::ListImageSources(page) if !page.items().is_empty() => {
            chunk_items(page, A2aLabResult::ListImageSources)
        }
        A2aLabResult::ListImages(page) if !page.items().is_empty() => {
            chunk_items(page, A2aLabResult::ListImages)
        }
        A2aLabResult::SearchImages(page) if !page.items().is_empty() => {
            chunk_items(page, A2aLabResult::SearchImages)
        }
        other => vec![other.clone()],
    }
}

fn chunk_items<T: Clone>(
    page: &Page<T>,
    wrap: impl Fn(Page<T>) -> A2aLabResult,
) -> Vec<A2aLabResult> {
    let len = page.items().len();
    page.items()
        .iter()
        .enumerate()
        .map(|(index, item)| wrap(chunk_page(page.next_cursor(), len, index, item.clone())))
        .collect()
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
    result: &A2aLabResult,
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

pub(crate) fn result_from_task(
    task: &Task,
    transport: ImageTransportConfig,
) -> Result<A2aLabResult, A2aLabError> {
    let mut results = Vec::new();
    for part in task
        .artifacts
        .iter()
        .flatten()
        .flat_map(|artifact| artifact.parts.iter())
    {
        let PartContent::Data(data) = &part.content else {
            continue;
        };
        if let Some(result) = decode_part(normalize_json(data.clone()), transport)? {
            results.push(result);
        }
    }
    merge_results(results)
}

fn decode_part(
    value: serde_json::Value,
    transport: ImageTransportConfig,
) -> Result<Option<A2aLabResult>, A2aLabError> {
    // `Image`'s `Deserialize` is fixed at the 64 MiB default. Image results use
    // the caller's limit so a raised maximum applies while the JSON is decoded.
    match value.get("operation").and_then(serde_json::Value::as_str) {
        Some("get_image" | "get_current_image") => decode_image_result(&value, transport).map(Some),
        _ => Ok(serde_json::from_value(value).ok()),
    }
}

fn decode_image_result(
    value: &serde_json::Value,
    transport: ImageTransportConfig,
) -> Result<A2aLabResult, A2aLabError> {
    let operation = value
        .get("operation")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let result = value
        .get("result")
        .cloned()
        .ok_or_else(|| A2aLabError::protocol("image result is missing"))?;
    let descriptor = serde_json::from_value(
        result
            .get("descriptor")
            .cloned()
            .ok_or_else(|| A2aLabError::protocol("image descriptor is missing"))?,
    )
    .map_err(|error| A2aLabError::protocol(error.to_string()))?;
    let encoded = result
        .get("data")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| A2aLabError::protocol("image data is missing"))?;
    let image = Image::from_base64(descriptor, encoded, transport)?;
    match operation {
        "get_image" => Ok(A2aLabResult::GetImage(image)),
        "get_current_image" => Ok(A2aLabResult::GetCurrentImage(image)),
        _ => Err(A2aLabError::protocol("unexpected image operation")),
    }
}

fn merge_results(results: Vec<A2aLabResult>) -> Result<A2aLabResult, A2aLabError> {
    match results.as_slice() {
        [] => Err(A2aLabError::protocol(
            "task artifact is missing a lab result",
        )),
        [A2aLabResult::QueryLogs(_), ..] => Ok(merge_page(
            results,
            |result| match result {
                A2aLabResult::QueryLogs(page) => Some(page),
                _ => None,
            },
            A2aLabResult::QueryLogs,
        )),
        [A2aLabResult::QueryMetric(_), ..] => Ok(merge_page(
            results,
            |result| match result {
                A2aLabResult::QueryMetric(page) => Some(page),
                _ => None,
            },
            A2aLabResult::QueryMetric,
        )),
        [A2aLabResult::ListImageSources(_), ..] => Ok(merge_page(
            results,
            |result| match result {
                A2aLabResult::ListImageSources(page) => Some(page),
                _ => None,
            },
            A2aLabResult::ListImageSources,
        )),
        [A2aLabResult::ListImages(_), ..] => Ok(merge_page(
            results,
            |result| match result {
                A2aLabResult::ListImages(page) => Some(page),
                _ => None,
            },
            A2aLabResult::ListImages,
        )),
        [A2aLabResult::SearchImages(_), ..] => Ok(merge_page(
            results,
            |result| match result {
                A2aLabResult::SearchImages(page) => Some(page),
                _ => None,
            },
            A2aLabResult::SearchImages,
        )),
        [first, ..] => Ok(first.clone()),
    }
}

fn merge_page<T: Clone>(
    results: Vec<A2aLabResult>,
    mut take: impl FnMut(A2aLabResult) -> Option<Page<T>>,
    wrap: impl FnOnce(Page<T>) -> A2aLabResult,
) -> A2aLabResult {
    let mut items = Vec::new();
    let mut next = None;
    for result in results {
        if let Some(page) = take(result) {
            next = page.next_cursor().map(ToOwned::to_owned);
            items.extend(page.items().iter().cloned());
        }
    }
    wrap(Page::new(items, next))
}

pub(crate) fn snapshot_from_task(
    task: &Task,
    transport: ImageTransportConfig,
) -> Result<TaskSnapshot, A2aLabError> {
    Ok(TaskSnapshot {
        id: task.id.clone(),
        context_id: task.context_id.clone(),
        state: lab_state(&task.status.state),
        result: result_from_task(task, transport)?,
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

#[cfg(test)]
mod tests {
    use super::{chunks, decode_part, merge_results};
    use crate::id::{ImageId, ImageSourceId};
    use crate::images::{Image, ImageDescriptor, ImageSource, ImageTransportConfig};
    use crate::json_object::JsonObject;
    use crate::page::Page;
    use crate::service::A2aLabResult;
    use crate::time::UtcTimestamp;

    #[test]
    fn paged_image_results_merge_in_chunk_order() {
        let page = Page::new(vec![source("zeta"), source("alpha")], Some("2".to_owned()));
        let parts = chunks(&A2aLabResult::ListImageSources(page));
        assert_eq!(parts.len(), 2);
        let A2aLabResult::ListImageSources(first) = &parts[0] else {
            panic!("source chunk");
        };
        let A2aLabResult::ListImageSources(second) = &parts[1] else {
            panic!("source chunk");
        };
        assert_eq!(first.items()[0].id.as_str(), "zeta");
        assert!(first.next_cursor().is_none());
        assert_eq!(second.items()[0].id.as_str(), "alpha");
        assert_eq!(second.next_cursor(), Some("2"));

        let A2aLabResult::ListImageSources(merged) = merge_results(parts).unwrap() else {
            panic!("merged sources");
        };
        assert_eq!(merged.items()[0].id.as_str(), "zeta");
        assert_eq!(merged.items()[1].id.as_str(), "alpha");
        assert_eq!(merged.next_cursor(), Some("2"));
    }

    #[test]
    fn image_bytes_stay_one_part_and_honor_the_decode_limit() {
        let image = Image::new(descriptor(), vec![1, 2, 3, 4]).unwrap();
        assert_eq!(chunks(&A2aLabResult::GetImage(image.clone())).len(), 1);
        assert_eq!(
            chunks(&A2aLabResult::GetCurrentImage(image.clone())).len(),
            1
        );

        let value = serde_json::to_value(A2aLabResult::GetImage(image.clone())).unwrap();
        assert!(serde_json::from_value::<A2aLabResult>(value.clone()).is_ok());
        let tight = decode_part(value.clone(), ImageTransportConfig::new(3).unwrap()).unwrap_err();
        assert_eq!(tight.code(), "invalid");
        let decoded = decode_part(value, ImageTransportConfig::new(4).unwrap())
            .unwrap()
            .unwrap();
        let A2aLabResult::GetImage(decoded) = decoded else {
            panic!("decoded image");
        };
        assert_eq!(decoded.data(), image.data());
    }

    fn source(id: &str) -> ImageSource {
        ImageSource {
            id: ImageSourceId::new(id).unwrap(),
            name: id.to_owned(),
            description: id.to_owned(),
            asset_id: None,
            semantic_id: None,
        }
    }

    fn descriptor() -> ImageDescriptor {
        ImageDescriptor::new(
            ImageId::new("frame").unwrap(),
            ImageSourceId::new("cam").unwrap(),
            UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
            "image/png",
            1,
            1,
            None,
            JsonObject::empty(),
        )
        .unwrap()
    }
}
