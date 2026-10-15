//! Accessible shell. Every agent-provided string is written as text.

use crate::card::{self, Access, Card};
use crate::codec::{self, Outcome};
use crate::shell::page::{self, Page};

#[derive(Clone, Copy)]
pub enum Connection {
    Loading,
    Connected,
    Offline,
    Unauthorized,
    Forbidden,
    Malformed,
    Unsupported,
}

impl Connection {
    fn slug(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Connected => "connected",
            Self::Offline => "offline",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::Malformed => "malformed",
            Self::Unsupported => "unsupported",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Loading => "Loading agent card.",
            Self::Connected => "Connected.",
            Self::Offline => "Offline.",
            Self::Unauthorized => "Unauthorized.",
            Self::Forbidden => "Forbidden.",
            Self::Malformed => "Malformed response.",
            Self::Unsupported => "Unsupported.",
        }
    }
}

pub fn mount(page: &Page) {
    let Some(root) = page.mount_root() else {
        return;
    };
    let logo_src = root.get_attribute("data-logo").unwrap_or_default();
    page::clear_children(&root);
    let shell = node(page, "div", Some("shell"));
    page::attr(&shell, "class", "shell");
    let skip = node(page, "a", None);
    page::attr(&skip, "class", "skip");
    page::attr(&skip, "href", "#main");
    skip.set_text_content(Some("Skip to content"));
    let header = node(page, "header", None);
    page::attr(&header, "class", "banner");
    let toggle = node(page, "button", Some("nav-toggle"));
    page::attr(&toggle, "type", "button");
    page::attr(&toggle, "aria-controls", "primary-nav");
    page::attr(&toggle, "aria-expanded", "false");
    page::attr(&toggle, "aria-label", "Open navigation");
    page::attr(&toggle, "hidden", "hidden");
    toggle.set_text_content(Some("Menu"));
    header.append_child(&toggle).ok();
    text(page, &header, "p", Some("view-title"), "Agent");
    let connection = node(page, "p", Some("connection"));
    page::attr(&connection, "role", "status");
    page::attr(&connection, "aria-live", "polite");
    header.append_child(&connection).ok();
    let nav = node(page, "nav", Some("primary-nav"));
    page::attr(&nav, "aria-label", "Lab sections");
    let brand = node(page, "div", Some("brand"));
    if !logo_src.is_empty() {
        let logo = node(page, "img", None);
        page::attr(&logo, "src", &logo_src);
        page::attr(&logo, "alt", "A3 Analytics");
        page::attr(&logo, "width", "2081");
        page::attr(&logo, "height", "691");
        brand.append_child(&logo).ok();
    }
    text(page, &brand, "p", Some("brand-name"), "A2A Lab");
    nav.append_child(&brand).ok();
    for (id, href, label) in [
        ("nav-agent", "#agent", "Agent"),
        ("nav-logs", "#logs", "Logs"),
        ("nav-metrics", "#metrics", "Metrics"),
        ("nav-tasks", "#tasks", "Tasks"),
        ("nav-images", "#images", "Images"),
        ("nav-chat", "#chat", "Chat"),
    ] {
        let link = node(page, "a", Some(id));
        page::attr(&link, "href", href);
        link.set_text_content(Some(label));
        nav.append_child(&link).ok();
    }
    let main = node(page, "main", Some("main"));
    let agent = view(page, "view-agent", "agent-heading", "Agent");
    section(page, &agent, "identity", "Identity", |body| {
        let name = node(page, "p", Some("agent-name"));
        page::attr(&name, "class", "agent-name");
        name.set_text_content(Some("A2A Lab"));
        body.append_child(&name).ok();
        text(page, body, "p", Some("agent-description"), "");
        text(page, body, "p", Some("agent-version"), "");
    });
    section(page, &agent, "interfaces", "Interfaces", |body| {
        list(page, body, "interfaces-list");
    });
    section(page, &agent, "skills", "Skills", |body| {
        list(page, body, "skills-list");
    });
    section(page, &agent, "capabilities", "Capabilities", |body| {
        list(page, body, "capabilities-list");
    });
    section(page, &agent, "security", "Security", |body| {
        text(page, body, "p", Some("access"), "");
        list(page, body, "security-schemes");
        let slot = node(page, "div", Some("credential"));
        body.append_child(&slot).ok();
    });
    let advanced = node(page, "details", Some("protocol-advanced"));
    let summary = node(page, "summary", Some("protocol-advanced-summary"));
    summary.set_text_content(Some("Advanced"));
    advanced.append_child(&summary).ok();
    section(page, &advanced, "protocol", "Protocol", |body| {
        field(page, body, "Message", "message-text", true);
        field(page, body, "Lab command", "command-json", true);
        field(page, body, "Context", "context-id", false);
        let actions = node(page, "div", None);
        page::attr(&actions, "class", "actions");
        button(page, &actions, "send-text", "Send text");
        button(page, &actions, "send-command", "Send command");
        button(page, &actions, "list-tasks", "List tasks");
        button(page, &actions, "retry", "Retry connection");
        body.append_child(&actions).ok();
        let alert = node(page, "div", Some("alert"));
        page::attr(&alert, "role", "alert");
        page::attr(&alert, "aria-live", "assertive");
        body.append_child(&alert).ok();
        let output = node(page, "pre", Some("output"));
        page::attr(&output, "aria-label", "Protocol result");
        body.append_child(&output).ok();
    });
    agent.append_child(&advanced).ok();
    let logs = view(page, "view-logs", "", "");
    let metrics = view(page, "view-metrics", "", "");
    let tasks = view(page, "view-tasks", "", "");
    let images = view(page, "view-images", "", "");
    super::logs::mount(page, &logs);
    super::metrics::mount(page, &metrics);
    super::inventory::mount(page, &logs, crate::inventory::Primitive::Logs);
    super::inventory::mount(page, &metrics, crate::inventory::Primitive::Metrics);
    super::inventory::mount(page, &tasks, crate::inventory::Primitive::Tasks);
    super::inventory::mount(page, &images, crate::inventory::Primitive::Images);
    super::workbench::mount(page, &logs);
    let chat = view(page, "view-chat", "", "");
    mount_chat(page, &chat);
    for child in [&skip, &header, &nav, &main] {
        shell.append_child(child).ok();
    }
    root.append_child(&shell).ok();
    page.set_data("security", "data-access", "pending");
    page.set_data("chat", "data-context", "");
    configure_chat(page, false);
    set_chat_status(
        page,
        "disabled",
        "Checking whether this agent accepts chat.",
    );
    set_connection(page, Connection::Loading);
}

