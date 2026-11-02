//! Videos reader. One viewer follows the newest point or plays a bounded clip.

use std::cell::{Cell, RefCell};

use crate::images::{self, SourcePhase};
use crate::inventory::{Fault, Item, Primitive};
use crate::shell::loading::{StrandSize, StrandsLoader};
use crate::shell::playback::{self, StreamTicket};
use crate::shell::session::{self, calls_allowed, message_id, take_secret};
use crate::videos::track::{self, Track, Window};
use crate::videos::{
    self, AutoPlayback, ClipHandle, Failure, Kind, LiveHandle, Playback, PlaybackSpeed, Selection,
    StreamPlan, Ticket, Timeline,
};
use dioxus::core::{Runtime, RuntimeGuard, ScopeId, current_scope_id};
use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::AbortController;

mod details;
mod playback_hooks;
mod speed_control;
mod timeline;
mod timeline_fields;
use details::VideoDetails;
use playback_hooks::{apply_stream, on_buffering, on_frame, on_gap_skipped, on_playing};
use speed_control::{
    active_stream_plan, change_speed, lock_live_speed, note_trick_fallback, speed_option_disabled,
};
use timeline::{
    abort_commands, alert_text, bands, begin_command, commit_at, commit_typed_range, display_clock,
    display_instant, displayed_window, ensure_timeline, field_to_utc, follow_newest,
    playhead_style, remember_window, replay_retained_start, reselect_range, selection_slug,
    slider_now, slider_text, timeline_click, timeline_down, timeline_key, timeline_move,
    timeline_up,
};

#[derive(Clone, PartialEq)]
pub struct VideoUi {
    pub reader: videos::Reader,
    pub sources: Vec<Item>,
    pub phase: SourcePhase,
    pub source_alert: String,
    pub draft: Option<Window>,
    pub session: Option<Window>,
    pub speed: PlaybackSpeed,
    pub range_start: String,
    pub range_end: String,
    pub range_draft_start: String,
    pub range_draft_end: String,
    pub range_error: String,
    pub local_time: bool,
}

