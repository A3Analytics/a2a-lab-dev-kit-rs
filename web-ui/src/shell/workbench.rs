//! Command workbench. History and the bearer token stay in page memory.

use std::cell::RefCell;

use dioxus::prelude::*;

use crate::inventory::{Item, Primitive};
use crate::shell::session::{self, calls_allowed, message_id, take_secret};
use crate::workbench::report::{self as outcome, ImagePreview, Report};
use crate::workbench::tree::Node;
use crate::workbench::{self, HISTORY_LIMIT, HistoryItem, Operation};

#[derive(Clone, PartialEq)]
pub struct WorkbenchUi {
    pub scoped: Option<Primitive>,
    pub operations: Vec<Operation>,
    pub targets: Vec<Item>,
    pub target_label: String,
    pub target_hidden: bool,
    pub status: String,
    pub kind: String,
    pub alert: String,
    pub sending: bool,
    pub facts: Vec<Fact>,
    pub tree: Node,
    pub image: Option<ImagePreview>,
    pub history: Vec<HistoryItem>,
    pub operation: String,
    pub tick: u32,
}

#[derive(Clone, PartialEq)]
pub struct Fact {
    pub name: String,
    pub label: String,
    pub value: String,
}

impl WorkbenchUi {
    pub fn new() -> Self {
        Self {
            scoped: None,
            operations: Vec::new(),
            targets: Vec::new(),
            target_label: "Choice".to_owned(),
            target_hidden: true,
            status: String::new(),
            kind: String::new(),
            alert: String::new(),
            sending: false,
            facts: Vec::new(),
            tree: Node::Text(String::new()),
            image: None,
            history: Vec::new(),
            operation: String::new(),
            tick: 0,
        }
    }
}

#[derive(Clone, PartialEq)]
enum WorkJob {
    Idle,
    Send {
        restored: bool,
        tick: u32,
        operation: Operation,
        params: serde_json::Value,
        body: String,
    },
}

#[derive(Clone, PartialEq)]
enum WorkOutcome {
    Idle,
    Done {
        tick: u32,
        restored: bool,
        operation: Operation,
        params: serde_json::Value,
        report: Box<Report>,
    },
}

thread_local! {
    static UI: RefCell<Option<Signal<WorkbenchUi>>> = const { RefCell::new(None) };
}

pub fn bind(ui: Signal<WorkbenchUi>) {
    UI.with(|slot| *slot.borrow_mut() = Some(ui));
}

pub(super) fn open(name: &str, identifier: &str) {
    let Some(operation) = Operation::parse(name) else {
        return;
    };
    session::write_control("workbench-operation", operation.name());
    write_params(&operation.prefill(identifier));
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    next.alert.clear();
    next.operation = operation.name().to_owned();
    sync_targets(&mut next);
    if ui.peek().clone() != next {
        ui.set(next);
    }
}

pub(super) fn scope(primitive: Primitive) {
    let Some(mut ui) = bound() else {
        return;
    };
    let commands = workbench::commands(primitive);
    let current = session::control_value("workbench-operation");
    let kept = commands.iter().any(|operation| operation.name() == current);
    let mut next = ui.peek().clone();
    next.scoped = Some(primitive);
    next.operations = commands.to_vec();
    if !kept && let Some(operation) = commands.first() {
        session::write_control("workbench-operation", operation.name());
        write_params(&operation.template());
        next.operation = operation.name().to_owned();
    } else if kept {
        next.operation = current;
    }
    sync_targets(&mut next);
    if ui.peek().clone() != next {
        ui.set(next);
    }
}

pub(super) fn note_loaded(primitive: Primitive) {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if next.scoped != Some(primitive) {
        return;
    }
    sync_targets(&mut next);
    if ui.peek().clone() != next {
        ui.set(next);
    }
}

fn bound() -> Option<Signal<WorkbenchUi>> {
    UI.with(|slot| *slot.borrow())
}

