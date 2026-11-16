//! Semantic search and frame decision panels inside the Videos reader.

use std::cell::RefCell;

use dioxus::prelude::*;

#[path = "semantic_transport.rs"]
mod semantic_transport;

use crate::shell::loading::{StrandSize, StrandsLoader};
use crate::shell::nav;
use crate::shell::session::{self, calls_allowed, take_secret};
use crate::videos;
use crate::videos::semantic::{
    self, ApiPhase, CoverageBanner, DecisionRow, HitThumb, ProvenancePick, SearchHit, SearchTicket,
};
use semantic_transport::{coverage_bounds, fault_text, post_with_abort};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::{VideoUi, bound};

const SEARCH_ALL_START: &str = "1900-01-01T00:00:00Z";
const SEARCH_ALL_END: &str = "2100-01-01T00:00:00Z";

#[derive(Clone, PartialEq)]
pub(crate) enum SearchJob {
    Idle,
    Run(SearchTicket),
}

#[derive(Clone, PartialEq)]
enum SearchOutcome {
    Idle,
    Ready {
        ticket: SearchTicket,
        hits: Vec<SearchHit>,
        incomplete: bool,
    },
    Failed {
        ticket: SearchTicket,
        message: String,
    },
}

#[derive(Clone, PartialEq)]
pub(crate) enum CapabilityJob {
    Idle,
    Run { generation: u32, source_id: String },
}

#[derive(Clone, PartialEq)]
enum CapabilityOutcome {
    Idle,
    Ready {
        generation: u32,
        semantic: bool,
        decisions: bool,
        provenance: Option<ProvenancePick>,
        coverage: Option<CoverageBanner>,
    },
    #[allow(dead_code)]
    Failed {
        generation: u32,
        message: String,
    },
}

#[derive(Clone, PartialEq)]
pub(crate) enum HistoryJob {
    Idle,
    #[allow(dead_code)]
    Run {
        generation: u32,
        index: usize,
    },
}

#[derive(Clone, PartialEq)]
enum HistoryOutcome {
    Idle,
    Ready {
        generation: u32,
        index: usize,
        rows: Vec<DecisionRow>,
    },
    Failed {
        generation: u32,
        index: usize,
        message: String,
    },
}

#[derive(Clone, Copy)]
struct Loader {
    search: Signal<SearchJob>,
    caps: Signal<CapabilityJob>,
    #[allow(dead_code)]
    history: Signal<HistoryJob>,
}

thread_local! {
    static LOADER: RefCell<Option<Loader>> = const { RefCell::new(None) };
    static PROBED_SOURCE: RefCell<String> = const { RefCell::new(String::new()) };
    static SEARCH_SEEK: RefCell<Option<(String, i64, bool)>> = const { RefCell::new(None) };
}

pub(super) fn remember_search_seek(source_id: &str, at_us: i64, autoplay: bool) {
    SEARCH_SEEK.with(|slot| *slot.borrow_mut() = Some((source_id.to_owned(), at_us, autoplay)));
}

pub fn take_search_seek(source_id: &str) -> Option<(i64, bool)> {
    SEARCH_SEEK.with(|slot| {
        let matches = slot
            .borrow()
            .as_ref()
            .is_some_and(|(id, _, _)| id == source_id);
        if matches {
            slot.borrow_mut()
                .take()
                .map(|(_, at_us, autoplay)| (at_us, autoplay))
        } else {
            None
        }
    })
}

pub fn bind_loader(
    search: Signal<SearchJob>,
    caps: Signal<CapabilityJob>,
    history: Signal<HistoryJob>,
) {
    LOADER.with(|slot| {
        *slot.borrow_mut() = Some(Loader {
            search,
            caps,
            history,
        });
    });
}

pub fn note_source_change(source_id: &str) {
    if source_id.is_empty() {
        PROBED_SOURCE.with(|slot| *slot.borrow_mut() = String::new());
        return;
    }
    let Some(mut ui) = bound() else {
        return;
    };
    let repeat = PROBED_SOURCE.with(|slot| !slot.borrow().is_empty());
    if repeat {
        return;
    }
    if source_id.is_empty() || !calls_allowed() {
        return;
    }
    if take_secret(&[source_id]).is_err() {
        return;
    }
    let cap_generation = ui.write().semantic.note_source();
    PROBED_SOURCE.with(|slot| *slot.borrow_mut() = source_id.to_owned());
    launch_capabilities(cap_generation, source_id.to_owned());
}

pub fn commit_semantic_query() {
    let Some(mut ui) = bound() else {
        return;
    };
    let text = session::control_value("video-semantic-query");
    let state = ui.peek().clone();
    let Some((start, end)) = commit_search_filter() else {
        return;
    };
    ui.write().semantic.query = text.clone();
    let sources = search_source_ids(&state);
    let ticket =
        ui.write()
            .semantic
            .begin_search(&sources, &text, start.as_deref(), end.as_deref());
    if let Some(ticket) = ticket {
        launch_search(ticket);
    }
}

fn search_source_ids(state: &VideoUi) -> Vec<String> {
    if state.search_sources.is_empty() {
        state.sources.iter().map(|item| item.id.clone()).collect()
    } else {
        state.search_sources.clone()
    }
}

