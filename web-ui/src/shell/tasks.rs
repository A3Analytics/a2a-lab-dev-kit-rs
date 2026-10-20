//! Task-definition reader. The sidebar lists definitions; the main column shows the form and a closed Details disclosure.

use std::cell::RefCell;

use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::HtmlElement;

use crate::inventory::{Fault, Item, Primitive};
use crate::shell::session::{self, calls_allowed, message_id, take_secret};
use crate::tasks::{self as schema, Field, Form, Kind, Run, Tracked};

#[derive(Clone, PartialEq)]
pub struct TaskUi {
    pub selected: String,
    pub form: Option<Form>,
    pub runs: Vec<Tracked>,
    pub status: String,
    pub alert: String,
    pub tick: u32,
}

impl TaskUi {
    pub fn new() -> Self {
        Self {
            selected: String::new(),
            form: None,
            runs: Vec::new(),
            status: "Choose a task.".to_owned(),
            alert: String::new(),
            tick: 0,
        }
    }
}

#[derive(Clone, PartialEq)]
enum TaskJob {
    Idle,
    Start {
        tick: u32,
        task_id: String,
        input: serde_json::Value,
    },
}

#[derive(Clone, PartialEq)]
enum TaskOutcome {
    Idle,
    Started {
        tick: u32,
        run: Run,
    },
    Failed {
        tick: u32,
        start: bool,
        message: String,
    },
}

