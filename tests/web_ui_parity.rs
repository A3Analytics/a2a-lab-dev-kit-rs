//! One headless-browser pass against a real `A2aServer`, compared with `A2aClient`.

#![cfg(feature = "web-ui")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabError, A2aLabService, A2aServer, AgentMessageFuture, AgentMessageHandler,
    AgentMessageReply, AgentMessageRequest, AuthError, Authenticator, ListImageSourcesRequest,
    MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks, Principal,
};
use async_trait::async_trait;
use serde_json::{Value, json};

#[path = "harness/browser.rs"]
mod browser;
#[path = "parity/matrix.rs"]
mod matrix;

use browser::{Browser, header, request_method, request_url};
use matrix::{
    assert_agent_message_advertised, assert_covers_commands, assert_feature_matrix,
    assert_public_assets, auth_cases, chat_cases, complete_runs, failure_cases, image_unavailable,
    listen, page_of, secured, seed, success_cases,
};

const MARKUP: &str = r#"<img id="parity-xss" src=x onerror="window.__parityXss=1">"#;
const PREVIEW_LIMIT: usize = 262_144;
const ALICE: &str = "alice-token";
const BOB: &str = "bob-token";
const NOSCOPE: &str = "noscope-token";
const START: &str = "2020-06-01T00:00:00Z";
const END: &str = "2020-06-02T00:00:00Z";
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
const OPERATIONS: [&str; 12] = [
    "list_log_sources",
    "query_logs",
    "list_metrics",
    "query_metric",
    "list_tasks",
    "start_task",
    "get_task_status",
    "list_image_sources",
    "list_images",
    "search_images",
    "get_image",
    "get_current_image",
];

struct Lab {
    service: Arc<dyn a2a_lab_dev_kit::A2aLabApi>,
    images: MemoryImages,
    tasks: MemoryTasks,
}

struct Success {
    name: &'static str,
    operation: String,
    command: Value,
    result: Value,
    protocol_state: String,
    absent: Vec<&'static str>,
    preview_bytes: Option<usize>,
}

struct Failure {
    name: &'static str,
    operation: String,
    command: Value,
    client: String,
    kind: &'static str,
    status: &'static str,
}

struct Script {
    lab: String,
    bare: String,
    secure: String,
    successes: Vec<Success>,
    failures: Vec<Failure>,
    bare_images: Failure,
    chat_unavailable: String,
    anonymous_auth: String,
    forbidden_auth: String,
    ping: mpsc::Sender<()>,
    go: mpsc::Receiver<String>,
    done: mpsc::Sender<()>,
}

struct Echo;

impl AgentMessageHandler for Echo {
    fn handle(
        &self,
        request: AgentMessageRequest,
    ) -> AgentMessageFuture<'_, Result<AgentMessageReply, A2aLabError>> {
        Box::pin(async move {
            if request.text == "unavailable" {
                return Err(A2aLabError::unavailable("agent-message is unavailable"));
            }
            Ok(AgentMessageReply {
                text: format!("echo:{}", request.text),
            })
        })
    }
}

struct Gate;

#[async_trait]
impl Authenticator for Gate {
    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        match token {
            ALICE => Ok(Principal::new("alice").with_scopes(["lab"])),
            BOB => Ok(Principal::new("bob").with_scopes(["lab"])),
            NOSCOPE => Ok(Principal::new("noscope")),
            _ => Err(AuthError::unauthenticated("rejected token")),
        }
    }
}

