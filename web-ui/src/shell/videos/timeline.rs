//! Timeline selection, coverage drawing, and command cancellation.

use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::AbortController;

use super::speed_control::{lock_live_speed, sync_live_speed};
use super::{
    Band, COMMANDS, CommandFlight, DRAG, Drag, SUPPRESS_CLICK, VideoUi, bound, launch_clip,
    launch_meta,
};
use crate::images::SourcePhase;
use crate::shell::playback;
use crate::shell::session;
use crate::videos::track::{self, SpanKind, Track, Window};
use crate::videos::{self, Selection};

pub(super) use super::timeline_fields::{
    display_clock, display_instant, field_to_utc, format_shown, reshow,
};

pub(super) fn ensure_timeline() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if next.phase != SourcePhase::Ready
        || next.reader.source_id.is_empty()
        || next.reader.timeline.is_some()
        || next.reader.loading
    {
        return;
    }
    let Ok(ticket) = next.reader.load_timeline() else {
        return;
    };
    ui.set(next);
    launch_meta(ticket);
}

pub(super) fn follow_newest() {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    playback::abort();
    abort_commands();
    playback::set_paused(false);
    let recorded = matches!(next.reader.selection, Selection::Recorded { .. });
    let ticket = if recorded {
        next.reader.return_to_newest()
    } else {
        next.reader.follow_newest()
    };
    let Ok(ticket) = ticket else {
        ui.set(next);
        playback::pause_element();
        return;
    };
    lock_live_speed(&mut next);
    ui.set(next);
    launch_meta(ticket);
}

pub(super) fn reselect_range() {
    playback::abort();
    abort_commands();
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if next.reader.reselect() {
        next.draft = None;
        next.session = None;
        ui.set(next);
        playback::pause_element();
    }
}

pub(super) fn commit_at(at_us: i64) {
    super::fetch_timeline(move |_| play_at(at_us));
}

fn play_at(at_us: i64) {
    let Some(track) = current_track() else {
        return;
    };
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if let Some(selection) = selection_window(&next)
        && selection.duration_us() > track::PLAYBACK_WINDOW_US
        && at_us >= selection.start_us
        && at_us < selection.end_us
    {
        let Some(session) = track::session_in_selection(&track, selection, at_us) else {
            return;
        };
        remember(&mut next, selection);
        request_session(&mut next, session);
        ui.set(next);
        return;
    }
    let Some(session) = track::session_at(&track, at_us) else {
        return;
    };
    ui.set(next);
    commit_selection(session, Some(at_us));
}

pub(super) fn replay_retained_start() {
    super::fetch_timeline(move |_| {
        let Some(track) = current_track() else {
            return;
        };
        let Some(window) = track::initial_window(&track) else {
            return;
        };
        commit_selection(window, Some(window.start_us));
    });
}

pub(super) fn commit_established() {
    let Some(track) = current_track() else {
        return;
    };
    let newest = bound().and_then(|ui| ui.peek().reader.timeline.as_ref()?.newest_us);
    let current = bound().and_then(|ui| displayed_window(&ui.peek()));
    let Some(window) = track::establish(&track, current, newest) else {
        return;
    };
    commit_selection(window, Some(window.start_us));
}

fn commit_selection(selection: Window, anchor: Option<i64>) {
    let Some(track) = current_track() else {
        return;
    };
    let anchor = anchor.unwrap_or(selection.start_us);
    let session = if selection.duration_us() <= track::PLAYBACK_WINDOW_US {
        selection
    } else {
        track::session_in_selection(&track, selection, anchor).unwrap_or(selection)
    };
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    remember(&mut next, selection);
    request_session(&mut next, session);
    ui.set(next);
}

fn request_session(next: &mut VideoUi, session: Window) {
    next.session = Some(session);
    let start = track::format_utc(session.start_us);
    let end = track::format_utc(session.end_us);
    playback::abort();
    abort_commands();
    playback::set_paused(false);
    playback::play_element();
    let ticket = match next.reader.select_range(&start, &end) {
        Ok(ticket) => ticket,
        Err(_) => {
            playback::pause_element();
            return;
        }
    };
    launch_clip(ticket);
}

fn selection_window(state: &VideoUi) -> Option<Window> {
    displayed_window(state)
}

pub(super) fn nudge(steps: i64, extend: bool) {
    let Some(track) = current_track() else {
        return;
    };
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    let newest = next
        .reader
        .timeline
        .as_ref()
        .and_then(|timeline| timeline.newest_us);
    let Some(window) = displayed_window(&next).or_else(|| track::establish(&track, None, newest))
    else {
        return;
    };
    remember(
        &mut next,
        if extend {
            track::extend_window(&track, window, steps)
        } else {
            track::move_window(&track, window, steps)
        },
    );
    ui.set(next);
}

