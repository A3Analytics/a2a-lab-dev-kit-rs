//! Agent Card for the seven lab skills.

use std::collections::HashMap;

use a2a_types::{
    AgentCapabilities, AgentCard, AgentInterface, AgentSkill, SecurityRequirement, SecurityScheme,
    TRANSPORT_PROTOCOL_GRPC, TRANSPORT_PROTOCOL_HTTP_JSON, TRANSPORT_PROTOCOL_JSONRPC,
};

use super::wire::{A2A_PROTOCOL_VERSION, LAB_MEDIA_TYPE};

/// Builds the well-known Agent Card for the HTTP origin and gRPC socket.
#[must_use]
pub(crate) fn agent_card(
    public_url: &str,
    grpc_url: &str,
    push_notifications: bool,
    extended_agent_card: bool,
    security_schemes: Option<HashMap<String, SecurityScheme>>,
    security_requirements: Option<Vec<SecurityRequirement>>,
) -> AgentCard {
    AgentCard {
        name: "a2a-lab".to_owned(),
        description: "Lab logs, metrics, and tasks".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        supported_interfaces: vec![
            interface(public_url, TRANSPORT_PROTOCOL_HTTP_JSON),
            interface(public_url, TRANSPORT_PROTOCOL_JSONRPC),
            interface(grpc_url, TRANSPORT_PROTOCOL_GRPC),
        ],
        capabilities: AgentCapabilities {
            streaming: Some(true),
            push_notifications: Some(push_notifications),
            extensions: None,
            extended_agent_card: Some(extended_agent_card),
        },
        default_input_modes: vec!["text/plain".to_owned(), LAB_MEDIA_TYPE.to_owned()],
        default_output_modes: vec!["text/plain".to_owned(), LAB_MEDIA_TYPE.to_owned()],
        skills: skills(),
        provider: None,
        documentation_url: None,
        icon_url: None,
        security_schemes,
        security_requirements,
        signatures: None,
    }
}

fn interface(url: &str, protocol_binding: &str) -> AgentInterface {
    AgentInterface {
        url: url.to_owned(),
        protocol_binding: protocol_binding.to_owned(),
        protocol_version: A2A_PROTOCOL_VERSION.to_owned(),
        tenant: None,
    }
}

fn skills() -> Vec<AgentSkill> {
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
            "Read the status of a started lab run",
        ),
    ]
    .into_iter()
    .map(|(id, name, description)| AgentSkill {
        id: id.to_owned(),
        name: name.to_owned(),
        description: description.to_owned(),
        tags: vec!["lab".to_owned()],
        examples: None,
        input_modes: Some(vec![LAB_MEDIA_TYPE.to_owned()]),
        output_modes: Some(vec![LAB_MEDIA_TYPE.to_owned()]),
        security_requirements: None,
    })
    .collect()
}
