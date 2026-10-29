//! Videos reader. One viewer follows the newest point or plays a bounded clip.

use std::cell::{Cell, RefCell};

use crate::images::{self, SourcePhase};
use crate::inventory::{Fault, Item, Primitive};
use crate::shell::playback::{self, StreamOutcome, StreamTicket};
use crate::shell::session::{self, calls_allowed, message_id, take_secret};
use crate::videos::track::{self, Track, Window};
use crate::videos::{
    self, AutoPlayback, ClipHandle, Failure, FrameAtTime, Kind, LiveHandle, Playback, Selection,
    Ticket, Timeline,
};
use dioxus::core::{Runtime, RuntimeGuard, ScopeId, current_scope_id};
use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::AbortController;

mod timeline;
use timeline::{
    abort_commands, alert_text, band_style, bands, begin_command, commit_at, commit_typed_range,
    display_instant, displayed_window, ensure_timeline, field_to_utc, follow_newest, frame_cursor,
    frame_cursor_now, newest_style, playhead_style, replay_retained_start, reselect_range,
    selection_slug, slider_now, slider_text, timeline_click, timeline_down, timeline_key,
    timeline_move, timeline_up,
};

#[derive(Clone, PartialEq)]
pub struct VideoUi {
    pub reader: videos::Reader,
    pub sources: Vec<Item>,
    pub phase: SourcePhase,
    pub source_alert: String,
    pub draft: Option<Window>,
    pub range_start: String,
    pub range_end: String,
}

impl VideoUi {
    pub fn new() -> Self {
        Self {
            reader: videos::Reader::new(),
            sources: Vec::new(),
            phase: SourcePhase::Loading,
            source_alert: String::new(),
            draft: None,
            range_start: String::new(),
            range_end: String::new(),
        }
    }
}

#[derive(Clone, PartialEq)]
enum ClipJob {
    Idle,
    Run(Ticket),
}

#[derive(Clone, PartialEq)]
enum ClipOutcome {
    Idle,
    Ready { ticket: Ticket, clip: ClipHandle },
    Failed { ticket: Ticket, message: String },
    Coded { ticket: Ticket, failure: Failure },
}

#[derive(Clone, PartialEq)]
enum FrameJob {
    Idle,
    Run(Ticket),
}

#[derive(Clone, PartialEq)]
enum FrameOutcome {
    Idle,
    Ready { ticket: Ticket, frame: FrameAtTime },
    Failed { ticket: Ticket, message: String },
    Coded { ticket: Ticket, failure: Failure },
}

enum MetaOutcome {
    Timeline { ticket: Ticket, timeline: Timeline },
    Follow { ticket: Ticket, follow: LiveHandle },
    Failed { ticket: Ticket, message: String },
    Coded { ticket: Ticket, failure: Failure },
}

#[derive(Clone, Copy)]
struct Loader {
    clip: Signal<ClipJob>,
    frame_at_time: Signal<FrameJob>,
}

struct CommandFlight {
    generation: u32,
    abort: AbortController,
}

#[derive(Clone, Copy)]
struct Drag {
    x: f64,
    at_us: i64,
}

thread_local! {
    static UI: RefCell<Option<Signal<VideoUi>>> = const { RefCell::new(None) };
    static LOADER: RefCell<Option<Loader>> = const { RefCell::new(None) };
    static RUNTIME: RefCell<Option<std::rc::Rc<Runtime>>> = const { RefCell::new(None) };
    static SCOPE: RefCell<Option<ScopeId>> = const { RefCell::new(None) };
    static COMMANDS: RefCell<Option<CommandFlight>> = const { RefCell::new(None) };
    static DRAG: RefCell<Option<Drag>> = const { RefCell::new(None) };
    static SUPPRESS_CLICK: Cell<bool> = const { Cell::new(false) };
    static REPLAY: Cell<bool> = const { Cell::new(false) };
    static REFRESH: Cell<Option<i32>> = const { Cell::new(None) };
}

pub fn bind(ui: Signal<VideoUi>) {
    UI.with(|slot| *slot.borrow_mut() = Some(ui));
}

pub fn catalog_changed() {
    let snap = super::inventory::snapshot(Primitive::Videos);
    let ids = snap
        .items
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let phase = videos::source_phase(snap.kind, ids.len());
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    next.sources = snap.items;
    next.phase = phase;
    next.source_alert = if phase == SourcePhase::Error {
        snap.alert
    } else {
        String::new()
    };
    let generation = next.reader.generation;
    next.reader.note_catalog(phase, &ids);
    if phase == SourcePhase::Ready {
        for item in &next.sources {
            if let Some(capability) = source_capability(item) {
                next.reader.note_capability(capability);
            }
        }
    }
    if next.reader.generation != generation {
        next.draft = None;
        playback::abort();
        abort_commands();
    }
    ui.set(next);
    ensure_timeline();
}

