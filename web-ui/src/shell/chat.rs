//! Text chat over `agent-message`. Context lives only for this page load.

use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

use crate::card::{Access, Card};
use crate::codec::{self, Outcome, ProtocolError};
use crate::shell::http::Transport;
use crate::shell::session::{self, calls_allowed, message_id, take_secret};

#[derive(Clone, PartialEq)]
pub struct ChatTurn {
    pub user: String,
    pub assistant: String,
    pub task_id: String,
}

#[derive(Clone, PartialEq)]
pub struct ChatUi {
    pub enabled: bool,
    pub pending: bool,
    pub retry: bool,
    pub status: String,
    pub status_state: String,
    pub alert: String,
    pub turns: Vec<ChatTurn>,
    pub context: String,
}

impl ChatUi {
    pub fn checking() -> Self {
        Self {
            enabled: false,
            pending: false,
            retry: false,
            status: "Checking whether this agent accepts chat.".to_owned(),
            status_state: "disabled".to_owned(),
            alert: String::new(),
            turns: Vec::new(),
            context: String::new(),
        }
    }
}

#[derive(Clone, PartialEq)]
enum ChatJob {
    Idle,
    Send { message: String, context: String },
}

#[derive(Clone, PartialEq)]
enum ChatOutcome {
    Idle,
    Sent {
        message: String,
        context: String,
        task_id: String,
        turn: String,
    },
    Failed {
        message: String,
        detail: String,
    },
}

#[component]
pub fn ChatView(chat: Signal<ChatUi>) -> Element {
    let job = use_signal(|| ChatJob::Idle);
    let tick = use_signal(|| 0u32);
    let focus_next = use_signal(|| false);
    use_effect(move || restore_focus(focus_next));
    let reply = use_resource(move || {
        let n = tick();
        let current = job();
        let secret = session::native_secret();
        async move { (n, run_job(current, secret).await) }
    });
    let mut applied = use_signal(|| 0u32);
    let _ = reply.read();
    if let Some((n, outcome)) = reply()
        && n > applied()
        && !matches!(outcome, ChatOutcome::Idle)
    {
        applied.set(n);
        apply_outcome(chat, outcome, focus_next);
    }
    let ui = chat();
    rsx! {
        ChatPanel { ui, chat, job, tick }
    }
}

#[component]
fn ChatPanel(ui: ChatUi, chat: Signal<ChatUi>, job: Signal<ChatJob>, tick: Signal<u32>) -> Element {
    let sendable = ui.enabled && !ui.pending;
    rsx! {
        section {
            id: "chat",
            aria_labelledby: "chat-heading",
            "data-enabled": if ui.enabled { "true" } else { "false" },
            "data-pending": if ui.pending { "true" } else { "false" },
            "data-context": "{ui.context}",
            h2 { id: "chat-heading", "Chat" }
            div { class: "chat-bar",
                button {
                    id: "chat-new",
                    r#type: "button",
                    disabled: !sendable,
                    onclick: move |_| new_session(chat),
                    "New conversation"
                }
            }
            p {
                id: "chat-status",
                role: "status",
                aria_live: "polite",
                "data-state": "{ui.status_state}",
                "{ui.status}"
            }
            div { id: "chat-alert", role: "alert", aria_live: "assertive", "{ui.alert}" }
            ChatLog { turns: ui.turns.clone() }
            ChatComposer { retry: ui.retry, sendable, chat, job, tick }
        }
    }
}

#[component]
fn ChatLog(turns: Vec<ChatTurn>) -> Element {
    rsx! {
        div {
            id: "chat-log",
            role: "log",
            aria_label: "Conversation",
            if !turns.is_empty() {
                ol { class: "chat-turns",
                    for (index, turn) in turns.into_iter().enumerate() {
                        TurnPair { key: "{index}", turn }
                    }
                }
            }
        }
    }
}

