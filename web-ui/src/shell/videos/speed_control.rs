//! Playback speed wiring for the Videos shell.

use dioxus::prelude::*;

use super::{VideoUi, bound, launch_stream, playback};
use crate::shell::session;
use crate::videos::speed::synced_with_live;
use crate::videos::{Playback, PlaybackSpeed, Selection, StreamPlan, plan_stream};

pub(super) fn speed_option_disabled(state: &VideoUi, rate: PlaybackSpeed) -> bool {
    live_synced(state) && rate != PlaybackSpeed::One
}

pub(super) fn lock_live_speed(next: &mut VideoUi) {
    next.speed = PlaybackSpeed::One;
}

pub(super) fn sync_live_speed() {
    let Some(ui) = bound() else {
        return;
    };
    let previous = ui.peek().clone();
    if !live_synced(&previous) || previous.speed == PlaybackSpeed::One {
        return;
    }
    session::write_control("video-speed", PlaybackSpeed::One.control_value());
    commit_speed(ui, previous, PlaybackSpeed::One);
}

fn live_synced(state: &VideoUi) -> bool {
    let timeline = state.reader.timeline.as_ref();
    synced_with_live(
        matches!(state.reader.selection, Selection::Live),
        timeline.is_some_and(|timeline| timeline.live_available),
        timeline.and_then(|timeline| timeline.newest_us),
        &state.reader.frame_time,
    )
}

fn plan_speed(state: &VideoUi) -> PlaybackSpeed {
    if live_synced(state) {
        PlaybackSpeed::One
    } else {
        state.speed
    }
}

pub(super) fn active_stream_plan(state: &VideoUi) -> Option<StreamPlan> {
    stream_plan(state, plan_speed(state))
}

fn stream_plan(state: &VideoUi, speed: PlaybackSpeed) -> Option<StreamPlan> {
    if let Some(clip) = &state.reader.clip {
        return Some(plan_stream(
            speed,
            &clip.path,
            &clip.media_type,
            &clip.playback_8x,
        ));
    }
    if let Some(live) = &state.reader.live {
        return Some(plan_stream(
            speed,
            &live.path,
            &live.media_type,
            &live.playback_8x,
        ));
    }
    None
}

pub(super) fn note_trick_fallback(reader: &mut crate::videos::Reader, plan: &StreamPlan) {
    if plan.trick_fallback
        && matches!(
            reader.playback,
            Playback::Connecting | Playback::Playing | Playback::Paused
        )
    {
        reader.playback_note = "8x trick play unavailable; playing at 1x.".to_owned();
    }
}

pub(super) fn change_speed() {
    let parsed = PlaybackSpeed::parse_control(&session::control_value("video-speed"));
    let Some(ui) = bound() else {
        return;
    };
    let previous = ui.peek().clone();
    let speed = if live_synced(&previous) {
        session::write_control("video-speed", PlaybackSpeed::One.control_value());
        PlaybackSpeed::One
    } else {
        parsed
    };
    if previous.speed == speed {
        return;
    }
    commit_speed(ui, previous, speed);
}

fn commit_speed(mut ui: Signal<VideoUi>, previous: VideoUi, speed: PlaybackSpeed) {
    let mut next = previous.clone();
    let paused = next.reader.playback == Playback::Paused;
    if paused {
        playback::set_paused(true);
    }
    next.speed = speed;
    if playback::frames_held() {
        ui.set(next);
        return;
    }
    if let Some(plan) = stream_plan(&next, speed) {
        note_trick_fallback(&mut next.reader, &plan);
        let path_changed = stream_plan(&previous, previous.speed)
            .is_none_or(|previous_plan| previous_plan.fetch_path != plan.fetch_path);
        ui.set(next.clone());
        if path_changed {
            relaunch_active_stream(&mut next);
            ui.set(next);
        } else {
            playback::apply_plan(&plan);
            ui.set(next);
        }
    } else {
        ui.set(next);
    }
    if paused {
        playback::set_paused(true);
        playback::pause_element();
    }
}

fn relaunch_active_stream(next: &mut VideoUi) {
    let paused = next.reader.playback == Playback::Paused;
    let Some(plan) = active_stream_plan(next) else {
        return;
    };
    playback::abort();
    if paused {
        playback::set_paused(true);
    }
    next.reader.generation = next.reader.generation.wrapping_add(1);
    launch_stream(next.reader.generation, plan);
}
