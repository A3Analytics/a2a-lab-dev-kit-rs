//! Playback speed wiring for the Videos shell.

use dioxus::prelude::*;

use super::{VideoUi, bound, launch_stream, playback};
use crate::shell::session;
use crate::videos::{Playback, PlaybackSpeed, StreamPlan, plan_stream};

pub(super) fn active_stream_plan(state: &VideoUi) -> Option<StreamPlan> {
    if let Some(clip) = &state.reader.clip {
        return Some(plan_stream(
            state.speed,
            &clip.path,
            &clip.media_type,
            &clip.playback_8x,
        ));
    }
    if let Some(live) = &state.reader.live {
        return Some(plan_stream(
            state.speed,
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
    let speed = PlaybackSpeed::parse_control(&session::control_value("video-speed"));
    let Some(mut ui) = bound() else {
        return;
    };
    let previous = ui.peek().clone();
    let mut next = ui.peek().clone();
    if next.speed == speed {
        return;
    }
    let paused = next.reader.playback == Playback::Paused;
    if paused {
        playback::set_paused(true);
    }
    next.speed = speed;
    if let Some(plan) = active_stream_plan(&next) {
        note_trick_fallback(&mut next.reader, &plan);
        let path_changed = active_stream_plan(&previous)
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
