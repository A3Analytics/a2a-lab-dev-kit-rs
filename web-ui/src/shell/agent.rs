//! Dioxus agent card, security, and Advanced protocol controls.

use dioxus::prelude::*;

use crate::shell::session::{self, ProtocolOutcome, ProtocolRequest, SessionUi};

#[component]
pub fn AgentView(session: Signal<SessionUi>, mut fetch_gen: Signal<u32>) -> Element {
    let request = use_signal(|| None::<ProtocolRequest>);
    let request_gen = use_signal(|| 0u32);
    let protocol = use_resource(move || {
        let tick = request_gen();
        let request = request();
        let secret = session::native_secret();
        async move { (tick, session::run_protocol(request, secret).await) }
    });
    let mut applied = use_signal(|| 0u32);
    let _ = protocol.read();
    if let Some((tick, outcome)) = protocol()
        && tick > applied()
        && !matches!(outcome, ProtocolOutcome::Idle)
    {
        applied.set(tick);
        session::apply_protocol(session, outcome);
    }
    let ui = session();
    rsx! {
        section {
            id: "identity",
            aria_labelledby: "identity-heading",
            h2 { id: "identity-heading", tabindex: "-1", "Identity" }
            p { id: "agent-name", class: "agent-name", "{ui.name}" }
            p { id: "agent-description", "{ui.description}" }
            p { id: "agent-version", "{ui.version}" }
        }
        ListSection {
            id: "interfaces",
            title: "Interfaces",
            empty: "No interfaces are advertised.",
            items: ui.interfaces.clone(),
        }
        ListSection {
            id: "skills",
            title: "Skills",
            empty: "No skills are advertised.",
            items: ui.skills.clone(),
        }
        ListSection {
            id: "capabilities",
            title: "Capabilities",
            empty: "No capabilities are advertised.",
            items: ui.capabilities.clone(),
        }
        section {
            id: "security",
            aria_labelledby: "security-heading",
            "data-access": "{ui.access}",
            h2 { id: "security-heading", tabindex: "-1", "Security" }
            p { id: "access", "{ui.access_label}" }
            if ui.schemes.is_empty() {
                p {
                    id: "security-schemes-empty",
                    class: "empty-state",
                    "No security schemes are advertised."
                }
            } else {
                ul { id: "security-schemes",
                    for line in ui.schemes.iter() {
                        li { "{line}" }
                    }
                }
            }
            div { id: "credential",
                if ui.access == "bearer" {
                    label { r#for: "bearer-token", "Bearer token" }
                    input {
                        id: "bearer-token",
                        r#type: "password",
                        autocomplete: "off",
                        spellcheck: "false",
                    }
                }
            }
        }
        details { id: "protocol-advanced",
            summary { id: "protocol-advanced-summary", "Advanced" }
            section {
                id: "protocol",
                aria_labelledby: "protocol-heading",
                h2 { id: "protocol-heading", tabindex: "-1", "Protocol" }
                label { r#for: "message-text", "Message" }
                textarea { id: "message-text", rows: "4" }
                label { r#for: "command-json", "Lab command" }
                textarea { id: "command-json", rows: "4" }
                label { r#for: "context-id", "Context" }
                input { id: "context-id" }
                div { class: "actions",
                    button {
                        id: "send-text",
                        r#type: "button",
                        onclick: move |_| start_job(session, request, request_gen, session::request_text),
                        "Send text"
                    }
                    button {
                        id: "send-command",
                        r#type: "button",
                        onclick: move |_| start_job(session, request, request_gen, session::request_command),
                        "Send command"
                    }
                    button {
                        id: "list-tasks",
                        r#type: "button",
                        onclick: move |_| start_job(session, request, request_gen, session::request_tasks),
                        "List tasks"
                    }
                    button {
                        id: "retry",
                        r#type: "button",
                        onclick: move |_| {
                            session::begin_retry(session);
                            fetch_gen.set(fetch_gen() + 1);
                        },
                        "Retry connection"
                    }
                }
                pre { id: "output", aria_label: "Protocol result", "{ui.output}" }
            }
        }
    }
}

fn start_job(
    session: Signal<SessionUi>,
    mut request: Signal<Option<ProtocolRequest>>,
    mut tick: Signal<u32>,
    build: fn(&SessionUi) -> Result<ProtocolRequest, String>,
) {
    match build(&session()) {
        Ok(job) => {
            request.set(Some(job));
            tick.set(tick() + 1);
        }
        Err(message) => session::set_alert_ui(session, &message),
    }
}

#[component]
fn ListSection(id: String, title: String, empty: String, items: Vec<String>) -> Element {
    let heading = format!("{id}-heading");
    let list = format!("{id}-list");
    let empty_id = format!("{id}-empty");
    rsx! {
        section {
            id: "{id}",
            aria_labelledby: "{heading}",
            h2 { id: "{heading}", tabindex: "-1", "{title}" }
            if items.is_empty() {
                p { id: "{empty_id}", class: "empty-state", "{empty}" }
            } else {
                ul { id: "{list}",
                    for line in items.iter() {
                        li { "{line}" }
                    }
                }
            }
        }
    }
}