fn bound() -> Option<Signal<VideoUi>> {
    UI.with(|slot| *slot.borrow())
}

fn loader() -> Option<Loader> {
    LOADER.with(|slot| *slot.borrow())
}

#[component]
pub fn VideoView(ui: Signal<VideoUi>, visible: bool) -> Element {
    let clip = use_signal(|| ClipJob::Idle);
    let frame_at_time = use_signal(|| FrameJob::Idle);
    use_hook(|| {
        RUNTIME.with(|slot| *slot.borrow_mut() = Some(Runtime::current()));
        SCOPE.with(|slot| *slot.borrow_mut() = Some(current_scope_id()));
        playback::bind(on_playing, on_frame, on_gap_skipped);
        LOADER.with(|slot| {
            *slot.borrow_mut() = Some(Loader {
                clip,
                frame_at_time,
            })
        });
    });
    use_drop(stop_playback);
    let mut shown = use_signal(|| false);
    if shown() != visible {
        shown.set(visible);
    }
    use_effect(move || {
        if shown() {
            playback::bind_media(native_play, native_pause);
            ensure_timeline();
            start_coverage_refresh();
        } else {
            stop_playback();
        }
    });
    let clip_reply = use_resource(move || {
        let job = clip();
        let secret = session::native_secret();
        async move { run_clip(job, secret).await }
    });
    let frame_reply = use_resource(move || {
        let job = frame_at_time();
        let secret = session::native_secret();
        async move { run_frame_at_time(job, secret).await }
    });
    let _ = clip_reply.read();
    let _ = frame_reply.read();
    if let Some(outcome) = clip_reply() {
        apply_clip(ui, outcome);
    }
    if let Some(outcome) = frame_reply() {
        apply_frame(ui, outcome);
    }
    let state = ui();
    let busy = state.phase == SourcePhase::Loading
        || state.reader.loading
        || matches!(
            state.reader.playback,
            Playback::Connecting | Playback::Playing
        );
    let status = videos::playback_status(&state.reader);
    let alert = alert_text(&state.reader);
    rsx! {
        section {
            id: "video-view",
            aria_labelledby: "video-view-heading",
            aria_busy: if busy { "true" } else { "false" },
            "data-gen": state.reader.generation.to_string(),
            h2 { id: "video-view-heading", class: "visually-hidden", tabindex: "-1", "Videos" }
            div { id: "video-browser", class: "log-browser",
                FilterRail { state: state.clone() }
                Results { state: state.clone(), status, alert }
            }
        }
    }
}

