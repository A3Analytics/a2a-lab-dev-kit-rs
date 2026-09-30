//! Agent Card for the seven lab skills.

use serde_json::{Value, json};

use super::wire::{A2A_PROTOCOL_VERSION, LAB_MEDIA_TYPE};

/// Builds the well-known Agent Card for `public_url`.
#[must_use]
pub(crate) fn agent_card(public_url: &str) -> Value {
    json!({
        "name": "a2a-lab",
        "description": "Lab logs, metrics, and workflows",
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
            "list-workflows",
            "List workflows",
            "List the workflows this agent can start",
        ),
        (
            "start-workflow",
            "Start workflow",
            "Start a workflow with a JSON object input",
        ),
        (
            "get-workflow-status",
            "Get workflow status",
            "Read the status of a workflow run",
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
