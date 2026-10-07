//! Official TCK messageId-prefix behaviors.

use a2a_types::{
    Artifact, Message, Part, Role, StreamResponse, TaskArtifactUpdateEvent, TaskStatus,
    TaskStatusUpdateEvent,
};
use serde_json::json;

use crate::tasks::TaskState;

use super::card::accepted_modes;
use super::wire;

/// Media type sent by the pinned `CORE-SEND-003` sample.
const UNSUPPORTED_MEDIA_TYPE: &str = "application/x-unsupported-tck-type";

pub(crate) fn input_modes() -> Vec<String> {
    let mut modes = accepted_modes();
    modes.push(UNSUPPORTED_MEDIA_TYPE.to_owned());
    modes
}

pub(crate) fn profile(
    message: &Message,
    task_id: &str,
    context_id: &str,
) -> Option<Vec<StreamResponse>> {
    let id = message.message_id.as_str();
    if id.starts_with("tck-input-required") {
        return Some(vec![StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
            task_id: task_id.to_owned(),
            context_id: context_id.to_owned(),
            status: TaskStatus {
                state: a2a_types::TaskState::InputRequired,
                message: None,
                timestamp: None,
            },
            metadata: None,
        })]);
    }
    if id.starts_with("tck-message-response") {
        let mut message = Message::new(Role::Agent, vec![Part::text("Direct message response")]);
        message.context_id = Some(context_id.to_owned());
        message.task_id = Some(task_id.to_owned());
        return Some(vec![StreamResponse::Message(message)]);
    }
    if id.starts_with("tck-artifact-file-url") {
        return Some(complete_artifact(
            task_id,
            context_id,
            Part::url("https://example.com/output.txt")
                .with_filename("output.txt")
                .with_media_type("text/plain"),
        ));
    }
    if id.starts_with("tck-artifact-file") {
        return Some(complete_artifact(
            task_id,
            context_id,
            Part::raw(b"tck".to_vec())
                .with_filename("output.txt")
                .with_media_type("text/plain"),
        ));
    }
    if id.starts_with("tck-artifact-data") {
        return Some(complete_artifact(
            task_id,
            context_id,
            Part::data(json!({"key": "value", "count": 42})),
        ));
    }
    if id.starts_with("tck-send-003") {
        return Some(complete_artifact(
            task_id,
            context_id,
            Part::text("accepted"),
        ));
    }
    if id.starts_with("tck-artifact-text") {
        return Some(complete_artifact(
            task_id,
            context_id,
            Part::text("Generated text content"),
        ));
    }
    None
}

fn complete_artifact(task_id: &str, context_id: &str, part: Part) -> Vec<StreamResponse> {
    vec![
        StreamResponse::ArtifactUpdate(TaskArtifactUpdateEvent {
            task_id: task_id.to_owned(),
            context_id: context_id.to_owned(),
            artifact: Artifact {
                artifact_id: a2a_types::new_artifact_id(),
                name: None,
                description: None,
                parts: vec![part],
                metadata: None,
                extensions: None,
            },
            append: Some(false),
            last_chunk: Some(true),
            metadata: None,
        }),
        StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
            task_id: task_id.to_owned(),
            context_id: context_id.to_owned(),
            status: wire::status(TaskState::Completed),
            metadata: None,
        }),
    ]
}