#[tokio::test]
async fn browser_and_rust_client_agree() {
    let lab = seed().await;
    let stop = Arc::new(AtomicBool::new(false));
    complete_runs(lab.tasks.clone(), Arc::clone(&stop));
    let opted = listen(
        A2aServer::new(&lab.service)
            .with_web_ui()
            .with_message_handler(Arc::new(Echo)),
    )
    .await;
    let headless = listen(A2aServer::new(&lab.service)).await;
    let bare_service =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share();
    let bare = listen(A2aServer::new(&bare_service).with_web_ui()).await;
    let secure_service =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share();
    let secure = listen(secured(&secure_service)).await;

    assert_feature_matrix(&opted, &headless).await;
    assert_public_assets(&opted).await;
    assert_agent_message_advertised(&opted, true).await;
    assert_agent_message_advertised(&bare, false).await;

    let client = A2aClient::new(&opted).expect("lab client");
    let successes = success_cases(&client).await;
    assert_covers_commands(&successes);
    let failures = failure_cases(&client).await;
    let bare_images = image_unavailable(&bare).await;
    let chat_unavailable = chat_cases(&client).await;
    let (anonymous_auth, forbidden_auth) = auth_cases(&secure).await;

    let (ping_tx, mut ping_rx) = mpsc::channel(1);
    let (go_tx, go_rx) = mpsc::channel(1);
    let (done_tx, mut done_rx) = mpsc::channel(1);
    let images = lab.images.clone();
    let failure_origin = opted.clone();
    let bridge = tokio::spawn(async move {
        ping_rx
            .recv()
            .await
            .expect("browser reached the provider failure");
        images.set_unavailable("camera pipeline failed").await;
        let failed = A2aClient::new(&failure_origin)
            .expect("failure client")
            .list_image_sources(ListImageSourcesRequest::new(page_of(10)).expect("page"))
            .await
            .expect_err("provider failure");
        go_tx
            .send(failed.to_string())
            .await
            .expect("send failure text");
        done_rx
            .recv()
            .await
            .expect("browser finished the provider failure");
        images.clear_unavailable().await;
    });
    let script = Script {
        lab: opted,
        bare,
        secure,
        successes,
        failures,
        bare_images,
        chat_unavailable,
        anonymous_auth,
        forbidden_auth,
        ping: ping_tx,
        go: go_rx,
        done: done_tx,
    };
    let browser = tokio::task::spawn_blocking(move || drive(script));
    let (bridge, browser) = tokio::join!(bridge, browser);
    stop.store(true, Ordering::Relaxed);
    bridge.expect("provider failure bridge");
    browser.expect("browser thread").expect("browser parity");
}

fn drive(mut script: Script) -> Result<(), String> {
    let mut page = Browser::open(&format!("{}/ui/", script.lab));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    for panel in [
        "inventory-logs",
        "inventory-metrics",
        "inventory-tasks",
        "inventory-images",
    ] {
        let slug = panel.trim_start_matches("inventory-");
        page.show(&format!("#inventory/{slug}"));
        page.wait_until(&format!(
            "document.getElementById('{panel}')?.getAttribute('aria-busy') === 'false'"
        ));
    }
    access(&mut page, "");
    for case in &script.successes {
        compare_success(&mut page, &script.lab, case);
    }
    tasks_reader_parity(&mut page, &script.lab);
    access(&mut page, "Success");
    chat_parity(&mut page, &script.chat_unavailable);
    for case in &script.failures {
        compare_failure(&mut page, &script.lab, case);
    }
    access(&mut page, "Status error");
    script
        .ping
        .blocking_send(())
        .map_err(|_| "ping".to_owned())?;
    let client = script
        .go
        .blocking_recv()
        .ok_or_else(|| "provider failure text".to_owned())?;
    compare_failure(
        &mut page,
        &script.lab,
        &Failure {
            name: "provider failure",
            operation: "list_image_sources".to_owned(),
            command: json!({
                "operation": "list_image_sources",
                "params": {"page": {"limit": 10}}
            }),
            client,
            kind: "unavailable",
            status: "Unavailable",
        },
    );
    script
        .done
        .blocking_send(())
        .map_err(|_| "done".to_owned())?;
    page.goto(&format!("{}/ui/", script.bare));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    page.show("#chat");
    page.wait_until(
        "document.getElementById('chat-status')?.textContent.includes('does not advertise agent-message')",
    );
    let denied = text(
        &mut page,
        "document.getElementById('chat-status')?.textContent",
    );
    let before = message_sends(&page, &script.bare);
    page.set_value("chat-draft", "hello");
    page.click("chat-send");
    let after = message_sends(&page, &script.bare);
    if before != after || !denied.contains("does not advertise agent-message") {
        return Err(format!(
            "parity difference: Chat reader, case unavailable agent-message\nexpected: skill absent\nobserved: {denied}"
        ));
    }
    compare_failure(&mut page, &script.bare, &script.bare_images);
    page.goto(&format!("{}/ui/", script.secure));
    page.wait_until("document.getElementById('bearer-token')");
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    security_parity(&mut page, &script);
    Ok(())
}

