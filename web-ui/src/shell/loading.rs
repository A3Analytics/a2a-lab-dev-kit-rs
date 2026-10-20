//! Shared loading region, strand loader, and layout-matched skeletons.

use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LoadProgress {
    Omitted,
    Unknown,
    Percent(u8),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StrandSize {
    Xs,
    Sm,
    Md,
}

#[component]
pub fn CardLoading(concealed: bool) -> Element {
    rsx! {
        div {
            id: "loading-root",
            hidden: concealed,
            LoadingSurface {
                id: "card-loading".to_owned(),
                title: Some("Loading agent card.".to_owned()),
                description: None,
                progress: LoadProgress::Omitted,
                concealed: false,
            }
            LoadingSurface {
                id: "loading-progress".to_owned(),
                title: Some("Uploading files".to_owned()),
                description: Some("2 of 3 files uploaded".to_owned()),
                progress: LoadProgress::Percent(66),
                concealed: true,
            }
            LoadingSurface {
                id: "loading-ready".to_owned(),
                title: None,
                description: None,
                progress: LoadProgress::Percent(100),
                concealed: true,
            }
            LoadingSurface {
                id: "loading-unknown".to_owned(),
                title: Some("Loading agent card.".to_owned()),
                description: None,
                progress: LoadProgress::Unknown,
                concealed: true,
            }
            div {
                id: "strand-sizes",
                hidden: true,
                aria_hidden: "true",
                StrandsLoader { size: StrandSize::Xs }
                StrandsLoader { size: StrandSize::Sm }
            }
            RailSkeleton {}
            ListSkeleton {}
            ChartSkeleton {}
        }
    }
}

#[component]
pub fn LoadingSurface(
    id: String,
    title: Option<String>,
    description: Option<String>,
    progress: LoadProgress,
    concealed: bool,
) -> Element {
    let progress = normalize(progress);
    let busy = if is_busy(progress) { "true" } else { "false" };
    let name = surface_title(title.as_deref(), progress);
    let title_id = format!("{id}-title");
    let region_id = id.clone();
    let meter_name = name.clone();
    let detail = description.filter(|value| !value.is_empty());
    let percent = percent_value(progress);
    rsx! {
        div {
            id: "{region_id}",
            class: "loading-surface",
            role: "status",
            aria_live: "polite",
            aria_busy: "{busy}",
            aria_labelledby: "{title_id}",
            hidden: concealed,
            if is_busy(progress) {
                StrandsLoader { size: StrandSize::Md }
            } else {
                StrandMark {}
            }
            p { id: "{title_id}", class: "loading-title", "{name}" }
            if let Some(detail) = detail {
                p { class: "loading-detail", "{detail}" }
            }
            if shows_meter(progress) {
                ProgressMeter {
                    id: format!("{id}-meter"),
                    name: meter_name,
                    progress,
                }
            }
            if let Some(value) = percent {
                p { class: "loading-percent", "{value}% complete" }
            }
        }
    }
}

#[component]
pub fn StrandsLoader(size: StrandSize) -> Element {
    let class = size.class_name();
    let first = strand_style(0, "var(--a3-strand-1)", "0s");
    let second = strand_style(1, "var(--a3-strand-2)", "0.12s");
    let third = strand_style(2, "var(--a3-strand-3)", "0.24s");
    let fourth = strand_style(3, "var(--a3-strand-4)", "0.36s");
    let fifth = strand_style(4, "var(--a3-strand-5)", "0.48s");
    rsx! {
        div { class: "{class}", aria_hidden: "true",
            div { class: "strand-wave", style: "{first}" }
            div { class: "strand-wave", style: "{second}" }
            div { class: "strand-wave", style: "{third}" }
            div { class: "strand-wave", style: "{fourth}" }
            div { class: "strand-wave", style: "{fifth}" }
        }
    }
}

#[component]
fn StrandMark() -> Element {
    rsx! {
        svg {
            class: "strand-mark",
            view_box: "0 0 24 24",
            fill: "none",
            "aria-hidden": "true",
            rect { x: "8", y: "2.6", width: "12", height: "2.6", rx: "1.3", fill: "var(--a3-strand-1)" }
            rect { x: "6.5", y: "6.75", width: "12", height: "2.6", rx: "1.3", fill: "var(--a3-strand-2)" }
            rect { x: "5", y: "10.9", width: "12", height: "2.6", rx: "1.3", fill: "var(--a3-strand-3)" }
            rect { x: "3.5", y: "15.05", width: "12", height: "2.6", rx: "1.3", fill: "var(--a3-strand-4)" }
            rect { x: "2", y: "19.2", width: "12", height: "2.6", rx: "1.3", fill: "var(--a3-strand-5)" }
        }
    }
}

#[component]
fn ProgressMeter(id: String, name: String, progress: LoadProgress) -> Element {
    let progress = normalize(progress);
    let state = progress_state(progress);
    let width = indicator_width(progress);
    rsx! {
        div { id: "{id}", class: "loading-meter",
            if is_unknown(progress) {
                div {
                    class: "progress",
                    role: "progressbar",
                    aria_label: "{name}",
                    aria_valuemin: "0",
                    aria_valuemax: "100",
                    "data-state": "{state}",
                    div { class: "progress-indicator", "data-state": "{state}" }
                }
            } else {
                MeasuredMeter { name, progress, state, width }
            }
        }
    }
}

#[component]
fn MeasuredMeter(
    name: String,
    progress: LoadProgress,
    state: &'static str,
    width: String,
) -> Element {
    let value = percent_value(progress).unwrap_or(0);
    rsx! {
        div {
            class: "progress",
            role: "progressbar",
            aria_label: "{name}",
            aria_valuemin: "0",
            aria_valuemax: "100",
            aria_valuenow: "{value}",
            aria_valuetext: "{value}%",
            "data-state": "{state}",
            div { class: "progress-indicator", "data-state": "{state}", style: "{width}" }
        }
    }
}

#[component]
fn RailSkeleton() -> Element {
    rsx! {
        div {
            id: "skeleton-rail",
            class: "skeleton skeleton-rail",
            role: "status",
            aria_live: "polite",
            aria_label: "Loading navigation row",
            span { class: "skeleton-block skeleton-glyph", aria_hidden: "true" }
            span { class: "skeleton-block skeleton-line", aria_hidden: "true" }
        }
    }
}

#[component]
fn ListSkeleton() -> Element {
    rsx! {
        div {
            id: "skeleton-list",
            class: "skeleton skeleton-list",
            role: "status",
            aria_live: "polite",
            aria_label: "Loading list row",
            span { class: "skeleton-block skeleton-title", aria_hidden: "true" }
            span { class: "skeleton-block skeleton-meta", aria_hidden: "true" }
        }
    }
}

#[component]
fn ChartSkeleton() -> Element {
    rsx! {
        div {
            id: "skeleton-chart",
            class: "skeleton skeleton-chart",
            role: "status",
            aria_live: "polite",
            aria_label: "Loading chart",
            span { class: "skeleton-block skeleton-title", aria_hidden: "true" }
            span { class: "skeleton-block skeleton-plot", aria_hidden: "true" }
        }
    }
}

impl StrandSize {
    fn class_name(self) -> &'static str {
        match self {
            Self::Xs => "strand-loader strand-xs",
            Self::Sm => "strand-loader strand-sm",
            Self::Md => "strand-loader strand-md",
        }
    }
}

