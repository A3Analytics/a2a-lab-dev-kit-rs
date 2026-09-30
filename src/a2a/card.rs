//! Agent Card for the seven lab skills.

use serde_json::{Value, json};

use super::wire::{A2A_PROTOCOL_VERSION, LAB_MEDIA_TYPE};

/// Builds the well-known Agent Card for `public_url`.
#[must_use]
pub(crate) fn agent_card(public_url: &str) -> Value {
    json!({
        "name": "a2a-lab",
        "description": "Lab logs, metrics, and tasks",
        "version": env!("CARGO_PKG_VERSION"),
        "protocolVersion": A2A_PROTOCOL_VERSION,
        "supportedInterfaces": [{
            "url": public_url,
            "protocolBinding": "HTTP+JSON",
            "protocolVersion": A2A_PROTOCOL_VERSION
        }],
        "capabilities": {
            "streaming": true,
            "pushNotifications": false
        },
        "defaultInputModes": [LAB_MEDIA_TYPE],
        "defaultOutputModes": [LAB_MEDIA_TYPE],
        "skills": skills()
    })
}

fn skills() -> Vec<Value> {
    [
        (
            "list-log-sources",
            "List log sources",
            "List the log sources this agent can read",
        ),
        (
            "query-logs",
            "Query logs",
            "Read structured logs from a source over a UTC time range",
        ),
        (
            "list-metrics",
            "List metrics",
            "List the metrics this agent can read",
        ),
        (
            "query-metric",
            "Query metric",
            "Read metric samples over a UTC time range",
        ),
        (
            "list-tasks",
            "List tasks",
            "List the tasks this agent can start",
        ),
        (
            "start-task",
            "Start task",
            "Start a task with a JSON object input. Waits until the run is terminal unless wait is false.",
        ),
        (
            "get-task-status",
            "Get task status",
            "Read the status of an A2A task",
        ),
    ]
    .into_iter()
    .map(|(id, name, description)| {
        json!({
            "id": id,
            "name": name,
            "description": description,
            "tags": ["lab"],
            "inputModes": [LAB_MEDIA_TYPE],
            "outputModes": [LAB_MEDIA_TYPE]
        })
    })
    .collect()
}
