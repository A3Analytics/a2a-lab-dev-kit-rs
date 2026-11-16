//! Videos reader. One viewer follows the newest point or plays a bounded clip.

use std::cell::{Cell, RefCell};

use crate::images::SourcePhase;
use crate::inventory::{Item, Primitive};
use crate::shell::nav::{self, VideoPanel};
use crate::shell::playback::{self, StreamTicket};
use crate::shell::session::{self, message_id, take_secret};
use crate::videos::browse_link::{self, BrowseApply};
use crate::videos::track::{self, Track, Window};
use crate::videos::{self, Kind, Playback, PlaybackSpeed, StreamPlan};
use dioxus::core::{Runtime, RuntimeGuard, ScopeId, current_scope_id};
use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::AbortController;

mod clip_delivery;
mod details;
mod filter_rail;
mod playback_hooks;
mod results_stage;
mod route_lifecycle;
mod timeline_refresh;
mod video_tabs;
use results_stage::Results;
pub(crate) use video_tabs::browse_streaming_allowed;
use video_tabs::{VideoTabs, arm_browse_stream, arm_browse_stream_for_timeline, sync_video_panel};
mod semantic;
mod speed_control;
mod timeline;
mod timeline_fields;
use clip_delivery::{ClipJob, apply_clip, run_clip};
pub(in crate::shell::videos) use clip_delivery::{launch_clip, launch_meta};
use filter_rail::FilterRail;
use playback_hooks::{apply_stream, on_buffering, on_frame, on_gap_skipped, on_playing};
pub(crate) use route_lifecycle::leave_videos;
use route_lifecycle::{mirror_status, suspend_browse_media, videos_visible};
use semantic::{SearchPanel, abort_semantic_commands, note_source_change, wire_resources};
use timeline::{
    abort_commands, alert_text, bands, begin_command, commit_at, display_clock, displayed_window,
    ensure_timeline, field_to_utc, follow_newest, playhead_style, replay_retained_start,
    replay_session, selection_slug, slider_now, slider_text, timeline_click, timeline_down,
    timeline_key, timeline_move, timeline_up, us_at,
};
use timeline_refresh::start_coverage_refresh;

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
    pub search_range_start: String,
    pub search_range_end: String,
    pub search_draft_start: String,
    pub search_draft_end: String,
    pub search_range_active: bool,
    pub search_range_error: String,
    pub local_time: bool,
    pub search_sources: Vec<String>,
    pub semantic: videos::semantic::Hub,
    pub active_tab: VideoTab,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum VideoTab {
    Browse,
    Search,
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
            search_range_start: String::new(),
            search_range_end: String::new(),
            search_draft_start: String::new(),
            search_draft_end: String::new(),
            search_range_active: false,
            search_range_error: String::new(),
            local_time: false,
            search_sources: Vec::new(),
            semantic: videos::semantic::Hub::new(),
            active_tab: VideoTab::Browse,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Loader {
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
    static APPLIED_BROWSE_QUERY: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn bind(ui: Signal<VideoUi>) {
    UI.with(|slot| *slot.borrow_mut() = Some(ui));
}

/// Stops browse playback when the shell navigates away from Videos.
pub fn leave_on_navigate() {
    route_lifecycle::leave_videos();
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
    let source = next.reader.source_id.clone();
    ui.set(next);
    ensure_timeline();
    if phase == SourcePhase::Ready && !source.is_empty() {
        note_source_change(&source);
    }
    apply_browse_deep_link();
}

pub(super) fn clear_applied_browse_query() {
    APPLIED_BROWSE_QUERY.with(|slot| *slot.borrow_mut() = None);
}

pub fn apply_browse_deep_link() {
    if browse_link::videos_panel(&crate::shell::nav::published_route())
        != browse_link::VideosPanel::Browse
    {
        return;
    }
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(search) = window.location().search() else {
        return;
    };
    let mut query = search.trim_start_matches('?').to_owned();
    if query.is_empty() {
        query = crate::shell::nav::last_open_query();
    }
    let Some(ui) = bound() else {
        return;
    };
    if ui.peek().phase != SourcePhase::Ready {
        return;
    }
    let catalog = ui
        .peek()
        .sources
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    if catalog.is_empty() {
        return;
    }
    if APPLIED_BROWSE_QUERY.with(|slot| slot.borrow().as_deref() == Some(query.as_str())) {
        return;
    }
    match browse_link::apply_browse_query(&catalog, &query) {
        BrowseApply::Unchanged => {
            APPLIED_BROWSE_QUERY.with(|slot| *slot.borrow_mut() = Some(query));
        }
        BrowseApply::Seek(frame) => {
            APPLIED_BROWSE_QUERY.with(|slot| *slot.borrow_mut() = Some(query));
            let current = ui.peek().reader.source_id.clone();
            if current == frame.source_id {
                commit_at(frame.captured_us, frame.autoplay);
            } else {
                semantic::remember_search_seek(&frame.source_id, frame.captured_us, frame.autoplay);
                choose_source(&frame.source_id);
            }
        }
    }
}

pub(super) fn bound() -> Option<Signal<VideoUi>> {
    UI.with(|slot| *slot.borrow())
}

pub(super) fn loader() -> Option<Loader> {
    LOADER.with(|slot| *slot.borrow())
}

#[component]
pub fn VideoView(ui: Signal<VideoUi>, visible: bool, panel: VideoPanel) -> Element {
    let clip = use_signal(|| ClipJob::Idle);
    let semantic_search = use_signal(|| semantic::SearchJob::Idle);
    let semantic_caps = use_signal(|| semantic::CapabilityJob::Idle);
    let semantic_history = use_signal(|| semantic::HistoryJob::Idle);
    use_hook(move || {
        RUNTIME.with(|slot| *slot.borrow_mut() = Some(Runtime::current()));
        SCOPE.with(|slot| *slot.borrow_mut() = Some(current_scope_id()));
        playback::bind(on_playing, on_frame, on_gap_skipped, on_buffering);
        LOADER.with(|slot| *slot.borrow_mut() = Some(Loader { clip }));
        semantic::bind_loader(semantic_search, semantic_caps, semantic_history);
    });
    use_drop(leave_videos);
    let mut shown = use_signal(|| false);
    if shown() != visible {
        shown.set(visible);
    }
    let mut active_panel = use_signal(|| panel);
    if active_panel() != panel {
        active_panel.set(panel);
    }
    let was_visible = use_hook(|| Cell::new(false));
    if visible {
        sync_video_panel(panel);
    }
    use_effect(move || {
        let visible = shown();
        let panel = active_panel();
        if !visible {
            if was_visible.replace(false) {
                leave_videos();
            }
            return;
        }
        was_visible.set(true);
        sync_video_panel(panel);
        if panel == VideoPanel::Browse
            || nav::video_panel_for(&nav::published_route()) == VideoPanel::Browse
        {
            playback::bind_media(native_play, native_pause, force_pause);
            ensure_timeline();
            start_coverage_refresh();
            apply_browse_deep_link();
        } else {
            suspend_browse_media();
        }
    });
    let clip_reply = use_resource(move || {
        let job = clip();
        let secret = session::native_secret();
        async move { run_clip(job, secret).await }
    });
    let _ = clip_reply.read();
    if visible && let Some(outcome) = clip_reply() {
        apply_clip(ui, outcome);
    }
    wire_resources(semantic_search, semantic_caps, semantic_history, ui);
    use_effect(move || {
        let source = ui.peek().reader.source_id.clone();
        if !source.is_empty() {
            note_source_change(&source);
        }
    });
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
                VideoTabs { state: state.clone() }
                div { id: "video-tab-panels", class: "video-tab-panels",
                    div {
                        id: "video-browse-panel",
                        class: "video-tab-panel",
                        hidden: state.active_tab != VideoTab::Browse,
                        role: "tabpanel",
                        aria_labelledby: "video-browse-tab",
                        FilterRail { state: state.clone() }
                        Results { state: state.clone(), status: status.clone(), alert: alert.clone() }
                    }
                    div {
                        id: "video-search-panel",
                        class: "video-tab-panel video-search-panel",
                        hidden: state.active_tab != VideoTab::Search,
                        role: "tabpanel",
                        aria_labelledby: "video-search-tab",
                        SearchPanel { state: state.clone() }
                    }
                }
            }
        }
    }
}