#[component]
fn TurnPair(turn: ChatTurn) -> Element {
    rsx! {
        if !turn.user.is_empty() {
            li { class: "chat-row chat-row-user", "data-role": "user",
                span { class: "visually-hidden", "You" }
                p { class: "chat-text", "{turn.user}" }
            }
        }
        if !turn.assistant.is_empty() || !turn.task_id.is_empty() {
            li { class: "chat-row chat-row-assistant", "data-role": "assistant",
                span { class: "visually-hidden", "Agent" }
                if !turn.assistant.is_empty() {
                    p { class: "chat-text", "{turn.assistant}" }
                }
                if !turn.task_id.is_empty() {
                    p { class: "chat-task", "data-task": "{turn.task_id}", "Task {turn.task_id}" }
                }
            }
        }
    }
}

#[component]
fn ChatComposer(
    retry: bool,
    sendable: bool,
    chat: Signal<ChatUi>,
    job: Signal<ChatJob>,
    tick: Signal<u32>,
) -> Element {
    rsx! {
        div { class: "chat-compose",
            div { class: "chat-field",
                label { class: "visually-hidden", r#for: "chat-draft", "Message" }
                textarea {
                    id: "chat-draft",
                    rows: "2",
                    placeholder: "Write a message",
                    autocomplete: "off",
                    spellcheck: "false",
                    disabled: !sendable,
                    onkeydown: move |event| draft_key(event, chat, job, tick),
                }
            }
            if retry && sendable {
                button {
                    id: "chat-retry",
                    r#type: "button",
                    onclick: move |_| start_send(chat, job, tick),
                    "Retry"
                }
            }
            button {
                id: "chat-send",
                r#type: "button",
                disabled: !sendable,
                onclick: move |_| start_send(chat, job, tick),
                "Send"
            }
        }
    }
}

pub fn apply(mut chat: Signal<ChatUi>, card: &Card) {
    if let Some(reason) = denial(card) {
        chat.set(ChatUi {
            enabled: false,
            pending: false,
            retry: false,
            status: reason.to_owned(),
            status_state: "disabled".to_owned(),
            alert: String::new(),
            turns: Vec::new(),
            context: String::new(),
        });
        return;
    }
    let mut ui = chat();
    ui.enabled = true;
    ui.pending = false;
    ui.retry = false;
    ui.status = "Chat is ready.".to_owned();
    ui.status_state = "ready".to_owned();
    ui.alert.clear();
    chat.set(ui);
}

fn draft_key(event: KeyboardEvent, chat: Signal<ChatUi>, job: Signal<ChatJob>, tick: Signal<u32>) {
    if event.key() != Key::Enter || event.data().is_auto_repeating() || skips_enter(&event) {
        return;
    }
    event.prevent_default();
    if event.modifiers().shift() {
        insert_newline();
        return;
    }
    if !ready(&chat()) {
        return;
    }
    start_send(chat, job, tick);
}

fn skips_enter(event: &KeyboardEvent) -> bool {
    event.data().is_composing()
        || event.modifiers().ctrl()
        || event.modifiers().alt()
        || event.modifiers().meta()
}

fn start_send(mut chat: Signal<ChatUi>, mut job: Signal<ChatJob>, mut tick: Signal<u32>) {
    if !ready(&chat()) {
        return;
    }
    let message = session::control_value("chat-draft");
    if message.trim().is_empty() {
        refuse_blank(chat);
        return;
    }
    if !calls_allowed() {
        return;
    }
    let context = chat().context;
    if take_secret(&[message.as_str(), context.as_str()]).is_err() {
        return;
    }
    let mut ui = chat();
    ui.pending = true;
    ui.retry = false;
    ui.alert.clear();
    ui.status = "Sending message.".to_owned();
    ui.status_state = "pending".to_owned();
    chat.set(ui);
    job.set(ChatJob::Send { message, context });
    tick.set(tick() + 1);
}

fn refuse_blank(mut chat: Signal<ChatUi>) {
    let mut ui = chat();
    ui.alert = "Enter a message.".to_owned();
    ui.status = "Enter a message.".to_owned();
    ui.status_state = "error".to_owned();
    chat.set(ui);
    focus_draft();
}

fn new_session(mut chat: Signal<ChatUi>) {
    if !ready(&chat()) {
        return;
    }
    chat.set(ChatUi {
        enabled: true,
        pending: false,
        retry: false,
        status: "New session.".to_owned(),
        status_state: "ready".to_owned(),
        alert: String::new(),
        turns: Vec::new(),
        context: String::new(),
    });
    write_draft("");
    focus_draft();
}

fn apply_outcome(chat: Signal<ChatUi>, outcome: ChatOutcome, mut focus_next: Signal<bool>) {
    match outcome {
        ChatOutcome::Idle => {}
        ChatOutcome::Sent {
            message,
            context,
            task_id,
            turn,
        } => {
            accept_turn(chat, &message, &context, &task_id, &turn);
            focus_next.set(true);
        }
        ChatOutcome::Failed { detail, .. } => {
            reject_turn(chat, &detail);
            focus_next.set(true);
        }
    }
}

fn accept_turn(mut chat: Signal<ChatUi>, message: &str, context: &str, task_id: &str, turn: &str) {
    let mut ui = chat();
    ui.pending = false;
    ui.retry = false;
    ui.alert.clear();
    ui.status = "Reply received.".to_owned();
    ui.status_state = "ready".to_owned();
    ui.context = context.to_owned();
    push_turn(&mut ui.turns, message, task_id, turn);
    chat.set(ui);
    if session::control_value("chat-draft").trim() == message {
        write_draft("");
    }
}

fn reject_turn(mut chat: Signal<ChatUi>, detail: &str) {
    let mut ui = chat();
    ui.pending = false;
    ui.retry = true;
    ui.alert = detail.to_owned();
    ui.status = "Message was not sent.".to_owned();
    ui.status_state = "error".to_owned();
    chat.set(ui);
}

fn restore_focus(mut flag: Signal<bool>) {
    if !flag() {
        return;
    }
    flag.set(false);
    focus_draft();
}

async fn run_job(job: ChatJob, secret: Option<String>) -> ChatOutcome {
    let ChatJob::Send { message, context } = job else {
        return ChatOutcome::Idle;
    };
    if !calls_allowed() {
        return ChatOutcome::Failed {
            message,
            detail: "This page will not send credentials or call an unsupported agent.".to_owned(),
        };
    }
    let Some(transport) = session::transport() else {
        return ChatOutcome::Failed {
            message,
            detail: "This page has no origin to call.".to_owned(),
        };
    };
    let body = codec::text_message(&message_id(), message.trim(), context.trim());
    finish_send(&transport, &message, secret.as_deref(), body).await
}

async fn finish_send(
    transport: &Transport,
    message: &str,
    secret: Option<&str>,
    body: String,
) -> ChatOutcome {
    match transport.send(&body, secret).await {
        Err(()) => ChatOutcome::Failed {
            message: message.to_owned(),
            detail: "The browser could not reach this origin.".to_owned(),
        },
        Ok(reply) => match codec::decode(&reply, secret) {
            Ok(outcome) => accept_send(&outcome, message),
            Err(ProtocolError::Unauthorized(detail) | ProtocolError::Forbidden(detail)) => {
                ChatOutcome::Failed {
                    message: message.to_owned(),
                    detail,
                }
            }
            Err(ProtocolError::Malformed) => ChatOutcome::Failed {
                message: message.to_owned(),
                detail: "The response could not be read.".to_owned(),
            },
        },
    }
}

fn accept_send(outcome: &Outcome, message: &str) -> ChatOutcome {
    if let Outcome::Status(status) = outcome {
        let detail = if status.message.is_empty() {
            "The agent rejected the message.".to_owned()
        } else {
            status.message.clone()
        };
        return ChatOutcome::Failed {
            message: message.to_owned(),
            detail,
        };
    }
    let Some(context) = codec::context_of(outcome) else {
        return ChatOutcome::Failed {
            message: message.to_owned(),
            detail: "The agent did not return a session.".to_owned(),
        };
    };
    let turn = codec::exchange_turn(outcome).unwrap_or_default();
    ChatOutcome::Sent {
        message: message.to_owned(),
        context: context.to_owned(),
        task_id: codec::task_id_of(outcome).to_owned(),
        turn: visible_turn(&turn, message),
    }
}

fn push_turn(turns: &mut Vec<ChatTurn>, message: &str, task_id: &str, turn: &str) {
    if turn.is_empty() {
        return;
    }
    if !task_id.is_empty() && turns.iter().any(|item| item.task_id == task_id) {
        return;
    }
    turns.push(ChatTurn {
        user: message.trim().to_owned(),
        assistant: assistant_body(turn, task_id),
        task_id: task_id.to_owned(),
    });
}

fn assistant_body(turn: &str, task_id: &str) -> String {
    strip_task(agent_section(turn), task_id)
}

fn agent_section(turn: &str) -> &str {
    if let Some((_, rest)) = turn.split_once("\nAgent\n") {
        return rest;
    }
    turn.strip_prefix("Agent\n").unwrap_or(turn)
}

fn strip_task(body: &str, task_id: &str) -> String {
    if task_id.is_empty() {
        return body.trim().to_owned();
    }
    let marker = format!("\nTask {task_id}");
    body.trim_end_matches(marker.as_str()).trim().to_owned()
}

fn visible_turn(turn: &str, message: &str) -> String {
    if message.is_empty() || turn.contains(message) {
        turn.to_owned()
    } else {
        format!("You\n{message}\n{turn}")
    }
}

fn ready(ui: &ChatUi) -> bool {
    ui.enabled && !ui.pending
}

fn denial(card: &Card) -> Option<&'static str> {
    if !card.offers_agent_message() {
        return Some(
            "Chat is unavailable because this agent does not advertise agent-message. Lab commands remain available.",
        );
    }
    if !card.supports_http_json() || matches!(card.access, Access::Unsupported { .. }) {
        return Some("Chat is unavailable for this agent. Lab commands remain available.");
    }
    None
}

