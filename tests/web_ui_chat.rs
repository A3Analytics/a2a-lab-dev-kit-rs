//! Chat transcript rows, pending lock, and retry.

#![cfg(feature = "web-ui")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aLabError, A2aLabService, A2aServer, AgentMessageFuture, AgentMessageHandler,
    AgentMessageReply, AgentMessageRequest, MemoryLogs, MemoryMetrics, MemoryTasks, bind_local,
};
use serde_json::Value;

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::{Browser, request_method, request_url};

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const CHAT_READY: &str = "document.getElementById('chat')?.dataset.enabled === 'true'";
const CHAT_OFF: &str = "document.getElementById('chat')?.dataset.enabled === 'false'";

#[tokio::test]
async fn browser_chat_rows_lock_retry_and_keys() {
    let base = chat_lab(Arc::new(Echo::slow_fail())).await;
    tokio::task::spawn_blocking(move || chat_flow(&base))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_chat_stays_disabled_without_agent_message() {
    let base = plain_lab().await;
    tokio::task::spawn_blocking(move || disabled_flow(&base))
        .await
        .unwrap();
}

fn chat_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#agent");
    page.show("#chat");
    page.wait_until(CHAT_READY);
    assert_route_focus(&mut page);
    assert_shift_enter_inserts_a_newline(&mut page, base);
    lock_pending_turn(&mut page, base);
    assert_retry_rows_and_scroll(&mut page, base);
    assert_new_conversation_then_enter(&mut page, base);
}

fn disabled_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#chat");
    page.wait_until(CHAT_OFF);
    let status = text(
        &mut page,
        "document.getElementById('chat-status')?.textContent ?? ''",
    );
    assert!(
        status.contains("does not advertise agent-message"),
        "{status}"
    );
    assert!(status.contains("Lab commands remain available"), "{status}");
    assert!(
        flag(
            &mut page,
            "document.getElementById('chat-draft')?.disabled === true && document.getElementById('chat-send')?.disabled === true && document.getElementById('chat-new')?.disabled === true"
        ),
        "disabled chat left a control enabled"
    );
    page.click("chat-send");
    page.click("chat-new");
    page.eval(
        "document.getElementById('chat-draft')?.dispatchEvent(new KeyboardEvent('keydown', {key: 'Enter', bubbles: true, cancelable: true}))",
    );
    assert!(send_count(&page, base) == 0, "disabled chat submitted");
    assert_eq!(
        text(
            &mut page,
            "document.getElementById('chat')?.dataset.enabled ?? ''"
        ),
        "false"
    );
    assert_eq!(
        text(
            &mut page,
            "document.getElementById('chat')?.dataset.pending ?? ''"
        ),
        "false"
    );
    assert!(
        text(
            &mut page,
            "document.getElementById('chat-status')?.textContent ?? ''"
        )
        .contains("does not advertise agent-message")
    );
}

fn assert_route_focus(page: &mut Browser) {
    let focus = page.eval(
        "(() => { const el = document.activeElement; const box = el.getBoundingClientRect(); const heading = document.getElementById('chat-heading'); const headingBox = heading.getBoundingClientRect(); const title = document.getElementById('view-title'); const titleBox = title.getBoundingClientRect(); return { id: el.id || '', height: box.height, title: title.textContent || '', titleHeight: titleBox.height, headingClipped: headingBox.height <= 1 && headingBox.width <= 1, onHeading: el.id === 'chat-heading' }; })()",
    );
    assert_eq!(focus["title"], "Chat", "{focus}");
    assert!(
        focus["titleHeight"].as_f64().unwrap_or(0.0) > 8.0,
        "{focus}"
    );
    assert_eq!(focus["onHeading"], Value::Bool(false), "{focus}");
    assert_ne!(focus["id"], "chat-heading", "{focus}");
    assert!(focus["height"].as_f64().unwrap_or(0.0) > 8.0, "{focus}");
    assert_eq!(focus["headingClipped"], Value::Bool(true), "{focus}");
}

fn assert_shift_enter_inserts_a_newline(page: &mut Browser, base: &str) {
    page.set_value("chat-draft", "line");
    caret_end(page);
    page.press_shift_enter();
    let draft = text(page, "document.getElementById('chat-draft')?.value ?? ''");
    assert!(
        draft.contains('\n'),
        "Shift+Enter did not insert a newline: {draft:?}"
    );
    assert!(draft.starts_with("line"), "{draft:?}");
    assert_eq!(send_count(page, base), 0);
}

fn lock_pending_turn(page: &mut Browser, base: &str) {
    page.set_value("chat-draft", "keep this text");
    page.click("chat-send");
    page.wait_until("document.getElementById('chat')?.dataset.pending === 'true'");
    assert!(
        flag(
            page,
            "document.getElementById('chat-draft')?.disabled === true && document.getElementById('chat-send')?.disabled === true && document.getElementById('chat-new')?.disabled === true"
        ),
        "pending chat left draft or send enabled"
    );
    page.click("chat-send");
    page.click("chat-new");
    page.eval(
        "document.getElementById('chat-draft')?.dispatchEvent(new KeyboardEvent('keydown', {key: 'Enter', bubbles: true, cancelable: true}))",
    );
    assert_eq!(send_count(page, base), 1);
    assert_eq!(
        text(page, "document.getElementById('chat-draft')?.value ?? ''"),
        "keep this text"
    );
}

fn assert_retry_rows_and_scroll(page: &mut Browser, base: &str) {
    resend_failed_draft(page, base);
    assert_turn_rows(page);
    assert_transcript_scroll(page);
}