#[component]
pub fn WorkbenchView(ui: Signal<WorkbenchUi>, hidden: bool) -> Element {
    let job = use_signal(|| WorkJob::Idle);
    let reply = use_resource(move || {
        let job = job();
        let secret = session::native_secret();
        async move { run_job(job, secret).await }
    });
    let mut applied = use_signal(|| 0u32);
    let _ = reply.read();
    if let Some(WorkOutcome::Done {
        tick,
        restored,
        operation,
        params,
        report,
    }) = reply()
        && tick > applied()
    {
        applied.set(tick);
        apply_report(ui, tick, restored, operation, params, *report);
    }
    let state = ui();
    let busy = state.sending;
    let busy_flag = if busy { "true" } else { "false" };
    rsx! {
        section {
            id: "workbench",
            class: "workbench",
            aria_labelledby: "workbench-heading",
            aria_busy: "{busy_flag}",
            "data-kind": "{state.kind}",
            hidden,
            h2 { id: "workbench-heading", tabindex: "-1", "Command" }
            p {
                id: "workbench-selected",
                class: "workbench-selected",
                "data-operation": "{state.operation}",
                hidden: state.operation.is_empty(),
                "{selected_line(&state.operation)}"
            }
            p { id: "workbench-note", "{workbench::preview_note()}" }
            p {
                id: "workbench-status",
                role: "status",
                aria_live: "polite",
                "data-state": "{state.kind}",
                "data-gen": state.tick.to_string(),
                "{state.status}"
            }
            div { id: "workbench-alert", role: "alert", aria_live: "assertive", "{state.alert}" }
            div { class: "command-form",
                div { class: "command-field",
                    label { r#for: "workbench-operation", "Operation" }
                    select {
                        id: "workbench-operation",
                        onchange: move |_| on_operation(ui),
                        for operation in state.operations.iter() {
                            option {
                                value: operation.name(),
                                selected: operation.name() == state.operation,
                                "{operation.label()}"
                            }
                        }
                    }
                }
                div { id: "workbench-target-field", class: "command-field", hidden: state.target_hidden,
                    label { id: "workbench-target-label", r#for: "workbench-target", "{state.target_label}" }
                    select {
                        id: "workbench-target",
                        onchange: move |_| on_target(),
                        if state.targets.is_empty() {
                            option { value: "", "None loaded" }
                        }
                        for item in state.targets.iter() {
                            option { value: "{item.id}", "{target_text(item)}" }
                        }
                    }
                }
                div { class: "command-field command-params",
                    label { r#for: "workbench-params", "Parameters" }
                    textarea {
                        id: "workbench-params",
                        rows: "4",
                        autocomplete: "off",
                        spellcheck: false,
                    }
                }
                div { class: "actions",
                    button {
                        id: "workbench-send",
                        r#type: "button",
                        disabled: busy,
                        aria_disabled: "{busy_flag}",
                        onclick: move |_| begin_send(ui, job, false),
                        "Send command"
                    }
                }
            }
            dl { id: "workbench-facts",
                for fact in state.facts.iter() {
                    div {
                        dt { "{fact.label}" }
                        dd { "data-fact": "{fact.name}", "{fact.value}" }
                    }
                }
            }
            div { id: "workbench-tree", class: "workbench-tree",
                TreeView { node: state.tree.clone() }
            }
            div { id: "workbench-image",
                if let Some(image) = state.image.as_ref() {
                    ImageSlot { image: image.clone() }
                }
            }
            ul { id: "workbench-history", class: "workbench-history", aria_label: "Command history",
                for (index, item) in state.history.iter().enumerate() {
                    li {
                        p { "{item.summary}" }
                        button {
                            r#type: "button",
                            "data-retry": index.to_string(),
                            onclick: move |_| retry(ui, job, index),
                            "Retry"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TreeView(node: Node) -> Element {
    match node {
        Node::Text(text) => rsx! { span { "{text}" } },
        Node::Group {
            name,
            children,
            collapsed,
        } => {
            if collapsed {
                rsx! {
                    details {
                        summary { "{name}" }
                        for child in children {
                            TreeView { node: child }
                        }
                    }
                }
            } else {
                rsx! {
                    details { open: true,
                        summary { "{name}" }
                        for child in children {
                            TreeView { node: child }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ImageSlot(image: ImagePreview) -> Element {
    rsx! {
        p { "Media type {image.media_type}" }
        p { "Encoded characters {image.encoded_chars}" }
        if let Some(size) = image.decoded_bytes {
            p { "Decoded bytes {size}" }
        }
        if !image.withheld.is_empty() {
            p { "Preview {image.withheld}" }
        }
        if let Some(url) = image.preview_url.as_deref() {
            img { id: "workbench-preview", alt: "Image preview", src: "{url}" }
        }
    }
}

fn begin_send(mut ui: Signal<WorkbenchUi>, mut job: Signal<WorkJob>, restored: bool) {
    if ui.peek().sending || !calls_allowed() {
        return;
    }
    let raw = session::control_value("workbench-params");
    let chosen = session::control_value("workbench-operation");
    let Some(operation) = Operation::parse(&chosen) else {
        let mut next = ui();
        next.alert = "Choose a lab command.".to_owned();
        ui.set(next);
        return;
    };
    let params = match workbench::parse_params(&raw) {
        Ok(params) => params,
        Err(fault) => {
            let mut next = ui();
            next.operation = operation.name().to_owned();
            next.alert = fault.message().to_owned();
            ui.set(next);
            return;
        }
    };
    if !restored {
        write_params(&params);
    }
    if take_secret(&[raw.as_str()]).is_err() {
        return;
    }
    let Ok(body) = workbench::envelope(&message_id(), operation, &params) else {
        let mut next = ui();
        next.alert = workbench::EditorFault::NotObject.message().to_owned();
        ui.set(next);
        return;
    };
    let mut next = ui();
    next.tick = next.tick.wrapping_add(1);
    next.sending = true;
    next.operation = operation.name().to_owned();
    next.alert.clear();
    next.status = format!("Sending {}.", operation.name());
    let tick = next.tick;
    ui.set(next);
    job.set(WorkJob::Send {
        restored,
        tick,
        operation,
        params,
        body,
    });
}

fn retry(ui: Signal<WorkbenchUi>, job: Signal<WorkJob>, index: usize) {
    let Some(item) = ui.peek().history.get(index).cloned() else {
        return;
    };
    session::write_control("workbench-operation", item.operation.name());
    write_params(&item.params);
    begin_send(ui, job, true);
}

fn on_operation(mut ui: Signal<WorkbenchUi>) {
    let Some(operation) = Operation::parse(&session::control_value("workbench-operation")) else {
        return;
    };
    write_params(&operation.template());
    let mut next = ui.peek().clone();
    next.operation = operation.name().to_owned();
    sync_targets(&mut next);
    if ui.peek().clone() != next {
        ui.set(next);
    }
}

fn on_target() {
    let Some(operation) = Operation::parse(&session::control_value("workbench-operation")) else {
        return;
    };
    let Some(field) = operation.target_field() else {
        return;
    };
    let id = session::control_value("workbench-target");
    if !id.is_empty() {
        write_param(field, &id);
    }
}

async fn run_job(job: WorkJob, secret: Option<String>) -> WorkOutcome {
    let WorkJob::Send {
        restored,
        tick,
        operation,
        params,
        body,
    } = job
    else {
        return WorkOutcome::Idle;
    };
    let Some(transport) = session::transport() else {
        return WorkOutcome::Done {
            tick,
            restored,
            operation,
            params,
            report: Box::new(outcome::transport_report(operation)),
        };
    };
    let report = match transport.send(&body, secret.as_deref()).await {
        Err(()) => outcome::transport_report(operation),
        Ok(reply) => outcome::interpret(operation, &reply, secret.as_deref()),
    };
    WorkOutcome::Done {
        tick,
        restored,
        operation,
        params,
        report: Box::new(report),
    }
}

fn apply_report(
    mut ui: Signal<WorkbenchUi>,
    tick: u32,
    _restored: bool,
    operation: Operation,
    params: serde_json::Value,
    report: Report,
) {
    let mut next = ui();
    if next.tick != tick {
        return;
    }
    next.sending = false;
    next.kind = report.kind.slug().to_owned();
    next.status = report.kind.label().to_owned();
    next.alert = result_alert(&report);
    next.facts = facts_from(&report);
    next.tree = report.tree.clone();
    next.image = report.image.clone();
    let item = outcome::history_item(
        operation,
        &params,
        &report,
        session::native_secret().as_deref(),
    );
    next.history.push(item);
    let extra = next.history.len().saturating_sub(HISTORY_LIMIT);
    if extra > 0 {
        next.history.drain(0..extra);
    }
    ui.set(next);
}

fn selected_line(operation: &str) -> String {
    Operation::parse(operation).map_or_else(String::new, |operation| {
        format!("{} ({})", operation.label(), operation.name())
    })
}

fn result_alert(report: &Report) -> String {
    if report.kind == outcome::Kind::Success {
        return String::new();
    }
    if report.message.is_empty() {
        return report.kind.label().to_owned();
    }
    report.message.clone()
}

fn sync_targets(ui: &mut WorkbenchUi) {
    let Some(operation) = Operation::parse(&session::control_value("workbench-operation")) else {
        ui.target_hidden = true;
        return;
    };
    let Some(field) = operation.target_field() else {
        ui.target_hidden = true;
        return;
    };
    let Some(primitive) = ui.scoped else {
        ui.target_hidden = true;
        return;
    };
    ui.target_hidden = false;
    ui.target_label = operation.target_label().unwrap_or("Choice").to_owned();
    let mut items = super::inventory::loaded(primitive);
    let chosen = param_string(field);
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
    ui.targets = items;
    if ui.targets.is_empty() {
        return;
    }
    let selected = if ui.targets.iter().any(|item| item.id == chosen) {
        chosen
    } else {
        ui.targets[0].id.clone()
    };
    session::write_control("workbench-target", &selected);
    if param_string(field).is_empty() {
        write_param(field, &selected);
    }
}

fn facts_from(report: &Report) -> Vec<Fact> {
    let mut facts = vec![
        fact("outcome", "Outcome", report.kind.label()),
        fact("operation", "Operation", &report.operation),
        fact("protocol-task", "Protocol task", &report.protocol_task_id),
        fact(
            "protocol-context",
            "Protocol context",
            &report.protocol_context_id,
        ),
        fact("protocol-state", "Protocol state", &report.protocol_state),
    ];
    optional(&mut facts, "lab-run", "Lab run", &report.lab_run_id);
    optional(
        &mut facts,
        "lab-run-state",
        "Lab run state",
        &report.lab_run_state,
    );
    optional(&mut facts, "status", "Status", &report.status);
    optional(&mut facts, "code", "Code", &report.code);
    optional(&mut facts, "message", "Message", &report.message);
    for line in &report.reasons {
        facts.push(fact("reason", "Reason", &line.reason));
        facts.push(fact("domain", "Domain", &line.domain));
    }
    for line in &report.fields {
        facts.push(fact("field", "Field", &line.field));
        facts.push(fact("detail", "Detail", &line.description));
    }
    for line in &report.correlations {
        facts.push(fact("correlation", line.label, &line.value));
    }
    facts
}

fn fact(name: &str, label: &str, value: &str) -> Fact {
    Fact {
        name: name.to_owned(),
        label: label.to_owned(),
        value: value.to_owned(),
    }
}

fn optional(facts: &mut Vec<Fact>, name: &str, label: &str, value: &str) {
    if !value.is_empty() {
        facts.push(fact(name, label, value));
    }
}

fn target_text(item: &Item) -> String {
    if item.name.is_empty() || item.name == item.id {
        item.id.clone()
    } else {
        format!("{} ({})", item.name, item.id)
    }
}

fn param_string(field: &str) -> String {
    let Ok(value) = workbench::parse_params(&session::control_value("workbench-params")) else {
        return String::new();
    };
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn write_param(field: &str, id: &str) {
    let Ok(mut value) = workbench::parse_params(&session::control_value("workbench-params")) else {
        return;
    };
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert(field.to_owned(), serde_json::Value::String(id.to_owned()));
    write_params(&value);
}

fn write_params(params: &serde_json::Value) {
    let text = serde_json::to_string_pretty(params).unwrap_or_else(|_| "{}".to_owned());
    session::write_control("workbench-params", &text);
}
