//! Inventory views. List calls use the page bearer field and do not write storage.

use std::cell::RefCell;

use dioxus::prelude::*;

use crate::card::Card;
use crate::codec;
use crate::inventory::{self, Fault, Item, Kind, Listed, Primitive, QueryFault};
use crate::shell::http::Transport;
use crate::shell::session::{self, calls_allowed, message_id, take_secret};

#[derive(Clone, PartialEq)]
pub struct InventoryUi {
    pub kind: Kind,
    pub advertised: bool,
    pub next: String,
    pub tick: u32,
    pub alert: String,
    pub items: Vec<Item>,
}

impl InventoryUi {
    pub fn pending() -> Self {
        Self {
            kind: Kind::Pending,
            advertised: false,
            next: String::new(),
            tick: 0,
            alert: String::new(),
            items: Vec::new(),
        }
    }

    pub fn status(&self, primitive: Primitive) -> String {
        let extra = match self.kind {
            Kind::Populated => self.items.len().to_string(),
            Kind::Unavailable => self.alert.clone(),
            _ => String::new(),
        };
        self.kind.message(primitive, &extra)
    }
}

#[derive(Clone, PartialEq)]
enum InventoryJob {
    Idle,
    Load {
        primitive: Primitive,
        append: bool,
        cursor: String,
        limit: String,
        tick: u32,
        secret: Option<String>,
    },
}

#[derive(Clone, PartialEq)]
enum InventoryOutcome {
    Idle,
    QueryAlert {
        tick: u32,
        message: String,
    },
    Ready {
        tick: u32,
        listed: Listed,
        append: bool,
    },
    Failed {
        tick: u32,
        fault: Fault,
    },
}

#[derive(Clone, Copy)]
struct Loader {
    ui: Signal<InventoryUi>,
    job: Signal<InventoryJob>,
    tick: Signal<u32>,
}

thread_local! {
    static LOGS: RefCell<Option<Signal<InventoryUi>>> = const { RefCell::new(None) };
    static METRICS: RefCell<Option<Signal<InventoryUi>>> = const { RefCell::new(None) };
    static TASKS: RefCell<Option<Signal<InventoryUi>>> = const { RefCell::new(None) };
    static IMAGES: RefCell<Option<Signal<InventoryUi>>> = const { RefCell::new(None) };
    static LOADERS: RefCell<[Option<Loader>; 4]> = const { RefCell::new([None, None, None, None]) };
}

pub fn bind(
    logs: Signal<InventoryUi>,
    metrics: Signal<InventoryUi>,
    tasks: Signal<InventoryUi>,
    images: Signal<InventoryUi>,
) {
    LOGS.with(|slot| *slot.borrow_mut() = Some(logs));
    METRICS.with(|slot| *slot.borrow_mut() = Some(metrics));
    TASKS.with(|slot| *slot.borrow_mut() = Some(tasks));
    IMAGES.with(|slot| *slot.borrow_mut() = Some(images));
}

pub fn loaded(primitive: Primitive) -> Vec<Item> {
    snapshot(primitive).items
}

pub fn snapshot(primitive: Primitive) -> InventoryUi {
    slot(primitive)
        .map(|ui| ui.peek().clone())
        .unwrap_or_else(InventoryUi::pending)
}

pub fn note_card(card: &Card) {
    for primitive in Primitive::ALL {
        let Some(mut ui) = slot(primitive) else {
            continue;
        };
        let advertised = primitive.is_advertised(card);
        let mut next = ui();
        next.advertised = advertised;
        if matches!(
            next.kind,
            Kind::Pending | Kind::Advertised | Kind::Unadvertised
        ) {
            next.kind = if advertised {
                Kind::Advertised
            } else {
                Kind::Unadvertised
            };
            next.alert.clear();
        }
        ui.set(next);
    }
}

pub fn reveal(primitive: Primitive) {
    let Some(loader) = loader(primitive) else {
        return;
    };
    if should_load(&loader.ui.peek()) {
        let _ = start_load(loader.ui, loader.job, loader.tick, primitive, false);
    }
}