pub fn toggle_search_source(source_id: &str) {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if source_id.is_empty() {
        next.search_sources.clear();
    } else if let Some(index) = next.search_sources.iter().position(|id| id == source_id) {
        next.search_sources.remove(index);
    } else {
        next.search_sources.push(source_id.to_owned());
    }
    ui.set(next);
}

pub fn choose_semantic_hit(index: usize) {
    let Some(mut ui) = bound() else {
        return;
    };
    if !ui.write().semantic.select(index) {
        let state = ui.peek().clone();
        if state.semantic.selected != index || index >= state.semantic.hits.len() {
            return;
        }
        ui.write().semantic.apply_decisions_from_hit(index);
    }
    let state = ui.peek().clone();
    let Some(hit) = state.semantic.selected_hit() else {
        return;
    };
    if let Some(href) = hit.browse_href() {
        nav::open(&href);
        return;
    }
    launch_thumb(index);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchMode {
    Discovering,
    Absent,
    Error,
    Loading,
    Prompt,
    Empty,
    Fault,
    Ready,
}

impl SearchMode {
    fn slug(self) -> &'static str {
        match self {
            Self::Discovering => "discovering",
            Self::Absent => "absent",
            Self::Error => "error",
            Self::Loading => "loading",
            Self::Prompt => "prompt",
            Self::Empty => "empty",
            Self::Fault => "fault",
            Self::Ready => "ready",
        }
    }

    fn text(self) -> &'static str {
        match self {
            Self::Discovering => "Discovering search capabilities…",
            Self::Absent => "Search is not configured.",
            Self::Error => "Search capabilities could not be read.",
            Self::Loading => "Searching…",
            Self::Prompt => "",
            Self::Empty => "No hits in this scope.",
            Self::Fault => "Search could not be completed.",
            Self::Ready => "Select a hit to inspect metadata and decisions.",
        }
    }
}

#[component]
pub fn SearchPanel(state: VideoUi) -> Element {
    let bounds = if state.search_range_active {
        resolved_search_bounds(&state)
    } else {
        (String::new(), String::new())
    };
    rsx! {
        div {
            id: "video-search-browser",
            class: "log-browser",
            "data-source-id": "{state.reader.source_id}",
            "data-search-sources": "{search_source_attr(&state)}",
            "data-start": "{bounds.0}",
            "data-end": "{bounds.1}",
            SearchRail { state: state.clone() }
            SearchResults { state: state.clone() }
        }
    }
}

