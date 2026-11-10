//! Semantic search and frame decision panels inside the Videos reader.

use std::cell::RefCell;

use dioxus::prelude::*;
use web_sys::AbortController;

use crate::inventory::Fault;
use crate::shell::session::{self, calls_allowed, message_id, take_secret};
use crate::videos;
use crate::videos::semantic::{
    self, ApiPhase, CoverageBanner, DecisionRow, HitThumb, ProvenancePick, SearchHit, SearchTicket,
};

use super::{VideoUi, bound};

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
    Run { generation: u32, index: usize },
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

struct SemanticFlight {
    generation: u32,
    abort: AbortController,
}

#[derive(Clone, Copy)]
struct Loader {
    search: Signal<SearchJob>,
    caps: Signal<CapabilityJob>,
    history: Signal<HistoryJob>,
}

thread_local! {
    static LOADER: RefCell<Option<Loader>> = const { RefCell::new(None) };
    static FLIGHTS: RefCell<Option<SemanticFlight>> = const { RefCell::new(None) };
    static PROBED_SOURCE: RefCell<String> = const { RefCell::new(String::new()) };
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
    let repeat = PROBED_SOURCE.with(|slot| slot.borrow().as_str() == source_id);
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
    ui.write().semantic.query.clear();
    PROBED_SOURCE.with(|slot| *slot.borrow_mut() = source_id.to_owned());
    launch_capabilities(cap_generation, source_id.to_owned());
}

pub fn commit_semantic_query() {
    let Some(mut ui) = bound() else {
        return;
    };
    let text = session::control_value("video-semantic-query");
    let state = ui.peek().clone();
    let (start, end) = match search_window(&state) {
        Some(bounds) => bounds,
        None => {
            ui.write().semantic.search_fault = "Enter a retained time range first.".to_owned();
            return;
        }
    };
    ui.write().semantic.query = text.clone();
    let ticket = ui
        .write()
        .semantic
        .begin_search(&state.reader.source_id, &text, &start, &end);
    if let Some(ticket) = ticket {
        launch_search(ticket);
    }
}

pub fn choose_semantic_hit(index: usize) {
    let Some(mut ui) = bound() else {
        return;
    };
    if !ui.write().semantic.select(index) {
        return;
    }
    let state = ui.peek().clone();
    let Some(hit) = state.semantic.selected_hit() else {
        return;
    };
    if let Some(us) = (hit.captured_us > 0).then_some(hit.captured_us) {
        super::timeline::commit_at(us);
    }
    if state.semantic.decisions_enabled() {
        launch_history(state.semantic.generation, index);
    }
    launch_thumb(index);
}