fn compare_success(page: &mut Browser, base: &str, case: &Success) {
    let label = format!("{} reader, case {}", reader_of(&case.operation), case.name);
    let observed = send_command(page, &case.command);
    let sent = latest_command(page, base, &case.operation);
    if sent != case.command {
        diverge(&label, &case.command.to_string(), &sent.to_string());
    }
    if observed["operation"] != case.operation {
        diverge(
            &label,
            &case.operation,
            observed["operation"].as_str().unwrap_or(""),
        );
    }
    if observed["protocol"] != case.protocol_state {
        diverge(
            &label,
            &case.protocol_state,
            observed["protocol"].as_str().unwrap_or(""),
        );
    }
    let tree = observed["tree"].as_str().unwrap_or("");
    if case.operation == "start_task" && !tree.contains("run-") {
        diverge(&label, "fresh run id", tree);
    }
    for needle in needles(&case.result) {
        if !tree.contains(&needle) {
            diverge(
                &label,
                &format!("result contains {needle}: {}", case.result),
                tree,
            );
        }
    }
    for absent in &case.absent {
        if tree.contains(absent) {
            diverge(&label, &format!("result omits {absent}"), tree);
        }
    }
    let cursor = case.result.get("next_cursor").and_then(Value::as_str);
    let has_cursor = tree.contains("next_cursor");
    if cursor.is_some() != has_cursor {
        diverge(&label, &format!("next_cursor {cursor:?}"), tree);
    }
    match case.preview_bytes {
        Some(size) => {
            let image = observed["image"].as_str().unwrap_or("");
            let preview = observed["preview"].as_str().unwrap_or("");
            if !image.contains("image/png")
                || !image.contains(&format!("Decoded bytes {size}"))
                || image.contains("exceeds")
                || !preview.starts_with("data:image/png;base64,")
            {
                diverge(
                    &label,
                    &format!("png preview of {size} bytes"),
                    &format!("{image} {preview}"),
                );
            }
        }
        None => {
            if !observed["preview"].as_str().unwrap_or("").is_empty() {
                diverge(
                    &label,
                    "no image preview",
                    observed["preview"].as_str().unwrap_or(""),
                );
            }
        }
    }
    assert_inert(page, &label);
}

fn compare_failure(page: &mut Browser, base: &str, case: &Failure) {
    let label = format!("{} reader, case {}", reader_of(&case.operation), case.name);
    let observed = send_command(page, &case.command);
    let sent = latest_command(page, base, &case.operation);
    if sent != case.command {
        diverge(&label, &case.command.to_string(), &sent.to_string());
    }
    let browser = format!(
        "{} {} {} {}",
        observed["kind"].as_str().unwrap_or(""),
        observed["status"].as_str().unwrap_or(""),
        observed["message"].as_str().unwrap_or(""),
        observed["tree"].as_str().unwrap_or("")
    );
    if observed["kind"] != case.kind
        || observed["status"] != case.status
        || !agrees(&case.client, &browser)
    {
        diverge(&label, &case.client, &browser);
    }
    assert_inert(page, &label);
}

fn tasks_reader_parity(page: &mut Browser, base: &str) {
    page.show("#tasks");
    page.wait_until("document.getElementById('task-pick-0')?.getAttribute('data-task') === 'heat'");
    let listed = latest_command(page, base, "list_tasks");
    if listed["operation"] != "list_tasks" {
        diverge(
            "Tasks reader, case list_tasks",
            "list_tasks",
            &listed.to_string(),
        );
    }
    let names = text(
        page,
        "[...document.querySelectorAll('#task-sources [data-task]')].map(el => el.getAttribute('data-task')).join(' ')",
    );
    if !names.contains("heat") || !names.contains("mix") {
        diverge(
            "Tasks reader, case list_tasks",
            "heat and mix in the Tasks reader",
            &names,
        );
    }
    let before = message_sends(page, base);
    page.click("task-start");
    page.wait_until("document.querySelector('#task-runs [data-run]')");
    if message_sends(page, base) <= before {
        diverge(
            "Tasks reader, case start_task",
            "a start_task message:send",
            "none",
        );
    }
    let started = latest_command(page, base, "start_task");
    if started["params"]["wait"] != false
        || started["params"]["task_id"] != "heat"
        || started["params"]["input"] != json!({})
    {
        diverge(
            "Tasks reader, case start_task",
            "task_id heat, wait false, empty input",
            &started.to_string(),
        );
    }
    let card = text(
        page,
        "document.querySelector('#task-runs [data-run]')?.textContent ?? ''",
    );
    if !card.contains("heat")
        || !(card.contains("completed") || card.contains("submitted") || card.contains("working"))
    {
        diverge(
            "Tasks reader, case start_task",
            "session-local heat run card",
            &card,
        );
    }
    let polled = page.requests(base).iter().any(|request| {
        request_url(request).contains("message:send")
            && request["params"]["request"]["postData"]
                .as_str()
                .unwrap_or("")
                .contains("get_task_status")
    });
    if polled {
        let status = latest_command(page, base, "get_task_status");
        if status["params"]["id"].as_str().unwrap_or("").is_empty() {
            diverge(
                "Tasks reader, case get_task_status",
                "lab run id",
                &status.to_string(),
            );
        }
    }
    if page
        .requests(base)
        .iter()
        .any(|request| request_url(request).contains("/tasks?"))
    {
        diverge(
            "Tasks reader, case protocol GET /tasks",
            "no protocol GET /tasks",
            "a GET /tasks request",
        );
    }
    if page.eval("document.getElementById('workbench')?.hasAttribute('hidden')")
        != Value::Bool(true)
        || page.eval("document.getElementById('inventory-tasks')?.hasAttribute('hidden')")
            != Value::Bool(true)
    {
        diverge(
            "Tasks reader, case hidden inventory and workbench",
            "inventory-tasks and workbench hidden",
            "visible on #tasks",
        );
    }
}

