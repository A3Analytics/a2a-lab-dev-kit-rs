//! Page session. The bearer token lives only in the password control.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use dioxus::core::{Runtime, RuntimeGuard, ScopeId, current_scope_id};
use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement};

use crate::card::{self, Access, Card};
use crate::codec::{self, ProtocolError};
use crate::shell::http::Transport;

#[derive(Clone, Copy, PartialEq, Eq)]
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
    pub fn slug(self) -> &'static str {
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

    pub fn label(self) -> &'static str {
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

#[derive(Clone, PartialEq)]
pub struct SessionUi {
    pub connection: Connection,
    pub banner: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub interfaces: Vec<String>,
    pub skills: Vec<String>,
    pub capabilities: Vec<String>,
    pub access: String,
    pub access_label: String,
    pub schemes: Vec<String>,
    pub alert: String,
    pub output: String,
}

pub fn refusal_text(ui: &SessionUi) -> String {
    let mut lines = Vec::new();
    push_notice(&mut lines, ui.alert.trim());
    push_notice(&mut lines, banner_detail(&ui.banner, ui.connection));
    lines.join("\n")
}

fn push_notice(lines: &mut Vec<String>, line: &str) {
    if line.is_empty() || lines.iter().any(|existing| existing == line) {
        return;
    }
    lines.push(line.to_owned());
}

fn banner_detail(banner: &str, connection: Connection) -> &str {
    if matches!(connection, Connection::Connected | Connection::Loading) {
        return "";
    }
    banner
        .strip_prefix(connection.label())
        .map(str::trim)
        .filter(|detail| !detail.is_empty())
        .unwrap_or("")
}

impl SessionUi {
    pub fn loading() -> Self {
        Self {
            connection: Connection::Loading,
            banner: Connection::Loading.label().to_owned(),
            name: "A2A Lab".to_owned(),
            description: String::new(),
            version: String::new(),
            interfaces: Vec::new(),
            skills: Vec::new(),
            capabilities: Vec::new(),
            access: "pending".to_owned(),
            access_label: String::new(),
            schemes: Vec::new(),
            alert: String::new(),
            output: String::new(),
        }
    }
}

#[derive(Clone)]
pub enum CardLoad {
    Idle,
    NoOrigin,
    Offline,
    Malformed,
    Ready(Card),
}

pub struct AppliedCard {
    pub ui: SessionUi,
    pub card: Option<Card>,
}

#[derive(Clone, PartialEq)]
pub enum ProtocolKind {
    SendText,
    SendCommand,
    ListTasks,
}

#[derive(Clone, PartialEq)]
pub struct ProtocolRequest {
    pub kind: ProtocolKind,
    pub message: String,
    pub command: String,
    pub context: String,
    pub access: String,
    pub connection: Connection,
}

#[derive(Clone, PartialEq)]
pub enum ProtocolOutcome {
    Idle,
    Alert(String),
    Fail {
        connection: Connection,
        detail: String,
    },
    Ok {
        output: String,
        context: Option<String>,
    },
}

thread_local! {
    static RUNTIME: RefCell<Option<Rc<Runtime>>> = const { RefCell::new(None) };
    static SCOPE: RefCell<Option<ScopeId>> = const { RefCell::new(None) };
    static UI: RefCell<Option<Signal<SessionUi>>> = const { RefCell::new(None) };
    static TRANSPORT: RefCell<Option<Transport>> = const { RefCell::new(None) };
}

pub fn bind(ui: Signal<SessionUi>) {
    RUNTIME.with(|slot| *slot.borrow_mut() = Some(Runtime::current()));
    SCOPE.with(|slot| *slot.borrow_mut() = Some(current_scope_id()));
    UI.with(|slot| *slot.borrow_mut() = Some(ui));
}

pub fn set_transport(transport: Transport) {
    TRANSPORT.with(|slot| *slot.borrow_mut() = Some(transport));
}

pub fn transport() -> Option<Transport> {
    TRANSPORT.with(|slot| slot.borrow().clone())
}

pub async fn load_card(ready: bool) -> CardLoad {
    if !ready {
        return CardLoad::Idle;
    }
    let Some(transport) = transport() else {
        return CardLoad::NoOrigin;
    };
    match transport.get_card().await {
        Err(()) => CardLoad::Offline,
        Ok(reply) => match crate::card::parse_card(&reply.body) {
            Err(()) => CardLoad::Malformed,
            Ok(card) => CardLoad::Ready(card),
        },
    }
}

