//! Inventory views. List calls use the page bearer field and do not write storage.

use std::cell::RefCell;
use std::collections::BTreeMap;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::Event;

use crate::card::Card;
use crate::codec;
use crate::inventory::{self, Fault, Item, Kind, Listed, Primitive, QueryFault};
use crate::shell::http::Transport;
use crate::shell::page::{self, Page};
use crate::shell::present;
use crate::shell::session::{self, calls_allowed, message_id, take_secret};

std::thread_local! {
    static VIEWS: RefCell<BTreeMap<Primitive, Vec<Item>>> = const { RefCell::new(BTreeMap::new()) };
    static TRANSPORT: RefCell<Option<Transport>> = const { RefCell::new(None) };
}

pub fn mount(page: &Page, main: &web_sys::Element, primitive: Primitive) {
    {
        let spec = primitive.spec();
        present::section(page, main, spec.section, spec.heading, |body| {
            page::attr(body, "class", "inventory");
            let status = present::node(page, "p", Some(spec.status));
            page::attr(&status, "role", "status");
            page::attr(&status, "aria-live", "polite");
            body.append_child(&status).ok();
            let alert = present::node(page, "div", Some(spec.alert));
            page::attr(&alert, "role", "alert");
            page::attr(&alert, "aria-live", "assertive");
            body.append_child(&alert).ok();
            if primitive == Primitive::Tasks {
                let note = present::node(page, "p", Some("inventory-tasks-note"));
                note.set_text_content(Some(
                    "Lab task definitions. This list does not use chat sessions or protocol tasks.",
                ));
                body.append_child(&note).ok();
            }
            present::field(page, body, spec.limit_label, spec.limit, false);
            present::field(page, body, spec.cursor_label, spec.cursor, false);
            quiet(page, spec.limit);
            quiet(page, spec.cursor);
            page.set_control(spec.limit, "100");
            let actions = present::node(page, "div", None);
            page::attr(&actions, "class", "actions");
            present::button(page, &actions, spec.more, spec.more_label);
            body.append_child(&actions).ok();
            let items = present::node(page, "ul", Some(spec.items));
            page::attr(&items, "class", "inventory-list");
            page::attr(&items, "aria-label", spec.heading);
            body.append_child(&items).ok();
        });
        show(page, primitive, Kind::Pending, "", "");
        if matches!(primitive, Primitive::Logs | Primitive::Metrics) {
            page.set_data(spec.section, "hidden", "hidden");
        }
        page.set_data(spec.more, "aria-disabled", "true");
        page.set_data(spec.status, "data-next", "");
        page.set_data(spec.status, "data-advertised", "false");
        page.set_data(spec.status, "data-gen", "0");
    }
}

pub fn install(page: &Page, transport: &Transport) {
    TRANSPORT.with(|slot| *slot.borrow_mut() = Some(transport.clone()));
    for primitive in Primitive::ALL {
        let more = primitive.spec().more;
        let items = primitive.spec().items;
        let spec = primitive.spec();
        on_field_change(page, spec.limit, primitive);
        on_field_change(page, spec.cursor, primitive);
        session::on_click(
            page,
            transport,
            more,
            move |page, transport, _event| async move {
                load(&page, &transport, primitive, true).await;
            },
        );
        session::on_click(
            page,
            transport,
            items,
            |page, _transport, event| async move {
                if let Some((command, identifier)) = super::workbench::invoked(&event) {
                    super::workbench::open(&page, &command, &identifier);
                }
            },
        );
    }
}

pub fn reveal(page: &Page, primitive: Primitive) {
    if !shown(page, primitive) {
        return;
    }
    let state = page.data(primitive.spec().status, "data-state");
    if !matches!(
        state.as_str(),
        "" | "pending" | "advertised" | "unadvertised"
    ) {
        return;
    }
    let Some(transport) = TRANSPORT.with(|slot| slot.borrow().clone()) else {
        return;
    };
    let page = page.clone();
    spawn_local(async move {
        load(&page, &transport, primitive, false).await;
    });
}

pub fn reveal_visible(page: &Page) {
    for primitive in Primitive::ALL {
        reveal(page, primitive);
    }
}

