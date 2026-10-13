//! Text chat over `agent-message`. The current context lives on the page.

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::KeyboardEvent;

use crate::card::{Access, Card};
use crate::codec::{self, Outcome, ProtocolError};
use crate::shell::http::Transport;
use crate::shell::page::Page;
use crate::shell::present;
use crate::shell::session::{self, calls_allowed, message_id, take_secret};

pub fn install(page: &Page, transport: &Transport) {
    session::on_click(
        page,
        transport,
        "chat-send",
        |page, transport, _event| async move {
            send(&page, &transport).await;
        },
    );
    session::on_click(
        page,
        transport,
        "chat-retry",
        |page, transport, _event| async move {
            send(&page, &transport).await;
        },
    );
    session::on_click(
        page,
        transport,
        "chat-new",
        |page, _transport, _event| async move {
            new_session(&page);
        },
    );
    on_draft_key(page, transport);
}

fn on_draft_key(page: &Page, transport: &Transport) {
    let Some(draft) = page.element("chat-draft") else {
        return;
    };
    let page = page.clone();
    let transport = transport.clone();
    let closure = Closure::wrap(Box::new(move |event: KeyboardEvent| {
        if !sends_on_enter(&event) {
            return;
        }
        event.prevent_default();
        if event.repeat() {
            return;
        }
        let page = page.clone();
        let transport = transport.clone();
        spawn_local(async move {
            send(&page, &transport).await;
        });
    }) as Box<dyn FnMut(KeyboardEvent)>);
    let _ = draft.add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
    closure.forget();
}

fn sends_on_enter(event: &KeyboardEvent) -> bool {
    event.key() == "Enter"
        && !event.shift_key()
        && !event.ctrl_key()
        && !event.alt_key()
        && !event.meta_key()
        && !event.is_composing()
}

pub fn apply(page: &Page, card: &Card) {
    if let Some(reason) = denial(card) {
        disable_chat(page, reason);
        return;
    }
    enable_chat(page);
}

async fn send(page: &Page, transport: &Transport) {
    if !ready(page) {
        return;
    }
    let draft = page.control_value("chat-draft");
    let message = draft.trim();
    if message.is_empty() {
        present::set_chat_alert(page, "Enter a message.");
        present::set_chat_status(page, "error", "Enter a message.");
        page.focus("chat-draft");
        return;
    }
    if !calls_allowed(page) {
        return;
    }
    let context = page.data("chat", "data-context");
    let Ok(secret) = take_secret(page, &[message, context.as_str()]) else {
        return;
    };
    let message = message.to_owned();
    let context = context.trim().to_owned();
    present::set_chat_pending(page, true, false);
    present::set_chat_alert(page, "");
    present::set_chat_status(page, "pending", "Sending message.");
    let body = codec::text_message(&message_id(), message.trim(), context.trim());
    let result = transport.send(&body, secret.as_deref()).await;
    finish_send(page, result, secret.as_deref(), &message);
    page.focus("chat-draft");
}

fn new_session(page: &Page) {
    if !ready(page) {
        return;
    }
    page.set_data("chat", "data-context", "");
    page.set_control("chat-draft", "");
    present::set_transcript(page, "");
    present::set_chat_alert(page, "");
    present::set_chat_pending(page, false, false);
    present::set_chat_status(page, "ready", "New session.");
    page.focus("chat-draft");
}

fn finish_send(page: &Page, result: Result<codec::Reply, ()>, secret: Option<&str>, message: &str) {
    let reply = match result {
        Ok(reply) => reply,
        Err(()) => {
            fail_send(page, "The browser could not reach this origin.");
            return;
        }
    };
    match codec::decode(&reply, secret) {
        Ok(outcome) => accept_send(page, &outcome, message),
        Err(ProtocolError::Unauthorized(detail) | ProtocolError::Forbidden(detail)) => {
            fail_send(page, &detail);
        }
        Err(ProtocolError::Malformed) => fail_send(page, "The response could not be read."),
    }
}

fn accept_send(page: &Page, outcome: &Outcome, message: &str) {
    if let Outcome::Status(status) = outcome {
        let detail = if status.message.is_empty() {
            "The agent rejected the message."
        } else {
            status.message.as_str()
        };
        fail_send(page, detail);
        return;
    }
    let Some(context_id) = codec::context_of(outcome) else {
        fail_send(page, "The agent did not return a session.");
        return;
    };
    let task_id = codec::task_id_of(outcome);
    page.set_data("chat", "data-context", context_id);
    let turn = codec::exchange_turn(outcome).unwrap_or_default();
    append_new_turn(page, task_id, &visible_turn(&turn, message));
    if page.control_value("chat-draft").trim() == message {
        page.set_control("chat-draft", "");
    }
    present::set_chat_pending(page, false, false);
    present::set_chat_alert(page, "");
    present::set_chat_status(page, "ready", "Reply received.");
}

fn fail_send(page: &Page, detail: &str) {
    present::set_chat_pending(page, false, true);
    present::set_chat_alert(page, detail);
    present::set_chat_status(page, "error", "Message was not sent.");
}

fn append_new_turn(page: &Page, task_id: &str, turn: &str) {
    if turn.is_empty() {
        return;
    }
    let existing = page
        .element("chat-log")
        .and_then(|element| element.text_content())
        .unwrap_or_default();
    if !task_id.is_empty() && existing.contains(&format!("Task {task_id}")) {
        return;
    }
    present::append_transcript(page, turn);
}

fn visible_turn(turn: &str, message: &str) -> String {
    if message.is_empty() || turn.contains(message) {
        turn.to_owned()
    } else {
        format!("You\n{message}\n{turn}")
    }
}

fn ready(page: &Page) -> bool {
    page.data("chat", "data-enabled") == "true" && page.data("chat", "data-pending") != "true"
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

fn disable_chat(page: &Page, reason: &str) {
    present::configure_chat(page, false);
    page.set_data("chat", "data-context", "");
    present::set_transcript(page, "");
    present::set_chat_alert(page, "");
    present::set_chat_status(page, "disabled", reason);
}

fn enable_chat(page: &Page) {
    present::configure_chat(page, true);
    present::set_chat_alert(page, "");
    present::set_chat_status(page, "ready", "Chat is ready.");
}