pub fn apply_card(load: CardLoad, previous: &SessionUi, secret: Option<&str>) -> AppliedCard {
    match load {
        CardLoad::Idle => AppliedCard {
            ui: previous.clone(),
            card: None,
        },
        CardLoad::NoOrigin => applied_fail(
            previous,
            Connection::Unsupported,
            "This page has no origin to call.",
        ),
        CardLoad::Offline => applied_fail(
            previous,
            Connection::Offline,
            "The browser could not reach this origin.",
        ),
        CardLoad::Malformed => applied_fail(
            previous,
            Connection::Malformed,
            "The agent card could not be read.",
        ),
        CardLoad::Ready(card) => apply_ready(previous, &card, secret),
    }
}

fn apply_ready(previous: &SessionUi, card: &Card, secret: Option<&str>) -> AppliedCard {
    let card = card::redact_card(card, secret);
    let mut ui = identity_from(&card, previous);
    if !card.supports_http_json() {
        return applied_fail_ui(
            ui,
            Some(card),
            Connection::Unsupported,
            "This agent does not advertise HTTP+JSON.",
        );
    }
    if matches!(card.access, Access::Unsupported { .. }) {
        return applied_fail_ui(
            ui,
            Some(card),
            Connection::Unsupported,
            "This agent requires a security scheme this page cannot use.",
        );
    }
    ui.connection = Connection::Connected;
    ui.banner = Connection::Connected.label().to_owned();
    ui.alert.clear();
    AppliedCard {
        ui,
        card: Some(card),
    }
}

fn identity_from(card: &Card, previous: &SessionUi) -> SessionUi {
    let access = match card.access {
        Access::Anonymous { .. } => "anonymous",
        Access::Bearer { .. } => "bearer",
        Access::Unsupported { .. } => "unsupported",
    };
    SessionUi {
        connection: previous.connection,
        banner: previous.banner.clone(),
        name: card.name.clone(),
        description: card.description.clone(),
        version: card.version.clone(),
        interfaces: card.interfaces.iter().map(card::interface_line).collect(),
        skills: card.skills.iter().map(card::skill_line).collect(),
        capabilities: card::capability_lines(card),
        access: access.to_owned(),
        access_label: card.access_label(),
        schemes: card.security_lines(),
        alert: String::new(),
        output: previous.output.clone(),
    }
}

fn applied_fail(previous: &SessionUi, connection: Connection, detail: &str) -> AppliedCard {
    let mut ui = previous.clone();
    fail_ui(&mut ui, connection, detail);
    AppliedCard { ui, card: None }
}

fn applied_fail_ui(
    mut ui: SessionUi,
    card: Option<Card>,
    connection: Connection,
    detail: &str,
) -> AppliedCard {
    fail_ui(&mut ui, connection, detail);
    AppliedCard { ui, card }
}

fn fail_ui(ui: &mut SessionUi, connection: Connection, detail: &str) {
    ui.connection = connection;
    ui.banner = format!("{} {detail}", connection.label());
    ui.alert = detail.to_owned();
}

pub fn begin_retry(mut session: Signal<SessionUi>) {
    let mut ui = session();
    ui.connection = Connection::Loading;
    ui.banner = Connection::Loading.label().to_owned();
    ui.alert.clear();
    session.set(ui);
}

pub fn request_text(session: &SessionUi) -> Result<ProtocolRequest, String> {
    let message = control_value("message-text");
    let context = control_value("context-id");
    if message.trim().is_empty() {
        return Err("Enter a message.".to_owned());
    }
    guard(session, &[message.as_str(), context.as_str()])?;
    Ok(ProtocolRequest {
        kind: ProtocolKind::SendText,
        message,
        command: String::new(),
        context,
        access: session.access.clone(),
        connection: session.connection,
    })
}

pub fn request_command(session: &SessionUi) -> Result<ProtocolRequest, String> {
    let command = control_value("command-json");
    let context = control_value("context-id");
    if codec::parse_command(command.trim()).is_err() {
        return Err("The lab command is not a JSON object.".to_owned());
    }
    guard(session, &[command.as_str(), context.as_str()])?;
    Ok(ProtocolRequest {
        kind: ProtocolKind::SendCommand,
        message: String::new(),
        command,
        context,
        access: session.access.clone(),
        connection: session.connection,
    })
}