fn insert_newline() {
    let Some(area) = draft_area() else {
        return;
    };
    let value = area.value();
    let start = byte_at(
        &value,
        area.selection_start().ok().flatten().unwrap_or(u32::MAX),
    );
    let end = byte_at(
        &value,
        area.selection_end().ok().flatten().unwrap_or(u32::MAX),
    )
    .max(start);
    let mut next = String::with_capacity(value.len() + 1);
    next.push_str(&value[..start]);
    next.push('\n');
    next.push_str(&value[end..]);
    let cursor = utf16_len(&next[..start + 1]);
    area.set_value(&next);
    let _ = area.set_selection_range(cursor, cursor);
}

fn byte_at(value: &str, utf16: u32) -> usize {
    let mut seen = 0u32;
    for (index, ch) in value.char_indices() {
        if seen >= utf16 {
            return index;
        }
        seen += u32::try_from(ch.len_utf16()).unwrap_or(1);
    }
    value.len()
}

fn utf16_len(value: &str) -> u32 {
    value
        .chars()
        .map(|ch| u32::try_from(ch.len_utf16()).unwrap_or(1))
        .sum()
}

fn draft_area() -> Option<web_sys::HtmlTextAreaElement> {
    document_element("chat-draft")?.dyn_into().ok()
}

fn write_draft(value: &str) {
    if let Some(area) = draft_area() {
        area.set_value(value);
    }
}

fn focus_draft() {
    if let Some(element) = document_element("chat-draft")
        && let Ok(html) = element.dyn_into::<HtmlElement>()
    {
        let _ = html.focus();
    }
}

fn document_element(id: &str) -> Option<web_sys::Element> {
    web_sys::window()?.document()?.get_element_by_id(id)
}
