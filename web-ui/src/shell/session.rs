//! Page session. The bearer token lives only in the password control.

use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::Event;

use crate::card::{Access, Card};
use std::future::Future;

use crate::codec::{self, ProtocolError};
use crate::shell::chat;
use crate::shell::http::Transport;
use crate::shell::inventory;
use crate::shell::page::Page;
use crate::shell::present::{self, Connection};

pub fn start() {
    spawn_local(async {
        boot().await;
    });
}

async fn boot() {
    let Some(page) = Page::bind() else {
        return;
    };
    present::mount(&page);
    super::nav::install(&page);
    let Some(origin) = page_origin() else {
        present::show_failure(
            &page,
            Connection::Unsupported,
            "This page has no origin to call.",
        );
        return;
    };
    let transport = Transport::new(origin);
    listen(&page, &transport);
    refresh(&page, &transport).await;
}

fn listen(page: &Page, transport: &Transport) {
    on_click(
        page,
        transport,
        "retry",
        |page, transport, _event| async move {
            refresh(&page, &transport).await;
        },
    );
    on_click(
        page,
        transport,
        "send-text",
        |page, transport, _event| async move {
            send_text(&page, &transport).await;
        },
    );
    on_click(
        page,
        transport,
        "send-command",
        |page, transport, _event| async move {
            send_command(&page, &transport).await;
        },
    );
    on_click(
        page,
        transport,
        "list-tasks",
        |page, transport, _event| async move {
            list_tasks(&page, &transport).await;
        },
    );
    chat::install(page, transport);
    inventory::install(page, transport);
    super::logs::install(page, transport);
    super::metrics::install(page, transport);
    super::tasks::install(page, transport);
    super::workbench::install(page, transport);
}

pub(super) fn on_click<F, Fut>(page: &Page, transport: &Transport, id: &str, action: F)
where
    F: Fn(Page, Transport, Event) -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    let Some(element) = page.element(id) else {
        return;
    };
    let page = page.clone();
    let transport = transport.clone();
    let action = Rc::new(action);
    let closure = Closure::wrap(Box::new(move |event: Event| {
        let page = page.clone();
        let transport = transport.clone();
        let action = Rc::clone(&action);
        spawn_local(async move {
            action(page, transport, event).await;
        });
    }) as Box<dyn FnMut(Event)>);
    let _ = element.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
    closure.forget();
}

async fn refresh(page: &Page, transport: &Transport) {
    present::set_connection(page, Connection::Loading);
    let secret = page.secret();
    match transport.get_card().await {
        Err(()) => present::show_failure(
            page,
            Connection::Offline,
            "The browser could not reach this origin.",
        ),
        Ok(reply) => match crate::card::parse_card(&reply.body) {
            Err(()) => present::show_failure(
                page,
                Connection::Malformed,
                "The agent card could not be read.",
            ),
            Ok(card) => apply_card(page, &card, secret.as_deref()),
        },
    }
}

fn apply_card(page: &Page, card: &Card, secret: Option<&str>) {
    present::show_card(page, card, secret);
    chat::apply(page, card);
    inventory::note_card(page, card);
    if !card.supports_http_json() {
        present::show_failure(
            page,
            Connection::Unsupported,
            "This agent does not advertise HTTP+JSON.",
        );
        return;
    }
    if matches!(card.access, Access::Unsupported { .. }) {
        present::show_failure(
            page,
            Connection::Unsupported,
            "This agent requires a security scheme this page cannot use.",
        );
        return;
    }
    present::set_connection(page, Connection::Connected);
    page.set_text("alert", "");
    inventory::reveal_visible(page);
}

async fn send_text(page: &Page, transport: &Transport) {
    if !calls_allowed(page) {
        return;
    }
    let text = page.control_value("message-text");
    if text.trim().is_empty() {
        page.set_text("alert", "Enter a message.");
        return;
    }
    let context = page.control_value("context-id");
    let Ok(secret) = take_secret(page, &[text.as_str(), context.as_str()]) else {
        return;
    };
    let body = codec::text_message(&message_id(), text.trim(), context.trim());
    let result = transport.send(&body, secret.as_deref()).await;
    finish(page, result, secret.as_deref());
}

async fn send_command(page: &Page, transport: &Transport) {
    if !calls_allowed(page) {
        return;
    }
    let raw = page.control_value("command-json");
    let context = page.control_value("context-id");
    let Ok(command) = codec::parse_command(raw.trim()) else {
        page.set_text("alert", "The lab command is not a JSON object.");
        return;
    };
    let Ok(secret) = take_secret(page, &[raw.as_str(), context.as_str()]) else {
        return;
    };
    let Ok(body) = codec::command_message(&message_id(), &command, context.trim()) else {
        page.set_text("alert", "The lab command is not a JSON object.");
        return;
    };
    let result = transport.send(&body, secret.as_deref()).await;
    finish(page, result, secret.as_deref());
}

async fn list_tasks(page: &Page, transport: &Transport) {
    if !calls_allowed(page) {
        return;
    }
    let context = page.control_value("context-id");
    if context.trim().is_empty() {
        page.set_text("alert", "Enter a context to list protocol tasks.");
        return;
    }
    let Ok(secret) = take_secret(page, &[context.as_str()]) else {
        return;
    };
    let path = codec::tasks_path(context.trim());
    let result = transport.tasks(&path, secret.as_deref()).await;
    finish(page, result, secret.as_deref());
}

fn finish(page: &Page, result: Result<codec::Reply, ()>, secret: Option<&str>) {
    match result {
        Err(()) => present::show_failure(
            page,
            Connection::Offline,
            "The browser could not reach this origin.",
        ),
        Ok(reply) => match codec::decode(&reply, secret) {
            Ok(outcome) => present::show_outcome(page, &outcome),
            Err(ProtocolError::Unauthorized(message)) => {
                present::show_failure(page, Connection::Unauthorized, &message);
            }
            Err(ProtocolError::Forbidden(message)) => {
                present::show_failure(page, Connection::Forbidden, &message);
            }
            Err(ProtocolError::Malformed) => present::show_failure(
                page,
                Connection::Malformed,
                "The response could not be read.",
            ),
        },
    }
}

pub(super) fn calls_allowed(page: &Page) -> bool {
    let access = page.data("security", "data-access");
    let state = page.data("connection", "data-state");
    let allowed = matches!(access.as_str(), "anonymous" | "bearer") && state != "unsupported";
    if !allowed {
        page.set_text(
            "alert",
            "This page will not send credentials or call an unsupported agent.",
        );
    }
    allowed
}

pub(super) fn take_secret(page: &Page, visible: &[&str]) -> Result<Option<String>, ()> {
    let secret = page.secret();
    if let Some(secret) = &secret
        && visible.iter().any(|value| value.contains(secret))
    {
        page.set_text(
            "alert",
            "The bearer token can only be sent in the Authorization header.",
        );
        return Err(());
    }
    Ok(secret)
}

pub(super) fn page_origin() -> Option<String> {
    web_sys::window()?.location().origin().ok()
}

pub(super) fn message_id() -> String {
    static SEQ: AtomicU32 = AtomicU32::new(1);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let millis = js_sys::Date::now() as u64;
    format!("ui-{millis}-{seq}")
}
