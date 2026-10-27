//! Timeline selection, coverage drawing, and command cancellation.

use dioxus::prelude::*;
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
        ui.set(next);
        playback::pause_element();
    }
}

pub(super) fn commit_at(at_us: i64) {
    let Some(track) = current_track() else {
        return;
    };
    let Some(window) = track::window_at(&track, at_us) else {
        return;
    };
    commit_window(window);
}

pub(super) fn commit_established() {
    let Some(track) = current_track() else {
        return;
    };
    let current = bound().and_then(|ui| ui.peek().draft);
    let Some(window) = current.or_else(|| track::initial_window(&track)) else {
        return;
    };
    commit_window(window);
}

pub(super) fn commit_window(window: Window) {
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    remember(&mut next, window);
    let start = next.range_start.clone();
    let end = next.range_end.clone();
    playback::abort();
    abort_commands();
    playback::set_paused(false);
    let ticket = match next.reader.select_range(&start, &end) {
        Ok(ticket) => ticket,
        Err(_) => {
            ui.set(next);
            playback::pause_element();
            return;
        }
    };
    ui.set(next);
    launch_clip(ticket);
}

pub(super) fn nudge(steps: i64, extend: bool) {
    let Some(track) = current_track() else {
        return;
    };
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    let Some(window) = next.draft.or_else(|| track::initial_window(&track)) else {
        return;
    };
    if next.draft.is_none() {
        remember(&mut next, window);
        ui.set(next);
        return;
    }
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
    let Some(window) = next.draft.or_else(|| track::initial_window(&track)) else {
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
    let Some(at_us) = us_at(event.data().client_coordinates().x) else {
        return;
    };
    commit_at(at_us);
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
    let start = session::control_value("video-range-start");
    let end = session::control_value("video-range-end");
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    next.range_start = start.clone();
    next.range_end = end.clone();
    playback::abort();
    abort_commands();
    playback::set_paused(false);
    let ticket = match next.reader.select_range(&start, &end) {
        Ok(ticket) => ticket,
        Err(_) => {
            ui.set(next);
            playback::pause_element();
            return;
        }
    };
    if let Selection::Recorded {
        start_us, end_us, ..
    } = next.reader.selection
    {
        remember(
            &mut next,
            Window {
                start_us,
                end_us,
            },
        );
    }
    ui.set(next);
    launch_clip(ticket);
}

fn remember(next: &mut VideoUi, window: Window) {
    next.range_start = track::format_utc(window.start_us);
    next.range_end = track::format_utc(window.end_us);
    next.draft = Some(window);
}

pub(super) fn current_track() -> Option<Track> {
    let ui = bound()?;
    let timeline = ui.peek().reader.timeline.clone()?;
    Some(track::coverage_track(&timeline))
}

pub(super) fn frame_cursor_now() -> Option<String> {
    let ui = bound()?;
    let state = ui.peek().clone();
    frame_cursor(&state)
}

pub(super) fn frame_cursor(state: &VideoUi) -> Option<String> {
    let timeline = state.reader.timeline.as_ref()?;
    let track = track::coverage_track(timeline);
    let at = state
        .draft
        .map(|window| window.start_us)
        .or_else(|| track::initial_window(&track).map(|window| window.start_us))?;
    track::retained_instant(&track, at)
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
        items.push(Band {
            id,
            class,
            kind,
            start: span.start.clone(),
            end: span.end.clone(),
            start_us: span.start_us,
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
            track::format_utc(window.start_us),
            track::format_utc(window.end_us)
        ),
        None => "No selection".to_owned(),
    }
}

pub(super) fn follow_label(selection: &Selection) -> &'static str {
    if matches!(selection, Selection::Recorded { .. }) {
        "Return to newest"
    } else {
        "Follow newest"
    }
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