pub(super) fn jump_selection(to_start: bool) {
    let Some(track) = current_track() else {
        return;
    };
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    let newest = next
        .reader
        .timeline
        .as_ref()
        .and_then(|timeline| timeline.newest_us);
    let Some(window) = displayed_window(&next).or_else(|| track::establish(&track, None, newest))
    else {
        return;
    };
    remember(&mut next, track::jump_window(&track, window, to_start));
    ui.set(next);
}

pub(super) fn timeline_key(event: KeyboardEvent) {
    match event.key() {
        Key::Enter => {
            event.prevent_default();
            commit_established();
        }
        Key::Home => {
            event.prevent_default();
            jump_selection(true);
        }
        Key::End => {
            event.prevent_default();
            jump_selection(false);
        }
        Key::ArrowRight => {
            event.prevent_default();
            nudge(1, event.modifiers().shift());
        }
        Key::ArrowLeft => {
            event.prevent_default();
            nudge(-1, event.modifiers().shift());
        }
        _ => {}
    }
}

pub(super) fn timeline_down(event: Event<PointerData>) {
    let x = event.data().client_coordinates().x;
    let Some(at_us) = held_at(x) else {
        return;
    };
    capture_pointer(&event);
    DRAG.with(|slot| *slot.borrow_mut() = Some(Drag { x, at_us }));
    playback::hold_stream();
    preview_at(at_us);
}

pub(super) fn timeline_move(event: Event<PointerData>) {
    let Some(mut drag) = DRAG.with(|slot| *slot.borrow()) else {
        return;
    };
    let x = event.data().client_coordinates().x;
    let Some(at_us) = held_at(x) else {
        return;
    };
    if at_us == drag.at_us {
        return;
    }
    drag.x = x;
    drag.at_us = at_us;
    DRAG.with(|slot| *slot.borrow_mut() = Some(drag));
    preview_at(at_us);
}

pub(super) fn timeline_up(event: Event<PointerData>) {
    let Some(drag) = DRAG.with(|slot| slot.borrow_mut().take()) else {
        return;
    };
    release_pointer(&event);
    SUPPRESS_CLICK.with(|cell| cell.set(true));
    cancel_preview();
    let x = event.data().client_coordinates().x;
    let at_us = held_at(x).unwrap_or(drag.at_us);
    commit_at(at_us);
}

fn held_at(x: f64) -> Option<i64> {
    let track = current_track()?;
    let at_us = us_at(x)?;
    track::session_at(&track, at_us).map(|window| window.start_us)
}

fn capture_pointer(event: &Event<PointerData>) {
    let Some(element) = timeline_element() else {
        return;
    };
    let _ = element.set_pointer_capture(event.data().pointer_id());
}

fn release_pointer(event: &Event<PointerData>) {
    let Some(element) = timeline_element() else {
        return;
    };
    let _ = element.release_pointer_capture(event.data().pointer_id());
}

fn scrub_element() -> Option<web_sys::Element> {
    let document = web_sys::window()?.document()?;
    document
        .query_selector("#video-timeline .video-track")
        .ok()
        .flatten()
        .or_else(|| document.get_element_by_id("video-timeline"))
}

fn timeline_element() -> Option<web_sys::Element> {
    web_sys::window()?
        .document()?
        .get_element_by_id("video-timeline")
}

struct Preview {
    seq: u32,
    inflight: bool,
    pending_us: Option<i64>,
}

thread_local! {
    static PREVIEW: std::cell::RefCell<Preview> = const { std::cell::RefCell::new(Preview {
        seq: 0,
        inflight: false,
        pending_us: None,
    }) };
}

fn cancel_preview() {
    PREVIEW.with(|slot| {
        let mut preview = slot.borrow_mut();
        preview.seq = preview.seq.wrapping_add(1);
        preview.inflight = false;
        preview.pending_us = None;
    });
}

fn preview_at(at_us: i64) {
    place_playhead(at_us);
    let launch = PREVIEW.with(|slot| {
        let mut preview = slot.borrow_mut();
        if preview.inflight {
            preview.pending_us = Some(at_us);
            None
        } else {
            preview.inflight = true;
            preview.pending_us = None;
            preview.seq = preview.seq.wrapping_add(1);
            Some(preview.seq)
        }
    });
    if let Some(seq) = launch {
        spawn_preview(seq, at_us);
    }
}

fn place_playhead(at_us: i64) {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    next.reader.frame_time = track::format_utc(at_us);
    ui.set(next);
    sync_live_speed();
}