pub fn configure_chat(page: &Page, enabled: bool) {
    page.set_data(
        "chat",
        "data-enabled",
        if enabled { "true" } else { "false" },
    );
    page.set_data("chat", "data-pending", "false");
    sync_chat(page, enabled, false, false);
}

pub fn set_chat_pending(page: &Page, pending: bool, retry: bool) {
    page.set_data(
        "chat",
        "data-pending",
        if pending { "true" } else { "false" },
    );
    let enabled = page.data("chat", "data-enabled") == "true";
    sync_chat(page, enabled, pending, retry);
}

pub fn set_chat_status(page: &Page, state: &str, message: &str) {
    page.set_data("chat-status", "data-state", state);
    page.set_text("chat-status", message);
}

pub fn set_chat_alert(page: &Page, message: &str) {
    page.set_text("chat-alert", message);
}

pub fn set_transcript(page: &Page, text: &str) {
    page.set_text("chat-log", text);
}

pub fn append_transcript(page: &Page, text: &str) {
    let current = page
        .element("chat-log")
        .and_then(|element| element.text_content())
        .unwrap_or_default();
    let next = if current.trim().is_empty() {
        text.to_owned()
    } else {
        format!("{current}\n\n{text}")
    };
    page.set_text("chat-log", &next);
}

