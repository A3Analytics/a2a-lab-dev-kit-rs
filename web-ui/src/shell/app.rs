//! Single Dioxus runtime on `#app`.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use dioxus::prelude::*;
use dioxus_web::Config;
use dioxus_web::launch::launch_cfg;

use crate::inventory::Primitive;
use crate::shell::chat::{self, ChatUi, ChatView};
use crate::shell::http::Transport;
use crate::shell::images::{self as image_ui, ImageUi, ImageView};
use crate::shell::inventory::{self, InventoryUi, InventoryView};
use crate::shell::loading::CardLoading;
use crate::shell::logs::{self, LogUi, LogView};
use crate::shell::metric_chart::MetricView;
use crate::shell::nav::{self, Route};
use crate::shell::session::{self, CardLoad, SessionUi};
use crate::shell::tasks::{self, TaskUi, TaskView};
use crate::shell::workbench::{self, WorkbenchUi, WorkbenchView};

pub fn start() {
    drop_bootstrap();
    nav::replace_legacy_hash();
    nav::apply_stored_theme();
    launch_cfg(LabApp, Config::new().rootname("app"));
}

#[component]
fn LabApp() -> Element {
    rsx! {
        Shell {}
    }
}

#[component]
fn Shell() -> Element {
    let path = use_signal(nav::current_route);
    let mut menu = use_signal(|| false);
    let mobile = use_signal(nav::is_mobile);
    let mut rail_open = use_signal(nav::read_sidebar_open);
    let mut theme = use_signal(nav::read_theme);
    let session = use_signal(SessionUi::loading);
    let chat = use_signal(ChatUi::checking);
    let logs = use_signal(InventoryUi::pending);
    let metrics = use_signal(InventoryUi::pending);
    let tasks = use_signal(InventoryUi::pending);
    let images = use_signal(InventoryUi::pending);
    let log_view = use_signal(LogUi::new);
    let image_view = use_signal(ImageUi::new);
    let task_view = use_signal(TaskUi::new);
    let workbench = use_signal(WorkbenchUi::new);
    let ready = use_signal(|| false);
    let fetch_gen = use_signal(|| 0u32);
    use_hook(|| {
        nav::bind(path, menu, mobile);
        nav::apply_theme(theme());
        session::bind(session);
        inventory::bind(logs, metrics, tasks, images);
        logs::bind(log_view);
        image_ui::bind(image_view);
        tasks::bind(task_view);
        workbench::bind(workbench);
    });
    use_effect(move || {
        boot_islands(session, ready);
    });
    let card = use_resource(move || {
        let ready = ready();
        let tick = fetch_gen();
        async move { (tick, session::load_card(ready).await) }
    });
    let mut applied = use_signal(|| None::<u32>);
    let _ = card.read();
    if let Some((tick, load)) = card()
        && tick == fetch_gen()
        && applied() != Some(tick)
        && !matches!(load, CardLoad::Idle)
    {
        applied.set(Some(tick));
        apply_card(session, chat, Some((tick, load)));
    }
    let current = path();
    let route = nav::route_for(&current);
    let panes = pane_hidden(
        session().connection == session::Connection::Loading,
        route,
        &current,
    );
    let skip_focus = use_hook(|| Rc::new(Cell::new(true)));
    let mut last_path = use_signal(String::new);
    use_effect(move || {
        nav::set_rail_inert(!mobile() && !rail_open());
    });
    use_effect(move || {
        let current = path();
        let route = nav::route_for(&current);
        nav::set_document_title(route.title);
        nav::set_layout_attr(mobile());
        if skip_focus.get() {
            skip_focus.set(false);
            last_path.set(current);
            return;
        }
        let focus = last_path.peek().as_str() != current;
        last_path.set(current);
        nav::apply_islands(focus);
    });
    let notice = session::refusal_text(&session());
    let notice_hidden = notice.is_empty();
    let logo = logo_src();
    let sheet_open = mobile() && menu();
    rsx! {
        div {
            id: "shell",
            class: "shell",
            "data-state": if rail_open() { "expanded" } else { "collapsed" },
            "data-nav": if sheet_open { "open" } else { "" },
            a {
                class: "skip",
                href: "#main",
                onclick: move |event| {
                    event.prevent_default();
                    nav::focus_heading("main");
                },
                "Skip to content"
            }
            div { class: "rail-gap",
                nav {
                    id: "primary-nav",
                    aria_label: "Lab sections",
                    hidden: mobile() && !menu(),
                    div { id: "brand",
                        if !logo.is_empty() {
                            img {
                                src: "{logo}",
                                alt: "A3 Analytics",
                                width: "2081",
                                height: "691",
                            }
                        }
                        p { id: "brand-name", class: "font-oxanium", "A2A Lab" }
                    }
                    NavLink { id: "nav-chat", href: "/ui/chat", route, label: "Chat", glyph: "C" }
                    NavLink { id: "nav-logs", href: "/ui/logs", route, label: "Logs", glyph: "L" }
                    NavLink { id: "nav-metrics", href: "/ui/metrics", route, label: "Metrics", glyph: "M" }
                    NavLink { id: "nav-tasks", href: "/ui/tasks", route, label: "Tasks", glyph: "T" }
                    NavLink { id: "nav-images", href: "/ui/images", route, label: "Images", glyph: "I" }
                }
            }
            div { class: "shell-main",
            header { class: "banner",
                div { class: "toggle-cell",
                    button {
                        id: "sidebar-toggle",
                        class: "icon-button",
                        r#type: "button",
                        aria_controls: "primary-nav",
                        aria_expanded: expanded(if mobile() { menu() } else { rail_open() }),
                        aria_label: "Toggle Sidebar",
                        onclick: move |_| {
                            if mobile() {
                                menu.set(!menu());
                                return;
                            }
                            let next = !rail_open();
                            rail_open.set(next);
                            nav::write_sidebar_open(next);
                        },
                        span { class: "panel-icon", aria_hidden: "true" }
                        span { class: "visually-hidden", "Toggle Sidebar" }
                    }
                }
                div { class: "banner-body",
                    h1 { id: "view-title", class: "font-oxanium", tabindex: "-1", "{route.title}" }
                    p {
                        id: "connection",
                        role: "status",
                        aria_live: "polite",
                        "data-state": "{session().connection.slug()}",
                        "data-access": "{session().access}",
                        "{session().banner}"
                    }
                    if session().access == "bearer" {
                        label { r#for: "bearer-token", class: "visually-hidden", "Bearer token" }
                        input {
                            id: "bearer-token",
                            class: "banner-token",
                            r#type: "password",
                            autocomplete: "off",
                            spellcheck: "false",
                            placeholder: "Bearer token",
                        }
                    }
                    button {
                        id: "theme-toggle",
                        class: "icon-button",
                        r#type: "button",
                        "data-choice": "{theme().slug()}",
                        aria_label: "{nav::theme_label(theme())}",
                        onclick: move |_| {
                            let next = theme().next();
                            nav::write_theme(next);
                            theme.set(next);
                        },
                        span { class: "theme-icon", aria_hidden: "true" }
                        span { class: "visually-hidden", "{nav::theme_label(theme())}" }
                    }
                }
            }
            main { id: "main", tabindex: "-1",
                div {
                    id: "alert",
                    role: "alert",
                    aria_live: "assertive",
                    hidden: notice_hidden,
                    "{notice}"
                }
                CardLoading { concealed: !panes.loading }
                div {
                    id: "view-logs",
                    class: "view",
                    hidden: panes.logs,
                    LogView { ui: log_view }
                    InventoryView {
                        primitive: Primitive::Logs,
                        ui: logs,
                        visible: route.primitive == Some(Primitive::Logs),
                    }
                }
                div {
                    id: "view-metrics",
                    class: "view",
                    hidden: panes.metrics,
                    MetricView {}
                    InventoryView {
                        primitive: Primitive::Metrics,
                        ui: metrics,
                        visible: route.primitive == Some(Primitive::Metrics),
                    }
                }
                div {
                    id: "view-tasks",
                    class: "view",
                    hidden: panes.tasks,
                    TaskView { ui: task_view }
                    InventoryView {
                        primitive: Primitive::Tasks,
                        ui: tasks,
                        visible: route.primitive == Some(Primitive::Tasks),
                    }
                }
                div {
                    id: "view-images",
                    class: "view",
                    hidden: panes.images,
                    ImageView { ui: image_view }
                    InventoryView {
                        primitive: Primitive::Images,
                        ui: images,
                        visible: route.primitive == Some(Primitive::Images),
                    }
                }
                div {
                    id: "view-chat",
                    class: "view",
                    hidden: panes.chat,
                    ChatView { chat }
                }
                WorkbenchView {
                    ui: workbench,
                    hidden: panes.workbench,
                }
            }
            }
        }
    }
}

fn boot_islands(mut session: Signal<SessionUi>, mut ready: Signal<bool>) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(origin) = session::page_origin() else {
        let applied = session::apply_card(CardLoad::NoOrigin, &session.peek(), None);
        session.set(applied.ui);
        return;
    };
    let transport = Transport::new(origin);
    session::set_transport(transport);
    nav::install();
    ready.set(true);
}

