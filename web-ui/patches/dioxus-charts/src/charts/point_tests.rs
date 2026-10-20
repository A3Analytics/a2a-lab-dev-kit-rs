use std::collections::HashMap;
use std::future::Future;

use dioxus::prelude::*;
use dioxus_core::AttributeValue;

use super::{shown_point_label, LineChart, LineChartPoint};

fn sample(series_index: usize, point_index: usize, value: f32) -> LineChartPoint {
    LineChartPoint {
        series_index,
        point_index,
        value,
    }
}

#[test]
fn hover_and_focus_choose_the_caller_label() {
    let point = sample(1, 2, 3.5);
    let label = Some("3.5 C");
    assert_eq!(
        shown_point_label(label, point, None, None),
        None,
        "hidden until hover or focus"
    );
    assert_eq!(
        shown_point_label(label, point, Some(point), None),
        Some("3.5 C")
    );
    assert_eq!(
        shown_point_label(label, point, None, Some(point)),
        Some("3.5 C")
    );
    assert_eq!(
        shown_point_label(Some("   "), point, Some(point), Some(point)),
        Some("   ")
    );
    assert_eq!(
        shown_point_label(Some(""), point, Some(point), Some(point)),
        None
    );
    assert_eq!(
        shown_point_label(None, point, Some(point), Some(point)),
        None
    );
    let other = sample(0, 0, 3.5);
    assert_eq!(
        shown_point_label(label, point, Some(other), Some(other)),
        None
    );
}

fn app() -> Element {
    rsx! {
        LineChart {
            series: vec![vec![1.0, 4.5], vec![2.0]],
            show_grid: false,
            show_labels: false,
            show_line_labels: false,
            point_label: |point: LineChartPoint| format!(
                "sample-{}-{}-{}",
                point.series_index, point.point_index, point.value
            ),
        }
    }
}

fn render_chart() -> (VirtualDom, String, dioxus_core::Mutations) {
    let mut dom = VirtualDom::new(app);
    let mutations = dom.rebuild_to_vec();
    let html = dioxus_ssr::render(&dom);
    (dom, html, mutations)
}

fn attribute_text(value: &AttributeValue) -> Option<String> {
    match value {
        AttributeValue::Text(text) => Some(text.clone()),
        AttributeValue::Int(number) => Some(number.to_string()),
        AttributeValue::Float(number) => Some(number.to_string()),
        _ => None,
    }
}

struct PointNode {
    id: dioxus_core::ElementId,
    series: String,
    point: String,
    value: String,
    listeners: Vec<String>,
}

fn point_nodes(mutations: &dioxus_core::Mutations) -> Vec<PointNode> {
    let mut series = HashMap::<dioxus_core::ElementId, String>::new();
    let mut points = HashMap::<dioxus_core::ElementId, String>::new();
    let mut values = HashMap::<dioxus_core::ElementId, String>::new();
    let mut listeners = HashMap::<dioxus_core::ElementId, Vec<String>>::new();
    for edit in &mutations.edits {
        match edit {
            dioxus_core::Mutation::SetAttribute {
                name, value, id, ..
            } => {
                let Some(text) = attribute_text(value) else {
                    continue;
                };
                match *name {
                    "data-series-index" => {
                        series.insert(*id, text);
                    }
                    "data-point-index" => {
                        points.insert(*id, text);
                    }
                    "data-value" => {
                        values.insert(*id, text);
                    }
                    _ => {}
                }
            }
            dioxus_core::Mutation::NewEventListener { name, id } => {
                listeners.entry(*id).or_default().push(name.clone());
            }
            _ => {}
        }
    }
    let mut nodes = Vec::new();
    for (id, series) in series {
        nodes.push(PointNode {
            id,
            series,
            point: points.remove(&id).unwrap_or_default(),
            value: values.remove(&id).unwrap_or_default(),
            listeners: listeners.remove(&id).unwrap_or_default(),
        });
    }
    nodes.sort_by(|left, right| (&left.series, &left.point).cmp(&(&right.series, &right.point)));
    nodes
}