fn send_command(page: &mut Browser, command: &Value) -> Value {
    let operation = command["operation"].as_str().unwrap_or("");
    let hash = match operation {
        "list_metrics" | "query_metric" => "#metrics",
        "list_tasks" | "start_task" | "get_task_status" => "#tasks",
        "list_image_sources" | "list_images" | "search_images" | "get_image"
        | "get_current_image" => "#images",
        _ => "#logs",
    };
    page.show(hash);
    let before = text(
        page,
        "document.getElementById('workbench-status')?.dataset.gen ?? ''",
    );
    page.set_value(
        "workbench-operation",
        command["operation"].as_str().unwrap_or(""),
    );
    page.set_value("workbench-params", &command["params"].to_string());
    page.click("workbench-send");
    page.wait_until(&format!(
        "document.getElementById('workbench-status')?.dataset.gen !== '{before}' && document.getElementById('workbench')?.getAttribute('aria-busy') === 'false'"
    ));
    page.eval(
        "({
            kind: document.getElementById('workbench')?.dataset.kind ?? '',
            status: document.getElementById('workbench-status')?.textContent ?? '',
            alert: document.getElementById('workbench-alert')?.textContent ?? '',
            tree: document.getElementById('workbench-tree')?.textContent ?? '',
            image: document.getElementById('workbench-image')?.textContent ?? '',
            preview: (document.getElementById('workbench-preview')?.getAttribute('src') ?? '').slice(0, 32),
            message: document.querySelector('#workbench-facts [data-fact=\"message\"]')?.textContent ?? '',
            operation: document.querySelector('#workbench-facts [data-fact=\"operation\"]')?.textContent ?? '',
            protocol: document.querySelector('#workbench-facts [data-fact=\"protocol-state\"]')?.textContent ?? ''
        })",
    )
}

fn latest_command(page: &Browser, base: &str, operation: &str) -> Value {
    let mut found = None;
    for request in page.requests(base) {
        if !request_url(&request).contains("message:send") {
            continue;
        }
        let post = request["params"]["request"]["postData"]
            .as_str()
            .unwrap_or("");
        let Ok(body) = serde_json::from_str::<Value>(post) else {
            continue;
        };
        let data = body["message"]["parts"][0]["data"].clone();
        if data["operation"] != operation {
            continue;
        }
        if body["message"].get("contextId").is_some() {
            diverge(
                &format!("{} reader, case {operation}", reader_of(operation)),
                "lab command omits contextId",
                post,
            );
        }
        assert_eq!(request_method(&request), "POST");
        assert!(header(&request, "authorization").is_none(), "{post}");
        found = Some(data);
    }
    found.unwrap_or_else(|| {
        diverge(
            &format!("{} reader, case {operation}", reader_of(operation)),
            "a message:send envelope",
            "none",
        )
    })
}

fn message_sends(page: &Browser, base: &str) -> usize {
    page.requests(base)
        .iter()
        .filter(|request| request_url(request).contains("message:send"))
        .count()
}

