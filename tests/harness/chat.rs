use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aLabError, A2aLabService, A2aServer, AgentMessageFuture, AgentMessageHandler,
    AgentMessageReply, AgentMessageRequest, HttpAuthSecurityScheme, MemoryLogs, MemoryMetrics,
    MemoryTasks, SecurityScheme, TaskDefinition, TaskId, bind_local,
};
use serde_json::Value;

use super::browser::{Browser, request_method, request_url};
use super::{COMMAND, MARKUP, TOKEN, lab};

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const CHAT_READY: &str = "document.getElementById('chat')?.dataset.enabled === 'true'";
const CHAT_OFF: &str = "document.getElementById('chat')?.dataset.enabled === 'false'";

#[tokio::test]
async fn browser_chat_continues_context_and_reloads_history() {
    let base = chat_lab(false, Arc::new(Echo::steady())).await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || chat_flow(&address))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_chat_is_disabled_without_agent_message() {
    let base = lab(false).await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || disabled_chat(&address))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_chat_reports_pending_errors_and_keeps_the_draft() {
    let base = chat_lab(false, Arc::new(Echo::slow_fail())).await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || chat_error_flow(&address))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_chat_hides_the_bearer_token_and_stale_auth() {
    let base = chat_lab(true, Arc::new(Echo::steady())).await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || bearer_chat(&address))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_chat_ignores_stored_session_documents() {
    let base = chat_lab(false, Arc::new(Echo::steady())).await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || ignore_stored_documents(&address))
        .await
        .unwrap();
}

fn chat_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#chat");
    page.wait_until(CHAT_READY);
    reject_blank(&mut page, base);
    let ids = send_two_turns(&mut page, base);
    reload_turns(&mut page, &ids);
    new_session_clears_the_page(&mut page, base);
}

fn reject_blank(page: &mut Browser, base: &str) {
    page.set_value("chat-draft", "   ");
    page.click("chat-send");
    page.wait_until(
        "document.getElementById('chat-alert')?.textContent.includes('Enter a message')",
    );
    assert!(send_bodies(page, base).is_empty());
}

fn send_two_turns(page: &mut Browser, base: &str) -> Vec<String> {
    page.set_value("chat-draft", "alpha secret phrase");
    page.click("chat-send");
    page.wait_until(
        "document.getElementById('chat-log')?.textContent.includes('echo alpha secret phrase')",
    );
    page.set_value("chat-draft", "beta secret phrase");
    page.focus("chat-draft");
    page.press_enter();
    page.wait_until(
        "document.getElementById('chat-log')?.textContent.includes('echo beta secret phrase')",
    );
    let bodies = send_bodies(page, base);
    assert!(bodies.len() >= 2, "{bodies:?}");
    let first = &bodies[0];
    let second = &bodies[1];
    assert!(!first.contains("contextId"), "{first}");
    assert!(second.contains("contextId"), "{second}");
    let context = js_string(
        page,
        "document.getElementById('chat')?.dataset.context ?? ''",
    );
    assert!(!context.is_empty());
    assert!(second.contains(&context), "{second}");
    let ids = task_lines(page);
    assert!(ids.len() >= 2, "{ids:?}");
    assert_ne!(ids[0], ids[1]);
    let origin = origin_of(page);
    assert_not_stored(
        page,
        &origin,
        &["alpha secret phrase", "beta secret phrase", TOKEN],
    );
    assert!(
        storage_get(page, &format!("a2a-lab.sessions.v1.{origin}")).is_empty(),
        "chat wrote a session index"
    );
    ids
}

fn reload_turns(page: &mut Browser, _ids: &[String]) {
    let origin = origin_of(page);
    page.reload();
    page.wait_until(CONNECTED);
    page.show("#chat");
    page.wait_until(CHAT_READY);
    assert_eq!(
        js_string(
            page,
            "document.getElementById('chat')?.dataset.context ?? ''"
        ),
        ""
    );
    assert_eq!(
        js_string(
            page,
            "document.getElementById('chat-log')?.textContent ?? ''"
        ),
        ""
    );
    assert_eq!(
        js_string(
            page,
            "document.getElementById('chat-session-0')?.textContent ?? ''"
        ),
        ""
    );
    assert_not_stored(
        page,
        &origin,
        &["alpha secret phrase", "beta secret phrase", TOKEN],
    );
}

fn new_session_clears_the_page(page: &mut Browser, base: &str) {
    page.show("#chat");
    page.set_value("chat-draft", "fresh phrase");
    page.click("chat-send");
    page.wait_until(
        "document.getElementById('chat-log')?.textContent.includes('echo fresh phrase')",
    );
    let first = js_string(
        page,
        "document.getElementById('chat')?.dataset.context ?? ''",
    );
    page.click("chat-new");
    page.wait_until("document.getElementById('chat-status')?.textContent.includes('New session')");
    assert_eq!(
        js_string(
            page,
            "document.getElementById('chat')?.dataset.context ?? ''"
        ),
        ""
    );
    assert_eq!(
        js_string(
            page,
            "document.getElementById('chat-log')?.textContent ?? ''"
        ),
        ""
    );
    page.set_value("chat-draft", "next phrase");
    page.click("chat-send");
    page.wait_until(
        "document.getElementById('chat-log')?.textContent.includes('echo next phrase')",
    );
    let second = js_string(
        page,
        "document.getElementById('chat')?.dataset.context ?? ''",
    );
    assert_ne!(first, second);
    page.set_value("chat-draft", "after phrase");
    page.click("chat-send");
    page.wait_until(
        "document.getElementById('chat-log')?.textContent.includes('echo after phrase')",
    );
    let bodies = send_bodies(page, base);
    let last = bodies.last().map_or("", String::as_str);
    assert!(!last.contains(&first), "{last}");
    assert!(last.contains(&second), "{last}");
}