pub fn request_tasks(session: &SessionUi) -> Result<ProtocolRequest, String> {
    let context = control_value("context-id");
    if context.trim().is_empty() {
        return Err("Enter a context to list protocol tasks.".to_owned());
    }
    guard(session, &[context.as_str()])?;
    Ok(ProtocolRequest {
        kind: ProtocolKind::ListTasks,
        message: String::new(),
        command: String::new(),
        context,
        access: session.access.clone(),
        connection: session.connection,
    })
}

pub fn set_alert_ui(mut session: Signal<SessionUi>, message: &str) {
    let mut ui = session();
    ui.alert = message.to_owned();
    session.set(ui);
}

pub async fn run_protocol(
    request: Option<ProtocolRequest>,
    secret: Option<String>,
) -> ProtocolOutcome {
    let Some(request) = request else {
        return ProtocolOutcome::Idle;
    };
    if !calls_allowed_values(&request.access, request.connection) {
        return ProtocolOutcome::Alert(
            "This page will not send credentials or call an unsupported agent.".to_owned(),
        );
    }
    let Some(transport) = transport() else {
        return ProtocolOutcome::Fail {
            connection: Connection::Unsupported,
            detail: "This page has no origin to call.".to_owned(),
        };
    };
    let result = match request.kind {
        ProtocolKind::SendText => {
            let body = codec::text_message(
                &message_id(),
                request.message.trim(),
                request.context.trim(),
            );
            transport.send(&body, secret.as_deref()).await
        }
        ProtocolKind::SendCommand => {
            let Ok(command) = codec::parse_command(request.command.trim()) else {
                return ProtocolOutcome::Alert("The lab command is not a JSON object.".to_owned());
            };
            let Ok(body) = codec::command_message(&message_id(), &command, request.context.trim())
            else {
                return ProtocolOutcome::Alert("The lab command is not a JSON object.".to_owned());
            };
            transport.send(&body, secret.as_deref()).await
        }
        ProtocolKind::ListTasks => {
            let path = codec::tasks_path(request.context.trim());
            transport.tasks(&path, secret.as_deref()).await
        }
    };
    finish(result, secret.as_deref())
}

pub fn apply_protocol(mut session: Signal<SessionUi>, outcome: ProtocolOutcome) {
    match outcome {
        ProtocolOutcome::Idle => {}
        ProtocolOutcome::Alert(message) => {
            let mut ui = session();
            ui.alert = message;
            session.set(ui);
        }
        ProtocolOutcome::Fail { connection, detail } => {
            let mut ui = session();
            fail_ui(&mut ui, connection, &detail);
            session.set(ui);
        }
        ProtocolOutcome::Ok { output, context } => {
            let mut ui = session();
            ui.alert.clear();
            ui.output = output;
            ui.connection = Connection::Connected;
            ui.banner = Connection::Connected.label().to_owned();
            session.set(ui);
            if let Some(context) = context {
                write_control("context-id", &context);
            }
        }
    }
}

fn finish(result: Result<codec::Reply, ()>, secret: Option<&str>) -> ProtocolOutcome {
    match result {
        Err(()) => ProtocolOutcome::Fail {
            connection: Connection::Offline,
            detail: "The browser could not reach this origin.".to_owned(),
        },
        Ok(reply) => match codec::decode(&reply, secret) {
            Ok(outcome) => ProtocolOutcome::Ok {
                output: codec::render_outcome(&outcome),
                context: codec::context_of(&outcome).map(str::to_owned),
            },
            Err(ProtocolError::Unauthorized(message)) => ProtocolOutcome::Fail {
                connection: Connection::Unauthorized,
                detail: message,
            },
            Err(ProtocolError::Forbidden(message)) => ProtocolOutcome::Fail {
                connection: Connection::Forbidden,
                detail: message,
            },
            Err(ProtocolError::Malformed) => ProtocolOutcome::Fail {
                connection: Connection::Malformed,
                detail: "The response could not be read.".to_owned(),
            },
        },
    }
}

fn guard(session: &SessionUi, visible: &[&str]) -> Result<(), String> {
    if !calls_allowed_values(&session.access, session.connection) {
        return Err("This page will not send credentials or call an unsupported agent.".to_owned());
    }
    take_secret_values(visible).map(|_| ())
}

