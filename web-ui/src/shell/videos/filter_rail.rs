//! Source picker and playback controls for the Videos browse panel.

use crate::images::{self, SourcePhase};
use crate::inventory::Item;
use crate::shell::session;
use crate::videos::{self, PlaybackSpeed};
use dioxus::prelude::*;

use super::details::VideoDetails;
use super::speed_control::{change_speed, speed_option_disabled};
use super::timeline::{display_clock, display_instant};
use super::{VideoUi, choose_source};

fn source_slug(phase: SourcePhase) -> &'static str {
    match phase {
        SourcePhase::Loading => "loading",
        SourcePhase::None => "sources",
        SourcePhase::Error => "error",
        SourcePhase::Ready => "ready",
    }
}

pub(crate) fn source_option(item: &Item) -> String {
    if item.name.is_empty() {
        item.id.clone()
    } else {
        item.name.clone()
    }
}

fn choose_listed_source() {
    let id = session::control_value("video-source");
    if !id.is_empty() {
        choose_source(&id);
    }
}

fn committed_utc(stored: &str, effective: Option<&str>) -> String {
    let source = effective
        .filter(|value| !value.is_empty())
        .unwrap_or(stored);
    display_instant(source)
}

#[component]
pub(super) fn FilterRail(state: VideoUi) -> Element {
    let track = state
        .reader
        .timeline
        .as_ref()
        .map(crate::videos::track::coverage_track);
    let available_start_utc = track
        .as_ref()
        .filter(|item| item.span_us() > 0)
        .map(|item| crate::videos::track::format_utc(item.origin_us));
    let available_end_utc = track
        .as_ref()
        .filter(|item| item.span_us() > 0)
        .map(|item| crate::videos::track::format_utc(item.end_us));
    let local = state.local_time;
    let available_start = available_start_utc
        .as_deref()
        .map(|value| display_clock(value, local));
    let available_end = available_end_utc
        .as_deref()
        .map(|value| display_clock(value, local));
    rsx! {
        nav { id: "video-filters", class: "video-filters", aria_label: "Video filters",
            div { id: "video-sources", class: "video-filter-group",
                p { id: "video-source-label", "Source" }
                p {
                    id: "video-source-status",
                    role: "status",
                    aria_live: "polite",
                    "data-state": source_slug(state.phase),
                    "{videos::source_copy(state.phase)}"
                }
                div { id: "video-source-alert", role: "alert", aria_live: "assertive", "{state.source_alert}" }
                select {
                    id: "video-source",
                    aria_labelledby: "video-source-label",
                    disabled: state.sources.is_empty(),
                    onchange: move |_| choose_listed_source(),
                    if state.sources.is_empty() {
                        option { value: "", "{images::source_placeholder(state.phase)}" }
                    }
                    for item in state.sources.iter() {
                        option {
                            key: "{item.id}",
                            value: "{item.id}",
                            selected: state.reader.source_id == item.id,
                            "{source_option(item)}"
                        }
                    }
                }
            }
            if !state.reader.source_id.is_empty() {
                VideoDetails {
                    state: state.clone(),
                    current_time: if state.range_draft_start.is_empty() {
                        display_clock(
                            if state.reader.frame_time.is_empty() {
                                &state.range_start
                            } else {
                                &state.reader.frame_time
                            },
                            local,
                        )
                    } else {
                        state.range_draft_start.clone()
                    },
                    local_time: local,
                    range_error: state.range_error.clone(),
                    current_time_utc: if state.reader.frame_time.is_empty() {
                        committed_utc(
                            &state.range_start,
                            state
                                .reader
                                .clip
                                .as_ref()
                                .map(|clip| clip.effective_start.as_str()),
                        )
                    } else {
                        state.reader.frame_time.clone()
                    },
                    available_start,
                    available_end,
                    available_start_utc,
                    available_end_utc,
                    live_enabled: videos::shows_live_button(&state.reader),
                    live_active: matches!(state.reader.selection, videos::Selection::Live),
                }
            }
            div { class: "video-filter-group video-actions",
                p { "Playback options" }
                div { class: "video-speed",
                    label { r#for: "video-speed", "Speed" }
                    select {
                        id: "video-speed",
                        aria_label: "Playback speed",
                        onchange: move |_| change_speed(),
                        for rate in PlaybackSpeed::all() {
                            option {
                                key: "{rate.label()}",
                                value: "{rate.control_value()}",
                                selected: state.speed == *rate,
                                disabled: speed_option_disabled(&state, *rate),
                                "{rate.label()}"
                            }
                        }
                    }
                }
            }
        }
    }
}