fn disabled_chat(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#chat");
    page.wait_until(CHAT_OFF);
    let text = js_string(&mut page, "document.body.innerText");
    assert!(text.contains("does not advertise agent-message"), "{text}");
    assert!(text.contains("Lab commands remain available"), "{text}");
    assert_eq!(
        page.eval("document.getElementById('chat-draft')?.disabled === true"),
        Value::Bool(true)
    );
    page.click("chat-send");
    assert!(send_bodies(&page, base).is_empty());
    page.show("#agent");
    if page.eval("document.getElementById('protocol-advanced')?.open === true") != Value::Bool(true)
    {
        page.click("protocol-advanced-summary");
    }
    page.wait_until("document.getElementById('protocol-advanced')?.open === true");
    page.set_value("command-json", COMMAND);
    page.click("send-command");
    page.wait_until("document.getElementById('output')?.textContent.includes('list_tasks')");
}

fn chat_error_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#chat");
    page.wait_until(CHAT_READY);
    page.show("#chat");
    page.set_value("chat-draft", "keep this text");
    page.focus("chat-send");
    page.press_enter();
    page.wait_until("document.getElementById('chat')?.dataset.pending === 'true'");
    page.click("chat-send");
    page.wait_until("document.getElementById('chat-status')?.dataset.state === 'error'");
    assert_eq!(draft_value(&mut page), "keep this text");
    assert_eq!(
        js_string(&mut page, "document.activeElement?.id ?? ''"),
        "chat-draft"
    );
    assert_eq!(
        js_string(
            &mut page,
            "document.getElementById('chat-alert')?.getAttribute('role') ?? ''"
        ),
        "alert"
    );
    assert_eq!(
        js_string(
            &mut page,
            "document.getElementById('chat-status')?.getAttribute('aria-live') ?? ''"
        ),
        "polite"
    );
    assert_eq!(send_bodies(&page, base).len(), 1);
    page.click("chat-retry");
    page.wait_until(
        "document.getElementById('chat-log')?.textContent.includes('echo keep this text')",
    );
    assert_eq!(draft_value(&mut page), "");
    assert_eq!(send_bodies(&page, base).len(), 2);
    let origin = origin_of(&mut page);
    assert!(storage_get(&mut page, &format!("a2a-lab.sessions.v1.{origin}")).is_empty());
    assert_not_stored(&mut page, &origin, &["keep this text", TOKEN]);
}

fn bearer_chat(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#chat");
    page.wait_until(CHAT_READY);
    page.show("#overview");
    page.set_value("bearer-token", TOKEN);
    page.show("#chat");
    page.set_value("chat-draft", "hidden phrase");
    page.click("chat-send");
    page.wait_until(
        "document.getElementById('chat-log')?.textContent.includes('echo hidden phrase')",
    );
    let origin = origin_of(&mut page);
    assert!(storage_get(&mut page, &format!("a2a-lab.sessions.v1.{origin}")).is_empty());
    assert_not_stored(&mut page, &origin, &["hidden phrase", TOKEN]);
    page.reload();
    page.wait_until(CONNECTED);
    page.show("#agent");
    assert_eq!(
        js_string(
            &mut page,
            "document.getElementById('bearer-token')?.value ?? 'missing'"
        ),
        ""
    );
    page.show("#chat");
    page.wait_until(CHAT_READY);
    assert_eq!(
        js_string(
            &mut page,
            "document.getElementById('chat-log')?.textContent ?? ''"
        ),
        ""
    );
    page.show("#agent");
    page.set_value("bearer-token", TOKEN);
    page.show("#chat");
    page.set_value("chat-draft", "hidden phrase");
    page.click("chat-send");
    page.wait_until(
        "document.getElementById('chat-log')?.textContent.includes('echo hidden phrase')",
    );
    assert_not_stored(&mut page, &origin, &["hidden phrase", TOKEN]);
}