impl VideoUi {
    pub fn new() -> Self {
        Self {
            reader: videos::Reader::new(),
            sources: Vec::new(),
            phase: SourcePhase::Loading,
            source_alert: String::new(),
            draft: None,
            session: None,
            speed: PlaybackSpeed::default(),
            range_start: String::new(),
            range_end: String::new(),
            range_draft_start: String::new(),
            range_draft_end: String::new(),
            range_error: String::new(),
            local_time: false,
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

enum MetaOutcome {
    Timeline { ticket: Ticket, timeline: Timeline },
    Follow { ticket: Ticket, follow: LiveHandle },
    Failed { ticket: Ticket, message: String },
    Coded { ticket: Ticket, failure: Failure },
}

#[derive(Clone, Copy)]
struct Loader {
    clip: Signal<ClipJob>,
}

struct CommandFlight {
    generation: u32,
    abort: AbortController,
}

#[derive(Clone, Copy)]
struct Drag {
    x: f64,
    at_us: i64,
    moved: bool,
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
        next.session = None;
        playback::abort();
        abort_commands();
    }
    ui.set(next);
    ensure_timeline();
}

pub(super) fn bound() -> Option<Signal<VideoUi>> {
    UI.with(|slot| *slot.borrow())
}

fn loader() -> Option<Loader> {
    LOADER.with(|slot| *slot.borrow())
}

#[component]
pub fn VideoView(ui: Signal<VideoUi>, visible: bool) -> Element {
    let clip = use_signal(|| ClipJob::Idle);
    use_hook(|| {
        RUNTIME.with(|slot| *slot.borrow_mut() = Some(Runtime::current()));
        SCOPE.with(|slot| *slot.borrow_mut() = Some(current_scope_id()));
        playback::bind(on_playing, on_frame, on_gap_skipped, on_buffering);
        LOADER.with(|slot| *slot.borrow_mut() = Some(Loader { clip }));
    });
    use_drop(stop_playback);
    let mut shown = use_signal(|| false);
    if shown() != visible {
        shown.set(visible);
    }
    use_effect(move || {
        if shown() {
            playback::bind_media(native_play, native_pause, force_pause);
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
    let _ = clip_reply.read();
    if let Some(outcome) = clip_reply() {
        apply_clip(ui, outcome);
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
    let track = state.reader.timeline.as_ref().map(track::coverage_track);
    let available_start_utc = track
        .as_ref()
        .filter(|item| item.span_us() > 0)
        .map(|item| track::format_utc(item.origin_us));
    let available_end_utc = track
        .as_ref()
        .filter(|item| item.span_us() > 0)
        .map(|item| track::format_utc(item.end_us));
    let local = state.local_time;
    let available_start = available_start_utc
        .as_deref()
        .map(|value| display_clock(value, local));
    let available_end = available_end_utc
        .as_deref()
        .map(|value| display_clock(value, local));
    rsx! {
        nav { id: "video-filters", class: "video-filters", aria_label: "Video filters",
            div { id: "video-sources", class: "video-filter-group",
                p { id: "video-source-label", "Source" }
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
                    aria_labelledby: "video-source-label",
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
                VideoDetails {
                    range_start: if state.range_draft_start.is_empty() {
                        display_clock(&state.range_start, local)
                    } else {
                        state.range_draft_start.clone()
                    },
                    range_end: if state.range_draft_end.is_empty() {
                        display_clock(&state.range_end, local)
                    } else {
                        state.range_draft_end.clone()
                    },
                    local_time: local,
                    range_error: state.range_error.clone(),
                    utc_start: committed_utc(
                        &state.range_start,
                        state
                            .reader
                            .clip
                            .as_ref()
                            .map(|clip| clip.effective_start.as_str()),
                    ),
                    utc_end: committed_utc(
                        &state.range_end,
                        state
                            .reader
                            .clip
                            .as_ref()
                            .map(|clip| clip.effective_end.as_str()),
                    ),
                    available_start,
                    available_end,
                    available_start_utc,
                    available_end_utc,
                    live_enabled: videos::shows_live_button(&state.reader),
                    live_active: matches!(state.reader.selection, Selection::Live),
                }
            }
            div { class: "video-filter-group video-actions",
                p { "Playback options" }
                div { class: "video-speed",
                    label { r#for: "video-speed", "Speed" }
                    select {
                        id: "video-speed",
                        aria_label: "Playback speed",
                        onchange: move |_| change_speed(),
                        for rate in PlaybackSpeed::all() {
                            option {
                                key: "{rate.label()}",
                                value: "{rate.control_value()}",
                                selected: state.speed == *rate,
                                disabled: speed_option_disabled(&state, *rate),
                                "{rate.label()}"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Results(state: VideoUi, status: String, alert: String) -> Element {
    use_effect(move || {
        playback::bind_media(native_play, native_pause, force_pause);
    });
    let media = state.reader.has_media();
    let show_empty = !media
        && !state.reader.loading
        && state.reader.playback != Playback::Connecting
        && alert.is_empty();
    let show_loading = alert.is_empty()
        && state.reader.frame_time.is_empty()
        && matches!(
            state.reader.playback,
            Playback::Connecting | Playback::Playing | Playback::Paused
        )
        && !matches!(state.reader.selection, Selection::Empty);
    let show_buffering = !show_loading && state.reader.buffering && alert.is_empty();
    let failure = state.reader.failure.map(|item| item.code()).unwrap_or("");
    rsx! {
        div { id: "video-results", class: "log-results",
            div { class: "log-scroll",
                div { id: "video-stage", class: "video-stage",
                    div {
                        class: "video-aspect",
                        "data-playback": playback_slug(state.reader.playback),
                        canvas {
                            id: "video-canvas",
                            class: "visually-hidden",
                            width: "640",
                            height: "360",
                            "aria-hidden": "true",
                        }
                        if show_empty {
                            p { id: "video-empty", class: "video-empty", "Select a source and a time." }
                        }
                        if matches!(state.reader.selection, Selection::Live) && !show_empty {
                            span { id: "video-live-badge", class: "video-live-badge", "LIVE" }
                        }
                        video {
                            id: "video-player",
                            class: "video-player",
                            hidden: !media,
                            muted: true,
                            playsinline: true,
                            preload: "none",
                            "aria-label": "Video playback",
                            onmounted: move |_| {
                                playback::bind_media(native_play, native_pause, force_pause);
                                playback::play_element();
                            },
                            onplay: move |_| native_play(),
                            onpause: move |_| native_pause(),
                            onclick: move |_| toggle_playback(),
                        }
                        TimelineScrubber { state: state.clone() }
                        if show_loading {
                            div {
                                id: "video-loading",
                                class: "video-loading",
                                aria_hidden: "true",
                                div { class: "loading-surface",
                                    StrandsLoader { size: StrandSize::Md }
                                    p { class: "loading-title", "Loading…" }
                                }
                            }
                        }
                        if show_buffering {
                            div {
                                id: "video-buffering",
                                class: "video-buffering",
                                aria_hidden: "true",
                                StrandsLoader { size: StrandSize::Md }
                                p { "Buffering…" }
                            }
                        }
                    }
                }
                p {
                    id: "video-status",
                    class: "visually-hidden",
                    role: "status",
                    aria_live: "polite",
                    "data-state": playback_slug(state.reader.playback),
                    "data-buffering": if state.reader.buffering { "true" } else { "false" },
                    "data-frame-time": if state.reader.frame_time.is_empty() {
                        "none"
                    } else {
                        state.reader.frame_time.as_str()
                    },
                    "{status}"
                }
                div {
                    id: "video-alert",
                    class: "video-alert",
                    role: "alert",
                    aria_live: "assertive",
                    "data-failure": "{failure}",
                    if !alert.is_empty() {
                        span { id: "video-alert-text", "{alert}" }
                    }
                    if state.reader.can_retry() {
                        button {
                            id: "video-retry",
                            class: "video-outline",
                            r#type: "button",
                            onclick: move |_| retry(),
                            "Retry"
                        }
                    }
                    if state.reader.can_reselect() {
                        button {
                            id: "video-reselect",
                            class: "video-outline",
                            r#type: "button",
                            onclick: move |_| reselect_range(),
                            "Choose another range"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TimelineScrubber(state: VideoUi) -> Element {
    let track = state.reader.timeline.as_ref().map(track::coverage_track);
    let window = displayed_window(&state);
    let local = state.local_time;
    let bars = track
        .as_ref()
        .map(|item| bands(item, local))
        .unwrap_or_default();
    let ready = state.reader.timeline.is_some();
    let empty_track = track.as_ref().is_some_and(|item| item.spans.is_empty());
    let value_now = slider_now(track.as_ref(), window);
    let value_max = track.as_ref().map(Track::span_us).unwrap_or(0);
    let available_start = track
        .as_ref()
        .filter(|item| item.span_us() > 0)
        .map(|item| display_clock(&track::format_utc(item.origin_us), local));
    let available_end = track
        .as_ref()
        .filter(|item| item.span_us() > 0)
        .map(|item| display_clock(&track::format_utc(item.end_us), local));
    let value_text = match (window, available_start.as_deref(), available_end.as_deref()) {
        (None, Some(start), Some(end)) => format!("Available from {start} to {end}"),
        _ => slider_text(window, local),
    };
    let duration = window.map(Window::duration_us).unwrap_or(0);
    let draft_start = window
        .map(|item| track::format_utc(item.start_us))
        .unwrap_or_default();
    let draft_end = window
        .map(|item| track::format_utc(item.end_us))
        .unwrap_or_default();
    let playhead = playhead_style(&state);
    let paused = state.reader.playback == Playback::Paused;
    let show_transport = state.reader.has_media();
    let now = if state.reader.frame_time.is_empty() {
        value_text.clone()
    } else {
        display_clock(&state.reader.frame_time, local)
    };
    rsx! {
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
            "data-session-us": "{track::PLAYBACK_WINDOW_US}",
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
            p { id: "video-timeline-now", class: "video-timeline-now", "{now}" }
            if show_transport || !bars.is_empty() {
                div { class: "video-seek",
                    if show_transport {
                        button {
                            id: "video-pause",
                            class: "video-pause",
                            r#type: "button",
                            aria_label: if paused { "Play" } else { "Pause" },
                            aria_pressed: if paused { "true" } else { "false" },
                            onpointerdown: move |event| event.stop_propagation(),
                            onclick: move |event| {
                                event.stop_propagation();
                                toggle_playback();
                            },
                            svg {
                                class: "video-transport-icon",
                                view_box: "0 0 16 16",
                                "aria-hidden": "true",
                                if paused {
                                    polygon { points: "4.5,2.5 13.5,8 4.5,13.5", fill: "currentColor" }
                                } else {
                                    rect { x: "3", y: "2.5", width: "3.2", height: "11", rx: "0.6", fill: "currentColor" }
                                    rect { x: "9.8", y: "2.5", width: "3.2", height: "11", rx: "0.6", fill: "currentColor" }
                                }
                            }
                        }
                    }
                    if !bars.is_empty() {
                        div { class: "video-track", aria_hidden: "true",
                            for band in bars {
                                BandMark { key: "{band.id}-{band.style}", band: band.clone() }
                            }
                            if let Some(style) = playhead.clone() {
                                Placed { key: "playhead-{style}", id: "video-playhead".to_owned(), class: "video-playhead", style }
                            }
                        }
                    }
                }
            }
            p { id: "video-timeline-hint", class: "video-timeline-hint visually-hidden",
                "Drag the timeline to move the current frame. Use LIVE to return to the live edge."
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
    let id = band.id.clone();
    let style = band.style.clone();
    let label = band.label.clone();
    let start_us = band.start_us;
    rsx! {
        div {
            id: "{band.id}",
            class: "{band.class}",
            "data-kind": "{band.kind}",
            "data-start": "{band.start}",
            "data-end": "{band.end}",
            "data-duration-us": "{band.duration_us}",
            title: "{label}",
            onmounted: move |_| place_style(&id, &style),
            onclick: move |event| {
                event.stop_propagation();
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

fn committed_utc(stored: &str, effective: Option<&str>) -> String {
    let source = effective
        .filter(|value| !value.is_empty())
        .unwrap_or(stored);
    display_instant(source)
}

fn source_option(item: &Item) -> String {
    if item.name.is_empty() {
        item.id.clone()
    } else {
        format!("{} · {}", item.name, item.id)
    }
}

pub(super) fn edit_range(start: bool) {
    let start_raw = session::control_value("video-range-start");
    let end_raw = session::control_value("video-range-end");
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if start {
        next.range_draft_start = start_raw;
    } else {
        next.range_draft_end = end_raw;
    }
    next.range_error.clear();
    ui.set(next);
}

pub(super) fn toggle_local_time() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    let from = next.local_time;
    next.local_time = !from;
    next.range_draft_start = timeline::reshow(&next.range_draft_start, from, next.local_time);
    next.range_draft_end = timeline::reshow(&next.range_draft_end, from, next.local_time);
    ui.set(next);
}

pub(super) fn apply_typed_range() {
    let local = bound().is_some_and(|ui| ui.peek().local_time);
    let start = field_to_utc(&session::control_value("video-range-start"), local);
    let end = field_to_utc(&session::control_value("video-range-end"), local);
    if let Err(fault) = videos::check_range(&start, &end) {
        if let Some(mut ui) = bound() {
            let mut next = ui.peek().clone();
            next.range_error = timeline::range_fault_message(fault, next.local_time);
            ui.set(next);
        }
        return;
    }
    if let Some(mut ui) = bound() {
        let mut next = ui.peek().clone();
        next.range_error.clear();
        ui.set(next);
    }
    commit_typed_range();
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
    next.session = None;
    playback::abort();
    abort_commands();
    ui.set(next);
    playback::pause_element();
    ensure_timeline();
}

fn native_play() {
    run(|| {
        if playback::start_paused() {
            return;
        }
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

fn force_pause() {
    run(|| {
        let _ = playback::take_ignore_pause();
        pause();
    });
}

fn toggle_playback() {
    let paused = bound().is_some_and(|ui| ui.peek().reader.playback == Playback::Paused);
    if paused {
        resume();
        playback::play_element();
    } else {
        pause();
        playback::pause_element();
    }
}

fn pause() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if next.reader.pause() {
        playback::set_paused(true);
        ui.set(next.clone());
        mirror_status(&next.reader);
    }
}

fn resume() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if next.reader.resume() {
        playback::set_paused(false);
        ui.set(next.clone());
        mirror_status(&next.reader);
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
    playback::set_paused(true);
    ui.set(next.clone());
    mirror_status(&next.reader);
    playback::pause_element();
}

fn mirror_status(reader: &videos::Reader) {
    let Some(status) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("video-status"))
    else {
        return;
    };
    let _ = status.set_attribute("data-state", playback_slug(reader.playback));
    status.set_text_content(Some(&videos::playback_status(reader)));
}

fn retry() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    let Some(ticket) = next.reader.retry() else {
        return;
    };
    playback::set_paused(false);
    ui.set(next);
    match &ticket.kind {
        Kind::Clip { .. } => launch_clip(ticket),
        Kind::Timeline { .. } | Kind::Follow { .. } => launch_meta(ticket),
        Kind::FrameAtTime { .. } => {}
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

pub(super) fn launch_stream(generation: u32, plan: StreamPlan) {
    let secret = session::native_secret();
    if take_secret(&[plan.fetch_path.as_str()]).is_err() {
        return;
    }
    playback::apply_plan(&plan);
    playback::play_element();
    let path = plan.fetch_path.clone();
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

pub(super) async fn post(
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
            if failure.allows_reselect() {
                next.session = None;
                if let Some(window) = displayed_window(&next) {
                    remember_window(&mut next, window);
                }
            }
            None
        }
    };
    ui.set(next);
    if let Some((generation, _path)) = path {
        let mut current = ui.peek().clone();
        let Some(plan) = active_stream_plan(&current) else {
            return;
        };
        note_trick_fallback(&mut current.reader, &plan);
        ui.set(current);
        launch_stream(generation, plan);
    } else {
        playback::pause_element();
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
        let window = Window { start_us, end_us };
        remember_window(next, window);
        next.session = Some(window);
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
            lock_live_speed(&mut next);
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
    if let Some((generation, _path)) = path {
        let mut current = ui.peek().clone();
        let Some(plan) = active_stream_plan(&current) else {
            return;
        };
        note_trick_fallback(&mut current.reader, &plan);
        ui.set(current);
        launch_stream(generation, plan);
    } else {
        playback::pause_element();
    }
}

fn fault_message(fault: &Fault) -> String {
    match fault {
        Fault::Unavailable(message) => message.clone(),
        Fault::Unauthorized(message) | Fault::Forbidden(message) => message.clone(),
        Fault::Malformed => "The video result could not be read.".to_owned(),
        Fault::Transport => "The browser could not reach this origin.".to_owned(),
    }
}

pub(super) fn run<F: FnOnce()>(job: F) {
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
