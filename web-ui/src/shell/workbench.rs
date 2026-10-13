//! Command workbench. History and the bearer token stay in page memory.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, Event};

use crate::inventory::{Item, Primitive};
use crate::shell::http::Transport;
use crate::shell::page::{self, Page};
use crate::shell::present;
use crate::shell::session::{self, calls_allowed, message_id, take_secret};
use crate::workbench::report::{self as outcome, ImagePreview, Report};
use crate::workbench::tree::Node;
use crate::workbench::{self, HISTORY_LIMIT, HistoryItem, Operation};

std::thread_local! {
    static HISTORY: RefCell<Vec<HistoryItem>> = const { RefCell::new(Vec::new()) };
    static SCOPED: Cell<Option<Primitive>> = const { Cell::new(None) };
}

pub fn mount(page: &Page, main: &Element) {
    present::section(page, main, "workbench", "Command", |body| {
        page::attr(body, "class", "workbench");
        let note = present::node(page, "p", Some("workbench-note"));
        note.set_text_content(Some(&workbench::preview_note()));
        body.append_child(&note).ok();
        let status = present::node(page, "p", Some("workbench-status"));
        page::attr(&status, "role", "status");
        page::attr(&status, "aria-live", "polite");
        body.append_child(&status).ok();
        let alert = present::node(page, "div", Some("workbench-alert"));
        page::attr(&alert, "role", "alert");
        page::attr(&alert, "aria-live", "assertive");
        body.append_child(&alert).ok();
        operation_select(page, body);
        target_select(page, body);
        present::field(page, body, "Parameters", "workbench-params", true);
        quiet(page, "workbench-params");
        let actions = present::node(page, "div", None);
        page::attr(&actions, "class", "actions");
        present::button(page, &actions, "workbench-send", "Send command");
        body.append_child(&actions).ok();
        let facts = present::node(page, "dl", Some("workbench-facts"));
        body.append_child(&facts).ok();
        let tree = present::node(page, "div", Some("workbench-tree"));
        page::attr(&tree, "class", "workbench-tree");
        body.append_child(&tree).ok();
        let image = present::node(page, "div", Some("workbench-image"));
        body.append_child(&image).ok();
        let history = present::node(page, "ul", Some("workbench-history"));
        page::attr(&history, "class", "workbench-history");
        page::attr(&history, "aria-label", "Command history");
        body.append_child(&history).ok();
    });
    page.set_data("workbench-status", "data-gen", "0");
    apply_template(page);
}

pub fn install(page: &Page, transport: &Transport) {
    session::on_click(
        page,
        transport,
        "workbench-send",
        |page, transport, _event| async move {
            send(&page, &transport, false).await;
        },
    );
    session::on_click(
        page,
        transport,
        "workbench-history",
        |page, transport, event| async move {
            if let Some(index) = retry_index(&event) {
                restore(&page, index);
                send(&page, &transport, true).await;
            }
        },
    );
    on_change(page, "workbench-operation", retarget);
    on_change(page, "workbench-target", choose_target);
}

pub(super) fn open(page: &Page, name: &str, identifier: &str) {
    let Some(operation) = Operation::parse(name) else {
        return;
    };
    page.set_control("workbench-operation", operation.name());
    show_params(page, &operation.prefill(identifier));
    page.set_text("workbench-alert", "");
    if let Some(primitive) = scoped() {
        sync_target(page, primitive);
    }
    if page.data("workbench-target-field", "hidden").is_empty() {
        page.focus("workbench-target");
    } else {
        page.focus("workbench-params");
    }
}

pub(super) fn scope(page: &Page, primitive: Primitive) {
    SCOPED.with(|slot| slot.set(Some(primitive)));
    let Some(select) = page.element("workbench-operation") else {
        return;
    };
    let commands = workbench::commands(primitive);
    let current = page.control_value("workbench-operation");
    let kept = commands.iter().any(|operation| operation.name() == current);
    page::clear_children(&select);
    for operation in commands {
        let option = present::node(page, "option", None);
        page::attr(&option, "value", operation.name());
        option.set_text_content(Some(operation.label()));
        select.append_child(&option).ok();
    }
    if kept {
        page.set_control("workbench-operation", &current);
    } else if let Some(operation) = commands.first() {
        page.set_control("workbench-operation", operation.name());
        apply_template(page);
    }
    sync_target(page, primitive);
}

