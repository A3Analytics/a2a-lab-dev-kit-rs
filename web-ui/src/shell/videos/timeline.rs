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

pub(super) fn format_shown(us: i64, local: bool) -> String {
    if local && let Some(text) = browser_local_stamp(us) {
        return text;
    }
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
    display_clock(stored, false)
}

pub(super) fn display_clock(stored: &str, local: bool) -> String {
    if stored.is_empty() {
        return String::new();
    }
    crate::videos::mjpeg::captured_us(stored)
        .map(|us| format_shown(us, local))
        .unwrap_or_else(|| stored.to_owned())
}

pub(super) fn reshow(text: &str, from_local: bool, to_local: bool) -> String {
    if text.trim().is_empty() || from_local == to_local {
        return text.to_owned();
    }
    let utc = field_to_utc(text, from_local);
    if crate::videos::mjpeg::captured_us(&utc).is_none() {
        return text.to_owned();
    }
    display_clock(&utc, to_local)
}

pub(super) fn range_fault_message(fault: videos::RangeFault, local: bool) -> String {
    if local && fault == videos::RangeFault::NotUtc {
        "Enter a local time range.".to_owned()
    } else {
        fault.message().to_owned()
    }
}

pub(super) fn field_to_utc(value: &str, local: bool) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    if let Some(us) = crate::videos::mjpeg::captured_us(value) {
        return track::format_utc(us);
    }
    let (body, zone) = split_zone(value);
    if zone.is_some_and(|item| item.eq_ignore_ascii_case("utc")) {
        return naive_text_to_utc(body).unwrap_or_else(|| value.to_owned());
    }
    if local && let Some(utc) = local_wall_to_utc(body) {
        return utc;
    }
    naive_text_to_utc(value).unwrap_or_else(|| value.to_owned())
}

fn split_zone(value: &str) -> (&str, Option<&str>) {
    let value = value.trim();
    let Some((body, zone)) = value.rsplit_once(' ') else {
        return (value, None);
    };
    if zone.is_empty() || zone.contains(':') || zone.chars().all(|ch| ch.is_ascii_digit()) {
        return (value, None);
    }
    (body.trim(), Some(zone))
}

struct Wall {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    digits: String,
}

fn naive_text_to_utc(value: &str) -> Option<String> {
    let wall = parse_wall(value)?;
    let rfc = wall_rfc(&wall);
    crate::videos::mjpeg::captured_us(&rfc)?;
    Some(rfc)
}

fn parse_wall(value: &str) -> Option<Wall> {
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
    Some(Wall {
        year,
        month,
        day,
        hour,
        minute,
        second,
        digits,
    })
}

fn wall_rfc(wall: &Wall) -> String {
    let frac = if wall.digits.is_empty() {
        String::new()
    } else {
        format!(".{}", wall.digits)
    };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{frac}Z",
        wall.year, wall.month, wall.day, wall.hour, wall.minute, wall.second
    )
}

fn local_wall_to_utc(value: &str) -> Option<String> {
    browser_local_to_utc(&parse_wall(value)?)
}

fn browser_local_stamp(us: i64) -> Option<String> {
    let date = browser_date_from_us(us)?;
    let text = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        date.year, date.month, date.day, date.hour, date.minute, date.second
    );
    if date.zone.is_empty() {
        Some(text)
    } else {
        Some(format!("{text} {}", date.zone))
    }
}

struct LocalParts {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    zone: String,
}

#[cfg(not(target_arch = "wasm32"))]
fn browser_local_to_utc(_wall: &Wall) -> Option<String> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn browser_date_from_us(_us: i64) -> Option<LocalParts> {
    None
}

#[cfg(target_arch = "wasm32")]
fn browser_local_to_utc(wall: &Wall) -> Option<String> {
    if wall.year < 0 {
        return None;
    }
    let millis = wall_millis(&wall.digits);
    let date = js_sys::Date::new_with_year_month_day_hr_min_sec(
        wall.year as u32,
        wall.month as i32 - 1,
        wall.day as i32,
        wall.hour as i32,
        wall.minute as i32,
        wall.second as i32,
    );
    date.set_milliseconds(millis);
    if date.get_full_year() as i32 != wall.year
        || date.get_month() + 1 != wall.month
        || date.get_date() != wall.day
        || date.get_hours() != wall.hour
        || date.get_minutes() != wall.minute
        || date.get_seconds() != wall.second
    {
        return None;
    }
    let micros = wall_micros(&wall.digits);
    let us = date.get_time().round() as i64 * 1000 + (micros % 1000) as i64;
    Some(track::format_utc(us))
}

#[cfg(target_arch = "wasm32")]
fn browser_date_from_us(us: i64) -> Option<LocalParts> {
    let ms = us as f64 / 1000.0;
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms));
    if date.get_time().is_nan() {
        return None;
    }
    Some(LocalParts {
        year: date.get_full_year() as i32,
        month: date.get_month() + 1,
        day: date.get_date(),
        hour: date.get_hours(),
        minute: date.get_minutes(),
        second: date.get_seconds(),
        zone: zone_abbrev(ms),
    })
}

#[cfg(target_arch = "wasm32")]
fn zone_abbrev(ms: f64) -> String {
    let Ok(intl) = js_sys::Reflect::get(&js_sys::global(), &"Intl".into()) else {
        return String::new();
    };
    let Ok(ctor) = js_sys::Reflect::get(&intl, &"DateTimeFormat".into()) else {
        return String::new();
    };
    let Ok(ctor) = ctor.dyn_into::<js_sys::Function>() else {
        return String::new();
    };
    let options = js_sys::Object::new();
    if js_sys::Reflect::set(&options, &"timeZoneName".into(), &"short".into()).is_err() {
        return String::new();
    }
    let Ok(formatter) = js_sys::Reflect::construct(
        &ctor,
        &js_sys::Array::of2(&wasm_bindgen::JsValue::UNDEFINED, &options),
    ) else {
        return String::new();
    };
    let Ok(format_to_parts) = js_sys::Reflect::get(&formatter, &"formatToParts".into()) else {
        return String::new();
    };
    let Ok(format_to_parts) = format_to_parts.dyn_into::<js_sys::Function>() else {
        return String::new();
    };
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms));
    let Ok(parts) = format_to_parts.call1(&formatter, &date) else {
        return String::new();
    };
    for part in js_sys::Array::from(&parts).iter() {
        let Ok(kind) = js_sys::Reflect::get(&part, &"type".into()) else {
            continue;
        };
        if kind.as_string().as_deref() != Some("timeZoneName") {
            continue;
        }
        return js_sys::Reflect::get(&part, &"value".into())
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_default();
    }
    String::new()
}

#[cfg(target_arch = "wasm32")]
fn wall_millis(digits: &str) -> u32 {
    wall_micros(digits) / 1000
}

#[cfg(target_arch = "wasm32")]
fn wall_micros(digits: &str) -> u32 {
    if digits.is_empty() {
        return 0;
    }
    format!("{digits:0<6}").parse().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::field_to_utc;

    #[test]
    fn explicit_utc_text_stays_utc_in_either_mode() {
        assert_eq!(
            field_to_utc("2024-01-01 00:00:00 UTC", false),
            "2024-01-01T00:00:00Z"
        );
        assert_eq!(
            field_to_utc("2024-01-01 00:00:00 UTC", true),
            "2024-01-01T00:00:00Z"
        );
        assert_eq!(
            field_to_utc("2024-01-01T00:00:00Z", true),
            "2024-01-01T00:00:00Z"
        );
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
