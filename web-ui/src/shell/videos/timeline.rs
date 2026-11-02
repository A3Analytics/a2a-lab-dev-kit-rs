//! Timeline selection, coverage drawing, and command cancellation.

use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::AbortController;

use super::{
    Band, COMMANDS, CommandFlight, DRAG, Drag, SUPPRESS_CLICK, VideoUi, bound, launch_clip,
    launch_meta,
};
use crate::images::SourcePhase;
use crate::shell::playback;
use crate::shell::session;
use crate::videos::track::{self, SpanKind, Track, Window};
use crate::videos::{self, Selection, Timeline};

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

pub(super) fn commit_window(selection: Window) {
    commit_selection(selection, Some(selection.start_us));
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
    let Some(at_us) = us_at(x) else {
        return;
    };
    DRAG.with(|slot| *slot.borrow_mut() = Some(Drag { x, at_us }));
}

pub(super) fn timeline_move(event: Event<PointerData>) {
    let Some(drag) = DRAG.with(|slot| *slot.borrow()) else {
        return;
    };
    let x = event.data().client_coordinates().x;
    if (x - drag.x).abs() < 4.0 {
        return;
    }
    let Some(at_us) = us_at(x) else {
        return;
    };
    let Some(track) = current_track() else {
        return;
    };
    let Some(window) = track::drag_window(&track, drag.at_us, at_us) else {
        return;
    };
    set_draft(window);
}

pub(super) fn timeline_up(event: Event<PointerData>) {
    let Some(drag) = DRAG.with(|slot| slot.borrow_mut().take()) else {
        return;
    };
    let x = event.data().client_coordinates().x;
    if (x - drag.x).abs() < 4.0 {
        return;
    }
    SUPPRESS_CLICK.with(|cell| cell.set(true));
    let Some(at_us) = us_at(x) else {
        return;
    };
    let Some(track) = current_track() else {
        return;
    };
    if let Some(window) = track::drag_window(&track, drag.at_us, at_us) {
        commit_window(window);
    }
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

pub(super) fn set_draft(window: Window) {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    remember(&mut next, window);
    ui.set(next);
}

pub(super) fn commit_typed_range() {
    let start = field_to_utc(&session::control_value("video-range-start"));
    let end = field_to_utc(&session::control_value("video-range-end"));
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
    next.range_draft_start = display_instant(&next.range_start);
    next.range_draft_end = display_instant(&next.range_end);
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
    let element = web_sys::window()?
        .document()?
        .get_element_by_id("video-timeline")?;
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

pub(super) fn bands(track: &Track) -> Vec<Band> {
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
        let start = format_local(span.start_us);
        let end = format_local(span.end_us);
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

pub(super) fn newest_style(track: Option<&Track>, timeline: Option<&Timeline>) -> Option<String> {
    let at = timeline?.newest_us?;
    Some(format!("left:{:.2}%", track::percent(track?, at)))
}

pub(super) fn slider_now(track: Option<&Track>, window: Option<Window>) -> i64 {
    let Some(track) = track else {
        return 0;
    };
    window
        .map(|item| item.start_us.saturating_sub(track.origin_us).max(0))
        .unwrap_or(0)
}

pub(super) fn slider_text(window: Option<Window>) -> String {
    match window {
        Some(window) => format!(
            "{} to {}",
            format_local(window.start_us),
            format_local(window.end_us)
        ),
        None => "No selection".to_owned(),
    }
}

pub(super) fn format_local(us: i64) -> String {
    pretty_utc(&track::format_utc(us))
}

fn pretty_utc(stored: &str) -> String {
    let body = stored.trim_end_matches('Z');
    let Some((date, time)) = body.split_once('T') else {
        return stored.to_owned();
    };
    let clock = time.split('.').next().unwrap_or(time);
    format!("{date} {clock} UTC")
}

pub(super) fn display_instant(stored: &str) -> String {
    if stored.is_empty() {
        return String::new();
    }
    crate::videos::mjpeg::captured_us(stored)
        .map(format_local)
        .unwrap_or_else(|| stored.to_owned())
}

pub(super) fn field_to_utc(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    if let Some(us) = crate::videos::mjpeg::captured_us(value) {
        return track::format_utc(us);
    }
    naive_text_to_utc(value).unwrap_or_else(|| value.to_owned())
}

fn naive_text_to_utc(value: &str) -> Option<String> {
    let value = value.trim();
    let value = value
        .strip_suffix(" UTC")
        .or_else(|| value.strip_suffix(" utc"))
        .unwrap_or(value)
        .trim();
    let (date, time) = value.split_once('T').or_else(|| value.split_once(' '))?;
    let mut date_parts = date.split('-');
    let year: i32 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (clock, fraction) = match time.split_once('.') {
        Some((clock, fraction)) => (clock, fraction),
        None => (time, ""),
    };
    let mut clock_parts = clock.split(':');
    let hour: u32 = clock_parts.next()?.parse().ok()?;
    let minute: u32 = clock_parts.next()?.parse().ok()?;
    let second: u32 = clock_parts.next()?.parse().ok()?;
    if clock_parts.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let digits: String = fraction
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    if digits.len() != fraction.len() || digits.len() > 6 {
        return None;
    }
    let frac = if digits.is_empty() {
        String::new()
    } else {
        format!(".{digits}")
    };
    let rfc = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}{frac}Z");
    crate::videos::mjpeg::captured_us(&rfc)?;
    Some(rfc)
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
