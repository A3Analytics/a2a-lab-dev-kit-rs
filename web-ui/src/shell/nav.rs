//! Hash routes for the shell. Dioxus owns layout; hash and viewport stay as browser integrations.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use dioxus::core::{Runtime, RuntimeGuard, ScopeId, current_scope_id};
use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Event, HtmlElement, KeyboardEvent};

use crate::inventory::Primitive;

type HashListener = Closure<dyn FnMut(Event)>;
type EscapeListener = Closure<dyn FnMut(KeyboardEvent)>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    System,
    Dark,
}

impl Theme {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::System => "system",
            Self::Dark => "dark",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Light => Self::System,
            Self::System => Self::Dark,
            Self::Dark => Self::Light,
        }
    }

    fn parse(value: Option<&str>) -> Self {
        match value {
            Some("light") => Self::Light,
            Some("dark") => Self::Dark,
            _ => Self::System,
        }
    }
}

pub fn theme_label(theme: Theme) -> String {
    format!(
        "Theme: {}. Click for {}.",
        theme.slug(),
        theme.next().slug()
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Route {
    pub view: &'static str,
    pub link: &'static str,
    pub title: &'static str,
    pub primitive: Option<Primitive>,
}

thread_local! {
    static RUNTIME: RefCell<Option<Rc<Runtime>>> = const { RefCell::new(None) };
    static SCOPE: RefCell<Option<ScopeId>> = const { RefCell::new(None) };
    static HASH: RefCell<Option<Signal<String>>> = const { RefCell::new(None) };
    static MENU: RefCell<Option<Signal<bool>>> = const { RefCell::new(None) };
    static MOBILE: RefCell<Option<Signal<bool>>> = const { RefCell::new(None) };
    static HASH_LISTENER: RefCell<Option<HashListener>> = const { RefCell::new(None) };
    static ESCAPE_LISTENER: RefCell<Option<EscapeListener>> = const { RefCell::new(None) };
    static LAYOUT_LISTENER: RefCell<Option<HashListener>> = const { RefCell::new(None) };
    static SCHEME_LISTENER: RefCell<Option<HashListener>> = const { RefCell::new(None) };
    static THEME: Cell<Theme> = const { Cell::new(Theme::System) };
}

pub fn bind(hash: Signal<String>, menu: Signal<bool>, mobile: Signal<bool>) {
    RUNTIME.with(|slot| *slot.borrow_mut() = Some(Runtime::current()));
    SCOPE.with(|slot| *slot.borrow_mut() = Some(current_scope_id()));
    HASH.with(|slot| *slot.borrow_mut() = Some(hash));
    MENU.with(|slot| *slot.borrow_mut() = Some(menu));
    MOBILE.with(|slot| *slot.borrow_mut() = Some(mobile));
}

pub fn install() {
    listen_hash();
    listen_escape();
    listen_layout();
    apply_islands(false);
    sync_layout();
}

pub fn current_hash() -> String {
    web_sys::window()
        .and_then(|window| window.location().hash().ok())
        .unwrap_or_default()
}

pub fn route_for(hash: &str) -> Route {
    match hash.trim().trim_start_matches('#').trim_matches('/') {
        "logs" | "inventory" | "inventory/logs" | "workbench" => {
            primitive(Primitive::Logs, "view-logs", "nav-logs", "Logs")
        }
        "metrics" | "inventory/metrics" => {
            primitive(Primitive::Metrics, "view-metrics", "nav-metrics", "Metrics")
        }
        "tasks" | "inventory/tasks" => {
            primitive(Primitive::Tasks, "view-tasks", "nav-tasks", "Tasks")
        }
        "images" | "inventory/images" => {
            primitive(Primitive::Images, "view-images", "nav-images", "Images")
        }
        "chat" => Route {
            view: "view-chat",
            link: "nav-chat",
            title: "Chat",
            primitive: None,
        },
        _ => primitive(Primitive::Logs, "view-logs", "nav-logs", "Logs"),
    }
}

pub fn read_sidebar_open() -> bool {
    !matches!(storage_get("sidebar_state").as_deref(), Some("false"))
}

pub fn write_sidebar_open(open: bool) {
    storage_set("sidebar_state", if open { "true" } else { "false" });
}

pub fn read_theme() -> Theme {
    Theme::parse(storage_get("theme").as_deref())
}

pub fn write_theme(theme: Theme) {
    storage_set("theme", theme.slug());
    apply_theme(theme);
}

pub fn apply_stored_theme() {
    apply_theme(read_theme());
    listen_scheme();
}

pub fn set_rail_inert(inert: bool) {
    let Some(nav) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("primary-nav"))
    else {
        return;
    };
    if inert {
        let _ = nav.set_attribute("inert", "");
        return;
    }
    let _ = nav.remove_attribute("inert");
}

pub fn apply_theme(theme: Theme) {
    THEME.with(|slot| slot.set(theme));
    let Some(root) = document_root() else {
        return;
    };
    let _ = root.set_attribute("data-theme", theme.slug());
    set_dark_class(&root, resolved_dark(theme));
}

pub fn is_mobile() -> bool {
    web_sys::window()
        .and_then(|window| window.match_media("(max-width: 47.99rem)").ok().flatten())
        .is_some_and(|query| query.matches())
}

pub fn current_link(route: Route, id: &str) -> Option<&'static str> {
    (route.link == id).then_some("page")
}

