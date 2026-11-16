//! Clip and metadata request delivery into the Videos reader.

use crate::inventory::Fault;
use crate::shell::nav;
use crate::shell::playback;
use crate::shell::session::{self, calls_allowed, take_secret};
use crate::videos::{
    self, AutoPlayback, ClipHandle, Failure, Kind, LiveHandle, Selection, Ticket, Timeline,
};
use dioxus::prelude::*;

use super::semantic::note_source_change;
use super::speed_control::{active_stream_plan, lock_live_speed, note_trick_fallback};
use super::timeline::{displayed_window, remember_window};
use super::video_tabs::{browse_commands_allowed, browse_streaming_allowed};
use super::{
    VideoUi, bound, clear_range_replay, launch_stream, loader, post, replay_retained_start, run,
    take_range_replay,
};

#[derive(Clone, PartialEq)]
pub(super) enum ClipJob {
    Idle,
    Run(Ticket),
}

#[derive(Clone, PartialEq)]
pub(super) enum ClipOutcome {
    Idle,
    Ready { ticket: Ticket, clip: ClipHandle },
    Failed { ticket: Ticket, message: String },
    Coded { ticket: Ticket, failure: Failure },
}

pub(super) enum MetaOutcome {
    Timeline { ticket: Ticket, timeline: Timeline },
    Follow { ticket: Ticket, follow: LiveHandle },
    Failed { ticket: Ticket, message: String },
    Coded { ticket: Ticket, failure: Failure },
}

pub(in crate::shell::videos) fn launch_clip(ticket: Ticket) {
    if !browse_streaming_allowed() {
        return;
    }
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

pub(super) async fn run_clip(job: ClipJob, secret: Option<String>) -> ClipOutcome {
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

pub(super) fn apply_clip(mut ui: Signal<VideoUi>, outcome: ClipOutcome) {
    if nav::route_for(&nav::published_route()).view != "view-videos" {
        return;
    }
    if !browse_streaming_allowed() || !super::videos_visible() {
        return;
    }
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

pub(in crate::shell::videos) fn launch_meta(ticket: Ticket) {
    if !browse_commands_allowed() {
        return;
    }
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
        let window = crate::videos::track::Window { start_us, end_us };
        remember_window(next, window);
        next.session = Some(window);
    }
}

pub(super) fn apply_meta(outcome: MetaOutcome) {
    if nav::route_for(&nav::published_route()).view != "view-videos" {
        return;
    }
    if !browse_streaming_allowed() || !super::videos_visible() {
        return;
    }
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
            if browse_streaming_allowed() {
                auto = videos::begin_auto_playback(&mut next.reader);
                sync_auto_range(&mut next, &auto);
            }
            note_source_change(&next.reader.source_id);
            let seek = super::semantic::take_search_seek(&next.reader.source_id);
            if let Some((us, autoplay)) = seek {
                ui.set(next);
                super::timeline::commit_at(us, autoplay);
                return;
            }
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

pub(super) fn fault_message(fault: &Fault) -> String {
    match fault {
        Fault::Unavailable(message) => message.clone(),
        Fault::Unauthorized(message) | Fault::Forbidden(message) => message.clone(),
        Fault::Malformed => "The video result could not be read.".to_owned(),
        Fault::Transport => "The browser could not reach this origin.".to_owned(),
    }
}
