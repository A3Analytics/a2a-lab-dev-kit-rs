//! Route and tab transitions that stop playback and mirror chrome state.

use dioxus::prelude::*;
use wasm_bindgen::JsCast;

use crate::shell::nav::{self, VideoPanel};
use crate::shell::playback;
use crate::videos::Playback;

use super::results_stage::playback_slug;
use super::semantic::abort_semantic_commands;
use super::timeline::abort_commands;
use super::timeline_refresh::stop_coverage_refresh;
use super::video_tabs::disarm_browse_stream;
use super::{bound, videos};

pub(super) fn suspend_browse_media() {
    if nav::video_panel_for(&nav::published_route()) == VideoPanel::Browse {
        return;
    }
    disarm_browse_stream();
    playback::abort();
    abort_commands();
    let Some(mut ui) = bound() else {
        playback::pause_element();
        return;
    };
    let mut next = ui.peek().clone();
    let active = next.reader.loading
        || next.reader.has_media()
        || matches!(
            next.reader.playback,
            Playback::Connecting | Playback::Playing | Playback::Paused
        );
    if active {
        next.reader.stop();
        playback::set_paused(true);
        ui.set(next.clone());
        mirror_reader(&next.reader);
    }
    playback::pause_element();
}

pub(crate) fn leave_videos() {
    super::clear_applied_browse_query();
    playback::bump_ignore_pause();
    disarm_browse_stream();
    playback::abort();
    abort_commands();
    abort_semantic_commands();
    stop_coverage_refresh();
    if let Some(mut loader) = super::loader() {
        loader.clip.set(super::ClipJob::Idle);
    }
    let Some(mut ui) = bound() else {
        playback::pause_element();
        return;
    };
    let mut next = ui.peek().clone();
    next.reader.stop();
    playback::set_paused(true);
    ui.set(next.clone());
    mirror_reader(&next.reader);
    schedule_reader_mirror();
    playback::pause_element();
}

fn schedule_reader_mirror() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let closure = wasm_bindgen::closure::Closure::once(move || {
        let Some(ui) = bound() else {
            return;
        };
        mirror_reader(&ui.peek().reader);
    });
    let _ = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(closure.as_ref().unchecked_ref(), 0);
    closure.forget();
}

pub(super) fn mirror_reader(reader: &videos::Reader) {
    mirror_status(reader);
    let Some(view) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("video-view"))
    else {
        return;
    };
    let _ = view.set_attribute("data-gen", &reader.generation.to_string());
}

pub(super) fn mirror_status(reader: &videos::Reader) {
    let Some(status) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("video-status"))
    else {
        return;
    };
    let _ = status.set_attribute("data-state", playback_slug(reader.playback));
    status.set_text_content(Some(&videos::playback_status(reader)));
}

pub(crate) fn videos_visible() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("view-videos"))
        .is_some_and(|element| !element.has_attribute("hidden"))
}