#[test]
fn points_expose_identity_and_focusable_groups() {
    let (_dom, html, mutations) = render_chart();
    assert!(
        !html.contains("sample-"),
        "labels stay hidden before hover or focus: {html}"
    );
    let nodes = point_nodes(&mutations);
    let identity: Vec<_> = nodes
        .iter()
        .map(|node| {
            (
                node.series.as_str(),
                node.point.as_str(),
                node.value.as_str(),
            )
        })
        .collect();
    assert_eq!(
        identity,
        vec![("0", "0", "1"), ("0", "1", "4.5"), ("1", "0", "2")]
    );
    for node in &nodes {
        assert!(
            html.contains(&format!("data-series-index=\"{}\"", node.series)),
            "series identity missing from {html}"
        );
        assert!(
            html.contains(&format!("data-point-index=\"{}\"", node.point)),
            "point identity missing from {html}"
        );
        assert!(
            html.contains(&format!("data-value=\"{}\"", node.value)),
            "value identity missing from {html}"
        );
        for event in ["pointerenter", "pointerleave", "focus", "blur"] {
            assert!(
                node.listeners.iter().any(|name| name == event),
                "point {}:{} missing {event} in {:?}",
                node.series,
                node.point,
                node.listeners
            );
        }
    }
    assert!(
        html.matches("tabindex=\"0\"").count() >= 3,
        "each point group should be focusable: {html}"
    );
    assert!(
        html.matches("dx-line-point").count() >= 3,
        "each point should be grouped: {html}"
    );
    assert!(
        html.contains("var(--chart-4)"),
        "series uses a chart token: {html}"
    );
    assert!(
        !html.contains("rgb(") && !html.contains("rgba(20, 20, 20, 0.8)"),
        "series and grid stay off the hardcoded red and dark strokes: {html}"
    );
}