fn chat_parity(page: &mut Browser, unavailable: &str) {
    send_chat(page, "hello parity");
    let first_context = text(
        page,
        "document.getElementById('chat')?.dataset.context ?? ''",
    );
    let first_tasks = task_ids(page);
    if first_context.is_empty() || first_tasks.len() != 1 {
        diverge(
            "Chat reader, case first turn",
            "one context and one protocol task",
            &format!("{first_context} {first_tasks:?} {}", chat_log(page)),
        );
    }
    send_chat(page, "again parity");
    let second_context = text(
        page,
        "document.getElementById('chat')?.dataset.context ?? ''",
    );
    let second_tasks = task_ids(page);
    if second_context != first_context
        || second_tasks.len() != 2
        || second_tasks[0] == second_tasks[1]
    {
        diverge(
            "Chat reader, case continued turn",
            &format!("reuse {first_context} with two task ids"),
            &format!("{second_context} {second_tasks:?}"),
        );
    }
    page.click("chat-new");
    page.wait_until(
        "document.getElementById('chat')?.dataset.context === '' && !(document.getElementById('chat-log')?.textContent || '').trim()",
    );
    send_chat(page, "unavailable");
    page.wait_until("document.getElementById('chat-alert')?.textContent.length > 0");
    let alert = text(page, "document.getElementById('chat-alert')?.textContent");
    if !agrees(unavailable, &alert) {
        diverge(
            "Chat reader, case unavailable agent-message",
            unavailable,
            &alert,
        );
    }
    assert_storage(
        page,
        &["hello parity", "again parity", "unavailable", ALICE],
    );
}

#[allow(clippy::too_many_lines)]
fn security_parity(page: &mut Browser, script: &Script) {
    let anonymous = send_command(
        page,
        &json!({
            "operation": "list_log_sources",
            "params": {"page": {"limit": 10}}
        }),
    );
    let anonymous_text = format!(
        "{} {}",
        anonymous["message"].as_str().unwrap_or(""),
        anonymous["status"].as_str().unwrap_or("")
    );
    if anonymous["kind"] != "unauthorized" || !agrees(&script.anonymous_auth, &anonymous_text) {
        diverge(
            "Agent reader, case anonymous 401",
            &script.anonymous_auth,
            &anonymous_text,
        );
    }
    assert!(
        page.requests(&script.secure)
            .iter()
            .rfind(|request| request_url(request).contains("message:send"))
            .is_some_and(|request| header(request, "authorization").is_none()),
        "anonymous protocol call sent a credential"
    );
    page.show("#overview");
    page.set_value("bearer-token", NOSCOPE);
    let forbidden = send_command(
        page,
        &json!({
            "operation": "list_log_sources",
            "params": {"page": {"limit": 10}}
        }),
    );
    let forbidden_text = format!(
        "{} {}",
        forbidden["message"].as_str().unwrap_or(""),
        forbidden["status"].as_str().unwrap_or("")
    );
    if forbidden["kind"] != "forbidden" || !agrees(&script.forbidden_auth, &forbidden_text) {
        diverge(
            "Agent reader, case noscope 403",
            &script.forbidden_auth,
            &forbidden_text,
        );
    }
    page.show("#overview");
    page.set_value("bearer-token", ALICE);
    send_chat(page, "alice-only");
    let alice_context = text(
        page,
        "document.getElementById('chat')?.dataset.context ?? ''",
    );
    page.show("#overview");
    page.set_value("bearer-token", BOB);
    page.show("#chat");
    page.click("chat-new");
    page.wait_until("document.getElementById('chat')?.dataset.context === ''");
    send_chat(page, "bob-only");
    let bob_context = text(
        page,
        "document.getElementById('chat')?.dataset.context ?? ''",
    );
    if bob_context.is_empty()
        || bob_context == alice_context
        || chat_log(page).contains("alice-only")
        || !chat_log(page).contains("echo:bob-only")
    {
        diverge(
            "Agent reader, case caller isolation",
            &format!("bob gets a new context besides {alice_context}"),
            &format!("{bob_context} {}", chat_log(page)),
        );
    }
    let token_header = page
        .requests(&script.secure)
        .iter()
        .rfind(|request| request_url(request).contains("message:send"))
        .and_then(|request| header(request, "authorization"))
        .unwrap_or("")
        .to_owned();
    if token_header != format!("Bearer {BOB}") {
        diverge(
            "Agent reader, case bearer header",
            &format!("Bearer {BOB}"),
            &token_header,
        );
    }
    page.show("#overview");
    assert_eq!(
        text(page, "document.getElementById('bearer-token')?.value ?? ''"),
        BOB
    );
    let stored = text(page, "JSON.stringify(localStorage)");
    if stored.contains("alice-only") || stored.contains("bob-only") {
        diverge(
            "shell reader, case local storage",
            "no saved transcripts",
            &stored,
        );
    }
    assert_storage(page, &[ALICE, BOB, NOSCOPE, "alice-only", "bob-only"]);
    access(page, "Forbidden");
}

