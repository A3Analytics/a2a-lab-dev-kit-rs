//! Agent Card for the lab skills.

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
    agent_message: bool,
) -> AgentCard {
    AgentCard {
        name: "a2a-lab".to_owned(),
        description: "Lab logs, metrics, tasks, and images".to_owned(),
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
        default_input_modes: accepted_modes(),
        default_output_modes: accepted_modes(),
        skills: skills(agent_message),
        provider: None,
        documentation_url: None,
        icon_url: None,
        security_schemes,
        security_requirements,
        signatures: None,
    }
}

pub(crate) fn accepted_modes() -> Vec<String> {
    vec!["text/plain".to_owned(), LAB_MEDIA_TYPE.to_owned()]
}

fn interface(url: &str, protocol_binding: &str) -> AgentInterface {
    AgentInterface {
        url: url.to_owned(),
        protocol_binding: protocol_binding.to_owned(),
        protocol_version: A2A_PROTOCOL_VERSION.to_owned(),
        tenant: None,
    }
}

fn skills(agent_message: bool) -> Vec<AgentSkill> {
    let mut items = vec![
        (
            "list-log-sources",
            "List log sources",
            "List the log sources this agent can read",
            LAB_MEDIA_TYPE,
        ),
        (
            "query-logs",
            "Query logs",
            "Read structured logs from a source over a UTC time range",
            LAB_MEDIA_TYPE,
        ),
        (
            "list-metrics",
            "List metrics",
            "List the metrics this agent can read",
            LAB_MEDIA_TYPE,
        ),
        (
            "query-metric",
            "Query metric",
            "Read metric samples over a UTC time range",
            LAB_MEDIA_TYPE,
        ),
        (
            "list-tasks",
            "List tasks",
            "List the tasks this agent can start",
            LAB_MEDIA_TYPE,
        ),
        (
            "start-task",
            "Start task",
            "Start a task with a JSON object input. Waits until the run is terminal unless wait is false.",
            LAB_MEDIA_TYPE,
        ),
        (
            "get-task-status",
            "Get task status",
            "Read the status of a started lab run",
            LAB_MEDIA_TYPE,
        ),
        (
            "list-image-sources",
            "List image sources",
            "List the image sources this agent can read",
            LAB_MEDIA_TYPE,
        ),
        (
            "list-images",
            "List images",
            "List image metadata from one source",
            LAB_MEDIA_TYPE,
        ),
        (
            "search-images",
            "Search images",
            "Search image metadata by time range or text",
            LAB_MEDIA_TYPE,
        ),
        (
            "get-image",
            "Get image",
            "Read one image, including its inline bytes",
            LAB_MEDIA_TYPE,
        ),
        (
            "get-current-image",
            "Get current image",
            "Read the current image for one source, including its inline bytes",
            LAB_MEDIA_TYPE,
        ),
    ];
    if agent_message {
        items.push((
            "agent-message",
            "Agent message",
            "Send plain text and continue the conversation by reusing the returned context",
            "text/plain",
        ));
    }
    items
        .into_iter()
        .map(|(id, name, description, media_type)| AgentSkill {
            id: id.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            tags: vec!["lab".to_owned()],
            examples: None,
            input_modes: Some(vec![media_type.to_owned()]),
            output_modes: Some(vec![media_type.to_owned()]),
            security_requirements: None,
        })
        .collect()
}