pub(super) fn invoked(event: &Event) -> Option<(String, String)> {
    let command = ancestor_attr(event, "data-command")?;
    let identifier = ancestor_attr(event, "data-identifier")?;
    Some((command, identifier))
}

async fn send(page: &Page, transport: &Transport, restored: bool) {
    if !calls_allowed(page) {
        return;
    }
    let raw = page.control_value("workbench-params");
    let Some(operation) = Operation::parse(&page.control_value("workbench-operation")) else {
        page.set_text("workbench-alert", "Choose a lab command.");
        return;
    };
    let params = match workbench::parse_params(&raw) {
        Ok(params) => params,
        Err(fault) => {
            page.set_text("workbench-alert", fault.message());
            return;
        }
    };
    if !restored {
        show_params(page, &params);
    }
    let Ok(secret) = take_secret(page, &[raw.as_str()]) else {
        return;
    };
    let Ok(body) = workbench::envelope(&message_id(), operation, &params) else {
        page.set_text(
            "workbench-alert",
            workbench::EditorFault::NotObject.message(),
        );
        return;
    };
    let generation = bump(page);
    page.set_data("workbench", "aria-busy", "true");
    page.set_text("workbench-alert", "");
    page.set_text(
        "workbench-status",
        &format!("Sending {}.", operation.name()),
    );
    let result = transport.send(&body, secret.as_deref()).await;
    if page.data("workbench-status", "data-gen") != generation.to_string() {
        return;
    }
    let report = match result {
        Err(()) => outcome::transport_report(operation),
        Ok(reply) => outcome::interpret(operation, &reply, secret.as_deref()),
    };
    remember(page, operation, &params, &report, secret.as_deref());
    render_report(page, &report);
    page.set_data("workbench", "aria-busy", "false");
}

fn restore(page: &Page, index: usize) {
    let Some(item) = HISTORY.with(|history| history.borrow().get(index).cloned()) else {
        return;
    };
    page.set_control("workbench-operation", item.operation.name());
    show_params(page, &item.params);
}

fn remember(
    page: &Page,
    operation: Operation,
    params: &serde_json::Value,
    report: &Report,
    secret: Option<&str>,
) {
    let item = outcome::history_item(operation, params, report, secret);
    HISTORY.with(|history| {
        let mut history = history.borrow_mut();
        history.push(item);
        let extra = history.len().saturating_sub(HISTORY_LIMIT);
        if extra > 0 {
            history.drain(0..extra);
        }
    });
    render_history(page);
}

fn apply_template(page: &Page) {
    let Some(operation) = Operation::parse(&page.control_value("workbench-operation")) else {
        return;
    };
    show_params(page, &operation.template());
}

fn retarget(page: &Page) {
    apply_template(page);
    if let Some(primitive) = scoped() {
        sync_target(page, primitive);
    }
}

fn choose_target(page: &Page) {
    let Some(operation) = Operation::parse(&page.control_value("workbench-operation")) else {
        return;
    };
    let Some(field) = operation.target_field() else {
        return;
    };
    let id = page.control_value("workbench-target");
    if !id.is_empty() {
        write_param(page, field, &id);
    }
}

pub(super) fn note_loaded(page: &Page, primitive: Primitive) {
    if scoped() == Some(primitive) {
        sync_target(page, primitive);
    }
}

fn scoped() -> Option<Primitive> {
    SCOPED.with(Cell::get)
}

fn sync_target(page: &Page, primitive: Primitive) {
    let Some(operation) = Operation::parse(&page.control_value("workbench-operation")) else {
        page.set_data("workbench-target-field", "hidden", "hidden");
        return;
    };
    let Some(field) = operation.target_field() else {
        page.set_data("workbench-target-field", "hidden", "hidden");
        return;
    };
    page.remove_attr("workbench-target-field", "hidden");
    page.set_text(
        "workbench-target-label",
        operation.target_label().unwrap_or("Choice"),
    );
    let mut items = super::inventory::loaded(primitive);
    let chosen = param_string(page, field);
    if !chosen.is_empty() && !items.iter().any(|item| item.id == chosen) {
        items.insert(
            0,
            Item {
                id: chosen.clone(),
                name: chosen.clone(),
                description: String::new(),
                details: Vec::new(),
            },
        );
    }
    let Some(select) = page.element("workbench-target") else {
        return;
    };
    page::clear_children(&select);
    if items.is_empty() {
        let option = present::node(page, "option", None);
        page::attr(&option, "value", "");
        option.set_text_content(Some("None loaded"));
        select.append_child(&option).ok();
        return;
    }
    for item in &items {
        let option = present::node(page, "option", None);
        page::attr(&option, "value", &item.id);
        option.set_text_content(Some(&target_text(item)));
        select.append_child(&option).ok();
    }
    let selected = if items.iter().any(|item| item.id == chosen) {
        chosen
    } else {
        items[0].id.clone()
    };
    page.set_control("workbench-target", &selected);
    if param_string(page, field).is_empty() {
        write_param(page, field, &selected);
    }
}

