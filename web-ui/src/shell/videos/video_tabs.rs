//! Browse/search tabs and browse-stream gating for the Videos reader.

use std::cell::Cell;

use crate::shell::nav::{self, VIDEOS_BROWSE_PATH, VIDEOS_SEARCH_PATH, VideoPanel};
use dioxus::prelude::ReadableExt;
use dioxus::prelude::*;

use super::{VideoTab, VideoUi, bound, suspend_browse_media, videos_visible};

thread_local! {
    static STREAM_ARMED: Cell<bool> = const { Cell::new(true) };
}

pub(crate) fn browse_streaming_allowed() -> bool {
    browse_panel_active() && STREAM_ARMED.with(Cell::get)
}

pub(crate) fn browse_commands_allowed() -> bool {
    browse_panel_active()
}

pub(super) fn arm_browse_stream_for_timeline() {
    STREAM_ARMED.with(|slot| slot.set(true));
}

#[allow(dead_code)]
pub(crate) fn open_browse_panel() {
    if nav::video_panel_for(&nav::current_route()) != VideoPanel::Browse {
        nav::open(VIDEOS_BROWSE_PATH);
    }
}

pub(super) fn disarm_browse_stream() {
    STREAM_ARMED.with(|slot| slot.set(false));
}

pub(super) fn arm_browse_stream() {
    STREAM_ARMED.with(|slot| slot.set(true));
}

fn browse_panel_active() -> bool {
    videos_visible()
        && matches!(
            nav::video_panel_for(&nav::published_route()),
            VideoPanel::Browse
        )
}

fn select_video_tab(tab: VideoTab) {
    let path = match tab {
        VideoTab::Browse => VIDEOS_BROWSE_PATH,
        VideoTab::Search => VIDEOS_SEARCH_PATH,
    };
    if nav::current_route() == path && bound().is_some_and(|ui| ui.peek().active_tab == tab) {
        return;
    }
    if tab == VideoTab::Search {
        suspend_browse_media();
    }
    sync_video_panel(match tab {
        VideoTab::Browse => VideoPanel::Browse,
        VideoTab::Search => VideoPanel::Search,
    });
    nav::open(path);
}

pub(super) fn sync_video_panel(panel: VideoPanel) {
    let tab = match panel {
        VideoPanel::Browse => VideoTab::Browse,
        VideoPanel::Search => VideoTab::Search,
    };
    let Some(mut ui) = bound() else {
        return;
    };
    let mut next = ui.peek().clone();
    if next.active_tab == tab {
        return;
    }
    next.active_tab = tab;
    ui.set(next);
}

#[component]
pub(super) fn VideoTabs(state: VideoUi) -> Element {
    rsx! {
        div {
            id: "video-tabs",
            class: "video-tabs",
            role: "tablist",
            aria_label: "Video views",
            button {
                id: "video-browse-tab",
                class: "video-tab",
                r#type: "button",
                role: "tab",
                aria_selected: if state.active_tab == VideoTab::Browse { "true" } else { "false" },
                aria_controls: "video-browse-panel",
                tabindex: if state.active_tab == VideoTab::Browse { "0" } else { "-1" },
                onclick: move |_| select_video_tab(VideoTab::Browse),
                "Browse"
            }
            button {
                id: "video-search-tab",
                class: "video-tab",
                r#type: "button",
                role: "tab",
                aria_selected: if state.active_tab == VideoTab::Search { "true" } else { "false" },
                aria_controls: "video-search-panel",
                tabindex: if state.active_tab == VideoTab::Search { "0" } else { "-1" },
                onclick: move |_| select_video_tab(VideoTab::Search),
                "Search"
            }
        }
    }
}