#[component]
fn SearchRail(state: VideoUi) -> Element {
    let local = state.local_time;
    let ready = state.semantic.search_capabilities.phase == ApiPhase::Ready;
    let start = shown_search_field(&state, true);
    let end = shown_search_field(&state, false);
    rsx! {
        div {
            id: "video-search-filters",
            role: "group",
            aria_label: "Search filters",
            div { class: "log-filter-group",
                p { "Actions" }
                div { class: "log-actions",
                    button {
                        id: "video-semantic-submit",
                        class: "log-primary",
                        r#type: "button",
                        disabled: !ready || state.semantic.search_loading,
                        onclick: move |_| commit_semantic_query(),
                        "Search"
                    }
                    button {
                        id: "video-search-clear",
                        class: "log-outline",
                        r#type: "button",
                        onclick: move |_| clear_search_filters(),
                        "Clear filters"
                    }
                }
            }
            div { id: "video-search-sources", class: "log-filter-group",
                p { id: "video-search-source-label", "Source" }
                if state.sources.is_empty() {
                    button {
                        id: "video-search-source-empty",
                        class: "log-facet",
                        r#type: "button",
                        disabled: true,
                        aria_pressed: "true",
                        span { class: "log-facet-mark", aria_hidden: "true" }
                        "None loaded"
                    }
                } else {
                    button {
                        id: "video-search-source-all",
                        class: "log-facet",
                        r#type: "button",
                        "data-source": "",
                        aria_pressed: if state.search_sources.is_empty() { "true" } else { "false" },
                        onclick: move |_| toggle_search_source(""),
                        span { class: "log-facet-mark", aria_hidden: "true" }
                        "All"
                    }
                }
                for (index, item) in state.sources.iter().enumerate() {
                    button {
                        key: "{item.id}",
                        id: "video-search-source-{index}",
                        class: "log-facet",
                        r#type: "button",
                        "data-source": "{item.id}",
                        aria_pressed: if state.search_sources.iter().any(|id| id == &item.id) { "true" } else { "false" },
                        onclick: {
                            let id = item.id.clone();
                            move |_| toggle_search_source(&id)
                        },
                        span { class: "log-facet-mark", aria_hidden: "true" }
                        "{super::source_option(item)}"
                    }
                }
            }
            div {
                id: "video-semantic-filters",
                class: "log-filter-group",
                role: "group",
                aria_label: "Search",
                label { id: "video-semantic-label", r#for: "video-semantic-query", "Search" }
                input {
                    id: "video-semantic-query",
                    r#type: "search",
                    autocomplete: "off",
                    spellcheck: "false",
                    placeholder: "Search",
                    disabled: !ready,
                    value: "{state.semantic.query}",
                    oninput: move |_| {
                        if let Some(mut ui) = bound() {
                            ui.write().semantic.query = session::control_value("video-semantic-query");
                        }
                    },
                    onkeydown: semantic_enter,
                }
            }
            div {
                id: "video-search-time",
                class: "log-filter-group",
                p { "Time range" }
                button {
                    id: "video-search-local-time",
                    class: "log-facet",
                    r#type: "button",
                    aria_pressed: if local { "true" } else { "false" },
                    aria_label: "Local time",
                    onclick: move |_| super::toggle_local_time(),
                    span { class: "log-facet-mark", aria_hidden: "true" }
                    "Local time"
                }
                label { r#for: "video-search-range-start", "Start" }
                input {
                    id: "video-search-range-start",
                    r#type: "datetime-local",
                    step: "1",
                    autocomplete: "off",
                    value: "{start}",
                    oninput: move |_| edit_search_filter(true),
                    onchange: move |_| edit_search_filter(true),
                    onkeydown: search_range_enter,
                }
                label { r#for: "video-search-range-end", "End" }
                input {
                    id: "video-search-range-end",
                    r#type: "datetime-local",
                    step: "1",
                    autocomplete: "off",
                    value: "{end}",
                    oninput: move |_| edit_search_filter(false),
                    onchange: move |_| edit_search_filter(false),
                    onkeydown: search_range_enter,
                }
                p { id: "video-search-range-error", class: "video-range-error", "{state.search_range_error}" }
            }
        }
    }
}

#[component]
fn SearchResults(state: VideoUi) -> Element {
    let mode = search_mode(&state);
    let selected = state.semantic.selected;
    let chips = search_chips(&state);
    let incomplete = state.semantic.incomplete_indexing
        || state
            .semantic
            .coverage
            .as_ref()
            .is_some_and(|banner| banner.incomplete);
    rsx! {
        div { id: "video-search-results", class: "log-results",
            p { id: "video-search-count", class: "log-count", "{search_count(&state)}" }
            div { class: "log-scroll",
                div { id: "video-search-chips", class: "log-chips", hidden: chips.is_empty(),
                    span { class: "log-chips-label", "Filters" }
                    for chip in chips.iter() {
                        SearchChip { key: "{chip.clear}", chip: chip.clone() }
                    }
                }
                p {
                    id: "video-semantic-capability",
                    role: "status",
                    aria_live: "polite",
                    hidden: state.semantic.search_capabilities.phase == ApiPhase::Ready,
                    "data-state": capability_slug(state.semantic.search_capabilities.phase),
                    "{capability_copy(state.semantic.search_capabilities.phase)}"
                }
                if let Some(coverage) = state.semantic.coverage.as_ref() {
                    p {
                        id: "video-semantic-coverage",
                        role: "status",
                        aria_live: "polite",
                        "data-incomplete": if incomplete { "true" } else { "false" },
                        "Coverage: {coverage.ready} ready, {coverage.pending} pending, {coverage.failed} failed, {coverage.expired} expired."
                    }
                }
                div {
                    id: "video-semantic-alert",
                    role: "alert",
                    aria_live: "assertive",
                    hidden: state.semantic.search_loading,
                    "{state.semantic.search_fault}"
                }
                div {
                    id: "video-search-result-state",
                    "data-state": mode.slug(),
                    hidden: mode == SearchMode::Ready || mode == SearchMode::Prompt,
                    if mode == SearchMode::Loading {
                        div {
                            id: "video-semantic-search-loading",
                            class: "loading-surface",
                            role: "status",
                            aria_live: "polite",
                            aria_busy: "true",
                            aria_label: "Searching…",
                            StrandsLoader { size: StrandSize::Md }
                            p { class: "loading-title", "{mode.text()}" }
                        }
                    }
                    p {
                        id: "video-semantic-status",
                        role: "status",
                        aria_live: "polite",
                        class: if mode == SearchMode::Loading { "visually-hidden" } else { "" },
                        "data-loading": if state.semantic.search_loading { "true" } else { "false" },
                        "data-state": mode.slug(),
                        if mode == SearchMode::Fault && !state.semantic.search_fault.is_empty() {
                            "{state.semantic.search_fault}"
                        } else {
                            "{mode.text()}"
                        }
                    }
                    if mode == SearchMode::Empty || mode == SearchMode::Fault {
                        button {
                            id: "video-search-result-clear",
                            class: "log-outline",
                            r#type: "button",
                            onclick: move |_| clear_search_filters(),
                            "Clear filters"
                        }
                    }
                }
                ul {
                    id: "video-semantic-hits",
                    class: "log-cards video-semantic-hits",
                    role: "listbox",
                    aria_label: "Search hits",
                    hidden: mode != SearchMode::Ready,
                    for (index, hit) in state
                        .semantic
                        .hits
                        .iter()
                        .enumerate()
                        .skip(state.semantic.result_page * semantic::SEARCH_PAGE_SIZE)
                        .take(semantic::SEARCH_PAGE_SIZE)
                    {
                        li { key: "{hit.frame_id}",
                            SemanticHitRow {
                                index,
                                hit: hit.clone(),
                                selected: selected == index,
                            }
                        }
                    }
                }
                if mode == SearchMode::Ready {
                    SearchPagination { state: state.clone() }
                }
            }
        }
    }
}

#[derive(Clone, PartialEq)]
struct SearchChipData {
    label: String,
    value: String,
    clear: String,
    removable: bool,
}

#[component]
fn SearchChip(chip: SearchChipData) -> Element {
    let clear = chip.clear.clone();
    let removable = chip.removable;
    rsx! {
        span { class: "log-meta", "data-chip": "{chip.clear}",
            span { class: "log-meta-label", "{chip.label}:" }
            span { "{chip.value}" }
            if removable {
                button {
                    r#type: "button",
                    "data-clear": "{chip.clear}",
                    aria_label: "Remove {chip.label} {chip.value}",
                    onclick: move |_| clear_search_chip(&clear),
                    "×"
                }
            }
        }
    }
}

#[component]
fn SearchPagination(state: VideoUi) -> Element {
    let page_count = state
        .semantic
        .hits
        .len()
        .div_ceil(semantic::SEARCH_PAGE_SIZE);
    if page_count <= 1 {
        return rsx! {};
    }
    let page = state.semantic.result_page;
    rsx! {
        nav {
            id: "video-semantic-pagination",
            class: "video-semantic-pagination",
            aria_label: "Search result pages",
            button {
                id: "video-semantic-previous",
                class: "video-outline",
                r#type: "button",
                disabled: page == 0,
                onclick: move |_| change_result_page(-1),
                "Previous"
            }
            p {
                id: "video-semantic-page-status",
                role: "status",
                aria_live: "polite",
                "Page {page + 1} of {page_count}"
            }
            button {
                id: "video-semantic-next",
                class: "video-outline",
                r#type: "button",
                disabled: page + 1 >= page_count,
                onclick: move |_| change_result_page(1),
                "Next"
            }
        }
    }
}

fn change_result_page(delta: i32) {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    let page_count = next
        .semantic
        .hits
        .len()
        .div_ceil(semantic::SEARCH_PAGE_SIZE);
    next.semantic.result_page = if delta.is_negative() {
        next.semantic
            .result_page
            .saturating_sub(delta.unsigned_abs() as usize)
    } else {
        (next.semantic.result_page + delta as usize).min(page_count.saturating_sub(1))
    };
    ui.set(next);
}

#[component]
fn SemanticHitRow(index: usize, hit: SearchHit, selected: bool) -> Element {
    let blocked = hit.media_blocked();
    let href = hit.browse_href();
    rsx! {
        if let Some(href) = href {
            a {
                id: "video-semantic-hit-{index}",
                class: "video-semantic-hit log-card",
                href: "{href}",
                role: "option",
                aria_selected: if selected { "true" } else { "false" },
                "data-source": "{hit.source_id}",
                "data-index-state": "{hit.index_state}",
                "data-media-state": "{hit.media_state}",
                onmounted: move |_| intercept_hit_navigation(index),
                onclick: move |event| {
                    if modified_click(&event) {
                        return;
                    }
                    event.prevent_default();
                    choose_semantic_hit(index);
                },
                SemanticHitBody { hit: hit.clone() }
            }
        } else {
            button {
                id: "video-semantic-hit-{index}",
                class: "video-semantic-hit log-card",
                r#type: "button",
                role: "option",
                aria_selected: if selected { "true" } else { "false" },
                "data-source": "{hit.source_id}",
                "data-index-state": "{hit.index_state}",
                "data-media-state": "{hit.media_state}",
                disabled: blocked,
                onclick: move |_| choose_semantic_hit(index),
                SemanticHitBody { hit: hit.clone() }
            }
        }
    }
}

#[component]
fn SemanticHitBody(hit: SearchHit) -> Element {
    rsx! {
        if let Some(thumb) = hit.thumb.as_ref() {
            SemanticThumb { thumb: thumb.clone() }
        }
        span { class: "video-semantic-hit-copy",
            time { datetime: "{hit.captured_at}", "{hit.captured_at}" }
            span { class: "video-semantic-source", "{hit.source_label}" }
            span { class: "video-semantic-description", "{hit.description}" }
        }
    }
}

fn modified_click(event: &MouseEvent) -> bool {
    event.modifiers().ctrl()
        || event.modifiers().meta()
        || event.modifiers().shift()
        || event.modifiers().alt()
}

fn intercept_hit_navigation(index: usize) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Some(element) = document.get_element_by_id(&format!("video-semantic-hit-{index}")) else {
        return;
    };
    if element.get_attribute("data-nav-intercept") == Some("1".to_owned()) {
        return;
    }
    let _ = element.set_attribute("data-nav-intercept", "1");
    let closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
        if event.ctrl_key() || event.meta_key() || event.shift_key() || event.alt_key() {
            return;
        }
        event.prevent_default();
    }) as Box<dyn FnMut(_)>);
    let _ = element.add_event_listener_with_callback_and_bool(
        "click",
        closure.as_ref().unchecked_ref(),
        true,
    );
    closure.forget();
}