pub fn set_connection(page: &Page, connection: Connection) {
    page.set_data("connection", "data-state", connection.slug());
    page.set_text("connection", connection.label());
}

pub fn show_card(page: &Page, card: &Card, secret: Option<&str>) {
    let card = card::redact_card(card, secret);
    page.set_text("agent-name", &card.name);
    page.set_text("agent-description", &card.description);
    page.set_text("agent-version", &card.version);
    fill(
        page,
        "interfaces-list",
        &card
            .interfaces
            .iter()
            .map(card::interface_line)
            .collect::<Vec<_>>(),
    );
    fill(
        page,
        "skills-list",
        &card.skills.iter().map(card::skill_line).collect::<Vec<_>>(),
    );
    fill(page, "capabilities-list", &card::capability_lines(&card));
    page.set_text("access", &card.access_label());
    fill(page, "security-schemes", &card.security_lines());
    let access = match card.access {
        Access::Anonymous { .. } => "anonymous",
        Access::Bearer { .. } => "bearer",
        Access::Unsupported { .. } => "unsupported",
    };
    page.set_data("security", "data-access", access);
    sync_credential(page, matches!(card.access, Access::Bearer { .. }));
}

pub fn show_outcome(page: &Page, outcome: &Outcome) {
    page.set_text("alert", "");
    page.set_text("output", &codec::render_outcome(outcome));
    if let Some(context) = codec::context_of(outcome) {
        page.set_control("context-id", context);
    }
    set_connection(page, Connection::Connected);
}

pub fn show_failure(page: &Page, connection: Connection, detail: &str) {
    set_connection(page, connection);
    page.set_text("connection", &format!("{} {detail}", connection.label()));
    page.set_text("alert", detail);
}

fn sync_credential(page: &Page, bearer: bool) {
    if bearer {
        if page.element("bearer-token").is_some() {
            return;
        }
        let Some(slot) = page.element("credential") else {
            return;
        };
        field(page, &slot, "Bearer token", "bearer-token", false);
        if let Some(input) = page.element("bearer-token") {
            page::attr(&input, "type", "password");
            page::attr(&input, "autocomplete", "off");
            page::attr(&input, "spellcheck", "false");
        }
        return;
    }
    page.clear("credential");
}

fn mount_chat(page: &Page, main: &web_sys::Element) {
    section(page, main, "chat", "Chat", |body| {
        let bar = node(page, "div", None);
        page::attr(&bar, "class", "chat-bar");
        button(page, &bar, "chat-new", "New session");
        button(page, &bar, "chat-retry", "Retry");
        body.append_child(&bar).ok();
        let status = node(page, "p", Some("chat-status"));
        page::attr(&status, "role", "status");
        page::attr(&status, "aria-live", "polite");
        body.append_child(&status).ok();
        let alert = node(page, "div", Some("chat-alert"));
        page::attr(&alert, "role", "alert");
        page::attr(&alert, "aria-live", "assertive");
        body.append_child(&alert).ok();
        let log = node(page, "pre", Some("chat-log"));
        page::attr(&log, "role", "log");
        page::attr(&log, "aria-label", "Conversation");
        body.append_child(&log).ok();
        let compose = node(page, "div", None);
        page::attr(&compose, "class", "chat-compose");
        let field = node(page, "div", None);
        page::attr(&field, "class", "chat-field");
        let label = node(page, "label", None);
        page::attr(&label, "class", "visually-hidden");
        page::attr(&label, "for", "chat-draft");
        label.set_text_content(Some("Message"));
        let draft = node(page, "textarea", Some("chat-draft"));
        page::attr(&draft, "rows", "2");
        page::attr(&draft, "placeholder", "Write a message");
        field.append_child(&label).ok();
        field.append_child(&draft).ok();
        compose.append_child(&field).ok();
        button(page, &compose, "chat-send", "Send");
        body.append_child(&compose).ok();
        quiet(page, "chat-draft");
    });
}