#[component]
pub fn InventoryView(primitive: Primitive, ui: Signal<InventoryUi>, visible: bool) -> Element {
    let spec = primitive.spec();
    let job = use_signal(|| InventoryJob::Idle);
    let tick = use_signal(|| 0u32);
    use_hook(|| bind_loader(primitive, ui, job, tick));
    let mut shown = use_signal(|| false);
    if shown() != visible {
        shown.set(visible);
    }
    use_effect(move || {
        prime_fields(spec.limit, spec.cursor);
        if shown() && should_load(&ui.peek()) {
            let _ = start_load(ui, job, tick, primitive, false);
        }
    });
    let reply = use_resource(move || {
        let n = tick();
        let job = job();
        async move { (n, run_job(job).await) }
    });
    let mut applied = use_signal(|| 0u32);
    let _ = reply.read();
    if let Some((n, outcome)) = reply()
        && n > applied()
        && !matches!(outcome, InventoryOutcome::Idle)
    {
        applied.set(n);
        apply_outcome(ui, primitive, outcome);
    }
    let mut armed = use_signal(|| true);
    let state = ui.peek().clone();
    if matches!(state.kind, Kind::Advertised | Kind::Unadvertised) {
        if visible && armed() && start_load(ui, job, tick, primitive, false) {
            armed.set(false);
        }
    } else if !armed() {
        armed.set(true);
    }
    let state = ui();
    let more = !state.next.is_empty();
    rsx! {
        section {
            id: spec.section,
            class: "inventory",
            aria_labelledby: spec.section.to_string() + "-heading",
            hidden: true,
            aria_busy: if state.kind == Kind::Loading { "true" } else { "false" },
            h2 { id: spec.section.to_string() + "-heading", tabindex: "-1", "{spec.heading}" }
            p {
                id: spec.status,
                role: "status",
                aria_live: "polite",
                "data-state": state.kind.slug(),
                "data-configured": if state.kind.configured() { "true" } else { "false" },
                "data-advertised": if state.advertised { "true" } else { "false" },
                "data-next": "{state.next}",
                "data-gen": state.tick.to_string(),
                "{state.status(primitive)}"
            }
            div { id: spec.alert, role: "alert", aria_live: "assertive", "{state.alert}" }
            if primitive == Primitive::Tasks {
                p { id: "inventory-tasks-note",
                    "Lab task definitions. This list does not use chat sessions or protocol tasks."
                }
            }
            label { r#for: spec.limit, "{spec.limit_label}" }
            input {
                id: spec.limit,
                autocomplete: "off",
                spellcheck: "false",
                inputmode: "numeric",
                onchange: move |_| {
                    start_load(ui, job, tick, primitive, false);
                },
            }
            label { r#for: spec.cursor, "{spec.cursor_label}" }
            input {
                id: spec.cursor,
                autocomplete: "off",
                spellcheck: "false",
                inputmode: "numeric",
                onchange: move |_| {
                    start_load(ui, job, tick, primitive, false);
                },
            }
            div { class: "actions",
                button {
                    id: spec.more,
                    r#type: "button",
                    "aria-disabled": if more { "false" } else { "true" },
                    onclick: move |_| {
                        start_load(ui, job, tick, primitive, true);
                    },
                    "{spec.more_label}"
                }
            }
            ul { id: spec.items, class: "inventory-list", aria_label: spec.heading,
                for item in state.items.iter() {
                    InventoryRow { primitive, item: item.clone() }
                }
            }
        }
    }
}

#[component]
fn InventoryRow(primitive: Primitive, item: Item) -> Element {
    if primitive == Primitive::Images {
        return rsx! { ImageCard { item } };
    }
    let identifier = item.id.clone();
    rsx! {
        li { "data-id": "{item.id}",
            h3 { "{item.name}" }
            p { "Id {item.id}" }
            p { "{item.description}" }
            if !item.details.is_empty() {
                dl {
                    for (label, value) in item.details.iter() {
                        dt { "{label}" }
                        dd { "{value}" }
                    }
                }
            }
            div { class: "actions",
                for operation in crate::workbench::openings(primitive) {
                    RowAction {
                        command: operation.name(),
                        identifier: identifier.clone(),
                        label: operation.action(),
                    }
                }
            }
        }
    }
}

#[component]
fn ImageCard(item: Item) -> Element {
    let identifier = item.id.clone();
    rsx! {
        li { class: "image-card", "data-id": "{item.id}",
            div { class: "image-card-head",
                h3 { class: "image-card-name", "{item.name}" }
                span { class: "image-card-id", "{item.id}" }
            }
            if !item.description.is_empty() {
                p { class: "image-card-desc", "{item.description}" }
            }
            if !item.details.is_empty() {
                div { class: "image-card-meta",
                    for (label, value) in item.details.iter() {
                        span { class: "image-meta",
                            span { class: "image-meta-label", "{label}" }
                            span { "{value}" }
                        }
                    }
                }
            }
            div { class: "actions",
                for operation in crate::workbench::openings(Primitive::Images) {
                    RowAction {
                        command: operation.name(),
                        identifier: identifier.clone(),
                        label: operation.action(),
                    }
                }
            }
        }
    }
}

#[component]
fn RowAction(command: &'static str, identifier: String, label: &'static str) -> Element {
    rsx! {
        button {
            r#type: "button",
            "data-command": command,
            "data-identifier": "{identifier}",
            onclick: move |_| open_row(command, &identifier),
            "{label}"
        }
    }
}

fn start_load(
    mut ui: Signal<InventoryUi>,
    mut job: Signal<InventoryJob>,
    mut tick: Signal<u32>,
    primitive: Primitive,
    append: bool,
) -> bool {
    if ui().kind == Kind::Loading {
        return false;
    }
    if !calls_allowed() {
        return false;
    }
    let spec = primitive.spec();
    let limit = session::control_value(spec.limit);
    let cursor = if append {
        ui().next
    } else {
        session::control_value(spec.cursor)
    };
    let query = if append {
        inventory::continued_query(&cursor, &limit)
    } else {
        inventory::field_query(&cursor, &limit)
    };
    if let Err(fault) = query {
        let mut next = ui();
        next.alert = QueryFault::message(fault).to_owned();
        ui.set(next);
        return false;
    }
    let secret = match take_secret(&[limit.as_str(), cursor.as_str()]) {
        Ok(secret) => secret,
        Err(()) => return false,
    };
    if session::transport().is_none() {
        return false;
    }
    let mut next = ui();
    next.tick = next.tick.wrapping_add(1);
    next.kind = Kind::Loading;
    next.alert.clear();
    let stamp = next.tick;
    ui.set(next);
    if matches!(primitive, Primitive::Tasks | Primitive::Images) {
        notify(primitive);
    }
    job.set(InventoryJob::Load {
        primitive,
        append,
        cursor,
        limit,
        tick: stamp,
        secret,
    });
    tick.set(tick() + 1);
    true
}

fn should_load(ui: &InventoryUi) -> bool {
    matches!(ui.kind, Kind::Advertised | Kind::Unadvertised)
}

fn apply_outcome(mut ui: Signal<InventoryUi>, primitive: Primitive, outcome: InventoryOutcome) {
    match outcome {
        InventoryOutcome::Idle => {}
        InventoryOutcome::QueryAlert { tick, message } => {
            let mut next = ui();
            if next.tick == tick {
                next.alert = message;
                ui.set(next);
            }
        }
        InventoryOutcome::Ready {
            tick,
            listed,
            append,
        } => {
            let mut next = ui();
            if next.tick != tick {
                return;
            }
            next.items = inventory::merge_items(&next.items, &listed.items, !append);
            next.next = listed.next_cursor.unwrap_or_default();
            next.kind = if next.items.is_empty() {
                Kind::Empty
            } else {
                Kind::Populated
            };
            next.alert.clear();
            ui.set(next);
            notify(primitive);
        }
        InventoryOutcome::Failed { tick, fault } => {
            let mut next = ui();
            if next.tick != tick {
                return;
            }
            next.items.clear();
            next.next.clear();
            next.kind = inventory::fault_kind(&fault);
            let (extra, alert) = match &fault {
                Fault::Unavailable(message) => (message.as_str(), message.as_str()),
                Fault::Unauthorized(message) | Fault::Forbidden(message) => ("", message.as_str()),
                Fault::Malformed => ("", "The list result could not be read."),
                Fault::Transport => ("", "The browser could not reach this origin."),
            };
            next.alert = alert.to_owned();
            if matches!(fault, Fault::Unavailable(_)) {
                next.alert = extra.to_owned();
            }
            ui.set(next);
            notify(primitive);
        }
    }
}

async fn run_job(job: InventoryJob) -> InventoryOutcome {
    let InventoryJob::Load {
        primitive,
        append,
        cursor,
        limit,
        tick,
        secret,
    } = job
    else {
        return InventoryOutcome::Idle;
    };
    let query = if append {
        inventory::continued_query(&cursor, &limit)
    } else {
        inventory::field_query(&cursor, &limit)
    };
    let query = match query {
        Ok(query) => query,
        Err(fault) => {
            return InventoryOutcome::QueryAlert {
                tick,
                message: QueryFault::message(fault).to_owned(),
            };
        }
    };
    let command = inventory::command_value(primitive, &query);
    let Ok(body) = codec::command_message(&message_id(), &command, "") else {
        return InventoryOutcome::Failed {
            tick,
            fault: Fault::Malformed,
        };
    };
    let Some(transport) = session::transport() else {
        return InventoryOutcome::Failed {
            tick,
            fault: Fault::Transport,
        };
    };
    finish_load(
        &transport,
        primitive,
        secret.as_deref(),
        &body,
        tick,
        append,
    )
    .await
}

async fn finish_load(
    transport: &Transport,
    primitive: Primitive,
    secret: Option<&str>,
    body: &str,
    tick: u32,
    append: bool,
) -> InventoryOutcome {
    match transport.send(body, secret).await {
        Err(()) => InventoryOutcome::Failed {
            tick,
            fault: Fault::Transport,
        },
        Ok(reply) => match inventory::interpret(primitive, &reply, secret) {
            Ok(listed) => InventoryOutcome::Ready {
                tick,
                listed,
                append,
            },
            Err(fault) => InventoryOutcome::Failed { tick, fault },
        },
    }
}

fn notify(primitive: Primitive) {
    super::workbench::note_loaded(primitive);
    match primitive {
        Primitive::Logs => super::logs::sources_changed(),
        Primitive::Metrics => super::metrics::catalog_changed(),
        Primitive::Tasks => super::tasks::catalog_changed(),
        Primitive::Images => super::images::catalog_changed(),
    }
}

fn open_row(command: &str, identifier: &str) {
    super::workbench::open(command, identifier);
}

fn bind_loader(
    primitive: Primitive,
    ui: Signal<InventoryUi>,
    job: Signal<InventoryJob>,
    tick: Signal<u32>,
) {
    LOADERS.with(|slot| {
        slot.borrow_mut()[loader_index(primitive)] = Some(Loader { ui, job, tick });
    });
}

fn loader(primitive: Primitive) -> Option<Loader> {
    LOADERS.with(|slot| slot.borrow()[loader_index(primitive)])
}

fn loader_index(primitive: Primitive) -> usize {
    match primitive {
        Primitive::Logs => 0,
        Primitive::Metrics => 1,
        Primitive::Tasks => 2,
        Primitive::Images => 3,
    }
}

fn slot(primitive: Primitive) -> Option<Signal<InventoryUi>> {
    match primitive {
        Primitive::Logs => LOGS.with(|slot| *slot.borrow()),
        Primitive::Metrics => METRICS.with(|slot| *slot.borrow()),
        Primitive::Tasks => TASKS.with(|slot| *slot.borrow()),
        Primitive::Images => IMAGES.with(|slot| *slot.borrow()),
    }
}

fn prime_fields(limit: &str, cursor: &str) {
    if session::control_value(limit).is_empty() {
        write_control(limit, "100");
    }
    let _ = cursor;
}

fn write_control(id: &str, value: &str) {
    session::write_control(id, value);
}