thread_local! {
    static UI: RefCell<Option<Signal<TaskUi>>> = const { RefCell::new(None) };
    static JOB: RefCell<Option<Signal<TaskJob>>> = const { RefCell::new(None) };
    static WAVE: RefCell<Option<Signal<u32>>> = const { RefCell::new(None) };
    static BEAT: RefCell<Option<Signal<u32>>> = const { RefCell::new(None) };
    static SNAPSHOTS: RefCell<Vec<Tracked>> = const { RefCell::new(Vec::new()) };
    static WATCHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn bind(ui: Signal<TaskUi>) {
    UI.with(|slot| *slot.borrow_mut() = Some(ui));
}

pub fn catalog_changed() {
    let items = super::inventory::loaded(Primitive::Tasks);
    let Some(mut ui) = bound() else {
        return;
    };
    let previous = ui.peek().selected.clone();
    let next = if items.iter().any(|item| item.id == previous) {
        previous
    } else {
        items
            .first()
            .map(|item| item.id.clone())
            .unwrap_or_default()
    };
    let mut state = ui.peek().clone();
    state.selected = next.clone();
    if next.is_empty() {
        state.form = Some(schema::form(None, None));
        let snap = super::inventory::snapshot(Primitive::Tasks);
        state.status = snap.status(Primitive::Tasks);
        state.alert = snap.alert;
        ui.set(state);
        return;
    }
    if let Some(item) = items.into_iter().find(|item| item.id == next) {
        state.form = Some(schema::form_for(&item, session::native_secret().as_deref()));
        state.status.clear();
        state.alert.clear();
    }
    ui.set(state);
}

fn bound() -> Option<Signal<TaskUi>> {
    UI.with(|slot| *slot.borrow())
}

#[component]
pub fn TaskView(ui: Signal<TaskUi>) -> Element {
    let job = use_signal(|| TaskJob::Idle);
    let wave = use_signal(|| 0u32);
    let mut beat = use_signal(|| 0u32);
    use_hook(|| {
        JOB.with(|slot| *slot.borrow_mut() = Some(job));
        WAVE.with(|slot| *slot.borrow_mut() = Some(wave));
        BEAT.with(|slot| *slot.borrow_mut() = Some(beat));
    });
    let reply = use_resource(move || {
        let n = wave();
        let job = job();
        let secret = session::native_secret();
        async move { (n, run_job(job, secret).await) }
    });
    let pulse = use_resource(move || {
        let n = beat();
        async move {
            if n > 0 {
                pause_ms(50).await;
            }
            n
        }
    });
    let mut applied = use_signal(|| 0u32);
    use_effect(move || {
        let form = ui.read().form.clone();
        if let Some(form) = form {
            prime_form(&form);
        }
    });
    let _ = pulse.read();
    if let Some(n) = pulse()
        && n == beat()
        && n > 0
    {
        sync_snapshots(ui);
        if watching() {
            beat.set(n.wrapping_add(1));
        }
    }
    let _ = reply.read();
    if let Some((n, outcome)) = reply()
        && !matches!(outcome, TaskOutcome::Idle)
    {
        apply_outcome(ui, &mut applied, n, outcome);
    }
    let state = ui();
    let items = super::inventory::loaded(Primitive::Tasks);
    let snap = super::inventory::snapshot(Primitive::Tasks);
    let selected = items.iter().find(|item| item.id == state.selected);
    let busy = snap.kind == crate::inventory::Kind::Loading;
    rsx! {
        section {
            id: "task-view",
            aria_labelledby: "task-view-heading",
            aria_busy: if busy { "true" } else { "false" },
            h2 { id: "task-view-heading", class: "visually-hidden", tabindex: "-1", "Tasks" }
            p {
                id: "task-view-status",
                role: "status",
                aria_live: "polite",
                "data-gen": state.tick.to_string(),
                "{state.status}"
            }
            div { id: "task-view-alert", role: "alert", aria_live: "assertive", "{state.alert}" }
            div { id: "task-browser", class: "log-browser",
                div { id: "task-filters", role: "group", aria_label: "Task filters",
                    div { class: "log-filter-group",
                        p { "Task" }
                        div {
                            id: "task-sources",
                            role: "radiogroup",
                            aria_label: "Task definitions",
                            onkeydown: pick_key,
                            if items.is_empty() {
                                button {
                                    id: "task-pick-0",
                                    r#type: "button",
                                    class: "task-source",
                                    role: "radio",
                                    tabindex: "0",
                                    aria_checked: "false",
                                    aria_disabled: "true",
                                    "None loaded"
                                }
                            } else {
                                for (index, item) in items.iter().enumerate() {
                                    TaskPick { index, item: item.clone(), selected: state.selected.clone() }
                                }
                            }
                        }
                    }
                }
                div { id: "task-results", class: "log-results",
                    div { class: "log-scroll",
                        if let Some(item) = selected {
                            header { class: "task-heading",
                                h3 { id: "task-detail-name", "{item.name}" }
                                p { id: "task-detail-description", "{item.description}" }
                            }
                        }
                        form {
                            id: "task-form",
                            class: "task-form",
                            aria_label: "Task input",
                            novalidate: true,
                            onsubmit: move |event| {
                                event.prevent_default();
                                start_selected();
                            },
                            if let Some(form) = state.form.as_ref() {
                                if form.needs_input() {
                                    TaskField { field: form.root.clone() }
                                } else {
                                    p { id: "task-form-empty", "This task has no input fields." }
                                }
                            }
                            if state.form.is_some() {
                                button { id: "task-start", r#type: "submit", "Start" }
                            }
                        }
                        if let Some(item) = selected {
                            details { id: "task-details",
                                summary { id: "task-details-summary", "Details" }
                                dl { id: "task-detail", class: "task-detail",
                                    TaskDetail { item: item.clone() }
                                }
                            }
                        }
                        ul { id: "task-runs", class: "log-cards", aria_label: "Task runs",
                            for tracked in state.runs.iter() {
                                RunCard { tracked: tracked.clone() }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TaskPick(index: usize, item: Item, selected: String) -> Element {
    let id = item.id.clone();
    let name = if item.name.is_empty() {
        item.id.clone()
    } else {
        item.name.clone()
    };
    let detail = source_detail(&item);
    let checked = item.id == selected;
    rsx! {
        button {
            id: "task-pick-{index}",
            r#type: "button",
            class: "task-source",
            role: "radio",
            tabindex: radio_tab(checked),
            "data-task": "{item.id}",
            aria_checked: if checked { "true" } else { "false" },
            onclick: move |_| choose(&id),
            span { class: "task-source-name", "{name}" }
            span { class: "task-source-detail", "{detail}" }
        }
    }
}

#[component]
fn TaskDetail(item: Item) -> Element {
    rsx! {
        dt { "Id" } dd { id: "task-detail-id", "{item.id}" }
        dt { "Asset id" } dd { id: "task-detail-asset-id", "{named(&item.details, \"Asset id\")}" }
        dt { "Semantic id" } dd { id: "task-detail-semantic-id", "{named(&item.details, \"Semantic id\")}" }
        dt { "Input schema" } dd { id: "task-detail-input-schema", "{named(&item.details, \"Input schema\")}" }
        dt { "Output schema" } dd { id: "task-detail-output-schema", "{named(&item.details, \"Output schema\")}" }
    }
}

#[component]
fn TaskField(field: Field) -> Element {
    match field.kind {
        Kind::Object => rsx! {
            fieldset { class: "task-field-group",
                legend { "{field.label}" }
                FieldHelp { field: field.clone() }
                for child in field.children.iter() {
                    TaskField { field: child.clone() }
                }
            }
        },
        Kind::Array => rsx! {
            fieldset { class: "task-field-group",
                legend { "{field.label}" }
                FieldHelp { field: field.clone() }
                for child in field.children.iter() {
                    div { class: "task-array-item",
                        TaskField { field: child.clone() }
                        button {
                            r#type: "button",
                            "data-task-remove": "{child.path}",
                            aria_label: "Remove {child.label}",
                            onclick: {
                                let path = child.path.clone();
                                move |_| remove_item(&path)
                            },
                            "Remove"
                        }
                    }
                }
                button {
                    r#type: "button",
                    "data-task-add": "{field.path}",
                    aria_label: "Add {field.label}",
                    onclick: {
                        let path = field.path.clone();
                        move |_| add_item(&path)
                    },
                    "Add item"
                }
            }
        },
        Kind::Enum => rsx! {
            div { class: "task-field",
                label { r#for: field.id(), "{field.label}" }
                FieldHelp { field: field.clone() }
                select {
                    id: field.id(),
                    "data-task-path": "{field.path}",
                    required: required_attr(&field),
                    aria_required: required_aria(&field),
                    aria_invalid: invalid_aria(&field),
                    aria_describedby: described(&field),
                    onchange: {
                        let path = field.path.clone();
                        move |_| commit_path(&path)
                    },
                    option { value: "", "Choose" }
                    for option in field.options.iter() {
                        option { value: option_label(option), "{option_label(option)}" }
                    }
                }
                FieldError { field: field.clone() }
            }
        },
        Kind::Json => rsx! {
            div { class: "task-field",
                label { r#for: field.id(), "{field.label}" }
                FieldHelp { field: field.clone() }
                textarea {
                    id: field.id(),
                    "data-task-path": "{field.path}",
                    rows: "6",
                    required: required_attr(&field),
                    aria_required: required_aria(&field),
                    aria_invalid: invalid_aria(&field),
                    aria_describedby: described(&field),
                    oninput: {
                        let path = field.path.clone();
                        move |_| commit_path(&path)
                    },
                }
                FieldError { field: field.clone() }
            }
        },
        Kind::Boolean => rsx! {
            div { class: "task-field task-field-check",
                input {
                    id: field.id(),
                    "data-task-path": "{field.path}",
                    r#type: "checkbox",
                    aria_invalid: invalid_aria(&field),
                    aria_describedby: described(&field),
                    onchange: {
                        let path = field.path.clone();
                        move |_| commit_path(&path)
                    },
                }
                label { r#for: field.id(), "{field.label}" }
                FieldHelp { field: field.clone() }
                FieldError { field: field.clone() }
            }
        },
        _ => rsx! {
            div { class: "task-field",
                label { r#for: field.id(), "{field.label}" }
                FieldHelp { field: field.clone() }
                input {
                    id: field.id(),
                    "data-task-path": "{field.path}",
                    r#type: field.kind.input_type().unwrap_or("text"),
                    step: step_attr(&field),
                    required: required_attr(&field),
                    aria_required: required_aria(&field),
                    aria_invalid: invalid_aria(&field),
                    aria_describedby: described(&field),
                    min: field.minimum.clone(),
                    max: field.maximum.clone(),
                    minlength: length_attr(field.min_length),
                    maxlength: length_attr(field.max_length),
                    pattern: field.pattern.clone(),
                    "data-exclusive-minimum": field.exclusive_minimum.clone().unwrap_or_default(),
                    "data-exclusive-maximum": field.exclusive_maximum.clone().unwrap_or_default(),
                    oninput: {
                        let path = field.path.clone();
                        move |_| commit_path(&path)
                    },
                    onchange: {
                        let path = field.path.clone();
                        move |_| commit_path(&path)
                    },
                }
                FieldError { field: field.clone() }
            }
        },
    }
}

#[component]
fn FieldHelp(field: Field) -> Element {
    if field.description.is_empty() {
        return rsx! {};
    }
    rsx! { p { id: field.help_id(), class: "task-field-help", "{field.description}" } }
}

#[component]
fn FieldError(field: Field) -> Element {
    let Some(message) = field.error.clone() else {
        return rsx! {};
    };
    rsx! { p { id: field.error_id(), class: "task-field-error", "{message}" } }
}

#[component]
fn RunCard(tracked: Tracked) -> Element {
    rsx! {
        li {
            div {
                class: "log-card",
                "data-run": "{tracked.run.id}",
                "data-gen": tracked.poll.to_string(),
                div { class: "log-card-identity",
                    span { class: "log-card-time", "{tracked.run.id}" }
                    p { class: "log-card-title", "{tracked.run.task_id}" }
                }
                div { class: "log-card-tags",
                    Meta { label: "State", value: tracked.run.state.clone() }
                    if !tracked.run.progress.is_empty() {
                        Meta { label: "Progress", value: tracked.run.progress.clone() }
                    }
                    if !tracked.run.message.is_empty() {
                        Meta { label: "Message", value: tracked.run.message.clone() }
                    }
                    if !tracked.run.result.is_empty() {
                        Meta { label: "Result", value: tracked.run.result.clone() }
                    }
                    if !tracked.run.error_kind.is_empty() {
                        Meta { label: "Error kind", value: tracked.run.error_kind.clone() }
                    }
                    if !tracked.run.error_identifier.is_empty() {
                        Meta { label: "Error identifier", value: tracked.run.error_identifier.clone() }
                    }
                }
            }
        }
    }
}

#[component]
fn Meta(label: String, value: String) -> Element {
    rsx! {
        span { class: "log-meta",
            span { class: "log-meta-label", "{label}:" }
            span { "{value}" }
        }
    }
}

fn choose(id: &str) {
    let Some(mut ui) = bound() else {
        return;
    };
    if ui.peek().selected == id {
        return;
    }
    let items = super::inventory::loaded(Primitive::Tasks);
    let Some(item) = items.into_iter().find(|item| item.id == id) else {
        return;
    };
    let mut next = ui.peek().clone();
    next.selected = id.to_owned();
    next.form = Some(schema::form_for(&item, session::native_secret().as_deref()));
    next.status.clear();
    next.alert.clear();
    ui.set(next);
}

fn pick_key(event: KeyboardEvent) {
    let delta: isize = match event.key() {
        Key::ArrowDown | Key::ArrowRight => 1,
        Key::ArrowUp | Key::ArrowLeft => -1,
        _ => return,
    };
    event.prevent_default();
    let items = super::inventory::loaded(Primitive::Tasks);
    if items.is_empty() {
        return;
    }
    let Some(ui) = bound() else {
        return;
    };
    let current = ui.peek().selected.clone();
    let index = items
        .iter()
        .position(|item| item.id == current)
        .unwrap_or(0);
    let next = (index as isize + delta).rem_euclid(items.len() as isize) as usize;
    let id = items[next].id.clone();
    choose(&id);
    focus_control(&format!("task-pick-{next}"));
}

fn bump_wave() {
    WAVE.with(|slot| {
        if let Some(mut wave) = *slot.borrow() {
            wave.set(wave().wrapping_add(1));
        }
    });
}

fn start_selected() {
    let Some(mut ui) = bound() else {
        return;
    };
    let Some(job) = JOB.with(|slot| *slot.borrow()) else {
        return;
    };
    let task_id = ui.peek().selected.clone();
    if task_id.is_empty() {
        return;
    }
    let mut state = ui();
    let input = state.form.as_mut().and_then(Form::submit);
    let Some(input) = input else {
        ui.set(state);
        return;
    };
    ui.set(state);
    begin_start(&task_id, input, ui, job);
}

fn begin_start(
    task_id: &str,
    input: serde_json::Value,
    mut ui: Signal<TaskUi>,
    mut job: Signal<TaskJob>,
) {
    if !calls_allowed() {
        return;
    }
    let rendered = input.to_string();
    if take_secret(&[task_id, rendered.as_str()]).is_err() {
        return;
    }
    let mut next = ui();
    next.tick = next.tick.wrapping_add(1);
    next.status = "Starting.".to_owned();
    next.alert.clear();
    let tick = next.tick;
    ui.set(next);
    set_watching(true);
    bump_wave();
    bump_beat();
    job.set(TaskJob::Start {
        tick,
        task_id: task_id.to_owned(),
        input,
    });
}

fn bump_beat() {
    BEAT.with(|slot| {
        if let Some(mut beat) = *slot.borrow() {
            beat.set(beat().wrapping_add(1).max(1));
        }
    });
}

fn set_watching(on: bool) {
    WATCHING.with(|slot| slot.set(on));
}

fn watching() -> bool {
    WATCHING.with(std::cell::Cell::get)
}

async fn poll_live_runs(secret: Option<&str>) -> Result<(), String> {
    loop {
        let live = SNAPSHOTS.with(|slot| {
            slot.borrow()
                .iter()
                .filter(|item| !item.run.terminal())
                .map(|item| item.run.id.clone())
                .collect::<Vec<_>>()
        });
        if live.is_empty() {
            return Ok(());
        }
        pause().await;
        for id in live {
            let run = send_command(&schema::status_command(&id), secret).await?;
            let seq = SNAPSHOTS.with(|slot| schema::next_seq(&slot.borrow(), &id));
            remember_run(run, seq);
        }
    }
}

fn remember_run(run: Run, seq: u32) {
    SNAPSHOTS.with(|slot| {
        let _ = schema::apply(&mut slot.borrow_mut(), run, seq);
    });
}

fn sync_snapshots(mut ui: Signal<TaskUi>) {
    let snapshots = SNAPSHOTS.with(|slot| slot.borrow().clone());
    if snapshots.is_empty() {
        return;
    }
    let mut next = ui.peek().clone();
    if next.runs == snapshots {
        return;
    }
    next.runs = snapshots;
    next.status.clear();
    ui.set(next);
}

fn apply_outcome(
    mut ui: Signal<TaskUi>,
    applied: &mut Signal<u32>,
    wave: u32,
    outcome: TaskOutcome,
) {
    match outcome {
        TaskOutcome::Idle => {}
        TaskOutcome::Started { tick, run: _ } => {
            if wave <= applied() {
                return;
            }
            applied.set(wave);
            let mut next = ui();
            if next.tick != tick {
                return;
            }
            next.status.clear();
            next.alert.clear();
            ui.set(next);
            sync_snapshots(ui);
        }
        TaskOutcome::Failed {
            tick,
            start,
            message,
        } => {
            if wave <= applied() {
                return;
            }
            applied.set(wave);
            let mut next = ui();
            if next.tick != tick {
                return;
            }
            next.status = if start {
                "The task could not be started.".to_owned()
            } else {
                "The task run could not be read.".to_owned()
            };
            next.alert = message;
            ui.set(next);
        }
    }
}

async fn run_job(job: TaskJob, secret: Option<String>) -> TaskOutcome {
    match job {
        TaskJob::Idle => TaskOutcome::Idle,
        TaskJob::Start {
            tick,
            task_id,
            input,
        } => {
            let command = schema::start_command(&task_id, &input);
            match send_command(&command, secret.as_deref()).await {
                Ok(run) => {
                    remember_run(run.clone(), 1);
                    if let Err(message) = poll_live_runs(secret.as_deref()).await {
                        set_watching(false);
                        return TaskOutcome::Failed {
                            tick,
                            start: false,
                            message,
                        };
                    }
                    set_watching(false);
                    TaskOutcome::Started { tick, run }
                }
                Err(message) => {
                    set_watching(false);
                    TaskOutcome::Failed {
                        tick,
                        start: true,
                        message,
                    }
                }
            }
        }
    }
}

async fn send_command(command: &serde_json::Value, secret: Option<&str>) -> Result<Run, String> {
    let Some(transport) = session::transport() else {
        return Err("The browser could not reach this origin.".to_owned());
    };
    let Ok(body) = crate::codec::command_message(&message_id(), command, "") else {
        return Err("The task start could not be built.".to_owned());
    };
    match transport.send(&body, secret).await {
        Err(()) => Err("The browser could not reach this origin.".to_owned()),
        Ok(reply) => schema::interpret(&reply, secret).map_err(fault_message),
    }
}

async fn pause() {
    pause_ms(250).await;
}

async fn pause_ms(ms: i32) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = JsFuture::from(promise).await;
}

fn commit_path(path: &str) {
    let Some(mut ui) = bound() else {
        return;
    };
    let value = control_at(path);
    let mut next = ui();
    if let Some(form) = next.form.as_mut() {
        form.set(path, &value);
    }
    ui.set(next);
}

fn add_item(path: &str) {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui();
    if let Some(form) = next.form.as_mut() {
        form.add(path);
    }
    ui.set(next);
}

fn remove_item(path: &str) {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui();
    if let Some(form) = next.form.as_mut() {
        form.remove(path);
    }
    ui.set(next);
}

fn prime_form(form: &Form) {
    prime_field(&form.root);
}

fn prime_field(field: &Field) {
    match field.kind {
        Kind::Object | Kind::Array => {
            for child in &field.children {
                prime_field(child);
            }
        }
        _ => write_field(field),
    }
}

fn write_field(field: &Field) {
    let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(&field.id()))
    else {
        return;
    };
    if let Ok(input) = element.clone().dyn_into::<web_sys::HtmlInputElement>() {
        if input.type_() == "checkbox" {
            input.set_checked(field.raw == "true");
        } else {
            input.set_value(&field.raw);
        }
        return;
    }
    if let Ok(select) = element.clone().dyn_into::<web_sys::HtmlSelectElement>() {
        select.set_value(&field.raw);
        return;
    }
    if let Ok(area) = element.dyn_into::<web_sys::HtmlTextAreaElement>() {
        area.set_value(&field.raw);
    }
}

fn control_at(path: &str) -> String {
    let field_id = if path.is_empty() {
        "task-input".to_owned()
    } else {
        format!("task-input-{}", path.replace('.', "-"))
    };
    let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(&field_id))
    else {
        return String::new();
    };
    if let Ok(input) = element.clone().dyn_into::<web_sys::HtmlInputElement>() {
        if input.type_() == "checkbox" {
            return if input.checked() {
                "true".to_owned()
            } else {
                "false".to_owned()
            };
        }
        return input.value();
    }
    if let Ok(select) = element.clone().dyn_into::<web_sys::HtmlSelectElement>() {
        return select.value();
    }
    element
        .dyn_into::<web_sys::HtmlTextAreaElement>()
        .map(|area| area.value())
        .unwrap_or_default()
}

fn focus_control(id: &str) {
    if let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
        && let Ok(html) = element.dyn_into::<HtmlElement>()
    {
        let _ = html.focus();
    }
}

fn source_detail(item: &Item) -> String {
    if item.description.is_empty() {
        item.id.clone()
    } else {
        format!("{} · {}", item.id, item.description)
    }
}

fn named(details: &[(&'static str, String)], label: &str) -> String {
    details
        .iter()
        .find(|(name, _)| *name == label)
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

fn option_label(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::Bool(flag) => flag.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn required_attr(field: &Field) -> bool {
    schema::attrs(field)
        .iter()
        .any(|(name, _)| *name == "required")
}

fn required_aria(field: &Field) -> Option<&'static str> {
    required_attr(field).then_some("true")
}

fn invalid_aria(field: &Field) -> Option<&'static str> {
    field.error.is_some().then_some("true")
}

fn described(field: &Field) -> String {
    let mut ids = Vec::new();
    if !field.description.is_empty() {
        ids.push(field.help_id());
    }
    if field.error.is_some() {
        ids.push(field.error_id());
    }
    ids.join(" ")
}

fn radio_tab(checked: bool) -> &'static str {
    if checked { "0" } else { "-1" }
}

fn length_attr(value: Option<u64>) -> Option<String> {
    value.map(|value| value.to_string())
}

fn step_attr(field: &Field) -> String {
    schema::attrs(field)
        .into_iter()
        .find(|(name, _)| *name == "step")
        .map(|(_, value)| value)
        .unwrap_or_default()
}

fn fault_message(fault: Fault) -> String {
    match fault {
        Fault::Unavailable(message) => message,
        Fault::Unauthorized(message) | Fault::Forbidden(message) => message,
        Fault::Malformed => "The task result could not be read.".to_owned(),
        Fault::Transport => "The browser could not reach this origin.".to_owned(),
    }
}