fn open_playhead_frame(event: Event<MouseData>, href: &str) {
    event.stop_propagation();
    if event.modifiers().ctrl()
        || event.modifiers().meta()
        || event.modifiers().shift()
        || event.modifiers().alt()
    {
        return;
    }
    event.prevent_default();
    nav::open(href);
}

#[component]
pub(super) fn TimelineScrubber(state: VideoUi) -> Element {
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
    let show_play = matches!(state.reader.playback, Playback::Paused | Playback::Stopped);
    let show_transport = state.reader.has_media();
    let now = if state.reader.frame_time.is_empty() {
        value_text.clone()
    } else {
        display_clock(&state.reader.frame_time, local)
    };
    let playhead_href =
        browse_link::encode_frame_href(&state.reader.source_id, &state.reader.frame_time, true);
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
            if let Some(href) = playhead_href {
                a {
                    id: "video-timeline-now",
                    class: "video-timeline-now",
                    href: "{href.clone()}",
                    onpointerdown: move |event| event.stop_propagation(),
                    onpointerup: move |event| event.stop_propagation(),
                    onclick: move |event| open_playhead_frame(event, &href),
                    "{now}"
                }
            } else {
                p { id: "video-timeline-now", class: "video-timeline-now", "{now}" }
            }
            if show_transport || !bars.is_empty() {
                div { class: "video-seek",
                    if show_transport {
                        button {
                            id: "video-pause",
                            class: "video-pause",
                            r#type: "button",
                            aria_label: if show_play { "Play" } else { "Pause" },
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
                                if show_play {
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
                if SUPPRESS_CLICK.with(|cell| cell.replace(false)) {
                    return;
                }
                let Some(at_us) = us_at(event.data().client_coordinates().x) else {
                    return;
                };
                commit_at(at_us, true);
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

pub(super) fn source_option(item: &Item) -> String {
    filter_rail::source_option(item)
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

pub(super) fn edit_current_time() {
    let current_time = session::control_value("video-current-time");
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    next.range_draft_start = current_time;
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
    next.search_draft_start =
        timeline::reshow_picker(&next.search_draft_start, from, next.local_time);
    next.search_draft_end = timeline::reshow_picker(&next.search_draft_end, from, next.local_time);
    ui.set(next);
}

pub(super) fn apply_current_time() {
    let local = bound().is_some_and(|ui| ui.peek().local_time);
    let current = field_to_utc(&session::control_value("video-current-time"), local);
    let Some(at_us) = videos::mjpeg::captured_us(&current) else {
        if let Some(mut ui) = bound() {
            let mut next = ui.peek().clone();
            next.range_error = "Enter a valid current time.".to_owned();
            ui.set(next);
        }
        return;
    };
    let autoplay = bound().is_some_and(|ui| ui.peek().reader.playback == Playback::Playing);
    if let Some(mut ui) = bound() {
        let mut next = ui.peek().clone();
        next.range_error.clear();
        ui.set(next);
    }
    commit_at(at_us, autoplay);
}

pub(super) fn choose_source(id: &str) {
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
    abort_semantic_commands();
    ui.set(next);
    playback::pause_element();
    ensure_timeline();
    note_source_change(id);
}

pub(super) fn native_play() {
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

pub(super) fn native_pause() {
    run(|| {
        if playback::take_ignore_pause() {
            return;
        }
        let connecting =
            bound().is_some_and(|ui| ui.peek().reader.playback == Playback::Connecting);
        if connecting {
            playback::bump_ignore_pause();
            return;
        }
        pause();
    });
}

pub(super) fn force_pause() {
    run(|| {
        let _ = playback::take_ignore_pause();
        pause();
    });
}

pub(super) fn toggle_playback() {
    let playback = bound().map(|ui| ui.peek().reader.playback);
    match playback {
        Some(Playback::Paused) => {
            arm_browse_stream();
            resume();
            playback::play_element();
        }
        Some(Playback::Stopped) => {
            arm_browse_stream();
            replay_session();
        }
        _ => {
            pause();
            playback::pause_element();
        }
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
    arm_browse_stream();
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

pub(super) fn retry() {
    arm_browse_stream();
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

pub(super) fn launch_stream(generation: u32, plan: StreamPlan) {
    if !browse_streaming_allowed() {
        return;
    }
    let autoplay = !playback::start_paused();
    let secret = session::native_secret();
    if take_secret(&[plan.fetch_path.as_str()]).is_err() {
        return;
    }
    playback::apply_plan(&plan);
    if autoplay {
        playback::play_element();
    }
    let path = plan.fetch_path.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let outcome = playback::run(StreamTicket { generation }, path, secret).await;
        apply_stream(outcome);
    });
}

pub(super) fn take_range_replay() -> bool {
    REPLAY.with(|slot| {
        if slot.get() {
            return false;
        }
        slot.set(true);
        true
    })
}

pub(super) fn clear_range_replay() {
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
