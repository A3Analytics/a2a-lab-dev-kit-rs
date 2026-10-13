//! Hash routes for the shell. Only the open section is in the document.
//! Detached sections stay alive so drafts and loaded results survive navigation.

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Event, KeyboardEvent};

use crate::inventory::Primitive;
use crate::shell::page::{self, Page};

const VIEWS: [&str; 6] = [
    "view-agent",
    "view-logs",
    "view-metrics",
    "view-tasks",
    "view-images",
    "view-chat",
];
const LINKS: [&str; 6] = [
    "nav-agent",
    "nav-logs",
    "nav-metrics",
    "nav-tasks",
    "nav-images",
    "nav-chat",
];

struct Route {
    view: &'static str,
    link: &'static str,
    title: &'static str,
    heading: &'static str,
    primitive: Option<Primitive>,
}

pub fn install(page: &Page) {
    listen_hash(page);
    listen_toggle(page);
    listen_escape(page);
    listen_layout(page);
    show(page, false);
    sync_layout(page);
}

fn listen_hash(page: &Page) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let page = page.clone();
    let closure =
        Closure::wrap(Box::new(move |_event: Event| show(&page, true)) as Box<dyn FnMut(Event)>);
    let _ = window.add_event_listener_with_callback("hashchange", closure.as_ref().unchecked_ref());
    closure.forget();
}

fn listen_toggle(page: &Page) {
    let Some(toggle) = page.element("nav-toggle") else {
        return;
    };
    let page = page.clone();
    let closure =
        Closure::wrap(Box::new(move |_event: Event| toggle_menu(&page)) as Box<dyn FnMut(Event)>);
    let _ = toggle.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
    closure.forget();
}

fn listen_escape(page: &Page) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let page = page.clone();
    let closure = Closure::wrap(Box::new(move |event: KeyboardEvent| {
        if event.key() == "Escape" && mobile() && menu_open(&page) {
            close_menu(&page, true);
        }
    }) as Box<dyn FnMut(KeyboardEvent)>);
    let _ = window.add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
    closure.forget();
}

fn listen_layout(page: &Page) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let page = page.clone();
    let closure =
        Closure::wrap(Box::new(move |_event: Event| sync_layout(&page)) as Box<dyn FnMut(Event)>);
    let _ = window.add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref());
    closure.forget();
}

fn show(page: &Page, focus: bool) {
    let route = route_for(&current_hash());
    if let Some(main) = page.element("main") {
        for view in VIEWS {
            place(page, view, &main, view == route.view);
        }
    }
    for link in LINKS {
        if link == route.link {
            page.set_data(link, "aria-current", "page");
        } else {
            page.remove_attr(link, "aria-current");
        }
    }
    if let Some(primitive) = route.primitive {
        if let Some(host) = page.element(route.view) {
            place(page, "workbench", &host, true);
        }
        if primitive == Primitive::Logs {
            page.set_data("workbench", "hidden", "hidden");
        } else {
            page.remove_attr("workbench", "hidden");
        }
        super::workbench::scope(page, primitive);
        super::inventory::reveal(page, primitive);
    } else if let Some(command) = page.element("workbench") {
        command.remove();
    }
    page.set_text("view-title", route.title);
    set_title(route.title);
    if mobile() {
        close_menu(page, false);
    }
    if focus {
        focus_heading(page, route.heading);
    }
}

fn toggle_menu(page: &Page) {
    if !mobile() {
        return;
    }
    if menu_open(page) {
        close_menu(page, true);
    } else {
        open_menu(page);
    }
}

fn open_menu(page: &Page) {
    page.set_data("shell", "data-nav", "open");
    page.remove_attr("primary-nav", "hidden");
    page.set_data("nav-toggle", "aria-expanded", "true");
    page.set_data("nav-toggle", "aria-label", "Close navigation");
    page.set_text("nav-toggle", "Close");
    page.focus("nav-agent");
}

fn close_menu(page: &Page, return_focus: bool) {
    page.remove_attr("shell", "data-nav");
    page.set_data("primary-nav", "hidden", "hidden");
    page.set_data("nav-toggle", "aria-expanded", "false");
    page.set_data("nav-toggle", "aria-label", "Open navigation");
    page.set_text("nav-toggle", "Menu");
    if return_focus {
        page.focus("nav-toggle");
    }
}

fn sync_layout(page: &Page) {
    if mobile() {
        set_layout("mobile");
        page.remove_attr("nav-toggle", "hidden");
        if menu_open(page) {
            page.remove_attr("primary-nav", "hidden");
        } else {
            page.set_data("primary-nav", "hidden", "hidden");
        }
        return;
    }
    set_layout("desktop");
    page.set_data("nav-toggle", "hidden", "hidden");
    page.set_data("nav-toggle", "aria-expanded", "false");
    page.remove_attr("primary-nav", "hidden");
    page.remove_attr("shell", "data-nav");
}

fn menu_open(page: &Page) -> bool {
    page.data("shell", "data-nav") == "open"
}

fn mobile() -> bool {
    web_sys::window()
        .and_then(|window| window.match_media("(max-width: 47.99rem)").ok().flatten())
        .is_some_and(|query| query.matches())
}

fn route_for(hash: &str) -> Route {
    match hash.trim().trim_start_matches('#').trim_matches('/') {
        "logs" | "inventory" | "inventory/logs" | "workbench" => primitive(
            Primitive::Logs,
            "view-logs",
            "nav-logs",
            "Logs",
            "log-view-heading",
        ),
        "metrics" | "inventory/metrics" => primitive(
            Primitive::Metrics,
            "view-metrics",
            "nav-metrics",
            "Metrics",
            "inventory-metrics-heading",
        ),
        "tasks" | "inventory/tasks" => primitive(
            Primitive::Tasks,
            "view-tasks",
            "nav-tasks",
            "Tasks",
            "inventory-tasks-heading",
        ),
        "images" | "inventory/images" => primitive(
            Primitive::Images,
            "view-images",
            "nav-images",
            "Images",
            "inventory-images-heading",
        ),
        "chat" => Route {
            view: "view-chat",
            link: "nav-chat",
            title: "Chat",
            heading: "chat-heading",
            primitive: None,
        },
        _ => Route {
            view: "view-agent",
            link: "nav-agent",
            title: "Agent",
            heading: "agent-heading",
            primitive: None,
        },
    }
}

fn primitive(
    primitive: Primitive,
    view: &'static str,
    link: &'static str,
    title: &'static str,
    heading: &'static str,
) -> Route {
    Route {
        view,
        link,
        title,
        heading,
        primitive: Some(primitive),
    }
}

fn current_hash() -> String {
    web_sys::window()
        .and_then(|window| window.location().hash().ok())
        .unwrap_or_default()
}

fn place(page: &Page, id: &str, parent: &web_sys::Element, present: bool) {
    let Some(element) = page.element(id) else {
        return;
    };
    if present {
        page.remove_attr(id, "hidden");
        let attached = element
            .parent_element()
            .is_some_and(|node| node.is_same_node(Some(parent)));
        if !attached {
            parent.append_child(&element).ok();
        }
        return;
    }
    element.remove();
}

fn focus_heading(page: &Page, id: &str) {
    if let Some(heading) = page.element(id) {
        page::attr(&heading, "tabindex", "-1");
        page.focus(id);
    }
}

fn set_title(title: &str) {
    if let Some(document) = web_sys::window().and_then(|window| window.document()) {
        document.set_title(&format!("{title} · A2A Lab"));
    }
}

fn set_layout(mode: &str) {
    let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
    else {
        return;
    };
    let _ = root.set_attribute("data-layout", mode);
}
