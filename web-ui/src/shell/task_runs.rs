//! Session-local lab run cards and status polling.

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;

use std::cell::RefCell;

use crate::inventory::Fault;
use crate::shell::http::Transport;
use crate::shell::log_chrome::meta;
use crate::shell::page::{self, Page};
use crate::shell::present;
use crate::shell::session::{calls_allowed, message_id, take_secret};
use crate::tasks::{self as schema, Run, Tracked};

pub fn install(transport: &Transport) {
    TRANSPORT.with(|slot| *slot.borrow_mut() = Some(transport.clone()));
}

pub fn start(page: &Page, task_id: String, input: serde_json::Value) {
    let Some(transport) = transport() else {
        return;
    };
    let page = page.clone();
    spawn_local(async move {
        send_start(&page, &transport, &task_id, input).await;
    });
}

async fn send_start(page: &Page, transport: &Transport, task_id: &str, input: serde_json::Value) {
    if !calls_allowed(page) {
        return;
    }
    let rendered = input.to_string();
    let Ok(secret) = take_secret(page, &[task_id, rendered.as_str()]) else {
        return;
    };
    let command = schema::start_command(task_id, &input);
    let Ok(body) = crate::codec::command_message(&message_id(), &command, "") else {
        show_start_alert(page, "The task start could not be built.");
        return;
    };
    page.set_text("task-view-status", "Starting.");
    page.set_text("task-view-alert", "");
    let result = transport.send(&body, secret.as_deref()).await;
    match result {
        Err(()) => show_start_alert(page, "The browser could not reach this origin."),
        Ok(reply) => match schema::interpret(&reply, secret.as_deref()) {
            Ok(run) => accept_run(page, transport, run),
            Err(fault) => show_start_alert(page, &fault_message(&fault)),
        },
    }
}

fn accept_run(page: &Page, transport: &Transport, run: Run) {
    let terminal = run.terminal();
    let id = run.id.clone();
    let generation = RUNS.with(|slot| {
        let mut runs = slot.borrow_mut();
        schema::apply(&mut runs, run, 1);
        if terminal {
            None
        } else {
            schema::begin_poll(&mut runs, &id)
        }
    });
    page.set_text("task-view-status", "");
    page.set_text("task-view-alert", "");
    paint_runs(page);
    if let Some(generation) = generation {
        schedule_poll(page.clone(), transport.clone(), id, generation);
    }
}

fn schedule_poll(page: Page, transport: Transport, run_id: String, generation: u32) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let closure = Closure::once(move || {
        spawn_local(async move {
            poll_status(&page, &transport, &run_id, generation).await;
        });
    });
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        closure.as_ref().unchecked_ref(),
        250,
    );
    closure.forget();
}

async fn poll_status(page: &Page, transport: &Transport, run_id: &str, generation: u32) {
    if !poll_current(run_id, generation) {
        return;
    }
    if !calls_allowed(page) {
        return;
    }
    let Ok(secret) = take_secret(page, &[run_id]) else {
        return;
    };
    let command = schema::status_command(run_id);
    let Ok(body) = crate::codec::command_message(&message_id(), &command, "") else {
        show_run_alert(page, "The task status query could not be built.");
        return;
    };
    let result = transport.send(&body, secret.as_deref()).await;
    if !poll_current(run_id, generation) {
        return;
    }
    match result {
        Err(()) => {
            show_run_alert(page, "The browser could not reach this origin.");
            schedule_poll(
                page.clone(),
                transport.clone(),
                run_id.to_owned(),
                generation,
            );
        }
        Ok(reply) => match schema::interpret(&reply, secret.as_deref()) {
            Ok(run) => apply_status(page, transport, run, generation),
            Err(fault) => {
                show_run_alert(page, &fault_message(&fault));
                schedule_poll(
                    page.clone(),
                    transport.clone(),
                    run_id.to_owned(),
                    generation,
                );
            }
        },
    }
}

fn apply_status(page: &Page, transport: &Transport, run: Run, generation: u32) {
    if !poll_current(&run.id, generation) {
        return;
    }
    let id = run.id.clone();
    let continue_poll = RUNS.with(|slot| {
        let mut runs = slot.borrow_mut();
        let seq = schema::next_seq(&runs, &id);
        let _ = schema::apply(&mut runs, run, seq);
        runs.iter()
            .any(|item| item.run.id == id && !item.run.terminal())
    });
    paint_runs(page);
    if continue_poll {
        schedule_poll(page.clone(), transport.clone(), id, generation);
    }
}

fn paint_runs(page: &Page) {
    let Some(list) = page.element("task-runs") else {
        return;
    };
    page::clear_children(&list);
    let runs = RUNS.with(|slot| slot.borrow().clone());
    for tracked in &runs {
        list.append_child(&run_card(page, tracked)).ok();
    }
}

fn run_card(page: &Page, tracked: &Tracked) -> web_sys::Element {
    let item = present::node(page, "li", None);
    let card = present::node(page, "div", None);
    page::attr(&card, "class", "log-card");
    page::attr(&card, "data-run", &tracked.run.id);
    page::attr(&card, "data-gen", &tracked.poll.to_string());
    let identity = present::node(page, "div", None);
    page::attr(&identity, "class", "log-card-identity");
    let id = present::node(page, "span", None);
    page::attr(&id, "class", "log-card-time");
    id.set_text_content(Some(&tracked.run.id));
    let title = present::node(page, "p", None);
    page::attr(&title, "class", "log-card-title");
    title.set_text_content(Some(&tracked.run.task_id));
    identity.append_child(&id).ok();
    identity.append_child(&title).ok();
    let tags = present::node(page, "div", None);
    page::attr(&tags, "class", "log-card-tags");
    meta(page, &tags, "State", &tracked.run.state, None);
    chip(page, &tags, "Progress", &tracked.run.progress);
    chip(page, &tags, "Message", &tracked.run.message);
    chip(page, &tags, "Result", &tracked.run.result);
    chip(page, &tags, "Error kind", &tracked.run.error_kind);
    chip(
        page,
        &tags,
        "Error identifier",
        &tracked.run.error_identifier,
    );
    card.append_child(&identity).ok();
    card.append_child(&tags).ok();
    item.append_child(&card).ok();
    item
}

fn chip(page: &Page, parent: &web_sys::Element, label: &str, value: &str) {
    if !value.is_empty() {
        meta(page, parent, label, value, None);
    }
}

fn poll_current(run_id: &str, generation: u32) -> bool {
    RUNS.with(|slot| schema::poll_live(&slot.borrow(), run_id, generation))
}

fn fault_message(fault: &Fault) -> String {
    match fault {
        Fault::Unavailable(message) => message.clone(),
        Fault::Unauthorized(message) | Fault::Forbidden(message) => message.clone(),
        Fault::Malformed => "The task result could not be read.".to_owned(),
        Fault::Transport => "The browser could not reach this origin.".to_owned(),
    }
}

fn show_start_alert(page: &Page, message: &str) {
    page.set_text("task-view-status", "The task could not be started.");
    page.set_text("task-view-alert", message);
}

fn show_run_alert(page: &Page, message: &str) {
    page.set_text("task-view-status", "The task run could not be read.");
    page.set_text("task-view-alert", message);
}

fn transport() -> Option<Transport> {
    TRANSPORT.with(|slot| slot.borrow().clone())
}

std::thread_local! {
    static TRANSPORT: RefCell<Option<Transport>> = const { RefCell::new(None) };
    static RUNS: RefCell<Vec<Tracked>> = const { RefCell::new(Vec::new()) };
}
