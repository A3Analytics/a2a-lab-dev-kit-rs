use dioxus::prelude::*;

use super::{apply_typed_range, edit_range, follow_newest, timeline::display_instant};

#[component]
pub(super) fn VideoDetails(
    range_start: String,
    range_end: String,
    range_error: String,
    effective_start: Option<String>,
    effective_end: Option<String>,
    live_enabled: bool,
    live_active: bool,
) -> Element {
    let start_utc = effective_start
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| display_instant(&range_start));
    let end_utc = effective_end
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| display_instant(&range_end));
    let invalid = !range_error.is_empty();
    rsx! {
        div {
            id: "video-details",
            class: "video-filter-group",
            role: "group",
            aria_label: "Time range",
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
                "data-utc": "{start_utc}",
                placeholder: "2024-01-01 00:00:00 UTC",
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
                "data-utc": "{end_utc}",
                placeholder: "2024-01-01 00:00:02 UTC",
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
        }
    }
}

fn apply_on_enter(event: KeyboardEvent) {
    if event.key() == Key::Enter {
        event.prevent_default();
        apply_typed_range();
    }
}