fn ignore_stored_documents(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#chat");
    page.wait_until(CHAT_READY);
    let origin = origin_of(&mut page);
    let key = format!("a2a-lab.sessions.v1.{origin}");
    let seeded = "not-json do-not-store-transcript";
    storage_set(&mut page, &key, seeded);
    page.reload();
    page.wait_until(CONNECTED);
    page.show("#chat");
    page.wait_until(CHAT_READY);
    assert_eq!(storage_get(&mut page, &key), seeded);
    assert_eq!(
        js_string(
            &mut page,
            "document.getElementById('chat-log')?.textContent ?? ''"
        ),
        ""
    );
    assert_eq!(
        js_string(
            &mut page,
            "document.getElementById('chat-session-0')?.textContent ?? ''"
        ),
        ""
    );
    page.set_value("chat-draft", "still local");
    page.click("chat-send");
    page.wait_until(
        "document.getElementById('chat-log')?.textContent.includes('echo still local')",
    );
    assert_eq!(storage_get(&mut page, &key), seeded);
}

fn send_bodies(page: &Browser, base: &str) -> Vec<String> {
    page.requests(base)
        .into_iter()
        .filter(|request| {
            request_url(request).contains("/message:send") && request_method(request) == "POST"
        })
        .map(|request| {
            request["params"]["request"]["postData"]
                .as_str()
                .unwrap_or("")
                .to_owned()
        })
        .filter(|body| !body.is_empty())
        .collect()
}

fn task_lines(page: &mut Browser) -> Vec<String> {
    let value = page.eval(
        "[...document.querySelectorAll('#chat-log [data-task]')].map((el) => el.getAttribute('data-task') || '')",
    );
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .filter(|item| !item.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn assert_not_stored(page: &mut Browser, origin: &str, phrases: &[&str]) {
    let stored = storage_get(page, &format!("a2a-lab.sessions.v1.{origin}"));
    let session = js_string(page, "JSON.stringify([...Object.entries(sessionStorage)])");
    let cookie = js_string(page, "document.cookie");
    let href = js_string(page, "location.href");
    let names = js_string(
        page,
        "(async () => { if (!indexedDB.databases) return ''; const rows = await indexedDB.databases(); return rows.map((row) => row.name || '').join(','); })()",
    );
    for phrase in phrases {
        assert!(
            !stored.contains(phrase),
            "{phrase} in localStorage: {stored}"
        );
        assert!(!session.contains(phrase), "{phrase} in sessionStorage");
        assert!(!cookie.contains(phrase), "{phrase} in cookie");
        assert!(!href.contains(phrase), "{phrase} in url");
    }
    assert!(!names.contains("a2a"), "{names}");
    assert!(!stored.contains("\"text\""));
    assert!(!stored.contains("\"token\""));
    assert!(!stored.contains("\"authorization\""));
    assert!(!stored.contains("\"password\""));
}

fn draft_value(page: &mut Browser) -> String {
    js_string(page, "document.getElementById('chat-draft')?.value ?? ''")
}

fn origin_of(page: &mut Browser) -> String {
    js_string(page, "location.origin")
}

fn storage_get(page: &mut Browser, key: &str) -> String {
    let key = serde_json::to_string(key).unwrap_or_default();
    js_string(page, &format!("localStorage.getItem({key}) ?? ''"))
}

fn storage_set(page: &mut Browser, key: &str, value: &str) {
    let key = serde_json::to_string(key).unwrap_or_default();
    let value = serde_json::to_string(value).unwrap_or_default();
    page.eval(&format!("localStorage.setItem({key}, {value}), true"));
}

fn js_string(page: &mut Browser, expression: &str) -> String {
    page.eval(expression)
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

struct Echo {
    delay_ms: u64,
    fail_first: AtomicBool,
}

impl Echo {
    fn steady() -> Self {
        Self {
            delay_ms: 0,
            fail_first: AtomicBool::new(false),
        }
    }

    fn slow_fail() -> Self {
        Self {
            delay_ms: 800,
            fail_first: AtomicBool::new(true),
        }
    }
}

impl AgentMessageHandler for Echo {
    fn handle(
        &self,
        request: AgentMessageRequest,
    ) -> AgentMessageFuture<'_, Result<AgentMessageReply, A2aLabError>> {
        let text = request.text.clone();
        let delay_ms = self.delay_ms;
        let fail = self.fail_first.swap(false, Ordering::Relaxed);
        Box::pin(async move {
            if fail && delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            if fail {
                Err(A2aLabError::unavailable("model offline"))
            } else {
                Ok(AgentMessageReply {
                    text: format!("echo {text}"),
                })
            }
        })
    }
}

async fn chat_lab(secured: bool, echo: Arc<Echo>) -> String {
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("build").unwrap(),
            name: "Build".to_owned(),
            description: MARKUP.to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: None,
            output_schema: None,
        })
        .await;
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), tasks).share();
    let (listener, address) = bind_local().await.unwrap();
    let mut server = A2aServer::new(&service)
        .with_message_handler(echo)
        .with_web_ui();
    if secured {
        server = server.with_security(bearer_schemes(), bearer_requirements());
    }
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

fn bearer_schemes() -> std::collections::HashMap<String, SecurityScheme> {
    [(
        "bearerAuth".to_owned(),
        SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
            scheme: "bearer".to_owned(),
            description: None,
            bearer_format: None,
        }),
    )]
    .into()
}

fn bearer_requirements() -> Vec<std::collections::HashMap<String, Vec<String>>> {
    vec![[("bearerAuth".to_owned(), Vec::new())].into()]
}