fn target_text(item: &Item) -> String {
    if item.name.is_empty() || item.name == item.id {
        item.id.clone()
    } else {
        format!("{} ({})", item.name, item.id)
    }
}

fn param_string(page: &Page, field: &str) -> String {
    let Ok(value) = workbench::parse_params(&page.control_value("workbench-params")) else {
        return String::new();
    };
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn write_param(page: &Page, field: &str, id: &str) {
    let Ok(mut value) = workbench::parse_params(&page.control_value("workbench-params")) else {
        return;
    };
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert(field.to_owned(), serde_json::Value::String(id.to_owned()));
    show_params(page, &value);
}

fn show_params(page: &Page, params: &serde_json::Value) {
    let text = serde_json::to_string_pretty(params).unwrap_or_else(|_| "{}".to_owned());
    page.set_control("workbench-params", &text);
}

fn render_report(page: &Page, report: &Report) {
    page.set_data("workbench", "data-kind", report.kind.slug());
    page.set_data("workbench-status", "data-state", report.kind.slug());
    page.set_text("workbench-status", report.kind.label());
    render_facts(page, report);
    render_tree(page, &report.tree);
    render_image(page, report.image.as_ref());
}

fn render_facts(page: &Page, report: &Report) {
    let Some(facts) = page.element("workbench-facts") else {
        return;
    };
    page::clear_children(&facts);
    fact(page, &facts, "outcome", "Outcome", report.kind.label());
    fact(page, &facts, "operation", "Operation", &report.operation);
    fact(
        page,
        &facts,
        "protocol-task",
        "Protocol task",
        &report.protocol_task_id,
    );
    fact(
        page,
        &facts,
        "protocol-context",
        "Protocol context",
        &report.protocol_context_id,
    );
    fact(
        page,
        &facts,
        "protocol-state",
        "Protocol state",
        &report.protocol_state,
    );
    optional_fact(page, &facts, "lab-run", "Lab run", &report.lab_run_id);
    optional_fact(
        page,
        &facts,
        "lab-run-state",
        "Lab run state",
        &report.lab_run_state,
    );
    optional_fact(page, &facts, "status", "Status", &report.status);
    optional_fact(page, &facts, "code", "Code", &report.code);
    optional_fact(page, &facts, "message", "Message", &report.message);
    for line in &report.reasons {
        fact(page, &facts, "reason", "Reason", &line.reason);
        fact(page, &facts, "domain", "Domain", &line.domain);
    }
    for line in &report.fields {
        fact(page, &facts, "field", "Field", &line.field);
        fact(page, &facts, "detail", "Detail", &line.description);
    }
    for line in &report.correlations {
        fact(page, &facts, "correlation", line.label, &line.value);
    }
}

fn optional_fact(page: &Page, parent: &Element, name: &str, label: &str, value: &str) {
    if !value.is_empty() {
        fact(page, parent, name, label, value);
    }
}

fn fact(page: &Page, parent: &Element, name: &str, label: &str, value: &str) {
    let row = present::node(page, "div", None);
    let term = present::node(page, "dt", None);
    term.set_text_content(Some(label));
    let data = present::node(page, "dd", None);
    page::attr(&data, "data-fact", name);
    data.set_text_content(Some(value));
    row.append_child(&term).ok();
    row.append_child(&data).ok();
    parent.append_child(&row).ok();
}

fn render_tree(page: &Page, node: &Node) {
    let Some(tree) = page.element("workbench-tree") else {
        return;
    };
    page::clear_children(&tree);
    render_node(page, &tree, node);
}

fn render_node(page: &Page, parent: &Element, node: &Node) {
    match node {
        Node::Text(text) => {
            let span = present::node(page, "span", None);
            span.set_text_content(Some(text));
            parent.append_child(&span).ok();
        }
        Node::Group {
            name,
            children,
            collapsed,
        } => {
            let details = present::node(page, "details", None);
            if !collapsed {
                page::attr(&details, "open", "");
            }
            let summary = present::node(page, "summary", None);
            summary.set_text_content(Some(name));
            details.append_child(&summary).ok();
            for child in children {
                render_node(page, &details, child);
            }
            parent.append_child(&details).ok();
        }
    }
}

fn render_image(page: &Page, image: Option<&ImagePreview>) {
    let Some(slot) = page.element("workbench-image") else {
        return;
    };
    page::clear_children(&slot);
    let Some(image) = image else {
        return;
    };
    image_line(page, &slot, "Media type", &image.media_type);
    image_line(
        page,
        &slot,
        "Encoded characters",
        &image.encoded_chars.to_string(),
    );
    if let Some(size) = image.decoded_bytes {
        image_line(page, &slot, "Decoded bytes", &size.to_string());
    }
    if !image.withheld.is_empty() {
        image_line(page, &slot, "Preview", &image.withheld);
    }
    let Some(url) = image.preview_url.as_deref() else {
        return;
    };
    let img = present::node(page, "img", Some("workbench-preview"));
    page::attr(&img, "alt", "Image preview");
    page::attr(&img, "src", url);
    slot.append_child(&img).ok();
}

fn image_line(page: &Page, parent: &Element, label: &str, value: &str) {
    let line = present::node(page, "p", None);
    line.set_text_content(Some(&format!("{label} {value}")));
    parent.append_child(&line).ok();
}

fn render_history(page: &Page) {
    let Some(list) = page.element("workbench-history") else {
        return;
    };
    page::clear_children(&list);
    let items = HISTORY.with(|history| history.borrow().clone());
    for (index, item) in items.iter().enumerate() {
        let row = present::node(page, "li", None);
        let summary = present::node(page, "p", None);
        summary.set_text_content(Some(&item.summary));
        let button = present::node(page, "button", None);
        page::attr(&button, "type", "button");
        page::attr(&button, "data-retry", &index.to_string());
        button.set_text_content(Some("Retry"));
        row.append_child(&summary).ok();
        row.append_child(&button).ok();
        list.append_child(&row).ok();
    }
}

fn operation_select(page: &Page, body: &Element) {
    let label = present::node(page, "label", None);
    page::attr(&label, "for", "workbench-operation");
    label.set_text_content(Some("Operation"));
    let select = present::node(page, "select", Some("workbench-operation"));
    body.append_child(&label).ok();
    body.append_child(&select).ok();
}

fn target_select(page: &Page, body: &Element) {
    let field = present::node(page, "div", Some("workbench-target-field"));
    page::attr(&field, "hidden", "hidden");
    let label = present::node(page, "label", Some("workbench-target-label"));
    page::attr(&label, "for", "workbench-target");
    label.set_text_content(Some("Choice"));
    let select = present::node(page, "select", Some("workbench-target"));
    field.append_child(&label).ok();
    field.append_child(&select).ok();
    body.append_child(&field).ok();
}

fn bump(page: &Page) -> u32 {
    let current = page
        .data("workbench-status", "data-gen")
        .parse::<u32>()
        .unwrap_or(0);
    let next = current.wrapping_add(1);
    page.set_data("workbench-status", "data-gen", &next.to_string());
    next
}

fn retry_index(event: &Event) -> Option<usize> {
    ancestor_attr(event, "data-retry")?.parse().ok()
}

fn ancestor_attr(event: &Event, name: &str) -> Option<String> {
    let mut node = event.target()?.dyn_into::<web_sys::Node>().ok()?;
    loop {
        if let Some(element) = node.dyn_ref::<Element>()
            && let Some(value) = element.get_attribute(name)
        {
            return Some(value);
        }
        node = node.parent_node()?;
    }
}

fn on_change(page: &Page, id: &str, action: fn(&Page)) {
    let Some(element) = page.element(id) else {
        return;
    };
    let page = page.clone();
    let action = Rc::new(action);
    let closure = Closure::wrap(Box::new(move |_event: Event| {
        action(&page);
    }) as Box<dyn FnMut(Event)>);
    let _ = element.add_event_listener_with_callback("change", closure.as_ref().unchecked_ref());
    closure.forget();
}

fn quiet(page: &Page, id: &str) {
    if let Some(element) = page.element(id) {
        page::attr(&element, "autocomplete", "off");
        page::attr(&element, "spellcheck", "false");
    }
}
