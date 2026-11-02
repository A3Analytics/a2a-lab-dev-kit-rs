use dioxus::prelude::*;

use super::{apply_typed_range, edit_range, follow_newest, toggle_local_time};

#[component]
pub(super) fn VideoDetails(
    range_start: String,
    range_end: String,
    local_time: bool,
    range_error: String,
    utc_start: String,
    utc_end: String,
    available_start: Option<String>,
    available_end: Option<String>,
    available_start_utc: Option<String>,
    available_end_utc: Option<String>,
    live_enabled: bool,
    live_active: bool,
) -> Element {
    let invalid = !range_error.is_empty();
    let placeholder_start = if local_time {
        "2024-01-01 00:00:00"
    } else {
        "2024-01-01 00:00:00 UTC"
    };
    let placeholder_end = if local_time {
        "2024-01-01 00:00:02"
    } else {
        "2024-01-01 00:00:02 UTC"
    };
    rsx! {
        div {
            id: "video-details",
            class: "video-filter-group",
            role: "group",
            aria_label: "Time range",
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
            p { "Time range" }
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
            label { r#for: "video-range-start", "Start" }
            input {
                id: "video-range-start",
                r#type: "text",
                spellcheck: false,
                autocomplete: "off",
                "aria-label": "Start",
                "aria-invalid": if invalid { "true" } else { "false" },
                aria_describedby: "video-range-error",
                "data-label": "Start",
                "data-utc": "{utc_start}",
                placeholder: "{placeholder_start}",
                value: "{range_start}",
                oninput: move |_| edit_range(true),
                onkeydown: apply_on_enter,
            }
            label { r#for: "video-range-end", "End" }
            input {
                id: "video-range-end",
                r#type: "text",
                spellcheck: false,
                autocomplete: "off",
                "aria-label": "End",
                "aria-invalid": if invalid { "true" } else { "false" },
                aria_describedby: "video-range-error",
                "data-label": "End",
                "data-utc": "{utc_end}",
                placeholder: "{placeholder_end}",
                value: "{range_end}",
                oninput: move |_| edit_range(false),
                onkeydown: apply_on_enter,
            }
            button {
                id: "video-range-apply",
                class: "video-outline",
                r#type: "button",
                onclick: move |_| apply_typed_range(),
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
        apply_typed_range();
    }
}