fn apply_card(mut session: Signal<SessionUi>, chat: Signal<ChatUi>, load: Option<(u32, CardLoad)>) {
    let Some((_tick, load)) = load else {
        return;
    };
    if matches!(load, CardLoad::Idle) {
        return;
    }
    let secret = session::native_secret();
    let applied = session::apply_card(load, &session.peek(), secret.as_deref());
    session.set(applied.ui);
    if let Some(card) = &applied.card {
        chat::apply(chat, card);
        inventory::note_card(card);
    }
}

#[component]
fn NavLink(id: String, href: String, route: Route, label: String, glyph: String) -> Element {
    let target = href.clone();
    rsx! {
        a {
            id: "{id}",
            href: "{href}",
            aria_current: nav::current_link(route, &id),
            onclick: move |event| {
                if event.modifiers().ctrl()
                    || event.modifiers().meta()
                    || event.modifiers().shift()
                    || event.modifiers().alt()
                {
                    return;
                }
                event.prevent_default();
                nav::open(&target);
            },
            span { class: "nav-glyph", aria_hidden: "true", "{glyph}" }
            span { "{label}" }
        }
    }
}

fn expanded(open: bool) -> &'static str {
    if open { "true" } else { "false" }
}

struct PaneHidden {
    logs: bool,
    metrics: bool,
    tasks: bool,
    images: bool,
    chat: bool,
    workbench: bool,
    loading: bool,
}

fn pane_hidden(loading: bool, route: Route, path: &str) -> PaneHidden {
    PaneHidden {
        loading,
        logs: loading || route.view != "view-logs",
        metrics: loading || route.view != "view-metrics",
        tasks: loading || route.view != "view-tasks",
        images: loading || route.view != "view-images",
        chat: loading || route.view != "view-chat",
        workbench: loading || path != "workbench",
    }
}

fn drop_bootstrap() {
    if let Some(app) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("app"))
    {
        app.set_text_content(Some(""));
    }
}

fn logo_src() -> String {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("app"))
        .and_then(|app| app.get_attribute("data-logo"))
        .unwrap_or_default()
}