#[component]
pub fn SemanticFilters(state: VideoUi) -> Element {
    let hidden = state.semantic.search_capabilities.phase == ApiPhase::Unknown;
    let incomplete = state.semantic.incomplete_indexing
        || state
            .semantic
            .coverage
            .as_ref()
            .is_some_and(|banner| banner.incomplete);
    rsx! {
        div {
            id: "video-semantic-filters",
            class: "video-filter-group video-semantic-filters",
            hidden: hidden,
            role: "group",
            aria_label: "Search",
            p { id: "video-semantic-label", "Search" }
            p {
                id: "video-semantic-capability",
                role: "status",
                aria_live: "polite",
                "data-state": capability_slug(state.semantic.search_capabilities.phase),
                match state.semantic.search_capabilities.phase {
                    ApiPhase::Absent => "Search is not configured.",
                    ApiPhase::Error => "Search capabilities could not be read.",
                    ApiPhase::Ready => "Search is available for this source.",
                    ApiPhase::Unknown => "Discovering search capabilities…",
                }
            }
            if state.semantic.search_capabilities.phase == ApiPhase::Ready {
            label { r#for: "video-semantic-query", "Describe a frame" }
            input {
                id: "video-semantic-query",
                r#type: "search",
                autocomplete: "off",
                spellcheck: "false",
                "aria-describedby": "video-semantic-hint",
                value: "{state.semantic.query}",
                oninput: move |_| {
                    if let Some(mut ui) = bound() {
                        ui.write().semantic.query = session::control_value("video-semantic-query");
                    }
                },
                onkeydown: semantic_enter,
            }
            p { id: "video-semantic-hint", class: "video-semantic-hint",
                "Descriptions are Luna search metadata only; they are not lab decisions."
            }
            button {
                id: "video-semantic-submit",
                class: "video-outline",
                r#type: "button",
                disabled: state.semantic.search_loading,
                onclick: move |_| commit_semantic_query(),
                "Search"
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
            div { id: "video-semantic-alert", role: "alert", aria_live: "assertive", "{state.semantic.search_fault}" }
            }
        }
    }
}

#[component]
pub fn SemanticResults(state: VideoUi) -> Element {
    if state.semantic.search_capabilities.phase != ApiPhase::Ready {
        return rsx! {};
    }
    let selected = state.semantic.selected;
    rsx! {
        div { id: "video-semantic-results", class: "video-semantic-results",
            p {
                id: "video-semantic-status",
                role: "status",
                aria_live: "polite",
                "data-loading": if state.semantic.search_loading { "true" } else { "false" },
                if state.semantic.search_loading {
                    "Searching…"
                } else if state.semantic.hits.is_empty() && state.semantic.query.is_empty() {
                    "Enter text to search indexed frames."
                } else if state.semantic.hits.is_empty() {
                    "No hits in this scope."
                } else {
                    "Select a hit to inspect metadata and decisions."
                }
            }
            div {
                id: "video-semantic-hits",
                class: "video-semantic-hits",
                role: "listbox",
                aria_label: "Search hits",
                for (index, hit) in state.semantic.hits.iter().enumerate() {
                    SemanticHitRow {
                        key: "{hit.frame_id}",
                        index,
                        hit: hit.clone(),
                        selected: selected == index,
                    }
                }
            }
            if let Some(hit) = state.semantic.selected_hit() {
                SemanticDetail {
                    hit: hit.clone(),
                    state: state.clone(),
                }
            }
        }
    }
}

#[component]
fn SemanticHitRow(index: usize, hit: SearchHit, selected: bool) -> Element {
    let blocked = hit.media_blocked();
    rsx! {
        button {
            id: "video-semantic-hit-{index}",
            class: "video-semantic-hit",
            r#type: "button",
            role: "option",
            aria_selected: if selected { "true" } else { "false" },
            "data-index-state": "{hit.index_state}",
            "data-media-state": "{hit.media_state}",
            disabled: blocked,
            onclick: move |_| choose_semantic_hit(index),
            if let Some(thumb) = hit.thumb.as_ref() {
                SemanticThumb { thumb: thumb.clone() }
            }
            span { class: "video-semantic-hit-copy",
                time { datetime: "{hit.captured_at}", "{hit.captured_at}" }
                span { class: "video-semantic-score", "Score {hit.retrieval_score}" }
                span { class: "video-semantic-description", "{hit.description}" }
            }
        }
    }
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

#[component]
fn SemanticDetail(hit: SearchHit, state: VideoUi) -> Element {
    rsx! {
        section {
            id: "video-semantic-detail",
            class: "video-semantic-detail",
            aria_label: "Semantic frame detail",
            div { id: "video-semantic-metadata", class: "video-metadata-panel",
                h3 { "Search metadata (Luna)" }
                p { class: "video-semantic-note",
                    "Generated description for retrieval only — not a verified lab decision."
                }
                dl {
                    dt { "Description" } dd { "{hit.description}" }
                    dt { "Model" } dd { "{hit.description_model}" }
                    dt { "Provider" } dd { "{hit.description_provider}" }
                    dt { "Retrieval score" } dd { "{hit.retrieval_score}" }
                    dt { "Index state" }
                    dd {
                        span { id: "video-semantic-index-state", "data-state": "{hit.index_state}", "{hit.index_state}" }
                    }
                    dt { "Media" }
                    dd {
                        span { id: "video-semantic-media-state", "data-state": "{hit.media_state}", "{hit.media_state}" }
                        if !hit.media_reason.is_empty() {
                            span { " — {hit.media_reason}" }
                        }
                    }
                }
            }
            if state.semantic.decisions_enabled() {
                div { id: "video-decision-history", class: "video-decision-panel",
                    h3 { "Frame decisions (Jev)" }
                    p { class: "video-decision-note",
                        "Authoritative predicate assessments from typesafe-jev only."
                    }
                    if state.semantic.decision_alert.is_empty() {
                        if state.semantic.decisions.is_empty() {
                            p {
                                id: "video-decision-empty",
                                role: "status",
                                "data-state": decision_empty_slug(&state),
                                "{decision_empty_copy(&state)}"
                            }
                        } else {
                            for (index, row) in state.semantic.decisions.iter().enumerate() {
                                DecisionRowView { key: "{row.decision_id}", index, row: row.clone() }
                            }
                        }
                    } else {
                        div { id: "video-decision-alert", role: "alert", "{state.semantic.decision_alert}" }
                    }
                }
            }
        }
    }
}

#[component]
fn DecisionRowView(index: usize, row: DecisionRow) -> Element {
    rsx! {
        article {
            id: "video-decision-row-{index}",
            class: "video-decision-row",
            "data-outcome": "{row.outcome}",
            "data-coverage": "{row.coverage}",
            "data-evidence": "{row.evidence_state}",
            dl {
                dt { "Predicate" } dd { "{row.predicate}" }
                dt { "Subject" } dd { "{row.subject}" }
                dt { "Outcome" } dd { "{row.outcome}" }
                dt { "Evaluation" } dd { "{row.coverage}" }
                dt { "Producer" } dd { "{row.producer} @ {row.model_revision}" }
                dt { "Probability" } dd { "{row.probability}" }
                dt { "Confidence" } dd { "{row.confidence}" }
                dt { "ROI" } dd { "{row.roi}" }
                dt { "Evaluated" } dd { time { datetime: "{row.evaluated_at}", "{row.evaluated_at}" } }
                dt { "Evidence" } dd { "{row.evidence_state}" }
            }
        }
    }
}

fn decision_empty_copy(state: &VideoUi) -> &'static str {
    match state.semantic.decision_phase {
        ApiPhase::Unknown => "Loading decision history…",
        ApiPhase::Ready => "No Jev decisions recorded for this frame.",
        ApiPhase::Absent => "Decision history is not configured.",
        ApiPhase::Error => "Decision history could not be read.",
    }
}

fn decision_empty_slug(state: &VideoUi) -> &'static str {
    match state.semantic.decision_phase {
        ApiPhase::Unknown => "loading",
        ApiPhase::Ready => "empty",
        ApiPhase::Absent => "absent",
        ApiPhase::Error => "error",
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
    if let Some(bounds) = explicit_semantic_range(state) {
        return Some(bounds);
    }
    state
        .reader
        .timeline
        .as_ref()
        .and_then(retained_search_window)
}

fn explicit_semantic_range(state: &VideoUi) -> Option<(String, String)> {
    let start_raw = session::control_value("video-range-start");
    let end_raw = session::control_value("video-range-end");
    if start_raw.trim().is_empty() || end_raw.trim().is_empty() {
        return None;
    }
    let start = super::field_to_utc(&start_raw, state.local_time);
    let end = super::field_to_utc(&end_raw, state.local_time);
    videos::check_range(&start, &end)
        .ok()
        .map(|range| (range.start, range.end))
}

fn retained_search_window(timeline: &videos::Timeline) -> Option<(String, String)> {
    let first = timeline.intervals.first()?;
    let last = timeline.intervals.last()?;
    Some((first.start.clone(), last.end.clone()))
}

fn loader() -> Option<Loader> {
    LOADER.with(|slot| *slot.borrow())
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
    let source_id = state.reader.source_id.clone();
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
    let provenance = bound().and_then(|ui| {
        ui.peek()
            .semantic
            .provenance
            .clone()
            .or_else(default_provenance)
    });
    let Some(provenance) = provenance else {
        return SearchOutcome::Failed {
            ticket,
            message: "Semantic provenance is not ready.".to_owned(),
        };
    };
    let command = semantic::search_command(
        &ticket.source_id,
        &ticket.start,
        &ticket.end,
        &ticket.text,
        &provenance,
    );
    match post_with_abort(command, secret.clone(), ticket.generation).await {
        Ok(reply) => match semantic::interpret_search(&reply, secret.as_deref()) {
            Ok((hits, incomplete)) => SearchOutcome::Ready {
                ticket,
                hits,
                incomplete,
            },
            Err(fault) => SearchOutcome::Failed {
                ticket,
                message: fault_text(&fault),
            },
        },
        Err(message) => SearchOutcome::Failed { ticket, message },
    }
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
    let semantic = semantic_reply
        .ok()
        .and_then(|reply| semantic::interpret_semantic_registered(&reply, secret.as_deref()).ok())
        .unwrap_or(false);
    let decisions = decision_reply
        .ok()
        .and_then(|reply| semantic::interpret_decision_registered(&reply, secret.as_deref()).ok())
        .unwrap_or(false);
    let (provenance, coverage) = if semantic {
        if let (Some(start), Some(end)) = coverage_bounds(&source_id)
            && let Some(pick) = default_provenance()
        {
            let command = semantic::coverage_command(&source_id, &start, &end, &pick);
            if let Ok(reply) = post_with_abort(command, secret.clone(), generation).await
                && let Ok((resolved, banner)) =
                    semantic::interpret_coverage(&reply, secret.as_deref())
            {
                return CapabilityOutcome::Ready {
                    generation,
                    semantic,
                    decisions,
                    provenance: Some(resolved),
                    coverage: Some(banner),
                };
            }
        }
        (default_provenance(), None)
    } else {
        (None, None)
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
    let command =
        semantic::history_command(&state.reader.source_id, &start, &end, &hit.command_frame);
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
            let generation = next.semantic.generation;
            let decisions_enabled = next.semantic.decisions_enabled();
            ui.set(next);
            if selected < ui.peek().semantic.hits.len() {
                launch_thumb(selected);
                if decisions_enabled {
                    launch_history(generation, selected);
                }
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

async fn post_with_abort(
    command: serde_json::Value,
    secret: Option<String>,
    generation: u32,
) -> Result<crate::codec::Reply, String> {
    let Some(signal) = begin_semantic_command(generation) else {
        return Err("The semantic request was canceled.".to_owned());
    };
    let Some(transport) = session::transport() else {
        return Err("The browser could not reach this origin.".to_owned());
    };
    let Ok(body) = crate::codec::command_message(&message_id(), &command, "") else {
        return Err("The semantic query could not be built.".to_owned());
    };
    match transport
        .send_signaled(&body, secret.as_deref(), Some(&signal))
        .await
    {
        Err(()) => Err("The browser could not reach this origin.".to_owned()),
        Ok(reply) => Ok(reply),
    }
}

fn begin_semantic_command(generation: u32) -> Option<web_sys::AbortSignal> {
    let abort = AbortController::new().ok()?;
    let signal = abort.signal();
    let flight = SemanticFlight { generation, abort };
    let previous = FLIGHTS.with(|slot| {
        let mut current = slot.borrow_mut();
        if current
            .as_ref()
            .is_some_and(|existing| existing.generation > generation)
        {
            return Err(flight);
        }
        Ok(current.replace(flight))
    });
    match previous {
        Ok(previous) => {
            if let Some(previous) = previous {
                previous.abort.abort();
            }
            Some(signal)
        }
        Err(_flight) => None,
    }
}

pub fn abort_semantic_commands() {
    FLIGHTS.with(|slot| {
        if let Some(flight) = slot.borrow_mut().take() {
            flight.abort.abort();
        }
    });
}

fn default_provenance() -> Option<ProvenancePick> {
    Some(ProvenancePick {
        model_space: serde_json::json!({
            "provider": "bedrock",
            "model": "text-embed-v1",
            "dimensions": 4,
            "normalization": "l2_unit",
            "metric": "cosine",
            "generation": 1
        }),
        description_generation: 1,
    })
}

fn coverage_bounds(source_id: &str) -> (Option<String>, Option<String>) {
    let Some(ui) = bound() else {
        return (None, None);
    };
    let state = ui.peek().clone();
    if let Some((start, end)) = search_window(&state) {
        return (Some(start), Some(end));
    }
    let Some((start, end)) = state
        .reader
        .timeline
        .as_ref()
        .and_then(retained_search_window)
    else {
        let _ = source_id;
        return (None, None);
    };
    (Some(start), Some(end))
}

fn fault_text(fault: &Fault) -> String {
    match fault {
        Fault::Unauthorized(message) | Fault::Forbidden(message) => message.clone(),
        Fault::Unavailable(message) => message.clone(),
        Fault::Malformed | Fault::Transport => "The response could not be read.".to_owned(),
    }
}
