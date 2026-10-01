//! Official TCK messageId-prefix behaviors.

use a2a_types::{
    Artifact, Message, Part, Role, StreamResponse, TaskArtifactUpdateEvent, TaskStatusUpdateEvent,
};
use serde_json::json;

use crate::tasks::TaskState;

use super::wire;

pub(crate) fn profile(
    message: &Message,
    task_id: &str,
    context_id: &str,
) -> Option<Vec<StreamResponse>> {
    let id = message.message_id.as_str();
    if id.starts_with("tck-message-response") {
        return Some(vec![StreamResponse::Message(Message::new(
            Role::Agent,
            vec![Part::text("Direct message response")],
        ))]);
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