fn normalize(progress: LoadProgress) -> LoadProgress {
    match progress {
        LoadProgress::Percent(value) => LoadProgress::Percent(value.min(100)),
        other => other,
    }
}

fn is_busy(progress: LoadProgress) -> bool {
    !matches!(normalize(progress), LoadProgress::Percent(100))
}

fn is_unknown(progress: LoadProgress) -> bool {
    matches!(progress, LoadProgress::Unknown)
}

fn shows_meter(progress: LoadProgress) -> bool {
    !matches!(progress, LoadProgress::Omitted)
}

fn percent_value(progress: LoadProgress) -> Option<u8> {
    match normalize(progress) {
        LoadProgress::Percent(value) => Some(value),
        _ => None,
    }
}

fn surface_title(title: Option<&str>, progress: LoadProgress) -> String {
    if let Some(title) = title.filter(|value| !value.is_empty()) {
        return title.to_owned();
    }
    if is_busy(progress) {
        "Loading…".to_owned()
    } else {
        "Ready".to_owned()
    }
}

fn progress_state(progress: LoadProgress) -> &'static str {
    match normalize(progress) {
        LoadProgress::Unknown => "indeterminate",
        LoadProgress::Percent(100) => "complete",
        LoadProgress::Percent(_) => "loading",
        LoadProgress::Omitted => "indeterminate",
    }
}

fn indicator_width(progress: LoadProgress) -> String {
    match percent_value(progress) {
        Some(value) => format!("width: {value}%"),
        None => String::new(),
    }
}

fn strand_style(index: usize, color: &str, delay: &str) -> String {
    let slant = 4 - index;
    format!(
        "background-color: {color}; width: calc(100% - var(--bar-slant-step, 0.25rem) * 4); margin-left: calc(var(--bar-slant-step, 0.25rem) * {slant}); animation-delay: {delay};"
    )
}