#[component]
fn SemanticThumb(thumb: HitThumb) -> Element {
    rsx! {
        if !thumb.url.is_empty() {
            img {
                class: "video-semantic-thumb",
                src: "{thumb.url}",
                alt: "Saved frame thumbnail",
                width: "96",
                height: "54",
            }
        } else {
            span { class: "video-semantic-thumb-placeholder", "{thumb.withheld}" }
        }
    }
}

fn capability_slug(phase: ApiPhase) -> &'static str {
    match phase {
        ApiPhase::Unknown => "unknown",
        ApiPhase::Absent => "absent",
        ApiPhase::Ready => "ready",
        ApiPhase::Error => "error",
    }
}

fn semantic_enter(event: KeyboardEvent) {
    if event.key() == Key::Enter {
        event.prevent_default();
        commit_semantic_query();
    }
}

fn search_window(state: &VideoUi) -> Option<(String, String)> {
    if !state.search_range_active {
        return None;
    }
    let (start, end) = resolved_search_bounds(state);
    if start.is_empty() || end.is_empty() {
        return None;
    }
    videos::check_range(&start, &end)
        .ok()
        .map(|range| (range.start, range.end))
        .or(Some((start, end)))
}

fn retained_search_window(timeline: &videos::Timeline) -> Option<(String, String)> {
    let first = timeline.intervals.first()?;
    let last = timeline.intervals.last()?;
    Some((first.start.clone(), last.end.clone()))
}

