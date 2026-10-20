//! Log reader. Source toggles choose which sources to load.

use std::cell::RefCell;

use dioxus::prelude::*;
use wasm_bindgen::JsCast;

use crate::inventory::{Fault, Item, Primitive};
use crate::logs::{self, QueryRange, Record};
use crate::shell::session::{self, calls_allowed, message_id, take_secret};

const MESSAGE_FOLD: usize = 72;

#[derive(Clone, PartialEq)]
pub struct LogUi {
    pub sources: Vec<Item>,
    pub catalog: Vec<String>,
    pub selected: Vec<String>,
    pub records: Vec<Record>,
    pub query: String,
    pub levels: Vec<String>,
    pub range: QueryRange,
    pub alert: String,
    pub tick: u32,
    pub sending: bool,
    pub known: bool,
    pub faulted: bool,
}

impl LogUi {
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            catalog: Vec::new(),
            selected: Vec::new(),
            records: Vec::new(),
            query: String::new(),
            levels: Vec::new(),
            range: QueryRange::Window,
            alert: String::new(),
            tick: 0,
            sending: false,
            known: false,
            faulted: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Loading,
    NoSources,
    Error,
    Empty,
    NoMatch,
    Ready,
}

impl Mode {
    fn slug(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::NoSources => "sources",
            Self::Error => "error",
            Self::Empty => "empty",
            Self::NoMatch => "nomatch",
            Self::Ready => "ready",
        }
    }

    fn text(self) -> &'static str {
        match self {
            Self::Loading => "Loading logs.",
            Self::NoSources => "No log sources.",
            Self::Error => "Logs could not be read.",
            Self::Empty => "No logs for this source.",
            Self::NoMatch => "No logs match these filters.",
            Self::Ready => "",
        }
    }
}

#[derive(Clone, PartialEq)]
enum LogJob {
    Idle,
    Load {
        tick: u32,
        sources: Vec<String>,
        range: QueryRange,
    },
}

#[derive(Clone, PartialEq)]
enum LogOutcome {
    Idle,
    Ready { tick: u32, records: Vec<Record> },
    Failed { tick: u32, message: String },
}

#[derive(Clone, Copy)]
struct Loader {
    ui: Signal<LogUi>,
    job: Signal<LogJob>,
}

thread_local! {
    static UI: RefCell<Option<Signal<LogUi>>> = const { RefCell::new(None) };
    static LOADER: RefCell<Option<Loader>> = const { RefCell::new(None) };
}

pub fn bind(ui: Signal<LogUi>) {
    UI.with(|slot| *slot.borrow_mut() = Some(ui));
}

pub fn sources_changed() {
    let items = super::inventory::loaded(Primitive::Logs);
    let ids = items.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    let catalog_changed = next.catalog != ids;
    next.sources = items;
    next.catalog = ids;
    next.known = true;
    next.selected
        .retain(|id| next.catalog.iter().any(|source| source == id));
    if next.catalog.is_empty() {
        next.records.clear();
        next.query.clear();
        next.levels.clear();
        next.sending = false;
        next.faulted = false;
        next.alert.clear();
        session::write_control("log-filter-search", "");
        ui.set(next);
        return;
    }
    ui.set(next);
    if catalog_changed {
        start_load();
    }
}

fn bound() -> Option<Signal<LogUi>> {
    UI.with(|slot| *slot.borrow())
}

fn loader() -> Option<Loader> {
    LOADER.with(|slot| *slot.borrow())
}

#[component]
pub fn LogView(ui: Signal<LogUi>) -> Element {
    let job = use_signal(|| LogJob::Idle);
    use_hook(|| {
        LOADER.with(|slot| *slot.borrow_mut() = Some(Loader { ui, job }));
    });
    let reply = use_resource(move || {
        let job = job();
        let secret = session::native_secret();
        async move { run_job(job, secret).await }
    });
    let mut applied = use_signal(|| 0u32);
    let _ = reply.read();
    if let Some(outcome) = reply()
        && !matches!(outcome, LogOutcome::Idle)
    {
        apply_outcome(ui, &mut applied, outcome);
    }
    let state = ui();
    let shown = visible_records(&state);
    let mode = presentation(&state, shown.len());
    rsx! {
        section {
            id: "log-view",
            aria_label: "Logs",
            aria_busy: if state.sending { "true" } else { "false" },
            div { id: "log-view-alert", role: "alert", aria_live: "assertive", "{state.alert}" }
            div { id: "log-browser", class: "log-browser",
                FilterRail { ui, state: state.clone() }
                Results { state: state.clone(), shown, mode }
            }
        }
    }
}