fn send_chat(page: &mut Browser, message: &str) {
    page.show("#chat");
    page.set_value("chat-draft", message);
    page.click("chat-send");
    let needle = if message == "unavailable" {
        "document.getElementById('chat-alert')?.textContent.includes('unavailable')"
    } else {
        ""
    };
    if needle.is_empty() {
        page.wait_until(&format!(
            "document.getElementById('chat-log')?.textContent.includes('echo:{message}') && document.getElementById('chat')?.dataset.pending !== 'true'"
        ));
    } else {
        page.wait_until(needle);
    }
}

fn access(page: &mut Browser, status: &str) {
    for (width, height) in [(360, 800), (1280, 800)] {
        page.viewport(width, height);
        page.show("#overview");
        let overflow = page.eval("document.documentElement.scrollWidth <= window.innerWidth + 24");
        if overflow != Value::Bool(true) {
            diverge(
                "shell reader, case viewport",
                &format!("{width}x{height} stays within the viewport"),
                "page scrolls horizontally",
            );
        }
        assert_names(page);
        assert_status(page, status);
        if width < 768 {
            assert_reachable(page, "shell", &["sidebar-toggle", "Skip to content"]);
            assert_unreachable(page, "shell", &["nav-logs", "workbench-send", "chat-send"]);
            page.click("sidebar-toggle");
            page.wait_until(
                "document.getElementById('sidebar-toggle')?.getAttribute('aria-expanded') === 'true'",
            );
            assert_reachable(
                page,
                "shell",
                &[
                    "nav-agent",
                    "nav-logs",
                    "nav-metrics",
                    "nav-tasks",
                    "nav-images",
                    "nav-chat",
                ],
            );
            page.eval(
                "document.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))",
            );
            page.wait_until(
                "document.getElementById('sidebar-toggle')?.getAttribute('aria-expanded') === 'false'",
            );
        } else {
            assert_reachable(
                page,
                "shell",
                &[
                    "nav-agent",
                    "nav-logs",
                    "nav-metrics",
                    "nav-tasks",
                    "nav-images",
                    "nav-chat",
                ],
            );
            assert_unreachable(page, "shell", &["nav-toggle"]);
        }
        assert_reader_keys(page);
        page.show("#agent");
        page.eval("document.getElementById('protocol-advanced').open = false");
        assert_reachable(page, "Agent", &["protocol-advanced-summary"]);
        assert_unreachable(page, "Agent", &["send-text", "workbench-send"]);
        page.click("protocol-advanced-summary");
        page.wait_until("document.getElementById('protocol-advanced')?.open === true");
        assert_reachable(page, "Agent", &["send-text"]);
        page.show("#chat");
        assert_reachable(page, "Chat", &["chat-draft", "chat-send"]);
    }
}

fn assert_reader_keys(page: &mut Browser) {
    page.show("#logs");
    assert_reachable(
        page,
        "Logs",
        &[
            "log-source-all",
            "log-filter-search",
            "log-range-start",
            "log-range-end",
        ],
    );
    assert_unreachable(
        page,
        "Logs",
        &[
            "inventory-logs-more",
            "inventory-metrics-more",
            "workbench-send",
            "send-text",
        ],
    );
    page.show("#metrics");
    assert_reachable(
        page,
        "Metrics",
        &["metric-pick-0", "metric-range-start", "metric-range-end"],
    );
    assert_unreachable(
        page,
        "Metrics",
        &[
            "inventory-metrics-more",
            "inventory-logs-more",
            "workbench-send",
        ],
    );
    page.show("#tasks");
    assert_reachable(page, "Tasks", &["task-pick-0"]);
    assert_unreachable(
        page,
        "Tasks",
        &[
            "inventory-tasks-more",
            "inventory-tasks-limit",
            "workbench-send",
            "inventory-logs-more",
        ],
    );
}

fn assert_reachable(page: &mut Browser, reader: &str, required: &[&str]) {
    let stops = tab_stops(page);
    for required in required {
        if !stops.iter().any(|stop| stop == required) {
            diverge(
                &format!("{reader} reader, case keyboard"),
                &format!("{required} is reachable"),
                &stops.join(", "),
            );
        }
    }
}

fn assert_unreachable(page: &mut Browser, reader: &str, hidden: &[&str]) {
    let stops = tab_stops(page);
    for hidden in hidden {
        if stops.iter().any(|stop| stop == hidden) {
            diverge(
                &format!("{reader} reader, case keyboard"),
                &format!("{hidden} stays out of the hidden view"),
                &stops.join(", "),
            );
        }
    }
}