fn spawn_preview(seq: u32, at_us: i64) {
    let Some(ui) = bound() else {
        finish_preview(seq);
        return;
    };
    let state = ui.peek().clone();
    let source_id = state.reader.source_id.clone();
    let generation = state.reader.generation;
    let secret = session::native_secret();
    if source_id.is_empty() {
        finish_preview(seq);
        return;
    }
    let at = track::format_utc(at_us);
    wasm_bindgen_futures::spawn_local(async move {
        let command = videos::frame_at_time_command(&source_id, &at);
        if let Ok(reply) = super::post(command, secret.clone(), generation).await
            && let Ok(frame) = videos::interpret_frame_at_time(&reply, secret.as_deref())
            && preview_current(seq)
        {
            if let Some(jpeg) = decode_frame(&frame.data) {
                playback::show_jpeg(&jpeg).await;
            }
            if preview_current(seq) {
                place_playhead_stamp(&frame.captured_at);
            }
        }
        let next = finish_preview(seq);
        if let Some(at_us) = next {
            preview_at(at_us);
        }
    });
}

fn preview_current(seq: u32) -> bool {
    PREVIEW.with(|slot| slot.borrow().seq == seq)
}

fn finish_preview(seq: u32) -> Option<i64> {
    PREVIEW.with(|slot| {
        let mut preview = slot.borrow_mut();
        if preview.seq != seq {
            return None;
        }
        preview.inflight = false;
        preview.pending_us.take()
    })
}

fn place_playhead_stamp(captured_at: &str) {
    if DRAG.with(|slot| slot.borrow().is_some()) {
        return;
    }
    let Some(us) = crate::videos::mjpeg::captured_us(captured_at) else {
        return;
    };
    place_playhead(us);
}

fn decode_frame(data: &str) -> Option<Vec<u8>> {
    let data = data
        .rsplit_once(',')
        .map(|(_, body)| body)
        .unwrap_or(data)
        .trim();
    let binary = web_sys::window()?.atob(data).ok()?;
    let mut bytes = Vec::with_capacity(binary.len());
    for ch in binary.chars() {
        let code = u32::from(ch);
        if code > 0xff {
            return None;
        }
        bytes.push(code as u8);
    }
    Some(bytes)
}

pub(super) fn timeline_click(event: Event<MouseData>) {
    if SUPPRESS_CLICK.with(|cell| cell.replace(false)) {
        return;
    }
    let point = event.data().client_coordinates();
    let Some(at_us) = interval_start_at(point.x, point.y).or_else(|| us_at(point.x)) else {
        return;
    };
    commit_at(at_us);
}

fn interval_start_at(x: f64, y: f64) -> Option<i64> {
    let nodes = web_sys::window()?
        .document()?
        .query_selector_all("#video-timeline .video-interval")
        .ok()?;
    for index in 0..nodes.length() {
        let element = nodes.get(index)?.dyn_into::<web_sys::Element>().ok()?;
        let rect = element.get_bounding_client_rect();
        if x < rect.left() || x > rect.right() || y < rect.top() || y > rect.bottom() {
            continue;
        }
        let start = element.get_attribute("data-start")?;
        return crate::videos::mjpeg::captured_us(&start);
    }
    None
}

pub(super) fn commit_typed_range() {
    let local = bound().is_some_and(|ui| ui.peek().local_time);
    let start = field_to_utc(&session::control_value("video-range-start"), local);
    let end = field_to_utc(&session::control_value("video-range-end"), local);
    commit_typed_bounds(&start, &end);
}

fn commit_typed_bounds(start: &str, end: &str) {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    next.range_start = start.to_owned();
    next.range_end = end.to_owned();
    let Ok(range) = videos::check_range(start, end) else {
        let _ = next.reader.select_range(start, end);
        ui.set(next);
        return;
    };
    let selection = Window {
        start_us: range.start_us,
        end_us: range.end_us,
    };
    if next.draft == Some(selection)
        && next.session.is_some_and(|session| {
            session.start_us == range.start_us && session.end_us <= range.end_us
        })
    {
        remember(&mut next, selection);
        ui.set(next);
        return;
    }
    ui.set(next);
    commit_selection(selection, Some(selection.start_us));
}

fn remember(next: &mut VideoUi, window: Window) {
    remember_window(next, window);
}

pub(super) fn remember_window(next: &mut VideoUi, window: Window) {
    next.range_start = track::format_utc(window.start_us);
    next.range_end = track::format_utc(window.end_us);
    next.range_draft_start = display_clock(&next.range_start, next.local_time);
    next.range_draft_end = display_clock(&next.range_end, next.local_time);
    next.range_error.clear();
    next.draft = Some(window);
}

pub(super) fn current_track() -> Option<Track> {
    let ui = bound()?;
    let timeline = ui.peek().reader.timeline.clone()?;
    Some(track::coverage_track(&timeline))
}

pub(super) fn us_at(client_x: f64) -> Option<i64> {
    let track = current_track()?;
    track::at_ratio(&track, timeline_ratio(client_x)?)
}