fn shown(page: &Page, primitive: Primitive) -> bool {
    page.element(primitive.spec().section)
        .and_then(|section| section.parent_element())
        .and_then(|view| view.parent_element())
        .is_some_and(|main| main.id() == "main")
}

pub fn note_card(page: &Page, card: &Card) {
    for primitive in Primitive::ALL {
        let spec = primitive.spec();
        let advertised = primitive.is_advertised(card);
        page.set_data(
            spec.status,
            "data-advertised",
            if advertised { "true" } else { "false" },
        );
        let state = page.data(spec.status, "data-state");
        if matches!(
            state.as_str(),
            "" | "pending" | "advertised" | "unadvertised"
        ) {
            let kind = if advertised {
                Kind::Advertised
            } else {
                Kind::Unadvertised
            };
            show(page, primitive, kind, "", "");
        }
    }
}

async fn load(page: &Page, transport: &Transport, primitive: Primitive, append: bool) {
    if !calls_allowed(page) {
        return;
    }
    let spec = primitive.spec();
    let limit = page.control_value(spec.limit);
    let cursor = if append {
        page.data(spec.status, "data-next")
    } else {
        page.control_value(spec.cursor)
    };
    let query = if append {
        inventory::continued_query(&cursor, &limit)
    } else {
        inventory::field_query(&cursor, &limit)
    };
    let query = match query {
        Ok(query) => query,
        Err(fault) => {
            page.set_text(spec.alert, QueryFault::message(fault));
            return;
        }
    };
    let Ok(secret) = take_secret(page, &[limit.as_str(), cursor.as_str()]) else {
        return;
    };
    let command = inventory::command_value(primitive, &query);
    let Ok(body) = codec::command_message(&message_id(), &command, "") else {
        show(
            page,
            primitive,
            Kind::Malformed,
            "",
            "The list command could not be built.",
        );
        return;
    };
    let generation = bump(page, spec.status);
    show_loading(page, primitive);
    let result = transport.send(&body, secret.as_deref()).await;
    if page.data(spec.status, "data-gen") != generation.to_string() {
        return;
    }
    match result {
        Err(()) => fail(page, primitive, &Fault::Transport),
        Ok(reply) => match inventory::interpret(primitive, &reply, secret.as_deref()) {
            Ok(listed) => succeed(page, primitive, listed, append),
            Err(fault) => fail(page, primitive, &fault),
        },
    }
}

fn succeed(page: &Page, primitive: Primitive, listed: Listed, append: bool) {
    let spec = primitive.spec();
    let merged = VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        let existing = views.get(&primitive).map(Vec::as_slice).unwrap_or(&[]);
        let merged = inventory::merge_items(existing, &listed.items, !append);
        views.insert(primitive, merged.clone());
        merged
    });
    render(page, primitive, &merged);
    super::workbench::note_loaded(page, primitive);
    if primitive == Primitive::Logs {
        super::logs::sources_changed(page);
    }
    if primitive == Primitive::Metrics {
        super::metrics::catalog_changed(page);
    }
    let next = listed.next_cursor.as_deref().unwrap_or("");
    page.set_data(spec.status, "data-next", next);
    page.set_data(
        spec.more,
        "aria-disabled",
        if next.is_empty() { "true" } else { "false" },
    );
    let kind = if merged.is_empty() {
        Kind::Empty
    } else {
        Kind::Populated
    };
    show(page, primitive, kind, &merged.len().to_string(), "");
}

fn fail(page: &Page, primitive: Primitive, fault: &Fault) {
    let spec = primitive.spec();
    VIEWS.with(|views| {
        views.borrow_mut().remove(&primitive);
    });
    if let Some(list) = page.element(spec.items) {
        page::clear_children(&list);
    }
    page.set_data(spec.status, "data-next", "");
    page.set_data(spec.more, "aria-disabled", "true");
    let kind = inventory::fault_kind(fault);
    let (extra, alert) = match fault {
        Fault::Unavailable(message) => (message.as_str(), message.as_str()),
        Fault::Unauthorized(message) | Fault::Forbidden(message) => ("", message.as_str()),
        Fault::Malformed => ("", "The list result could not be read."),
        Fault::Transport => ("", "The browser could not reach this origin."),
    };
    show(page, primitive, kind, extra, alert);
    super::workbench::note_loaded(page, primitive);
    if primitive == Primitive::Logs {
        super::logs::sources_changed(page);
    }
    if primitive == Primitive::Metrics {
        super::metrics::catalog_changed(page);
    }
}