#[component]
fn FilterRail(ui: Signal<LogUi>, state: LogUi) -> Element {
    rsx! {
        div { id: "log-filters", role: "group", aria_label: "Log filters",
            div { class: "log-filter-group",
                p { "Actions" }
                div { class: "log-actions",
                    button {
                        id: "log-refresh",
                        class: "log-primary",
                        r#type: "button",
                        disabled: state.sending,
                        onclick: move |_| start_load(),
                        "Refresh"
                    }
                    button {
                        id: "log-clear",
                        class: "log-outline",
                        r#type: "button",
                        onclick: move |_| clear_filters(),
                        "Clear filters"
                    }
                }
            }
            SourceFacets { state: state.clone() }
            div { class: "log-filter-group",
                label { r#for: "log-filter-search", "Search" }
                input {
                    id: "log-filter-search",
                    r#type: "search",
                    placeholder: "Message",
                    oninput: move |_| commit_search(ui),
                    onchange: move |_| commit_search(ui),
                    onkeydown: move |event| {
                        if event.key() == Key::Enter {
                            event.prevent_default();
                            commit_search(ui);
                        }
                    },
                }
            }
            DateFacets { ui }
            LevelFacets { state: state.clone() }
        }
    }
}

#[component]
fn SourceFacets(state: LogUi) -> Element {
    rsx! {
        div { id: "log-view-sources", class: "log-filter-group",
            p { "Source" }
            Facet {
                id: "log-source-all",
                attr: "data-source",
                value: "",
                label: if state.sources.is_empty() { "None loaded" } else { "All" },
                pressed: state.sources.is_empty() || state.selected.is_empty(),
                onclick: move |_| toggle_source(""),
            }
            for (index, item) in state.sources.iter().enumerate() {
                Facet {
                    key: "{item.id}",
                    id: format!("log-source-{index}"),
                    attr: "data-source",
                    value: item.id.clone(),
                    label: source_label(item),
                    pressed: state.selected.iter().any(|id| id == &item.id),
                    onclick: {
                        let id = item.id.clone();
                        move |_| toggle_source(&id)
                    },
                }
            }
        }
    }
}

#[component]
fn DateFacets(ui: Signal<LogUi>) -> Element {
    rsx! {
        div { class: "log-filter-group",
            p { "Date range" }
            label { r#for: "log-range-start", "Start" }
            input {
                id: "log-range-start",
                r#type: "date",
                oninput: move |_| commit_range(ui),
                onchange: move |_| commit_range(ui),
                onkeydown: move |event| range_tab(event, "log-range-end", false),
            }
            label { r#for: "log-range-end", "End" }
            input {
                id: "log-range-end",
                r#type: "date",
                oninput: move |_| commit_range(ui),
                onchange: move |_| commit_range(ui),
                onkeydown: move |event| range_tab(event, "log-filter-all", true),
            }
        }
    }
}

#[component]
fn LevelFacets(state: LogUi) -> Element {
    rsx! {
        div { class: "log-filter-group",
            p { "Severity" }
            Facet { id: "log-filter-all", attr: "data-level", value: "", label: "All", pressed: state.levels.is_empty(), onclick: move |_| toggle_level("") }
            Facet { id: "log-filter-trace", attr: "data-level", value: "trace", label: "Trace", pressed: has_level(&state, "trace"), onclick: move |_| toggle_level("trace") }
            Facet { id: "log-filter-debug", attr: "data-level", value: "debug", label: "Debug", pressed: has_level(&state, "debug"), onclick: move |_| toggle_level("debug") }
            Facet { id: "log-filter-info", attr: "data-level", value: "info", label: "Info", pressed: has_level(&state, "info"), onclick: move |_| toggle_level("info") }
            Facet { id: "log-filter-warn", attr: "data-level", value: "warn", label: "Warn", pressed: has_level(&state, "warn"), onclick: move |_| toggle_level("warn") }
            Facet { id: "log-filter-error", attr: "data-level", value: "error", label: "Error", pressed: has_level(&state, "error"), onclick: move |_| toggle_level("error") }
        }
    }
}

#[component]
fn Results(state: LogUi, shown: Vec<Record>, mode: Mode) -> Element {
    let chips = chip_rows(&state);
    rsx! {
        div { id: "log-results", class: "log-results",
            p { id: "log-filter-count", class: "log-count", "{count_text(shown.len(), state.records.len())}" }
            div { class: "log-scroll",
                div { id: "log-filter-chips", class: "log-chips", hidden: chips.is_empty(),
                    span { class: "log-chips-label", "Filters" }
                    for chip in chips.iter() {
                        Chip { key: "{chip.clear}", chip: chip.clone() }
                    }
                }
                ResultState { state: state.clone(), mode }
                ul { id: "log-view-entries", class: "log-cards", hidden: mode != Mode::Ready,
                    for record in shown.iter() {
                        LogCard {
                            key: "{record.source_id}:{record.timestamp}:{record.message}",
                            record: record.clone(),
                            source: source_text(&state, &record.source_id),
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ResultState(state: LogUi, mode: Mode) -> Element {
    rsx! {
        div {
            id: "log-result-state",
            "data-state": mode.slug(),
            "data-gen": state.tick.to_string(),
            hidden: mode == Mode::Ready,
            p {
                id: "log-view-status",
                role: "status",
                aria_live: "polite",
                "data-state": mode.slug(),
                "{mode.text()}"
            }
            if mode == Mode::NoMatch {
                button {
                    id: "log-result-clear",
                    class: "log-outline",
                    r#type: "button",
                    onclick: move |_| clear_filters(),
                    "Clear filters"
                }
            }
        }
    }
}

#[component]
fn Facet(
    id: String,
    attr: &'static str,
    value: String,
    label: String,
    pressed: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        button {
            id: "{id}",
            r#type: "button",
            class: "log-facet",
            "data-source": if attr == "data-source" { Some(value.clone()) } else { None },
            "data-level": if attr == "data-level" { Some(value.clone()) } else { None },
            aria_pressed: if pressed { "true" } else { "false" },
            onclick: move |event| onclick.call(event),
            span { class: "log-facet-mark", aria_hidden: "true" }
            span { "{label}" }
        }
    }
}

#[derive(Clone, PartialEq)]
struct ChipData {
    label: String,
    value: String,
    clear: String,
}

#[component]
fn Chip(chip: ChipData) -> Element {
    let clear = chip.clear.clone();
    rsx! {
        span { class: "log-meta",
            span { class: "log-meta-label", "{chip.label}:" }
            span { "{chip.value}" }
            button {
                r#type: "button",
                "data-clear": "{chip.clear}",
                aria_label: "Remove {chip.label} {chip.value}",
                onclick: move |_| clear_chip(&clear),
                "×"
            }
        }
    }
}

#[component]
fn LogCard(record: Record, source: String) -> Element {
    rsx! {
        li {
            div { class: "log-card",
                p { class: "log-card-id", "{record.source_id}" }
                LogMessage { message: record.message.clone() }
                div { class: "log-card-tags",
                    span { class: "log-meta",
                        span { class: "log-meta-label", "Time:" }
                        time { datetime: "{record.timestamp}", "{record.timestamp}" }
                    }
                    span { class: "log-meta",
                        span { class: "log-meta-label", "Source:" }
                        span { "{source}" }
                    }
                    span { class: "log-meta",
                        span { class: "log-meta-label", "Severity:" }
                        span { "{record.level}" }
                    }
                    if !record.attributes.is_empty() {
                        span { class: "log-meta",
                            span { class: "log-meta-label", "Attributes:" }
                            span { "{record.attributes}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn LogMessage(message: String) -> Element {
    let mut open = use_signal(|| false);
    let long = message.chars().count() > MESSAGE_FOLD;
    let expanded = long && open();
    rsx! {
        p {
            class: if expanded { "log-card-title is-open" } else { "log-card-title" },
            "{message}"
        }
        if long {
            button {
                r#type: "button",
                class: "log-expand",
                aria_expanded: if expanded { "true" } else { "false" },
                onclick: move |_| open.set(!open()),
                if expanded { "Collapse" } else { "Expand" }
            }
        }
    }
}

fn presentation(state: &LogUi, shown: usize) -> Mode {
    if !state.known || state.sending {
        return Mode::Loading;
    }
    if state.faulted {
        return Mode::Error;
    }
    if state.sources.is_empty() {
        return Mode::NoSources;
    }
    if shown > 0 {
        return Mode::Ready;
    }
    if state.records.is_empty() && !filters_active(state) {
        return Mode::Empty;
    }
    Mode::NoMatch
}

fn apply_outcome(mut ui: Signal<LogUi>, applied: &mut Signal<u32>, outcome: LogOutcome) {
    match outcome {
        LogOutcome::Idle => {}
        LogOutcome::Ready { tick, records } => {
            if tick <= applied() {
                return;
            }
            applied.set(tick);
            let mut next = ui();
            if next.tick != tick {
                return;
            }
            next.sending = false;
            next.faulted = false;
            next.records = records;
            next.alert.clear();
            ui.set(next);
        }
        LogOutcome::Failed { tick, message } => {
            if tick <= applied() {
                return;
            }
            applied.set(tick);
            let mut next = ui();
            if next.tick != tick {
                return;
            }
            next.sending = false;
            next.faulted = true;
            next.alert = message;
            ui.set(next);
        }
    }
}

fn start_load() {
    let Some(loader) = loader() else {
        return;
    };
    begin_load(loader.ui, loader.job);
}

fn begin_load(mut ui: Signal<LogUi>, mut job: Signal<LogJob>) {
    if !calls_allowed() {
        return;
    }
    let sources = active_sources(&ui.peek());
    if sources.is_empty() {
        let mut next = ui();
        next.sending = false;
        next.faulted = false;
        ui.set(next);
        return;
    }
    let range = ui.peek().range.clone();
    let mut visible = match &range {
        QueryRange::Window => Vec::new(),
        QueryRange::Bounded { start, end } => vec![start.clone(), end.clone()],
    };
    visible.extend(sources.iter().cloned());
    let visible = visible.iter().map(String::as_str).collect::<Vec<_>>();
    if take_secret(&visible).is_err() {
        return;
    }
    let mut next = ui();
    next.tick = next.tick.wrapping_add(1);
    next.sending = true;
    next.faulted = false;
    next.alert.clear();
    let tick = next.tick;
    ui.set(next);
    job.set(LogJob::Load {
        tick,
        sources,
        range,
    });
}

async fn run_job(job: LogJob, secret: Option<String>) -> LogOutcome {
    let LogJob::Load {
        tick,
        sources,
        range,
    } = job
    else {
        return LogOutcome::Idle;
    };
    let Some(transport) = session::transport() else {
        return LogOutcome::Failed {
            tick,
            message: "The browser could not reach this origin.".to_owned(),
        };
    };
    let mut records = Vec::new();
    for source in &sources {
        let command = logs::command(source, None, &range);
        let Ok(body) = crate::codec::command_message(&message_id(), &command, "") else {
            return LogOutcome::Failed {
                tick,
                message: "The log query could not be built.".to_owned(),
            };
        };
        match transport.send(&body, secret.as_deref()).await {
            Err(()) => {
                return LogOutcome::Failed {
                    tick,
                    message: "The browser could not reach this origin.".to_owned(),
                };
            }
            Ok(reply) => match logs::interpret(&reply, secret.as_deref()) {
                Ok(read) => records.extend(read.records),
                Err(fault) => {
                    return LogOutcome::Failed {
                        tick,
                        message: fault_message(&fault),
                    };
                }
            },
        }
    }
    records.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
    LogOutcome::Ready { tick, records }
}

fn toggle_source(source: &str) {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if source.is_empty() {
        if next.selected.is_empty() {
            return;
        }
        next.selected.clear();
    } else if let Some(index) = next.selected.iter().position(|id| id == source) {
        next.selected.remove(index);
    } else {
        next.selected.push(source.to_owned());
    }
    ui.set(next);
    start_load();
}

fn toggle_level(level: &str) {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui();
    if level.is_empty() {
        next.levels.clear();
    } else if let Some(index) = next.levels.iter().position(|item| item == level) {
        next.levels.remove(index);
    } else {
        next.levels.push(level.to_owned());
    }
    ui.set(next);
}

fn commit_search(mut ui: Signal<LogUi>) {
    let query = session::control_value("log-filter-search")
        .trim()
        .to_owned();
    let mut next = ui();
    if next.query == query {
        return;
    }
    next.query = query;
    ui.set(next);
}

fn commit_range(mut ui: Signal<LogUi>) {
    let start = session::control_value("log-range-start");
    let end = session::control_value("log-range-end");
    match logs::date_range(&start, &end) {
        Ok(range) => {
            let mut next = ui();
            if next.range == range {
                next.alert.clear();
                ui.set(next);
                return;
            }
            next.range = range;
            next.alert.clear();
            ui.set(next);
            start_load();
        }
        Err(message) => {
            let mut next = ui();
            next.alert = message.to_owned();
            ui.set(next);
        }
    }
}

fn clear_filters() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui();
    next.query.clear();
    next.levels.clear();
    next.selected.clear();
    next.range = QueryRange::Window;
    next.alert.clear();
    next.faulted = false;
    ui.set(next);
    session::write_control("log-filter-search", "");
    session::write_control("log-range-start", "");
    session::write_control("log-range-end", "");
    start_load();
}

fn clear_chip(clear: &str) {
    let Some(mut ui) = bound() else {
        return;
    };
    match clear {
        "search" => {
            let mut next = ui();
            next.query.clear();
            session::write_control("log-filter-search", "");
            ui.set(next);
        }
        "range" => {
            let mut next = ui();
            next.range = QueryRange::Window;
            session::write_control("log-range-start", "");
            session::write_control("log-range-end", "");
            ui.set(next);
            start_load();
        }
        clear if clear.starts_with("source:") => {
            let id = clear.trim_start_matches("source:");
            let mut next = ui();
            next.selected.retain(|source| source != id);
            ui.set(next);
            start_load();
        }
        level => {
            let mut next = ui();
            next.levels.retain(|item| item != level);
            ui.set(next);
        }
    }
}

fn range_tab(event: KeyboardEvent, next: &str, back: bool) {
    if event.key() != Key::Tab {
        return;
    }
    if event.modifiers().shift() {
        if back {
            event.prevent_default();
            focus_control("log-range-start");
        }
        return;
    }
    event.prevent_default();
    focus_control(next);
}

fn focus_control(id: &str) {
    if let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
        && let Ok(html) = element.dyn_into::<web_sys::HtmlElement>()
    {
        let _ = html.focus();
    }
}

fn visible_records(state: &LogUi) -> Vec<Record> {
    state
        .records
        .iter()
        .filter(|record| matches_filter(state, record))
        .cloned()
        .collect()
}

fn matches_filter(state: &LogUi, record: &Record) -> bool {
    if !state.levels.is_empty() && !state.levels.iter().any(|level| level == &record.level) {
        return false;
    }
    let query = state.query.trim().to_lowercase();
    query.is_empty()
        || record.message.to_lowercase().contains(&query)
        || record.attributes.to_lowercase().contains(&query)
}

fn chip_rows(state: &LogUi) -> Vec<ChipData> {
    let mut chips = Vec::new();
    for id in &state.selected {
        let label = state
            .sources
            .iter()
            .find(|item| item.id == *id)
            .map(source_label)
            .unwrap_or_else(|| id.clone());
        chips.push(ChipData {
            label: "Source".to_owned(),
            value: label,
            clear: format!("source:{id}"),
        });
    }
    if !state.query.is_empty() {
        chips.push(ChipData {
            label: "Search".to_owned(),
            value: state.query.clone(),
            clear: "search".to_owned(),
        });
    }
    if let QueryRange::Bounded { .. } = &state.range {
        let start = session::control_value("log-range-start");
        let end = session::control_value("log-range-end");
        chips.push(ChipData {
            label: "Date".to_owned(),
            value: format!("{start} – {end}"),
            clear: "range".to_owned(),
        });
    }
    for level in &state.levels {
        chips.push(ChipData {
            label: "Severity".to_owned(),
            value: level.clone(),
            clear: level.clone(),
        });
    }
    chips
}

fn count_text(shown: usize, total: usize) -> String {
    if total == 0 || shown == total {
        match total {
            1 => "1 log".to_owned(),
            count => format!("{count} logs"),
        }
    } else {
        format!("{shown} of {total} logs")
    }
}

fn filters_active(state: &LogUi) -> bool {
    !state.query.trim().is_empty()
        || !state.levels.is_empty()
        || !state.selected.is_empty()
        || matches!(state.range, QueryRange::Bounded { .. })
}

fn active_sources(state: &LogUi) -> Vec<String> {
    if state.selected.is_empty() {
        state.sources.iter().map(|item| item.id.clone()).collect()
    } else {
        state
            .selected
            .iter()
            .filter(|id| state.sources.iter().any(|item| item.id == **id))
            .cloned()
            .collect()
    }
}

fn source_text(state: &LogUi, source_id: &str) -> String {
    state
        .sources
        .iter()
        .find(|item| item.id == source_id)
        .map(|item| {
            if item.name.is_empty() {
                item.id.clone()
            } else {
                item.name.clone()
            }
        })
        .unwrap_or_else(|| source_id.to_owned())
}

fn source_label(item: &Item) -> String {
    let identity = if item.name.is_empty() || item.name == item.id {
        item.id.clone()
    } else {
        format!("{} ({})", item.name, item.id)
    };
    if item.description.is_empty() {
        identity
    } else {
        format!("{identity} — {}", item.description)
    }
}

fn has_level(state: &LogUi, level: &str) -> bool {
    state.levels.iter().any(|item| item == level)
}

fn fault_message(fault: &Fault) -> String {
    match fault {
        Fault::Unavailable(message) => message.clone(),
        Fault::Unauthorized(message) | Fault::Forbidden(message) => message.clone(),
        Fault::Malformed => "The log result could not be read.".to_owned(),
        Fault::Transport => "The browser could not reach this origin.".to_owned(),
    }
}