fn assert_names(page: &mut Browser) {
    let unnamed = page.eval(
        "(() => {
            const missing = [];
            for (const el of document.querySelectorAll('input, textarea, select, button, a')) {
                const label = el.id ? document.querySelector(`label[for=\"${CSS.escape(el.id)}\"]`)?.textContent.trim() : '';
                const name = label || el.getAttribute('aria-label') || el.textContent.trim();
                if (!name) missing.push(el.id || el.tagName);
            }
            return missing;
        })()",
    );
    if unnamed.as_array().is_some_and(|items| !items.is_empty()) {
        diverge(
            "shell reader, case accessible name",
            "every control has a name",
            &unnamed.to_string(),
        );
    }
}

fn assert_status(page: &mut Browser, workbench: &str) {
    let nav = page.eval("document.querySelector('nav')?.getAttribute('aria-label') || ''");
    if nav.as_str() != Some("Lab sections") {
        diverge(
            "shell reader, case status text",
            "Lab sections",
            &nav.to_string(),
        );
    }
    expect_live(page, "connection", "status", "polite");
    page.show("#workbench");
    assert_names(page);
    expect_live(page, "workbench-status", "status", "polite");
    expect_live(page, "workbench-alert", "alert", "assertive");
    expect_labelled(page, "workbench");
    let text = page.eval("document.getElementById('workbench-status')?.textContent.trim()");
    if text.as_str() != Some(workbench) {
        diverge(
            "shell reader, case workbench status",
            workbench,
            &text.to_string(),
        );
    }
    assert_only_view(page, "view-logs");
    page.show("#chat");
    assert_names(page);
    expect_live(page, "chat-status", "status", "polite");
    expect_live(page, "chat-alert", "alert", "assertive");
    expect_labelled(page, "chat");
    assert_only_view(page, "view-chat");
    for (hash, section, view) in [
        ("#logs", "inventory-logs", "view-logs"),
        ("#metrics", "inventory-metrics", "view-metrics"),
        ("#tasks", "inventory-tasks", "view-tasks"),
        ("#images", "inventory-images", "view-images"),
    ] {
        page.show(hash);
        assert_names(page);
        expect_live(page, &format!("{section}-status"), "status", "polite");
        if section == "inventory-logs" {
            expect_live(page, "inventory-logs-alert", "alert", "assertive");
        }
        if section == "inventory-tasks" {
            expect_live(page, "task-view-status", "status", "polite");
            expect_live(page, "task-view-alert", "alert", "assertive");
            expect_labelled(page, "task-view");
        }
        expect_labelled(page, section);
        assert_only_view(page, view);
        let others = [
            "inventory-logs",
            "inventory-metrics",
            "inventory-tasks",
            "inventory-images",
        ]
        .into_iter()
        .filter(|id| *id != section)
        .map(|id| {
            format!(
                "(document.getElementById('{id}') && !document.getElementById('{id}').closest('.view')?.hasAttribute('hidden'))"
            )
        })
        .collect::<Vec<_>>()
        .join(" || ");
        if page.eval(&format!("!!({others})")) == Value::Bool(true) {
            diverge(
                "shell reader, case inventory",
                &format!("{section} is the only inventory panel"),
                "another inventory panel is in the page",
            );
        }
    }
    page.show("#overview");
}

fn expect_live(page: &mut Browser, id: &str, role: &str, live: &str) {
    let value = page.eval(&format!(
        "(() => {{ const el = document.getElementById('{id}'); return [el?.getAttribute('role') || '', el?.getAttribute('aria-live') || '']; }})()"
    ));
    if value[0] != role || value[1] != live {
        diverge(id, &format!("{role}/{live}"), &value.to_string());
    }
}

fn expect_labelled(page: &mut Browser, id: &str) {
    let value = page.eval(&format!(
        "document.getElementById('{id}')?.getAttribute('aria-labelledby')"
    ));
    if value.as_str() != Some(format!("{id}-heading").as_str()) {
        diverge(id, &format!("{id}-heading"), &value.to_string());
    }
}

fn assert_only_view(page: &mut Browser, id: &str) {
    let count = page.eval("document.querySelectorAll('#main > .view:not([hidden])').length");
    let current = page.eval("document.querySelector('#main > .view:not([hidden])')?.id");
    if count != json!(1) || current.as_str() != Some(id) {
        diverge(id, "only this section is in the page", &current.to_string());
    }
}