fn loader() -> Option<Loader> {
    LOADER.with(|slot| *slot.borrow())
}

fn search_mode(state: &VideoUi) -> SearchMode {
    match state.semantic.search_capabilities.phase {
        ApiPhase::Unknown => SearchMode::Discovering,
        ApiPhase::Absent => SearchMode::Absent,
        ApiPhase::Error => SearchMode::Error,
        ApiPhase::Ready if state.semantic.search_loading => SearchMode::Loading,
        ApiPhase::Ready if !state.semantic.search_fault.is_empty() => SearchMode::Fault,
        ApiPhase::Ready if state.semantic.query.trim().is_empty() => SearchMode::Prompt,
        ApiPhase::Ready if state.semantic.hits.is_empty() => SearchMode::Empty,
        ApiPhase::Ready => SearchMode::Ready,
    }
}

fn capability_copy(phase: ApiPhase) -> &'static str {
    match phase {
        ApiPhase::Absent => "Search is not configured.",
        ApiPhase::Error => "Search capabilities could not be read.",
        ApiPhase::Ready => "",
        ApiPhase::Unknown => "Discovering search capabilities…",
    }
}

fn search_count(state: &VideoUi) -> String {
    match search_mode(state) {
        SearchMode::Loading => "Searching…".to_owned(),
        SearchMode::Ready | SearchMode::Empty | SearchMode::Fault => {
            let shown = state.semantic.hits.len();
            if shown == 1 {
                "1 hit".to_owned()
            } else {
                format!("{shown} hits")
            }
        }
        SearchMode::Discovering => "Discovering search".to_owned(),
        SearchMode::Absent => "Search is not configured".to_owned(),
        SearchMode::Error => "Search unavailable".to_owned(),
        SearchMode::Prompt => "No search yet".to_owned(),
    }
}

fn search_source_attr(state: &VideoUi) -> String {
    if state.search_sources.is_empty() {
        "all".to_owned()
    } else {
        state.search_sources.join(",")
    }
}

fn search_chips(state: &VideoUi) -> Vec<SearchChipData> {
    let mut chips = Vec::new();
    for id in &state.search_sources {
        let source = state
            .sources
            .iter()
            .find(|item| item.id == *id)
            .map(super::source_option)
            .unwrap_or_else(|| id.clone());
        chips.push(SearchChipData {
            label: "Source".to_owned(),
            value: source,
            clear: format!("source:{id}"),
            removable: true,
        });
    }
    if !state.semantic.query.trim().is_empty() {
        chips.push(SearchChipData {
            label: "Search".to_owned(),
            value: state.semantic.query.clone(),
            clear: "search".to_owned(),
            removable: true,
        });
    }
    if state.search_range_active {
        let (start, end) = resolved_search_bounds(state);
        chips.push(SearchChipData {
            label: "Time".to_owned(),
            value: format!(
                "{} – {}",
                super::display_clock(&start, state.local_time),
                super::display_clock(&end, state.local_time)
            ),
            clear: "range".to_owned(),
            removable: true,
        });
    }
    chips
}

fn clear_search_filters() {
    clear_search_query();
    restore_retained_range();
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    next.search_sources.clear();
    ui.set(next);
}

fn clear_search_chip(clear: &str) {
    match clear {
        "search" => clear_search_query(),
        "range" => restore_retained_range(),
        source if source.starts_with("source:") => {
            toggle_search_source(source.trim_start_matches("source:"));
        }
        _ => {}
    }
}

fn clear_search_query() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    next.semantic.query.clear();
    next.semantic.hits.clear();
    next.semantic.incomplete_indexing = false;
    next.semantic.selected = usize::MAX;
    next.semantic.result_page = 0;
    next.semantic.decisions.clear();
    next.semantic.decision_alert.clear();
    next.semantic.search_loading = false;
    next.semantic.search_fault.clear();
    next.semantic.expected = None;
    ui.set(next);
    session::write_control("video-semantic-query", "");
}