#[component]
fn FilterRail(state: VideoUi) -> Element {
    rsx! {
        div { id: "video-filters", role: "group", aria_label: "Video filters",
            div { id: "video-sources", class: "log-filter-group",
                label { r#for: "video-source", "Source" }
                p {
                    id: "video-source-status",
                    role: "status",
                    aria_live: "polite",
                    "data-state": source_slug(state.phase),
                    "{videos::source_copy(state.phase)}"
                }
                div { id: "video-source-alert", role: "alert", aria_live: "assertive", "{state.source_alert}" }
                select {
                    id: "video-source",
                    disabled: state.sources.is_empty(),
                    onchange: move |_| choose_listed_source(),
                    if state.sources.is_empty() {
                        option { value: "", "{images::source_placeholder(state.phase)}" }
                    }
                    for item in state.sources.iter() {
                        option {
                            key: "{item.id}",
                            value: "{item.id}",
                            selected: state.reader.source_id == item.id,
                            "{source_option(item)}"
                        }
                    }
                }
            }
            if !state.reader.source_id.is_empty() {
                VideoDetails { details: state.reader.details() }
            }
        }
    }
}

#[component]
fn Results(state: VideoUi, status: String, alert: String) -> Element {
    use_effect(move || {
        playback::bind_media(native_play, native_pause);
    });
    let media = state.reader.has_media();
    let show_empty = !media
        && !state.reader.loading
        && state.reader.playback != Playback::Connecting
        && alert.is_empty();
    let track = state.reader.timeline.as_ref().map(track::coverage_track);
    let window = displayed_window(&state);
    let bars = track.as_ref().map(bands).unwrap_or_default();
    let selection_style = track
        .as_ref()
        .zip(window)
        .map(|(item, selected)| band_style(item, selected.start_us, selected.end_us));
    let newest_style = newest_style(track.as_ref(), state.reader.timeline.as_ref());
    let frame_ready = frame_cursor(&state).is_some();
    let follow = videos::live_button_label(&state.reader);
    let playhead_style = playhead_style(&state);
    let failure = state.reader.failure.map(|item| item.code()).unwrap_or("");
    let ready = state.reader.timeline.is_some();
    let empty_track = track.as_ref().is_some_and(|item| item.spans.is_empty());
    let value_now = slider_now(track.as_ref(), window);
    let value_max = track.as_ref().map(Track::span_us).unwrap_or(0);
    let available_start_utc = track
        .as_ref()
        .filter(|item| item.span_us() > 0)
        .map(|item| track::format_utc(item.origin_us));
    let available_end_utc = track
        .as_ref()
        .filter(|item| item.span_us() > 0)
        .map(|item| track::format_utc(item.end_us));
    let available_start = available_start_utc.as_deref().map(display_instant);
    let available_end = available_end_utc.as_deref().map(display_instant);
    let value_text = match (window, available_start.as_deref(), available_end.as_deref()) {
        (None, Some(start), Some(end)) => format!("Available from {start} to {end}"),
        _ => slider_text(window),
    };
    let duration = window.map(Window::duration_us).unwrap_or(0);
    let draft_start = window
        .map(|item| track::format_utc(item.start_us))
        .unwrap_or_default();
    let draft_end = window
        .map(|item| track::format_utc(item.end_us))
        .unwrap_or_default();
    rsx! {
        div { id: "video-results", class: "log-results",
            div { class: "log-scroll",
                div { id: "video-stage", class: "video-stage",
                    div { class: "video-aspect",
                        canvas {
                            id: "video-canvas",
                            class: "visually-hidden",
                            width: "640",
                            height: "360",
                            "aria-hidden": "true",
                        }
                        if show_empty {
                            p { id: "video-empty", "Select a source and a time." }
                        }
                        video {
                            id: "video-player",
                            class: "video-player",
                            controls: media,
                            hidden: !media,
                            muted: true,
                            playsinline: true,
                            preload: "none",
                            "aria-label": "Video playback",
                            onmounted: move |_| {
                                playback::bind_media(native_play, native_pause);
                                playback::play_element();
                            },
                            onplay: move |_| native_play(),
                            onpause: move |_| native_pause(),
                        }
                    }
                }
                p {
                    id: "video-status",
                    role: "status",
                    aria_live: "polite",
                    "data-state": playback_slug(state.reader.playback),
                    "data-frame-time": "{state.reader.frame_time}",
                    "{status}"
                }
                div {
                    id: "video-alert",
                    role: "alert",
                    aria_live: "assertive",
                    "data-failure": "{failure}",
                    if !alert.is_empty() {
                        span { id: "video-alert-text", "{alert}" }
                    }
                    if state.reader.can_retry() {
                        button {
                            id: "video-retry",
                            class: "log-outline",
                            r#type: "button",
                            onclick: move |_| retry(),
                            "Retry"
                        }
                    }
                    if state.reader.can_reselect() {
                        button {
                            id: "video-reselect",
                            class: "log-outline",
                            r#type: "button",
                            onclick: move |_| reselect_range(),
                            "Choose another range"
                        }
                    }
                }
                if let (Some(start_utc), Some(end_utc), Some(start), Some(end)) = (
                    available_start_utc.as_ref(),
                    available_end_utc.as_ref(),
                    available_start.as_ref(),
                    available_end.as_ref(),
                ) {
                    div { id: "video-available", class: "video-available",
                        time { id: "video-available-start", datetime: "{start_utc}", "{start}" }
                        span { class: "video-available-label", "Available" }
                        time { id: "video-available-end", datetime: "{end_utc}", "{end}" }
                    }
                }
                div {
                    id: "video-timeline",
                    class: "video-timeline",
                    role: "slider",
                    tabindex: "0",
                    aria_label: "Recording availability",
                    aria_orientation: "horizontal",
                    aria_valuemin: "0",
                    aria_valuemax: "{value_max}",
                    aria_valuenow: "{value_now}",
                    aria_valuetext: "{value_text}",
                    "data-ready": if ready { "true" } else { "false" },
                    "data-empty": if empty_track { "true" } else { "false" },
                    "data-max-us": "{videos::MAX_CLIP_US}",
                    "data-duration-us": "{duration}",
                    "data-start": "{draft_start}",
                    "data-end": "{draft_end}",
                    "data-selection": selection_slug(&state.reader.selection),
                    onkeydown: timeline_key,
                    onpointerdown: timeline_down,
                    onpointermove: timeline_move,
                    onpointerup: timeline_up,
                    onclick: timeline_click,
                    if empty_track {
                        p { id: "video-timeline-empty", "No retained video." }
                    }
                    if !bars.is_empty() {
                        div { class: "video-track", aria_hidden: "true",
                            for band in bars {
                                BandMark { key: "{band.id}-{band.style}", band: band.clone() }
                            }
                            if let Some(style) = selection_style.clone() {
                                Placed { key: "selection-{style}", id: "video-selection".to_owned(), class: "video-selection", style }
                            }
                            if let Some(style) = newest_style.clone() {
                                Placed { key: "newest-{style}", id: "video-newest".to_owned(), class: "video-newest", style }
                            }
                            if let Some(style) = playhead_style.clone() {
                                Placed { key: "playhead-{style}", id: "video-playhead".to_owned(), class: "video-playhead", style }
                            }
                        }
                    }
                    p { id: "video-timeline-hint", class: "video-timeline-hint",
                        "Click or drag the timeline to scrub recorded video (gaps are skipped). Use LIVE to return to the live edge."
                    }
                }
                div { class: "video-range",
                    div {
                        label { r#for: "video-range-start", "Start" }
                        input {
                            id: "video-range-start",
                            r#type: "text",
                            spellcheck: false,
                            autocomplete: "off",
                            placeholder: "2024-01-01 00:00:00",
                            value: "{display_instant(&state.range_start)}",
                            oninput: move |_| edit_range(true),
                        }
                    }
                    div {
                        label { r#for: "video-range-end", "End" }
                        input {
                            id: "video-range-end",
                            r#type: "text",
                            spellcheck: false,
                            autocomplete: "off",
                            placeholder: "2024-01-01 00:00:02",
                            value: "{display_instant(&state.range_end)}",
                            oninput: move |_| edit_range(false),
                        }
                    }
                }
                div { class: "video-actions",
                    button {
                        id: "video-follow-newest",
                        class: if matches!(state.reader.selection, Selection::Live) { "video-live-active" } else { "" },
                        r#type: "button",
                        disabled: !state.reader.can_return_to_newest(),
                        aria_pressed: if matches!(state.reader.selection, Selection::Live) { "true" } else { "false" },
                        onclick: move |_| follow_newest(),
                        "{follow}"
                    }
                    button {
                        id: "video-frame-at",
                        r#type: "button",
                        disabled: !frame_ready,
                        onclick: move |_| view_frame_at_time(),
                        "{videos::frame_at_time_label()}"
                    }
                }
            }
        }
    }
}

#[component]
fn VideoDetails(details: videos::Details) -> Element {
    let facts = details.facts;
    rsx! {
        if details.open {
            div {
                id: "video-details",
                class: "video-details",
                role: "region",
                aria_label: "Video details",
                dl {
                    for (index, fact) in facts.iter().enumerate() {
                        dt { key: "video-detail-{index}", "{fact.label}" }
                        dd {
                            "data-label": "{fact.label}",
                            "data-utc": "{fact.value}",
                            "{display_instant(&fact.value)}"
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, PartialEq)]
struct Band {
    id: String,
    class: &'static str,
    kind: &'static str,
    start: String,
    end: String,
    start_us: i64,
    end_us: i64,
    duration_us: i64,
    label: String,
    style: String,
}

#[component]
fn BandMark(band: Band) -> Element {
    let start_us = band.start_us;
    let id = band.id.clone();
    let style = band.style.clone();
    let label = band.label.clone();
    rsx! {
        div {
            id: "{band.id}",
            class: "{band.class}",
            role: "img",
            aria_label: "{label}",
            "data-kind": "{band.kind}",
            "data-start": "{band.start}",
            "data-end": "{band.end}",
            "data-duration-us": "{band.duration_us}",
            title: "{label}",
            onmounted: move |_| place_style(&id, &style),
            onclick: move |event| {
                event.stop_propagation();
                if SUPPRESS_CLICK.with(|cell| cell.replace(false)) {
                    return;
                }
                commit_at(start_us);
            },
        }
    }
}

#[component]
fn Placed(id: String, class: &'static str, style: String) -> Element {
    let placed_id = id.clone();
    let placed_style = style.clone();
    rsx! {
        div {
            id: "{id}",
            class: "{class}",
            onmounted: move |_| place_style(&placed_id, &placed_style),
        }
    }
}

fn place_style(id: &str, style: &str) {
    let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
    else {
        return;
    };
    let Ok(html) = element.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };
    let inline = html.style();
    for part in style.split(';') {
        let Some((name, value)) = part.split_once(':') else {
            continue;
        };
        let _ = inline.set_property(name.trim(), value.trim());
    }
}

fn source_capability(item: &Item) -> Option<videos::SourceCapability> {
    Some(videos::SourceCapability {
        id: item.id.clone(),
        name: item.name.clone(),
        width: item.width?,
        height: item.height?,
        fps: item.fps?,
    })
}

fn source_slug(phase: SourcePhase) -> &'static str {
    match phase {
        SourcePhase::Loading => "loading",
        SourcePhase::None => "sources",
        SourcePhase::Error => "error",
        SourcePhase::Ready => "ready",
    }
}

fn playback_slug(playback: Playback) -> &'static str {
    match playback {
        Playback::Idle => "idle",
        Playback::Connecting => "connecting",
        Playback::Playing => "playing",
        Playback::Paused => "paused",
        Playback::Stopped => "stopped",
    }
}

fn source_option(item: &Item) -> String {
    if item.name.is_empty() {
        item.id.clone()
    } else {
        format!("{} · {}", item.name, item.id)
    }
}

fn edit_range(start: bool) {
    let start_raw = session::control_value("video-range-start");
    let end_raw = session::control_value("video-range-end");
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if start {
        next.range_start = field_to_utc(&start_raw);
    } else {
        next.range_end = field_to_utc(&end_raw);
    }
    ui.set(next);
    if complete_clock(&start_raw) && complete_clock(&end_raw) {
        commit_typed_range();
    }
}

fn complete_clock(value: &str) -> bool {
    let Some(time) = value.trim().split(['T', ' ']).nth(1) else {
        return false;
    };
    let clock = time.split(['.', 'Z', '+']).next().unwrap_or(time);
    let mut parts = clock.split(':');
    let hour = parts.next().unwrap_or("");
    let minute = parts.next().unwrap_or("");
    let second = parts.next().unwrap_or("");
    parts.next().is_none()
        && [hour, minute, second]
            .iter()
            .all(|part| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn choose_listed_source() {
    let id = session::control_value("video-source");
    if !id.is_empty() {
        choose_source(&id);
    }
}

fn choose_source(id: &str) {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if !next.reader.select_source(id) {
        return;
    }
    next.draft = None;
    playback::abort();
    abort_commands();
    ui.set(next);
    playback::pause_element();
    ensure_timeline();
}

fn native_play() {
    run(|| {
        let Some(ui) = bound() else {
            return;
        };
        if ui.peek().reader.playback == Playback::Paused {
            resume();
        }
    });
}

fn native_pause() {
    run(|| {
        if playback::take_ignore_pause() {
            return;
        }
        pause();
    });
}

fn view_frame_at_time() {
    let Some(at) = frame_cursor_now() else {
        return;
    };
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    playback::abort();
    abort_commands();
    playback::set_paused(false);
    let ticket = match next.reader.frame_at_time(&at) {
        Ok(ticket) => ticket,
        Err(_) => {
            ui.set(next);
            return;
        }
    };
    ui.set(next);
    launch_frame(ticket);
}

fn pause() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if next.reader.pause() {
        playback::set_paused(true);
        ui.set(next);
    }
}

fn resume() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if next.reader.resume() {
        playback::set_paused(false);
        ui.set(next);
    }
}

fn stop_playback() {
    playback::abort();
    abort_commands();
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    let active = next.reader.loading
        || next.reader.has_media()
        || matches!(
            next.reader.playback,
            Playback::Connecting | Playback::Playing | Playback::Paused
        );
    if !active {
        return;
    }
    next.reader.stop();
    ui.set(next);
    playback::pause_element();
}

fn retry() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    let Some(ticket) = next.reader.retry() else {
        return;
    };
    ui.set(next);
    match &ticket.kind {
        Kind::Clip { .. } => launch_clip(ticket),
        Kind::FrameAtTime { .. } => launch_frame(ticket),
        Kind::Timeline { .. } | Kind::Follow { .. } => launch_meta(ticket),
    }
}

fn launch_clip(ticket: Ticket) {
    let Some(mut loader) = loader() else {
        return;
    };
    let parts = match &ticket.kind {
        Kind::Clip {
            source_id,
            start,
            end,
        } => vec![source_id.as_str(), start.as_str(), end.as_str()],
        Kind::FrameAtTime { .. } | Kind::Timeline { .. } | Kind::Follow { .. } => Vec::new(),
    };
    if !calls_allowed() || take_secret(&parts).is_err() {
        let Some(mut ui) = bound() else {
            return;
        };
        let mut next = ui.peek().clone();
        let _ = next
            .reader
            .reject(&ticket, "The clip could not be requested.");
        ui.set(next);
        return;
    }
    loader.clip.set(ClipJob::Run(ticket));
}

fn launch_frame(ticket: Ticket) {
    let Some(mut loader) = loader() else {
        return;
    };
    let parts = match &ticket.kind {
        Kind::FrameAtTime { source_id, at } => vec![source_id.as_str(), at.as_str()],
        Kind::Clip { .. } | Kind::Timeline { .. } | Kind::Follow { .. } => Vec::new(),
    };
    if !calls_allowed() || take_secret(&parts).is_err() {
        let Some(mut ui) = bound() else {
            return;
        };
        let mut next = ui.peek().clone();
        let _ = next
            .reader
            .reject(&ticket, "The frame at time could not be requested.");
        ui.set(next);
        return;
    }
    loader.frame_at_time.set(FrameJob::Run(ticket));
}

fn launch_stream(generation: u32, path: String) {
    let secret = session::native_secret();
    if take_secret(&[path.as_str()]).is_err() {
        return;
    }
    playback::play_element();
    wasm_bindgen_futures::spawn_local(async move {
        let outcome = playback::run(StreamTicket { generation }, path, secret).await;
        apply_stream(outcome);
    });
}

async fn run_clip(job: ClipJob, secret: Option<String>) -> ClipOutcome {
    let ClipJob::Run(ticket) = job else {
        return ClipOutcome::Idle;
    };
    let Kind::Clip {
        source_id,
        start,
        end,
    } = &ticket.kind
    else {
        return ClipOutcome::Failed {
            ticket,
            message: "The clip request was invalid.".to_owned(),
        };
    };
    let command = videos::clip_command(source_id, start, end);
    let reply = match post(command, secret.clone(), ticket.generation).await {
        Ok(reply) => reply,
        Err(message) => return ClipOutcome::Failed { ticket, message },
    };
    match videos::interpret_clip(&reply, secret.as_deref()) {
        Ok(clip) => ClipOutcome::Ready { ticket, clip },
        Err(fault) => match videos::stable_failure(&reply.body) {
            Some(failure) => ClipOutcome::Coded { ticket, failure },
            None => ClipOutcome::Failed {
                ticket,
                message: fault_message(&fault),
            },
        },
    }
}

async fn run_frame_at_time(job: FrameJob, secret: Option<String>) -> FrameOutcome {
    let FrameJob::Run(ticket) = job else {
        return FrameOutcome::Idle;
    };
    let Kind::FrameAtTime { source_id, at } = &ticket.kind else {
        return FrameOutcome::Failed {
            ticket,
            message: "The frame at time request was invalid.".to_owned(),
        };
    };
    let command = videos::frame_at_time_command(source_id, at);
    let reply = match post(command, secret.clone(), ticket.generation).await {
        Ok(reply) => reply,
        Err(message) => return FrameOutcome::Failed { ticket, message },
    };
    match videos::interpret_frame_at_time(&reply, secret.as_deref()) {
        Ok(frame) => FrameOutcome::Ready { ticket, frame },
        Err(fault) => match videos::stable_failure(&reply.body) {
            Some(failure) => FrameOutcome::Coded { ticket, failure },
            None => FrameOutcome::Failed {
                ticket,
                message: fault_message(&fault),
            },
        },
    }
}

fn fetch_timeline(done: impl FnOnce(Option<Timeline>) + 'static) {
    let Some(ui) = bound() else {
        done(None);
        return;
    };
    let source_id = ui.peek().reader.source_id.clone();
    if source_id.is_empty() || !calls_allowed() || take_secret(&[source_id.as_str()]).is_err() {
        done(None);
        return;
    }
    wasm_bindgen_futures::spawn_local(async move {
        let secret = session::native_secret();
        let timeline = request_timeline(&source_id, secret).await;
        run(move || {
            if let Some(timeline) = timeline.clone()
                && let Some(mut ui) = bound()
            {
                let mut next = ui.peek().clone();
                if next.reader.install_timeline(timeline) {
                    ui.set(next);
                }
            }
            done(timeline);
        });
    });
}

async fn request_timeline(source_id: &str, secret: Option<String>) -> Option<Timeline> {
    let transport = session::transport()?;
    let command = videos::timeline_command(source_id);
    let body = crate::codec::command_message(&message_id(), &command, "").ok()?;
    let reply = transport
        .send_signaled(&body, secret.as_deref(), None)
        .await
        .ok()?;
    videos::interpret_timeline(&reply, secret.as_deref()).ok()
}

fn start_coverage_refresh() {
    REFRESH.with(|slot| {
        if slot.get().is_some() {
            return;
        }
        let closure = wasm_bindgen::closure::Closure::wrap(Box::new(|| {
            if videos_visible() {
                fetch_timeline(|_| {});
            }
        }) as Box<dyn FnMut()>);
        let id = web_sys::window().and_then(|window| {
            window
                .set_interval_with_callback_and_timeout_and_arguments_0(
                    closure.as_ref().unchecked_ref(),
                    2000,
                )
                .ok()
        });
        closure.forget();
        slot.set(id);
    });
}

fn videos_visible() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("view-videos"))
        .is_some_and(|element| !element.has_attribute("hidden"))
}

fn take_range_replay() -> bool {
    REPLAY.with(|slot| {
        if slot.get() {
            return false;
        }
        slot.set(true);
        true
    })
}

fn clear_range_replay() {
    REPLAY.with(|slot| slot.set(false));
}

async fn post(
    command: serde_json::Value,
    secret: Option<String>,
    generation: u32,
) -> Result<crate::codec::Reply, String> {
    let Some(signal) = begin_command(generation) else {
        return Err("The video request was canceled.".to_owned());
    };
    let Some(transport) = session::transport() else {
        return Err("The browser could not reach this origin.".to_owned());
    };
    let Ok(body) = crate::codec::command_message(&message_id(), &command, "") else {
        return Err("The video query could not be built.".to_owned());
    };
    match transport
        .send_signaled(&body, secret.as_deref(), Some(&signal))
        .await
    {
        Err(()) => Err("The browser could not reach this origin.".to_owned()),
        Ok(reply) => Ok(reply),
    }
}

fn apply_clip(mut ui: Signal<VideoUi>, outcome: ClipOutcome) {
    let mut next = ui.peek().clone();
    let path = match outcome {
        ClipOutcome::Idle => return,
        ClipOutcome::Ready { ticket, clip } => {
            let path = clip.path.clone();
            if !next.reader.accept_clip(&ticket, clip) {
                return;
            }
            clear_range_replay();
            Some((ticket.generation, path))
        }
        ClipOutcome::Failed { ticket, message } => {
            if !next.reader.reject(&ticket, &message) {
                return;
            }
            None
        }
        ClipOutcome::Coded { ticket, failure } => {
            if failure == Failure::NoRecording
                && next.reader.expects(&ticket)
                && take_range_replay()
            {
                replay_retained_start();
                return;
            }
            if !next.reader.apply_failure(&ticket, failure) {
                return;
            }
            None
        }
    };
    ui.set(next);
    if let Some((generation, path)) = path {
        launch_stream(generation, path);
    } else {
        playback::pause_element();
    }
}

fn apply_frame(mut ui: Signal<VideoUi>, outcome: FrameOutcome) {
    let mut next = ui.peek().clone();
    let frame = match outcome {
        FrameOutcome::Idle => return,
        FrameOutcome::Ready { ticket, frame } => {
            if !next.reader.accept_frame(&ticket, frame.clone()) {
                return;
            }
            Some(frame)
        }
        FrameOutcome::Failed { ticket, message } => {
            if !next.reader.reject(&ticket, &message) {
                return;
            }
            None
        }
        FrameOutcome::Coded { ticket, failure } => {
            if !next.reader.apply_failure(&ticket, failure) {
                return;
            }
            None
        }
    };
    ui.set(next);
    if let Some(frame) = frame {
        wasm_bindgen_futures::spawn_local(async move {
            let _ = playback::paint_frame(&frame.media_type, &frame.data).await;
        });
    }
}

fn launch_meta(ticket: Ticket) {
    let source_id = match &ticket.kind {
        Kind::Timeline { source_id } | Kind::Follow { source_id } => source_id.clone(),
        Kind::Clip { .. } | Kind::FrameAtTime { .. } => return,
    };
    if !calls_allowed() || take_secret(&[source_id.as_str()]).is_err() {
        let Some(mut ui) = bound() else {
            return;
        };
        let mut next = ui.peek().clone();
        let _ = next
            .reader
            .reject(&ticket, "The video request was invalid.");
        ui.set(next);
        return;
    }
    wasm_bindgen_futures::spawn_local(async move {
        let secret = session::native_secret();
        let outcome = request_meta(ticket, secret).await;
        run(move || apply_meta(outcome));
    });
}

async fn request_meta(ticket: Ticket, secret: Option<String>) -> MetaOutcome {
    let command = match &ticket.kind {
        Kind::Timeline { source_id } => videos::timeline_command(source_id),
        Kind::Follow { source_id } => videos::follow_command(source_id),
        Kind::Clip { .. } | Kind::FrameAtTime { .. } => {
            return MetaOutcome::Failed {
                ticket,
                message: "The video request was invalid.".to_owned(),
            };
        }
    };
    let reply = match post(command, secret.clone(), ticket.generation).await {
        Ok(reply) => reply,
        Err(message) => return MetaOutcome::Failed { ticket, message },
    };
    if let Some(failure) = videos::stable_failure(&reply.body) {
        return MetaOutcome::Coded { ticket, failure };
    }
    match &ticket.kind {
        Kind::Timeline { .. } => match videos::interpret_timeline(&reply, secret.as_deref()) {
            Ok(timeline) => MetaOutcome::Timeline { ticket, timeline },
            Err(fault) => MetaOutcome::Failed {
                ticket,
                message: fault_message(&fault),
            },
        },
        Kind::Follow { .. } => match videos::interpret_follow(&reply, secret.as_deref()) {
            Ok(follow) => MetaOutcome::Follow { ticket, follow },
            Err(fault) => MetaOutcome::Failed {
                ticket,
                message: fault_message(&fault),
            },
        },
        Kind::Clip { .. } | Kind::FrameAtTime { .. } => MetaOutcome::Failed {
            ticket,
            message: "The video request was invalid.".to_owned(),
        },
    }
}

fn sync_auto_range(next: &mut VideoUi, auto: &Option<AutoPlayback>) {
    if !matches!(auto, Some(AutoPlayback::Clip(_))) {
        return;
    }
    if let Selection::Recorded {
        start_us, end_us, ..
    } = next.reader.selection
    {
        next.range_start = track::format_utc(start_us);
        next.range_end = track::format_utc(end_us);
        next.draft = Some(Window { start_us, end_us });
    }
}

fn apply_meta(outcome: MetaOutcome) {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    let mut auto = None;
    let path = match outcome {
        MetaOutcome::Timeline { ticket, timeline } => {
            if !next.reader.accept_timeline(&ticket, timeline) {
                return;
            }
            auto = videos::begin_auto_playback(&mut next.reader);
            sync_auto_range(&mut next, &auto);
            None
        }
        MetaOutcome::Follow { ticket, follow } => {
            let path = follow.path.clone();
            if !next.reader.accept_follow(&ticket, follow) {
                return;
            }
            Some((ticket.generation, path))
        }
        MetaOutcome::Failed { ticket, message } => {
            if !next.reader.reject(&ticket, &message) {
                return;
            }
            None
        }
        MetaOutcome::Coded { ticket, failure } => {
            if !next.reader.apply_failure(&ticket, failure) {
                return;
            }
            None
        }
    };
    ui.set(next);
    if let Some(auto) = auto {
        match auto {
            AutoPlayback::Follow(ticket) => launch_meta(ticket),
            AutoPlayback::Clip(ticket) => launch_clip(ticket),
        }
        return;
    }
    if let Some((generation, path)) = path {
        launch_stream(generation, path);
    } else {
        playback::pause_element();
    }
}

fn apply_stream(outcome: StreamOutcome) {
    run(move || {
        let Some(mut ui) = bound() else {
            return;
        };
        let mut next = ui.peek().clone();
        let changed = match outcome {
            StreamOutcome::Stopped { generation } => next.reader.finish(generation),
            StreamOutcome::Failed {
                generation,
                message,
                body,
            } => {
                if next.reader.fail_media(generation, &body) {
                    true
                } else {
                    next.reader.fail_stream(generation, &message)
                }
            }
        };
        if changed {
            ui.set(next);
            playback::pause_element();
        }
    });
}

fn on_playing(generation: u32) {
    run(move || {
        let Some(mut ui) = bound() else {
            return;
        };
        let mut next = ui.peek().clone();
        if next.reader.playing(generation) {
            ui.set(next);
        }
    });
}

fn on_frame(generation: u32, captured_at: &str) {
    let captured_at = captured_at.to_owned();
    run(move || {
        let Some(mut ui) = bound() else {
            return;
        };
        let mut next = ui.peek().clone();
        if next.reader.note_frame(generation, &captured_at) {
            ui.set(next);
        }
    });
}

fn on_gap_skipped(generation: u32) {
    run(move || {
        let Some(mut ui) = bound() else {
            return;
        };
        let mut next = ui.peek().clone();
        if next.reader.note_gap_skipped(generation) {
            ui.set(next);
        }
    });
}

fn fault_message(fault: &Fault) -> String {
    match fault {
        Fault::Unavailable(message) => message.clone(),
        Fault::Unauthorized(message) | Fault::Forbidden(message) => message.clone(),
        Fault::Malformed => "The video result could not be read.".to_owned(),
        Fault::Transport => "The browser could not reach this origin.".to_owned(),
    }
}

fn run<F: FnOnce()>(job: F) {
    let runtime = RUNTIME.with(|slot| slot.borrow().clone());
    let scope = SCOPE.with(|slot| *slot.borrow());
    if let (Some(runtime), Some(scope)) = (runtime, scope) {
        let _guard = RuntimeGuard::new(runtime.clone());
        runtime.in_scope(scope, job);
        runtime.needs_update(scope);
        return;
    }
    job();
}
