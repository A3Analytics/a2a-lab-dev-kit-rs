//! Stream lifecycle callbacks for the Videos shell.

use dioxus::prelude::*;

use super::{bound, playback, run};
use crate::shell::playback::StreamOutcome;

pub(super) fn apply_stream(outcome: StreamOutcome) {
    run(move || {
        let Some(mut ui) = bound() else {
            return;
        };
        let mut next = ui.peek().clone();
        let changed = match outcome {
            StreamOutcome::Stopped { generation } => {
                if playback::live(generation) {
                    false
                } else {
                    next.reader.finish(generation)
                }
            }
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

pub(super) fn on_playing(generation: u32) {
    run(move || {
        if playback::start_paused() {
            return;
        }
        let Some(mut ui) = bound() else {
            return;
        };
        let mut next = ui.peek().clone();
        if next.reader.playing(generation) {
            ui.set(next);
        }
    });
}

pub(super) fn on_frame(generation: u32, captured_at: &str) {
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

pub(super) fn on_gap_skipped(generation: u32) {
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