fn restore_retained_range() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    next.search_range_start.clear();
    next.search_range_end.clear();
    next.search_draft_start.clear();
    next.search_draft_end.clear();
    next.search_range_active = false;
    next.search_range_error.clear();
    ui.set(next);
    session::write_control("video-search-range-start", "");
    session::write_control("video-search-range-end", "");
}

fn search_range_enter(event: KeyboardEvent) {
    if event.key() == Key::Enter {
        event.prevent_default();
        commit_semantic_query();
    }
}

fn edit_search_filter(start: bool) {
    let value = session::control_value(if start {
        "video-search-range-start"
    } else {
        "video-search-range-end"
    });
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if start {
        next.search_draft_start = value;
    } else {
        next.search_draft_end = value;
    }
    ui.set(next);
    let _ = commit_search_filter();
}

fn commit_search_filter() -> Option<(Option<String>, Option<String>)> {
    let mut ui = bound()?;
    let state = ui.peek().clone();
    let start_raw = session::control_value("video-search-range-start");
    let end_raw = session::control_value("video-search-range-end");
    if start_raw.trim().is_empty() && end_raw.trim().is_empty() {
        let mut next = ui.peek().clone();
        next.search_range_start.clear();
        next.search_range_end.clear();
        next.search_range_active = false;
        next.search_range_error.clear();
        ui.set(next);
        return Some((None, None));
    }
    let start_raw = if start_raw.trim().is_empty() {
        shown_search_field(&state, true)
    } else {
        start_raw
    };
    let end_raw = if end_raw.trim().is_empty() {
        shown_search_field(&state, false)
    } else {
        end_raw
    };
    if start_raw.trim().is_empty() || end_raw.trim().is_empty() {
        let mut next = ui.peek().clone();
        next.search_range_error = "Enter a retained time range first.".to_owned();
        ui.set(next);
        return None;
    }
    let start = super::field_to_utc(&start_raw, state.local_time);
    let end = super::field_to_utc(&end_raw, state.local_time);
    match videos::check_range(&start, &end) {
        Ok(range) => {
            let mut next = ui.peek().clone();
            next.search_range_start = range.start.clone();
            next.search_range_end = range.end.clone();
            next.search_range_active = true;
            next.search_range_error.clear();
            ui.set(next);
            Some((Some(range.start), Some(range.end)))
        }
        Err(fault) => {
            let mut next = ui.peek().clone();
            next.search_range_error = super::timeline::range_fault_message(fault, next.local_time);
            ui.set(next);
            None
        }
    }
}

fn shown_search_field(state: &VideoUi, start: bool) -> String {
    let draft = if start {
        state.search_draft_start.as_str()
    } else {
        state.search_draft_end.as_str()
    };
    if !draft.is_empty() {
        return draft.to_owned();
    }
    if !state.search_range_active {
        return String::new();
    }
    let stored = if start {
        state.search_range_start.as_str()
    } else {
        state.search_range_end.as_str()
    };
    if !stored.is_empty() {
        return super::timeline::display_picker(stored, state.local_time);
    }
    let (fallback_start, fallback_end) = retained_or_playback(state);
    super::timeline::display_picker(
        if start {
            &fallback_start
        } else {
            &fallback_end
        },
        state.local_time,
    )
}

fn resolved_search_bounds(state: &VideoUi) -> (String, String) {
    if !state.search_range_active {
        return (String::new(), String::new());
    }
    if !state.search_range_start.is_empty() && !state.search_range_end.is_empty() {
        return (
            state.search_range_start.clone(),
            state.search_range_end.clone(),
        );
    }
    (SEARCH_ALL_START.to_owned(), SEARCH_ALL_END.to_owned())
}

fn retained_or_playback(state: &VideoUi) -> (String, String) {
    if let Some(bounds) = state
        .reader
        .timeline
        .as_ref()
        .and_then(retained_search_window)
    {
        return bounds;
    }
    (state.range_start.clone(), state.range_end.clone())
}

fn launch_capabilities(generation: u32, source_id: String) {
    let Some(mut loader) = loader() else {
        return;
    };
    loader.caps.set(CapabilityJob::Run {
        generation,
        source_id,
    });
}

fn launch_search(ticket: SearchTicket) {
    let Some(mut loader) = loader() else {
        return;
    };
    loader.search.set(SearchJob::Run(ticket));
}

#[allow(dead_code)]
fn launch_history(generation: u32, index: usize) {
    let Some(mut loader) = loader() else {
        return;
    };
    loader.history.set(HistoryJob::Run { generation, index });
}