fn resend_failed_draft(page: &mut Browser, base: &str) {
    page.wait_until("document.getElementById('chat-status')?.dataset.state === 'error'");
    assert_eq!(
        text(page, "document.getElementById('chat-draft')?.value ?? ''"),
        "keep this text"
    );
    assert!(retry_visible(page), "retry was hidden");
    page.click("chat-retry");
    page.wait_until(
        "document.querySelector('#chat-log [data-role=\"assistant\"]')?.textContent.includes('echo keep this text')",
    );
    assert_eq!(send_count(page, base), 2);
    assert_eq!(
        text(page, "document.getElementById('chat-draft')?.value ?? ''"),
        ""
    );
}

fn retry_visible(page: &mut Browser) -> bool {
    flag(
        page,
        "(() => { const el = document.getElementById('chat-retry'); if (!el || el.disabled) return false; const box = el.getBoundingClientRect(); return getComputedStyle(el).display !== 'none' && box.height > 8 && box.width > 8; })()",
    )
}

fn assert_turn_rows(page: &mut Browser) {
    let rows = row_report(page);
    assert_eq!(rows["separate"], Value::Bool(true), "{rows}");
    assert_eq!(rows["pre"], Value::Bool(false), "{rows}");
    let user = rows["user"].as_str().unwrap_or("");
    let agent = rows["agent"].as_str().unwrap_or("");
    assert!(user.contains("keep this text"), "{rows}");
    assert!(agent.contains("echo keep this text"), "{rows}");
    assert!(agent.contains("Task "), "{rows}");
    assert!(!user.contains("echo keep this text"), "{rows}");
}

fn assert_transcript_scroll(page: &mut Browser) {
    let rows = row_report(page);
    assert_eq!(rows["border"], Value::Bool(true), "{rows}");
    assert_eq!(rows["overflow"], "auto", "{rows}");
    assert_eq!(rows["outside"], Value::Bool(true), "{rows}");
    assert_eq!(rows["below"], Value::Bool(true), "{rows}");
    assert_eq!(rows["scrolled"], Value::Bool(true), "{rows}");
    assert_eq!(rows["stable"], Value::Bool(true), "{rows}");
}

fn row_report(page: &mut Browser) -> Value {
    page.eval(
        "(() => { const log = document.getElementById('chat-log'); const compose = document.querySelector('.chat-compose'); const user = log.querySelector('[data-role=\"user\"]'); const agent = log.querySelector('[data-role=\"assistant\"]'); const style = getComputedStyle(log); const logBox = log.getBoundingClientRect(); const composeBox = compose.getBoundingClientRect(); log.style.maxHeight = '1px'; const before = compose.getBoundingClientRect().top; log.scrollTop = log.scrollHeight; const after = compose.getBoundingClientRect().top; const scrolled = log.scrollTop > 0; log.style.maxHeight = ''; return { border: parseFloat(style.borderTopWidth) >= 1, overflow: style.overflowY, outside: !log.contains(compose), below: composeBox.top >= logBox.bottom - 2, scrolled, stable: Math.abs(after - before) < 1.5, user: user?.textContent || '', agent: agent?.textContent || '', separate: !!(user && agent && user !== agent), pre: log.tagName === 'PRE' }; })()",
    )
}

fn assert_new_conversation_then_enter(page: &mut Browser, base: &str) {
    let first = text(
        page,
        "document.getElementById('chat')?.dataset.context ?? ''",
    );
    assert!(!first.is_empty(), "missing context");
    page.click("chat-new");
    page.wait_until("document.getElementById('chat-status')?.textContent.includes('New session')");
    assert_eq!(
        text(
            page,
            "document.getElementById('chat')?.dataset.context ?? ''"
        ),
        ""
    );
    assert_eq!(
        text(
            page,
            "document.getElementById('chat-log')?.textContent ?? ''"
        ),
        ""
    );
    assert_eq!(
        text(page, "document.getElementById('chat-draft')?.value ?? ''"),
        ""
    );
    page.set_value("chat-draft", "enter phrase");
    page.focus("chat-draft");
    page.press_enter();
    page.wait_until(
        "document.querySelector('#chat-log [data-role=\"assistant\"]')?.textContent.includes('echo enter phrase')",
    );
    let second = text(
        page,
        "document.getElementById('chat')?.dataset.context ?? ''",
    );
    assert_ne!(first, second);
    assert!(
        !text(
            page,
            "document.getElementById('chat-log')?.textContent ?? ''"
        )
        .contains("keep this text")
    );
    assert!(send_count(page, base) >= 3);
}

fn caret_end(page: &mut Browser) {
    page.eval(
        "(() => { const el = document.getElementById('chat-draft'); el.focus(); const n = el.value.length; el.setSelectionRange(n, n); return true; })()",
    );
}

fn send_count(page: &Browser, base: &str) -> usize {
    page.requests(base)
        .into_iter()
        .filter(|request| {
            request_url(request).contains("/message:send") && request_method(request) == "POST"
        })
        .count()
}

fn text(page: &mut Browser, expression: &str) -> String {
    page.eval(expression)
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn flag(page: &mut Browser, expression: &str) -> bool {
    page.eval(expression) == Value::Bool(true)
}

struct Echo {
    delay_ms: u64,
    fail_first: AtomicBool,
}

impl Echo {
    fn slow_fail() -> Self {
        Self {
            delay_ms: 1200,
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

async fn chat_lab(echo: Arc<Echo>) -> String {
    let service =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service)
        .with_message_handler(echo)
        .with_web_ui();
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

async fn plain_lab() -> String {
    let service =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service).with_web_ui();
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}