fn calls_allowed_values(access: &str, connection: Connection) -> bool {
    matches!(access, "anonymous" | "bearer") && connection != Connection::Unsupported
}

pub(super) fn calls_allowed() -> bool {
    let (access, connection) = session_gate();
    if connection == Connection::Loading || access.is_empty() || access == "pending" {
        return false;
    }
    let allowed = calls_allowed_values(&access, connection);
    if !allowed {
        set_alert("This page will not send credentials or call an unsupported agent.");
    }
    allowed
}

fn session_gate() -> (String, Connection) {
    if let Some(gate) = UI.with(|slot| {
        slot.borrow().as_ref().map(|ui| {
            let current = ui.peek();
            (current.access.clone(), current.connection)
        })
    }) {
        return gate;
    }
    let state = attr("connection", "data-state");
    let connection = match state.as_str() {
        "connected" => Connection::Connected,
        "offline" => Connection::Offline,
        "unauthorized" => Connection::Unauthorized,
        "forbidden" => Connection::Forbidden,
        "malformed" => Connection::Malformed,
        "unsupported" => Connection::Unsupported,
        _ => Connection::Loading,
    };
    (attr("security", "data-access"), connection)
}

pub(super) fn take_secret(visible: &[&str]) -> Result<Option<String>, ()> {
    take_secret_values(visible).map_err(|_| ())
}

fn take_secret_values(visible: &[&str]) -> Result<Option<String>, String> {
    let secret = native_secret();
    if let Some(secret) = &secret
        && visible.iter().any(|value| value.contains(secret))
    {
        let message = "The bearer token can only be sent in the Authorization header.".to_owned();
        set_alert(&message);
        return Err(message);
    }
    Ok(secret)
}

pub(super) fn page_origin() -> Option<String> {
    web_sys::window()?.location().origin().ok()
}

pub(super) fn message_id() -> String {
    static SEQ: AtomicU32 = AtomicU32::new(1);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let millis = js_sys::Date::now() as u64;
    format!("ui-{millis}-{seq}")
}

fn set_alert(message: &str) {
    let message_owned = message.to_owned();
    // Publish after render so a refusal from the active route cannot re-enter the shell.
    wasm_bindgen_futures::spawn_local(async move {
        publish_alert(message_owned);
    });
}

fn publish_alert(message: String) {
    run(move || {
        UI.with(|slot| {
            if let Some(mut ui) = *slot.borrow() {
                let mut next = ui();
                next.alert = message;
                ui.set(next);
            }
        });
    });
}

pub fn native_secret() -> Option<String> {
    let value = control_value("bearer-token");
    (!value.is_empty()).then_some(value)
}

pub fn control_value(id: &str) -> String {
    let Some(element) = document_element(id) else {
        return String::new();
    };
    if let Ok(input) = element.clone().dyn_into::<HtmlInputElement>() {
        return input.value();
    }
    if let Ok(select) = element.clone().dyn_into::<HtmlSelectElement>() {
        return select.value();
    }
    element
        .dyn_into::<HtmlTextAreaElement>()
        .map(|area| area.value())
        .unwrap_or_default()
}

pub fn write_control(id: &str, value: &str) {
    let Some(element) = document_element(id) else {
        return;
    };
    if let Ok(input) = element.clone().dyn_into::<HtmlInputElement>() {
        input.set_value(value);
        return;
    }
    if let Ok(select) = element.clone().dyn_into::<HtmlSelectElement>() {
        select.set_value(value);
        return;
    }
    if let Ok(area) = element.dyn_into::<HtmlTextAreaElement>() {
        area.set_value(value);
    }
}

fn document_element(id: &str) -> Option<web_sys::Element> {
    web_sys::window()?.document()?.get_element_by_id(id)
}

fn attr(id: &str, name: &str) -> String {
    document_element(id)
        .and_then(|element| element.get_attribute(name))
        .unwrap_or_default()
}

fn run<F: FnOnce()>(job: F) {
    let runtime = RUNTIME.with(|slot| slot.borrow().clone());
    let scope = SCOPE.with(|slot| *slot.borrow());
    if let (Some(runtime), Some(scope)) = (runtime, scope) {
        let _guard = RuntimeGuard::new(runtime.clone());
        runtime.in_scope(scope, job);
        return;
    }
    job();
}