fn launch_thumb(index: usize) {
    let Some(ui) = bound() else {
        return;
    };
    let state = ui.peek().clone();
    let Some(hit) = state.semantic.hits.get(index) else {
        return;
    };
    if hit.media_blocked() {
        return;
    }
    let source_id = if hit.source_id.is_empty() {
        state.reader.source_id.clone()
    } else {
        hit.source_id.clone()
    };
    let captured_at = hit.captured_at.clone();
    let secret = session::native_secret();
    wasm_bindgen_futures::spawn_local(async move {
        let command = videos::frame_at_time_command(&source_id, &captured_at);
        if let Ok(reply) = super::post(command, secret.clone(), state.reader.generation).await
            && let Ok(frame) = videos::interpret_frame_at_time(&reply, secret.as_deref())
            && let Some(thumb) =
                semantic::thumb_from_frame(&frame.data, &frame.media_type, secret.as_deref())
            && let Some(mut ui) = bound()
        {
            let mut next = ui.peek().clone();
            if let Some(slot) = next.semantic.hits.get_mut(index) {
                slot.thumb = Some(thumb);
            }
            ui.set(next);
        }
    });
}

pub fn wire_resources(
    search: Signal<SearchJob>,
    caps: Signal<CapabilityJob>,
    history: Signal<HistoryJob>,
    ui: Signal<VideoUi>,
) {
    bind_loader(search, caps, history);
    let search_reply = use_resource(move || {
        let job = search();
        let secret = session::native_secret();
        async move { run_search(job, secret).await }
    });
    if let Some(outcome) = search_reply() {
        apply_search(ui, outcome);
    }
    let caps_reply = use_resource(move || {
        let job = caps();
        let secret = session::native_secret();
        async move { run_capabilities(job, secret).await }
    });
    if let Some(outcome) = caps_reply() {
        apply_capabilities(ui, outcome);
    }
    let history_reply = use_resource(move || {
        let job = history();
        let secret = session::native_secret();
        async move { run_history(job, secret).await }
    });
    if let Some(outcome) = history_reply() {
        apply_history(ui, outcome);
    }
}

async fn run_search(job: SearchJob, secret: Option<String>) -> SearchOutcome {
    let SearchJob::Run(ticket) = job else {
        return SearchOutcome::Idle;
    };
    let provenance = bound().and_then(|ui| ui.peek().semantic.provenance.clone());
    let Some(provenance) = provenance else {
        return SearchOutcome::Failed {
            ticket,
            message: "Semantic provenance is not ready.".to_owned(),
        };
    };
    let mut hits = Vec::new();
    let mut incomplete = false;
    let mut failure = None;
    for source_id in &ticket.source_ids {
        let range = match (&ticket.range_start, &ticket.range_end) {
            (Some(start), Some(end)) => Some((start.as_str(), end.as_str())),
            _ => None,
        };
        let command = semantic::search_command(source_id, range, &ticket.text, &provenance);
        match post_with_abort(command, secret.clone(), ticket.generation).await {
            Ok(reply) => match semantic::interpret_search(&reply, secret.as_deref()) {
                Ok((mut found, source_incomplete)) => {
                    let label = source_label(source_id);
                    for hit in &mut found {
                        if hit.source_id.is_empty() {
                            hit.source_id = source_id.clone();
                        }
                        hit.source_label = label.clone();
                    }
                    hits.append(&mut found);
                    incomplete |= source_incomplete;
                }
                Err(fault) => failure = Some(fault_text(&fault)),
            },
            Err(message) => failure = Some(message),
        }
    }
    if hits.is_empty()
        && let Some(message) = failure
    {
        return SearchOutcome::Failed { ticket, message };
    }
    hits.sort_by(|left, right| hit_score(right).total_cmp(&hit_score(left)));
    SearchOutcome::Ready {
        ticket,
        hits,
        incomplete,
    }
}

fn hit_score(hit: &semantic::SearchHit) -> f64 {
    hit.retrieval_score.trim().parse().unwrap_or(0.0)
}

fn source_label(source_id: &str) -> String {
    bound()
        .and_then(|ui| {
            ui.peek()
                .sources
                .iter()
                .find(|item| item.id == source_id)
                .map(super::source_option)
        })
        .unwrap_or_else(|| source_id.to_owned())
}

async fn run_capabilities(job: CapabilityJob, secret: Option<String>) -> CapabilityOutcome {
    let CapabilityJob::Run {
        generation,
        source_id,
    } = job
    else {
        return CapabilityOutcome::Idle;
    };
    let semantic_reply = post_with_abort(
        semantic::semantic_capabilities_command(),
        secret.clone(),
        generation,
    )
    .await;
    let decision_reply = post_with_abort(
        semantic::decision_capabilities_command(),
        secret.clone(),
        generation,
    )
    .await;
    let (semantic, provenance) = match semantic_reply {
        Ok(reply) => match semantic::interpret_semantic_capabilities(&reply, secret.as_deref()) {
            Ok((pick, registered)) => (registered, pick),
            Err(fault) => {
                return CapabilityOutcome::Failed {
                    generation,
                    message: fault_text(&fault),
                };
            }
        },
        Err(message) => {
            return CapabilityOutcome::Failed {
                generation,
                message,
            };
        }
    };
    let decisions = decision_reply
        .ok()
        .and_then(|reply| semantic::interpret_decision_registered(&reply, secret.as_deref()).ok())
        .unwrap_or(false);
    let coverage = if semantic {
        let Some(pick) = provenance.as_ref() else {
            return CapabilityOutcome::Failed {
                generation,
                message: "Semantic search is registered but active provenance is missing."
                    .to_owned(),
            };
        };
        if let (Some(start), Some(end)) =
            coverage_bounds(&source_id, search_window, retained_search_window)
        {
            let command =
                semantic::coverage_command(&source_id, Some((start.as_str(), end.as_str())), pick);
            post_with_abort(command, secret.clone(), generation)
                .await
                .ok()
                .and_then(|reply| semantic::interpret_coverage(&reply, secret.as_deref()).ok())
                .map(|(_, banner)| banner)
        } else {
            None
        }
    } else {
        None
    };
    CapabilityOutcome::Ready {
        generation,
        semantic,
        decisions,
        provenance,
        coverage,
    }
}