thread_local! {
    static ENTERED: std::cell::RefCell<Vec<LineChartPoint>> = const { std::cell::RefCell::new(Vec::new()) };
    static LEFT: std::cell::RefCell<Vec<LineChartPoint>> = const { std::cell::RefCell::new(Vec::new()) };
    static FOCUSED_POINTS: std::cell::RefCell<Vec<LineChartPoint>> = const { std::cell::RefCell::new(Vec::new()) };
    static BLURRED: std::cell::RefCell<Vec<LineChartPoint>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn clear_points() {
    ENTERED.with(|points| points.borrow_mut().clear());
    LEFT.with(|points| points.borrow_mut().clear());
    FOCUSED_POINTS.with(|points| points.borrow_mut().clear());
    BLURRED.with(|points| points.borrow_mut().clear());
}

fn interactive() -> Element {
    rsx! {
        LineChart {
            series: vec![vec![1.0, 4.5], vec![2.0]],
            show_grid: false,
            show_labels: false,
            show_line_labels: false,
            on_point_pointer_enter: |point| {
                ENTERED.with(|points| points.borrow_mut().push(point));
            },
            on_point_pointer_leave: |point| {
                LEFT.with(|points| points.borrow_mut().push(point));
            },
            on_point_focus: |point| {
                FOCUSED_POINTS.with(|points| points.borrow_mut().push(point));
            },
            on_point_blur: |point| {
                BLURRED.with(|points| points.borrow_mut().push(point));
            },
            point_label: |point: LineChartPoint| format!(
                "sample-{}-{}-{}",
                point.series_index, point.point_index, point.value
            ),
        }
    }
}

struct EmptyFocus;

impl dioxus::events::HasFocusData for EmptyFocus {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct EmptyPointer;

impl dioxus::html::InteractionLocation for EmptyPointer {
    fn client_coordinates(&self) -> dioxus::html::geometry::ClientPoint {
        dioxus::html::geometry::ClientPoint::new(0.0, 0.0)
    }

    fn screen_coordinates(&self) -> dioxus::html::geometry::ScreenPoint {
        dioxus::html::geometry::ScreenPoint::new(0.0, 0.0)
    }

    fn page_coordinates(&self) -> dioxus::html::geometry::PagePoint {
        dioxus::html::geometry::PagePoint::new(0.0, 0.0)
    }
}

impl dioxus::html::InteractionElementOffset for EmptyPointer {
    fn element_coordinates(&self) -> dioxus::html::geometry::ElementPoint {
        dioxus::html::geometry::ElementPoint::new(0.0, 0.0)
    }
}

impl dioxus::html::ModifiersInteraction for EmptyPointer {
    fn modifiers(&self) -> dioxus::html::Modifiers {
        dioxus::html::Modifiers::empty()
    }
}

impl dioxus::html::PointerInteraction for EmptyPointer {
    fn trigger_button(&self) -> Option<dioxus::html::input_data::MouseButton> {
        None
    }

    fn held_buttons(&self) -> dioxus::html::input_data::MouseButtonSet {
        dioxus::html::input_data::MouseButtonSet::empty()
    }
}

impl dioxus::events::HasPointerData for EmptyPointer {
    fn pointer_id(&self) -> i32 {
        1
    }

    fn width(&self) -> f64 {
        1.0
    }

    fn height(&self) -> f64 {
        1.0
    }

    fn pressure(&self) -> f32 {
        0.0
    }

    fn tangential_pressure(&self) -> f32 {
        0.0
    }

    fn tilt_x(&self) -> i32 {
        0
    }

    fn tilt_y(&self) -> i32 {
        0
    }

    fn twist(&self) -> i32 {
        0
    }

    fn pointer_type(&self) -> String {
        "mouse".to_owned()
    }

    fn is_primary(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct TestConverter;

macro_rules! unimplemented_event {
    ($($method:ident -> $ty:ty),* $(,)?) => {
        $(
            fn $method(&self, _: &dioxus::events::PlatformEventData) -> $ty {
                unimplemented!()
            }
        )*
    };
}

impl dioxus::events::HtmlEventConverter for TestConverter {
    unimplemented_event! {
        convert_animation_data -> dioxus::events::AnimationData,
        convert_cancel_data -> dioxus::events::CancelData,
        convert_clipboard_data -> dioxus::events::ClipboardData,
        convert_composition_data -> dioxus::events::CompositionData,
        convert_drag_data -> dioxus::events::DragData,
        convert_form_data -> dioxus::events::FormData,
        convert_image_data -> dioxus::events::ImageData,
        convert_keyboard_data -> dioxus::events::KeyboardData,
        convert_media_data -> dioxus::events::MediaData,
        convert_mounted_data -> dioxus::events::MountedData,
        convert_mouse_data -> dioxus::events::MouseData,
        convert_resize_data -> dioxus::events::ResizeData,
        convert_scroll_data -> dioxus::events::ScrollData,
        convert_selection_data -> dioxus::events::SelectionData,
        convert_toggle_data -> dioxus::events::ToggleData,
        convert_touch_data -> dioxus::events::TouchData,
        convert_transition_data -> dioxus::events::TransitionData,
        convert_visible_data -> dioxus::events::VisibleData,
        convert_wheel_data -> dioxus::events::WheelData
    }

    fn convert_pointer_data(
        &self,
        _: &dioxus::events::PlatformEventData,
    ) -> dioxus::events::PointerData {
        dioxus::events::PointerData::new(EmptyPointer)
    }

    fn convert_focus_data(
        &self,
        _: &dioxus::events::PlatformEventData,
    ) -> dioxus::events::FocusData {
        dioxus::events::FocusData::new(EmptyFocus)
    }
}

fn flush(dom: &mut VirtualDom) {
    loop {
        let ready = {
            let mut waiting = std::pin::pin!(dom.wait_for_work());
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            waiting.as_mut().poll(&mut context).is_ready()
        };
        if !ready {
            break;
        }
        dom.render_immediate(&mut dioxus_core::NoOpMutations);
    }
}

fn dispatch(dom: &mut VirtualDom, name: &str, id: dioxus_core::ElementId) {
    let event = Event::new(
        std::rc::Rc::new(dioxus::events::PlatformEventData::new(Box::new(())))
            as std::rc::Rc<dyn std::any::Any>,
        true,
    );
    dom.runtime().handle_event(name, event, id);
    flush(dom);
}

#[test]
fn pointer_and_focus_show_the_caller_label() {
    clear_points();
    dioxus::events::set_event_converter(Box::new(TestConverter));
    let mut dom = VirtualDom::new(interactive);
    let mutations = dom.rebuild_to_vec();
    let nodes = point_nodes(&mutations);
    let hovered = nodes
        .iter()
        .find(|node| node.series == "0" && node.point == "1")
        .expect("second point of the first series");
    let focused = nodes
        .iter()
        .find(|node| node.series == "1" && node.point == "0")
        .expect("first point of the second series");
    let hovered_point = sample(0, 1, 4.5);
    let focused_point = sample(1, 0, 2.0);

    assert!(!dioxus_ssr::render(&dom).contains("sample-"));
    dispatch(&mut dom, "pointerenter", hovered.id);
    let html = dioxus_ssr::render(&dom);
    assert!(
        html.contains("sample-0-1-4.5"),
        "hover label missing: {html}"
    );
    assert!(
        html.contains("dx-line-point-label"),
        "hover label is not visible"
    );
    assert_eq!(
        ENTERED.with(|points| points.borrow().clone()),
        vec![hovered_point]
    );

    dispatch(&mut dom, "pointerleave", hovered.id);
    assert!(!dioxus_ssr::render(&dom).contains("sample-"));
    assert_eq!(
        LEFT.with(|points| points.borrow().clone()),
        vec![hovered_point]
    );

    dispatch(&mut dom, "focus", focused.id);
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("sample-1-0-2"), "focus label missing: {html}");
    assert_eq!(
        FOCUSED_POINTS.with(|points| points.borrow().clone()),
        vec![focused_point]
    );

    dispatch(&mut dom, "blur", focused.id);
    assert!(!dioxus_ssr::render(&dom).contains("sample-"));
    assert_eq!(
        BLURRED.with(|points| points.borrow().clone()),
        vec![focused_point]
    );
}