fn show_loading(page: &Page, primitive: Primitive) {
    let spec = primitive.spec();
    page.set_data(spec.status, "data-state", Kind::Loading.slug());
    page.set_text(spec.status, &Kind::Loading.message(primitive, ""));
    page.set_text(spec.alert, "");
    page.set_data(spec.section, "aria-busy", "true");
}

fn show(page: &Page, primitive: Primitive, kind: Kind, extra: &str, alert: &str) {
    let spec = primitive.spec();
    page.set_data(spec.status, "data-state", kind.slug());
    page.set_data(
        spec.status,
        "data-configured",
        if kind.configured() { "true" } else { "false" },
    );
    page.set_text(spec.status, &kind.message(primitive, extra));
    page.set_text(spec.alert, alert);
    page.set_data(spec.section, "aria-busy", "false");
}

fn render(page: &Page, primitive: Primitive, items: &[Item]) {
    let Some(list) = page.element(primitive.spec().items) else {
        return;
    };
    page::clear_children(&list);
    for item in items {
        let row = present::node(page, "li", None);
        page::attr(&row, "data-id", &item.id);
        let heading = present::node(page, "h3", None);
        heading.set_text_content(Some(&item.name));
        let id = present::node(page, "p", None);
        id.set_text_content(Some(&format!("Id {}", item.id)));
        let description = present::node(page, "p", None);
        description.set_text_content(Some(&item.description));
        row.append_child(&heading).ok();
        row.append_child(&id).ok();
        row.append_child(&description).ok();
        if !item.details.is_empty() {
            let meta = present::node(page, "dl", None);
            for (label, value) in &item.details {
                let term = present::node(page, "dt", None);
                term.set_text_content(Some(label));
                let data = present::node(page, "dd", None);
                data.set_text_content(Some(value));
                meta.append_child(&term).ok();
                meta.append_child(&data).ok();
            }
            row.append_child(&meta).ok();
        }
        let actions = present::node(page, "div", None);
        page::attr(&actions, "class", "actions");
        for operation in crate::workbench::openings(primitive) {
            let button = present::node(page, "button", None);
            page::attr(&button, "type", "button");
            page::attr(&button, "data-command", operation.name());
            page::attr(&button, "data-identifier", &item.id);
            button.set_text_content(Some(operation.action()));
            actions.append_child(&button).ok();
        }
        row.append_child(&actions).ok();
        list.append_child(&row).ok();
    }
}

fn bump(page: &Page, status_id: &str) -> u32 {
    let current = page.data(status_id, "data-gen").parse::<u32>().unwrap_or(0);
    let next = current.wrapping_add(1);
    page.set_data(status_id, "data-gen", &next.to_string());
    next
}

fn on_field_change(page: &Page, id: &str, primitive: Primitive) {
    let Some(element) = page.element(id) else {
        return;
    };
    let page = page.clone();
    let closure = Closure::wrap(Box::new(move |_event: Event| {
        let Some(transport) = TRANSPORT.with(|slot| slot.borrow().clone()) else {
            return;
        };
        let page = page.clone();
        spawn_local(async move {
            load(&page, &transport, primitive, false).await;
        });
    }) as Box<dyn FnMut(Event)>);
    let _ = element.add_event_listener_with_callback("change", closure.as_ref().unchecked_ref());
    closure.forget();
}

pub(super) fn loaded(primitive: Primitive) -> Vec<Item> {
    VIEWS.with(|views| views.borrow().get(&primitive).cloned().unwrap_or_default())
}

fn quiet(page: &Page, id: &str) {
    if let Some(element) = page.element(id) {
        page::attr(&element, "autocomplete", "off");
        page::attr(&element, "spellcheck", "false");
        page::attr(&element, "inputmode", "numeric");
    }
}