async fn run_history(job: HistoryJob, secret: Option<String>) -> HistoryOutcome {
    let HistoryJob::Run { generation, index } = job else {
        return HistoryOutcome::Idle;
    };
    let Some(ui) = bound() else {
        return HistoryOutcome::Failed {
            generation,
            index,
            message: "Decision history could not run.".to_owned(),
        };
    };
    let state = ui.peek().clone();
    let Some(hit) = state.semantic.hits.get(index) else {
        return HistoryOutcome::Failed {
            generation,
            index,
            message: "The frame is no longer selected.".to_owned(),
        };
    };
    let (start, end) = match search_window(&state) {
        Some(bounds) => bounds,
        None => {
            return HistoryOutcome::Failed {
                generation,
                index,
                message: "Enter a time range.".to_owned(),
            };
        }
    };
    let source_id = if hit.source_id.is_empty() {
        state.reader.source_id.clone()
    } else {
        hit.source_id.clone()
    };
    let command = semantic::history_command(&source_id, &start, &end, &hit.command_frame);
    match post_with_abort(command, secret.clone(), generation).await {
        Ok(reply) => match semantic::interpret_history(&reply, secret.as_deref()) {
            Ok(rows) => HistoryOutcome::Ready {
                generation,
                index,
                rows,
            },
            Err(fault) => HistoryOutcome::Failed {
                generation,
                index,
                message: fault_text(&fault),
            },
        },
        Err(message) => HistoryOutcome::Failed {
            generation,
            index,
            message,
        },
    }
}

fn apply_search(mut ui: Signal<VideoUi>, outcome: SearchOutcome) {
    match outcome {
        SearchOutcome::Idle => {}
        SearchOutcome::Ready {
            ticket,
            hits,
            incomplete,
        } => {
            let mut next = ui.peek().clone();
            if !next
                .semantic
                .accepts_search(&ticket, hits, incomplete, None)
            {
                return;
            }
            let selected = next.semantic.selected;
            ui.set(next);
            if selected < ui.peek().semantic.hits.len() {
                launch_thumb(selected);
            }
        }
        SearchOutcome::Failed { ticket, message } => {
            let mut next = ui.peek().clone();
            if next
                .semantic
                .accepts_search(&ticket, Vec::new(), false, Some(message))
            {
                ui.set(next);
            }
        }
    }
}

fn apply_capabilities(mut ui: Signal<VideoUi>, outcome: CapabilityOutcome) {
    match outcome {
        CapabilityOutcome::Idle => {}
        CapabilityOutcome::Ready {
            generation,
            semantic,
            decisions,
            provenance,
            coverage,
        } => {
            let mut next = ui.peek().clone();
            if next.semantic.generation != generation {
                return;
            }
            next.semantic.search_capabilities.phase = if semantic {
                ApiPhase::Ready
            } else {
                ApiPhase::Absent
            };
            next.semantic.decision_capabilities.phase = if decisions {
                ApiPhase::Ready
            } else {
                ApiPhase::Absent
            };
            next.semantic.provenance = provenance;
            next.semantic.coverage = coverage;
            ui.set(next);
        }
        CapabilityOutcome::Failed {
            generation,
            message,
        } => {
            let mut next = ui.peek().clone();
            if next.semantic.generation != generation {
                return;
            }
            next.semantic.search_capabilities.phase = ApiPhase::Error;
            next.semantic.search_capabilities.alert = message;
            ui.set(next);
        }
    }
}

fn apply_history(mut ui: Signal<VideoUi>, outcome: HistoryOutcome) {
    match outcome {
        HistoryOutcome::Idle => {}
        HistoryOutcome::Ready {
            generation,
            index,
            rows,
        } => {
            let mut next = ui.peek().clone();
            if next.semantic.generation != generation || next.semantic.selected != index {
                return;
            }
            next.semantic.decisions = rows;
            next.semantic.decision_phase = ApiPhase::Ready;
            next.semantic.decision_alert.clear();
            ui.set(next);
        }
        HistoryOutcome::Failed {
            generation,
            index,
            message,
        } => {
            let mut next = ui.peek().clone();
            if next.semantic.generation != generation || next.semantic.selected != index {
                return;
            }
            next.semantic.decisions.clear();
            next.semantic.decision_phase = ApiPhase::Error;
            next.semantic.decision_alert = message;
            ui.set(next);
        }
    }
}

pub use semantic_transport::abort_semantic_commands;
