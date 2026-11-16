//! Playback stage and status chrome for the Videos browse panel.

use crate::shell::loading::{StrandSize, StrandsLoader};
use crate::shell::playback;
use crate::videos::{Playback, Selection};
use dioxus::prelude::*;

use super::timeline::reselect_range;
use super::{
    TimelineScrubber, VideoUi, force_pause, native_pause, native_play, retry, toggle_playback,
};

pub(super) fn playback_slug(playback: Playback) -> &'static str {
    match playback {
        Playback::Idle => "idle",
        Playback::Connecting => "connecting",
        Playback::Playing => "playing",
        Playback::Paused => "paused",
        Playback::Stopped => "stopped",
    }
}

#[component]
pub(super) fn Results(state: VideoUi, status: String, alert: String) -> Element {
    use_effect(move || {
        playback::bind_media(native_play, native_pause, force_pause);
    });
    let media = state.reader.has_media();
    let show_empty = !media
        && !state.reader.loading
        && state.reader.playback != Playback::Connecting
        && alert.is_empty();
    let show_loading = alert.is_empty()
        && state.reader.frame_time.is_empty()
        && matches!(
            state.reader.playback,
            Playback::Connecting | Playback::Playing | Playback::Paused
        )
        && !matches!(state.reader.selection, Selection::Empty);
    let show_buffering = !show_loading && state.reader.buffering && alert.is_empty();
    let failure = state.reader.failure.map(|item| item.code()).unwrap_or("");
    rsx! {
        div { id: "video-results", class: "log-results",
            div { class: "log-scroll",
                div { id: "video-stage", class: "video-stage",
                    div {
                        class: "video-aspect",
                        "data-playback": playback_slug(state.reader.playback),
                        canvas {
                            id: "video-canvas",
                            class: "visually-hidden",
                            width: "640",
                            height: "360",
                            "aria-hidden": "true",
                        }
                        if show_empty {
                            p { id: "video-empty", class: "video-empty", "Select a source and a time." }
                        }
                        if matches!(state.reader.selection, Selection::Live) && !show_empty {
                            span { id: "video-live-badge", class: "video-live-badge", "LIVE" }
                        }
                        video {
                            id: "video-player",
                            class: "video-player",
                            hidden: !media,
                            muted: true,
                            playsinline: true,
                            preload: "none",
                            "aria-label": "Video playback",
                            onmounted: move |_| {
                                playback::bind_media(native_play, native_pause, force_pause);
                                playback::play_element();
                            },
                            onclick: move |_| toggle_playback(),
                        }
                        TimelineScrubber { state: state.clone() }
                        if show_loading {
                            div {
                                id: "video-loading",
                                class: "video-loading",
                                aria_hidden: "true",
                                div { class: "loading-surface",
                                    StrandsLoader { size: StrandSize::Md }
                                    p { class: "loading-title", "Loading…" }
                                }
                            }
                        }
                        if show_buffering {
                            div {
                                id: "video-buffering",
                                class: "video-buffering",
                                aria_hidden: "true",
                                StrandsLoader { size: StrandSize::Md }
                                p { "Buffering…" }
                            }
                        }
                    }
                }
                p {
                    id: "video-status",
                    class: "visually-hidden",
                    role: "status",
                    aria_live: "polite",
                    "data-state": playback_slug(state.reader.playback),
                    "data-buffering": if state.reader.buffering { "true" } else { "false" },
                    "data-frame-time": if state.reader.frame_time.is_empty() {
                        "none"
                    } else {
                        state.reader.frame_time.as_str()
                    },
                    "{status}"
                }
                div {
                    id: "video-alert",
                    class: "video-alert",
                    role: "alert",
                    aria_live: "assertive",
                    "data-failure": "{failure}",
                    if !alert.is_empty() {
                        span { id: "video-alert-text", "{alert}" }
                    }
                    if state.reader.can_retry() {
                        button {
                            id: "video-retry",
                            class: "video-outline",
                            r#type: "button",
                            onclick: move |_| retry(),
                            "Retry"
                        }
                    }
                    if state.reader.can_reselect() {
                        button {
                            id: "video-reselect",
                            class: "video-outline",
                            r#type: "button",
                            onclick: move |_| reselect_range(),
                            "Choose another range"
                        }
                    }
                }
            }
        }
    }
}