fn tab_stops(page: &mut Browser) -> Vec<String> {
    page.eval("document.querySelector('a.skip')?.focus()");
    let mut stops = Vec::new();
    let mut seen = Vec::new();
    let mut previous = String::new();
    let mut stuck = 0;
    for _ in 0..400 {
        let value = page.eval(
            "(() => { const el = document.activeElement; if (!el || el === document.body) return ['', '']; const name = el.id || el.getAttribute('aria-label') || el.textContent.trim().replace(/\\s+/g, ' ').slice(0, 80); const path = []; let node = el; while (node && node.parentElement && path.length < 12) { path.push([...node.parentElement.children].indexOf(node)); node = node.parentElement; } return [name, path.join('.')]; })()",
        );
        page.press_tab();
        let name = value
            .get(0)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let key = value
            .get(1)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if name.is_empty() {
            continue;
        }
        if key == previous {
            stuck += 1;
            if stuck > 2 {
                break;
            }
            continue;
        }
        if seen.iter().any(|item| item == &key) {
            break;
        }
        stuck = 0;
        previous.clone_from(&key);
        seen.push(key);
        stops.push(name);
    }
    stops
}

fn assert_storage(page: &mut Browser, forbidden: &[&str]) {
    let stored = text(page, "JSON.stringify(localStorage)");
    for phrase in forbidden {
        if stored.contains(phrase) {
            diverge(
                "shell reader, case local storage",
                &format!("omits {phrase}"),
                &stored,
            );
        }
    }
    let keys = page.eval("Object.keys(localStorage)");
    let allowed = keys.as_array().is_some_and(|items| {
        items
            .iter()
            .all(|item| matches!(item.as_str(), Some("sidebar_state" | "theme")))
    });
    if !allowed {
        diverge(
            "shell reader, case local storage",
            "only sidebar_state and theme",
            &stored,
        );
    }
    let session = text(page, "JSON.stringify(sessionStorage)");
    let cookie = text(page, "document.cookie");
    if session != "{}" || !cookie.is_empty() {
        diverge(
            "shell reader, case storage",
            "sessionStorage and cookies stay empty",
            &format!("{session} {cookie}"),
        );
    }
}

fn assert_inert(page: &mut Browser, case: &str) {
    let inert =
        page.eval("document.getElementById('parity-xss') == null && window.__parityXss !== 1");
    if inert != Value::Bool(true) {
        diverge(case, "markup stays text", "markup executed");
    }
}

fn task_ids(page: &mut Browser) -> Vec<String> {
    match page.eval(
        "[...document.getElementById('chat-log')?.textContent.matchAll(/Task (\\S+)/g) ?? []].map(match => match[1])",
    ) {
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn chat_log(page: &mut Browser) -> String {
    text(
        page,
        "document.getElementById('chat-log')?.textContent ?? ''",
    )
}

fn text(page: &mut Browser, expression: &str) -> String {
    page.eval(expression).as_str().unwrap_or("").to_owned()
}

fn needles(result: &Value) -> Vec<String> {
    let mut found = Vec::new();
    walk(result, &mut found);
    found
}

fn walk(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key == "data" && child.is_string() {
                    continue;
                }
                found.push(key.clone());
                walk(child, found);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item, found);
            }
        }
        Value::String(text) if text.chars().count() <= 80 => found.push(text.clone()),
        Value::Number(number) => found.push(number.to_string()),
        Value::Bool(flag) => found.push(flag.to_string()),
        _ => {}
    }
}

fn agrees(client: &str, browser: &str) -> bool {
    if client.is_empty() || browser.is_empty() {
        return false;
    }
    if client.contains(browser) || browser.contains(client) {
        return true;
    }
    shares_phrase(client, browser) || shares_phrase(browser, client)
}

fn shares_phrase(source: &str, other: &str) -> bool {
    let chars: Vec<char> = source.chars().collect();
    chars.len() >= 12
        && chars.windows(12).any(|window| {
            let phrase: String = window.iter().collect();
            other.contains(&phrase)
        })
}

fn reader_of(operation: &str) -> &'static str {
    match operation {
        "list_tasks" | "start_task" | "get_task_status" => "Tasks",
        "list_metrics" | "query_metric" => "Metrics",
        "list_log_sources" | "query_logs" => "Logs",
        "list_image_sources" | "list_images" | "search_images" | "get_image"
        | "get_current_image" => "Images",
        _ => "browser",
    }
}

fn diverge(case: &str, expected: &str, observed: &str) -> ! {
    let expected = clip(expected);
    let observed = clip(observed);
    panic!("parity difference: {case}\nexpected: {expected}\nobserved: {observed}");
}

fn clip(text: &str) -> String {
    let end = text
        .char_indices()
        .nth(1_500)
        .map_or(text.len(), |(index, _)| index);
    text[..end].to_owned()
}