pub fn set_document_title(title: &str) {
    if let Some(document) = web_sys::window().and_then(|window| window.document()) {
        document.set_title(&format!("{title} · A2A Lab"));
    }
}

pub fn set_layout_attr(mobile: bool) {
    let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
    else {
        return;
    };
    let _ = root.set_attribute("data-layout", if mobile { "mobile" } else { "desktop" });
}

pub fn focus_heading(id: &str) {
    if let Some(heading) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
    {
        let _ = heading.set_attribute("tabindex", "-1");
        if let Ok(html) = heading.dyn_into::<HtmlElement>() {
            let _ = html.focus();
        }
    }
}

pub fn apply_islands(focus: bool) {
    let route = route_for(&current_hash());
    if let Some(primitive) = route.primitive {
        super::workbench::scope(primitive);
        super::inventory::reveal(primitive);
        if primitive == Primitive::Metrics {
            super::metrics::redraw();
        }
    }
    if focus {
        focus_heading("view-title");
    }
}

fn listen_hash() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let closure = Closure::wrap(Box::new(move |_event: Event| {
        let next = current_hash();
        run(move || {
            HASH.with(|slot| {
                if let Some(mut hash) = *slot.borrow() {
                    hash.set(next.clone());
                }
            });
            MENU.with(|slot| {
                if let Some(mut menu) = *slot.borrow()
                    && is_mobile()
                {
                    menu.set(false);
                }
            });
        });
    }) as Box<dyn FnMut(Event)>);
    let _ = window.add_event_listener_with_callback("hashchange", closure.as_ref().unchecked_ref());
    HASH_LISTENER.with(|slot| *slot.borrow_mut() = Some(closure));
}

fn listen_escape() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let closure = Closure::wrap(Box::new(move |event: KeyboardEvent| {
        if event.key() == "Escape" && is_mobile() {
            run(|| {
                MENU.with(|slot| {
                    if let Some(mut menu) = *slot.borrow() {
                        menu.set(false);
                    }
                });
            });
        }
    }) as Box<dyn FnMut(KeyboardEvent)>);
    let _ = window.add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
    ESCAPE_LISTENER.with(|slot| *slot.borrow_mut() = Some(closure));
}

fn listen_layout() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let closure =
        Closure::wrap(Box::new(move |_event: Event| sync_layout()) as Box<dyn FnMut(Event)>);
    let _ = window.add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref());
    LAYOUT_LISTENER.with(|slot| *slot.borrow_mut() = Some(closure));
}

fn sync_layout() {
    let mobile = is_mobile();
    run(move || {
        MOBILE.with(|slot| {
            if let Some(mut signal) = *slot.borrow() {
                signal.set(mobile);
            }
        });
        if !mobile {
            MENU.with(|slot| {
                if let Some(mut menu) = *slot.borrow() {
                    menu.set(false);
                }
            });
        }
    });
    set_layout_attr(mobile);
}

fn primitive(
    primitive: Primitive,
    view: &'static str,
    link: &'static str,
    title: &'static str,
) -> Route {
    Route {
        view,
        link,
        title,
        primitive: Some(primitive),
    }
}

fn storage_get(key: &str) -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()
        .flatten()?
        .get_item(key)
        .ok()
        .flatten()
}

fn storage_set(key: &str, value: &str) {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let _ = storage.set_item(key, value);
    }
}

fn document_root() -> Option<web_sys::Element> {
    web_sys::window()?.document()?.document_element()
}

fn prefers_dark() -> bool {
    web_sys::window()
        .and_then(|window| {
            window
                .match_media("(prefers-color-scheme: dark)")
                .ok()
                .flatten()
        })
        .is_some_and(|query| query.matches())
}

fn resolved_dark(theme: Theme) -> bool {
    match theme {
        Theme::Dark => true,
        Theme::Light => false,
        Theme::System => prefers_dark(),
    }
}

fn set_dark_class(root: &web_sys::Element, dark: bool) {
    let current = root.get_attribute("class").unwrap_or_default();
    let mut classes = current
        .split_whitespace()
        .filter(|item| *item != "dark")
        .collect::<Vec<_>>();
    if dark {
        classes.push("dark");
    }
    if classes.is_empty() {
        let _ = root.remove_attribute("class");
        return;
    }
    let _ = root.set_attribute("class", &classes.join(" "));
}

fn listen_scheme() {
    let installed = SCHEME_LISTENER.with(|slot| slot.borrow().is_some());
    if installed {
        return;
    }
    let Some(query) = web_sys::window().and_then(|window| {
        window
            .match_media("(prefers-color-scheme: dark)")
            .ok()
            .flatten()
    }) else {
        return;
    };
    let closure = Closure::wrap(Box::new(move |_event: Event| {
        if THEME.with(|slot| slot.get()) == Theme::System {
            apply_theme(Theme::System);
        }
    }) as Box<dyn FnMut(Event)>);
    if query
        .add_event_listener_with_callback("change", closure.as_ref().unchecked_ref())
        .is_err()
    {
        return;
    }
    SCHEME_LISTENER.with(|slot| *slot.borrow_mut() = Some(closure));
}

fn run<F: FnOnce()>(job: F) {
    let runtime = RUNTIME.with(|slot| slot.borrow().clone());
    let scope = SCOPE.with(|slot| *slot.borrow());
    if let (Some(runtime), Some(scope)) = (runtime, scope) {
        let _guard = RuntimeGuard::new(runtime.clone());
        runtime.in_scope(scope, job);
        runtime.needs_update(scope);
        return;
    }
    job();
}
