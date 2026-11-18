use dioxus::prelude::*;

use super::{VideoUi, apply_current_time, edit_current_time, follow_newest, toggle_local_time};

#[component]
pub(super) fn VideoDetails(
    state: VideoUi,
    current_time: String,
    local_time: bool,
    range_error: String,
    current_time_utc: String,
    available_start: Option<String>,
    available_end: Option<String>,
    available_start_utc: Option<String>,
    available_end_utc: Option<String>,
    live_enabled: bool,
    live_active: bool,
) -> Element {
    let invalid = !range_error.is_empty();
    let placeholder = if local_time {
        "2024-01-01 00:00:00"
    } else {
        "2024-01-01 00:00:00 UTC"
    };
    rsx! {
        div {
            id: "video-details",
            class: "video-filter-group",
            role: "group",
            aria_label: "Current time",
            "data-source-id": "{state.reader.source_id}",
            "data-time-zone": if local_time { "local" } else { "utc" },
            button {
                id: "video-follow-newest",
                class: if live_active { "video-primary video-live-active" } else { "video-primary" },
                r#type: "button",
                disabled: !live_enabled,
                aria_pressed: if live_active { "true" } else { "false" },
                onclick: move |_| follow_newest(),
                "LIVE"
            }
            p { "Current time" }
            button {
                id: "video-local-time",
                class: "log-facet",
                r#type: "button",
                aria_pressed: if local_time { "true" } else { "false" },
                aria_label: "Local time",
                onclick: move |_| toggle_local_time(),
                span { class: "log-facet-mark", aria_hidden: "true" }
                "Local time"
            }
            label { r#for: "video-current-time", "Current time" }
            input {
                id: "video-current-time",
                r#type: "text",
                spellcheck: false,
                autocomplete: "off",
                "aria-label": "Current time",
                "aria-invalid": if invalid { "true" } else { "false" },
                aria_describedby: "video-range-error",
                "data-label": "Current time",
                "data-utc": "{current_time_utc}",
                placeholder: "{placeholder}",
                value: "{current_time}",
                oninput: move |event| edit_current_time(event.value()),
                onkeydown: apply_on_enter,
            }
            button {
                id: "video-range-apply",
                class: "video-outline",
                r#type: "button",
                onclick: move |_| apply_current_time(),
                "Apply"
            }
            p { id: "video-range-error", class: "video-range-error", "{range_error}" }
            if let (Some(start_utc), Some(end_utc), Some(start), Some(end)) = (
                available_start_utc.as_ref(),
                available_end_utc.as_ref(),
                available_start.as_ref(),
                available_end.as_ref(),
            ) {
                div { id: "video-available", class: "video-available",
                    p { "Available" }
                    dl {
                        dt { "Start" }
                        dd {
                            time { id: "video-available-start", datetime: "{start_utc}", "{start}" }
                        }
                        dt { "End" }
                        dd {
                            if live_enabled {
                                span { id: "video-available-end", "data-live": "true", "Live" }
                            } else {
                                time { id: "video-available-end", datetime: "{end_utc}", "{end}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn apply_on_enter(event: KeyboardEvent) {
    if event.key() == Key::Enter {
        event.prevent_default();
        apply_current_time();
    }
}