fn quiet(page: &Page, id: &str) {
    if let Some(element) = page.element(id) {
        page::attr(&element, "autocomplete", "off");
        page::attr(&element, "spellcheck", "false");
    }
}

fn sync_chat(page: &Page, enabled: bool, pending: bool, retry: bool) {
    set_disabled(page, "chat-draft", !enabled);
    set_aria_enabled(page, "chat-send", enabled && !pending);
    set_aria_enabled(page, "chat-retry", enabled && !pending && retry);
    set_aria_enabled(page, "chat-new", enabled && !pending);
}

fn set_aria_enabled(page: &Page, id: &str, enabled: bool) {
    page.set_data(id, "aria-disabled", if enabled { "false" } else { "true" });
}

fn set_disabled(page: &Page, id: &str, disabled: bool) {
    if disabled {
        page.set_data(id, "disabled", "disabled");
    } else {
        page.remove_attr(id, "disabled");
    }
}

pub(super) fn section(
    page: &Page,
    parent: &web_sys::Element,
    id: &str,
    title: &str,
    fill: impl FnOnce(&web_sys::Element),
) {
    let section = node(page, "section", Some(id));
    let heading_id = format!("{id}-heading");
    let heading = node(page, "h2", Some(&heading_id));
    page::attr(&heading, "tabindex", "-1");
    heading.set_text_content(Some(title));
    section.append_child(&heading).ok();
    page::attr(&section, "aria-labelledby", &heading_id);
    fill(&section);
    parent.append_child(&section).ok();
}

pub(super) fn field(
    page: &Page,
    parent: &web_sys::Element,
    label_text: &str,
    id: &str,
    area: bool,
) {
    let label = node(page, "label", None);
    page::attr(&label, "for", id);
    label.set_text_content(Some(label_text));
    let control = if area {
        let element = node(page, "textarea", Some(id));
        page::attr(&element, "rows", "4");
        element
    } else {
        node(page, "input", Some(id))
    };
    parent.append_child(&label).ok();
    parent.append_child(&control).ok();
}

pub(super) fn button(page: &Page, parent: &web_sys::Element, id: &str, label: &str) {
    let button = node(page, "button", Some(id));
    page::attr(&button, "type", "button");
    button.set_text_content(Some(label));
    parent.append_child(&button).ok();
}

fn view(page: &Page, id: &str, heading_id: &str, title: &str) -> web_sys::Element {
    let view = node(page, "div", Some(id));
    page::attr(&view, "class", "view");
    if !heading_id.is_empty() {
        let heading = node(page, "h1", Some(heading_id));
        page::attr(&heading, "tabindex", "-1");
        heading.set_text_content(Some(title));
        view.append_child(&heading).ok();
    }
    view
}

fn list(page: &Page, parent: &web_sys::Element, id: &str) {
    let list = node(page, "ul", Some(id));
    parent.append_child(&list).ok();
}

fn text(page: &Page, parent: &web_sys::Element, tag: &str, id: Option<&str>, value: &str) {
    let element = node(page, tag, id);
    element.set_text_content(Some(value));
    parent.append_child(&element).ok();
}

fn fill(page: &Page, id: &str, lines: &[String]) {
    let Some(list) = page.element(id) else {
        return;
    };
    page::clear_children(&list);
    for line in lines {
        let item = node(page, "li", None);
        item.set_text_content(Some(line));
        list.append_child(&item).ok();
    }
}

pub(super) fn node(page: &Page, tag: &str, id: Option<&str>) -> web_sys::Element {
    let Some(element) = page.create(tag).or_else(|| page.create("span")) else {
        wasm_bindgen::throw_str("The lab interface could not build its page.");
    };
    if let Some(id) = id {
        page::attr(&element, "id", id);
        page::remember(id, &element);
    }
    element
}