pub(super) fn timeline_ratio(client_x: f64) -> Option<f64> {
    let element = scrub_element()?;
    let rect = element.get_bounding_client_rect();
    let width = rect.width();
    if width <= 0.0 {
        return None;
    }
    Some(((client_x - rect.left()) / width).clamp(0.0, 1.0))
}

pub(super) fn displayed_window(state: &VideoUi) -> Option<Window> {
    if let Some(window) = state.draft {
        return Some(window);
    }
    if !state.range_start.is_empty() && !state.range_end.is_empty() {
        let start_us = crate::videos::mjpeg::captured_us(&state.range_start)?;
        let end_us = crate::videos::mjpeg::captured_us(&state.range_end)?;
        if end_us > start_us {
            return Some(Window { start_us, end_us });
        }
    }
    match &state.reader.selection {
        Selection::Recorded {
            start_us, end_us, ..
        } => Some(Window {
            start_us: *start_us,
            end_us: *end_us,
        }),
        Selection::Empty | Selection::Live => None,
    }
}

pub(super) fn bands(track: &Track, local: bool) -> Vec<Band> {
    let mut retained = 0_usize;
    let mut gaps = 0_usize;
    let mut items = Vec::new();
    for span in &track.spans {
        let (class, kind, id) = match span.kind {
            SpanKind::Retained => {
                let id = format!("video-interval-{retained}");
                retained += 1;
                ("video-interval", "retained", id)
            }
            SpanKind::Gap => {
                let id = format!("video-gap-{gaps}");
                gaps += 1;
                ("video-gap", "gap", id)
            }
        };
        let duration_us = span.end_us.saturating_sub(span.start_us);
        let start = format_shown(span.start_us, local);
        let end = format_shown(span.end_us, local);
        let seconds = duration_us as f64 / 1_000_000.0;
        let label = match span.kind {
            SpanKind::Retained => format!("Retained recording from {start} to {end}"),
            SpanKind::Gap => {
                format!("Recording gap from {start} to {end} ({seconds:.3} seconds unplayable)")
            }
        };
        items.push(Band {
            id,
            class,
            kind,
            start: span.start.clone(),
            end: span.end.clone(),
            start_us: span.start_us,
            end_us: span.end_us,
            duration_us,
            label,
            style: band_style(track, span.start_us, span.end_us),
        });
    }
    items
}

pub(super) fn band_style(track: &Track, start_us: i64, end_us: i64) -> String {
    let left = track::percent(track, start_us);
    let width = (track::percent(track, end_us) - left).max(0.0);
    format!("left:{left:.2}%;width:{width:.2}%")
}

pub(super) fn slider_now(track: Option<&Track>, window: Option<Window>) -> i64 {
    let Some(track) = track else {
        return 0;
    };
    window
        .map(|item| item.start_us.saturating_sub(track.origin_us).max(0))
        .unwrap_or(0)
}

pub(super) fn slider_text(window: Option<Window>, local: bool) -> String {
    match window {
        Some(window) => format!(
            "{} to {}",
            format_shown(window.start_us, local),
            format_shown(window.end_us, local)
        ),
        None => "No selection".to_owned(),
    }
}

pub(super) fn range_fault_message(fault: videos::RangeFault, local: bool) -> String {
    if local && fault == videos::RangeFault::NotUtc {
        "Enter a local time range.".to_owned()
    } else {
        fault.message().to_owned()
    }
}

pub(super) fn playhead_style(state: &VideoUi) -> Option<String> {
    let frame_time = state.reader.frame_time.as_str();
    if frame_time.is_empty() {
        return None;
    }
    let timeline = state.reader.timeline.as_ref()?;
    let track = track::coverage_track(timeline);
    let percent = track::playhead_percent(&track, frame_time)?;
    Some(format!("left:{percent:.2}%"))
}

pub(super) fn selection_slug(selection: &Selection) -> &'static str {
    match selection {
        Selection::Empty => "empty",
        Selection::Live => "live",
        Selection::Recorded { .. } => "recorded",
    }
}

pub(super) fn alert_text(reader: &videos::Reader) -> String {
    if let Some(failure) = reader.failure {
        return failure.message().to_owned();
    }
    if !reader.validation.is_empty() {
        return reader.validation.clone();
    }
    reader.fault.clone()
}

pub(super) fn begin_command(generation: u32) -> Option<web_sys::AbortSignal> {
    let abort = AbortController::new().ok()?;
    let signal = abort.signal();
    let flight = CommandFlight { generation, abort };
    let previous = COMMANDS.with(|slot| {
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

pub(super) fn abort_commands() {
    COMMANDS.with(|slot| {
        if let Some(flight) = slot.borrow_mut().take() {
            flight.abort.abort();
        }
    });
}
