//! Product chrome: rail, sheet, theme, and focus.

#![cfg(feature = "web-ui")]

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, MemoryLogs, MemoryMetrics, MemoryTasks, TaskDefinition, TaskId,
    bind_local,
};
use serde_json::{Value, json};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::Browser;

#[tokio::test]
async fn browser_chrome_shell_matches_the_product_rail() {
    let base = lab().await;
    let address = base.clone();
    tokio::task::spawn_blocking(move || chrome_flow(&address))
        .await
        .unwrap();
}

async fn lab() -> String {
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("build").unwrap(),
            name: "Build".to_owned(),
            description: "Build".to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: None,
            output_schema: None,
        })
        .await;
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), tasks).share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service).with_web_ui();
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

fn chrome_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until("document.getElementById('connection')?.dataset.state === 'connected'");
    page.viewport(768, 800);
    assert_desktop_rail(&mut page);
    assert_sidebar_persistence(&mut page);
    page.viewport(767, 800);
    assert_mobile_sheet(&mut page);
    page.viewport(768, 800);
    assert_theme(&mut page);
    assert_chrome_focus(&mut page);
    assert_chrome_storage(&mut page);
    page.show("#workbench");
    assert_eq!(
        page.eval("document.getElementById('nav-logs')?.getAttribute('aria-current')"),
        json!("page")
    );
    assert_eq!(
        page.eval("document.getElementById('view-logs')?.hasAttribute('hidden')"),
        Value::Bool(false)
    );
    assert_connection_states(&mut page);
}

fn assert_desktop_rail(page: &mut Browser) {
    let rail = chrome_metrics(page);
    assert_desktop_frame(&rail);
    assert_desktop_chrome(&rail);
}

fn assert_desktop_frame(rail: &Value) {
    assert!(rail["align"].as_bool().unwrap_or(false), "{rail}");
    assert_eq!(rail["header"], json!(56));
    assert_eq!(rail["brand"], json!(56));
    assert_eq!(rail["gap"], json!(256));
    assert_eq!(rail["nav"], json!(256));
    assert_eq!(rail["position"], "absolute");
    assert_eq!(rail["hidden"], false);
    assert_eq!(rail["full"], true);
    assert_eq!(rail["layout"], "desktop");
}

fn assert_desktop_chrome(rail: &Value) {
    assert_eq!(rail["dots"], true);
    assert_eq!(rail["glyph"], json!(20));
    assert_eq!(rail["glyphH"], json!(20));
    assert_eq!(rail["toggle"], true);
    assert_eq!(rail["oxanium"], true);
    assert_eq!(rail["stripe"], "none");
    assert_eq!(rail["extra"], false);
    assert_eq!(rail["headings"], json!(1));
    assert_eq!(rail["title"], "Agent");
    assert_eq!(rail["live"], "polite");
    assert_eq!(rail["role"], "status");
    assert_eq!(rail["quiet"], true);
}

fn assert_sidebar_persistence(page: &mut Browser) {
    page.click("sidebar-toggle");
    page.wait_until(
        "document.getElementById('shell')?.dataset.state === 'collapsed' && document.querySelector('.rail-gap').getBoundingClientRect().width < 1",
    );
    assert_eq!(
        page.eval("localStorage.getItem('sidebar_state')"),
        json!("false")
    );
    let collapsed = chrome_metrics(page);
    assert_eq!(collapsed["gap"], json!(0));
    assert_eq!(collapsed["full"], true);
    assert_eq!(collapsed["nav"], json!(256));
    assert!(collapsed["toggleWidth"].as_f64().unwrap_or(0.0) > 0.0);
    page.reload();
    page.wait_until(
        "document.getElementById('connection')?.dataset.state === 'connected' && document.getElementById('shell')?.dataset.state === 'collapsed' && document.querySelector('.rail-gap').getBoundingClientRect().width < 1",
    );
    assert_eq!(chrome_metrics(page)["gap"], json!(0));
    page.click("sidebar-toggle");
    page.wait_until(
        "localStorage.getItem('sidebar_state') === 'true' && document.querySelector('.rail-gap').getBoundingClientRect().width > 250",
    );
    page.reload();
    page.wait_until(
        "document.getElementById('connection')?.dataset.state === 'connected' && document.getElementById('shell')?.dataset.state === 'expanded' && document.querySelector('.rail-gap').getBoundingClientRect().width > 250",
    );
    assert_eq!(chrome_metrics(page)["gap"], json!(256));
}

fn assert_mobile_sheet(page: &mut Browser) {
    let stored = page.eval("localStorage.getItem('sidebar_state')");
    assert_eq!(
        page.eval("document.getElementById('primary-nav')?.hasAttribute('hidden')"),
        Value::Bool(true)
    );
    assert_eq!(
        page.eval("document.documentElement.dataset.layout"),
        json!("mobile")
    );
    page.click("sidebar-toggle");
    page.wait_until(
        "document.getElementById('sidebar-toggle')?.getAttribute('aria-expanded') === 'true'",
    );
    let open = chrome_metrics(page);
    assert_eq!(open["hidden"], false);
    assert_eq!(open["position"], "fixed");
    assert_eq!(open["nav"], json!(256));
    assert_eq!(open["navTop"], json!(0));
    assert_eq!(open["navLeft"], json!(0));
    assert!(
        page.eval("window.innerWidth").as_f64().unwrap_or(0.0)
            > open["nav"].as_f64().unwrap_or(767.0)
    );
    page.eval(
        "document.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))",
    );
    page.wait_until("document.getElementById('primary-nav')?.hasAttribute('hidden')");
    page.click("sidebar-toggle");
    page.click("nav-logs");
    page.wait_until(
        "document.getElementById('view-logs')?.parentElement?.id === 'main' && document.getElementById('sidebar-toggle')?.getAttribute('aria-expanded') === 'false'",
    );
    assert_eq!(page.eval("localStorage.getItem('sidebar_state')"), stored);
}

fn assert_theme(page: &mut Browser) {
    let light = tone_is("light");
    let dark = tone_is("dark");
    page.eval("localStorage.setItem('theme','light')");
    page.reload();
    page.wait_until(
        "document.documentElement.dataset.theme === 'light' && document.getElementById('connection')?.dataset.state === 'connected'",
    );
    page.color_scheme("dark");
    page.wait_until(&format!(
        "document.documentElement.dataset.theme === 'light' && !document.documentElement.classList.contains('dark') && {light}"
    ));
    page.click("theme-toggle");
    page.wait_until(
        "document.documentElement.dataset.theme === 'system' && localStorage.getItem('theme') === 'system'",
    );
    page.wait_until(&format!(
        "document.documentElement.classList.contains('dark') && {dark}"
    ));
    page.color_scheme("light");
    page.wait_until(&format!(
        "document.documentElement.dataset.theme === 'system' && !document.documentElement.classList.contains('dark') && {light}"
    ));
    page.click("theme-toggle");
    page.wait_until(&format!(
        "document.documentElement.dataset.theme === 'dark' && localStorage.getItem('theme') === 'dark' && document.documentElement.classList.contains('dark') && {dark}"
    ));
    page.color_scheme("light");
    page.wait_until(&format!(
        "document.documentElement.dataset.theme === 'dark' && {dark}"
    ));
    page.click("theme-toggle");
    page.wait_until(&format!(
        "document.documentElement.dataset.theme === 'light' && localStorage.getItem('theme') === 'light' && {light}"
    ));
}

fn tone_is(kind: &str) -> String {
    let check = if kind == "light" { "> 200" } else { "< 80" };
    format!(
        "(() => {{ const paint = (node) => {{ const ctx = document.createElement('canvas').getContext('2d'); ctx.fillStyle = getComputedStyle(node).backgroundColor; ctx.fillRect(0, 0, 1, 1); return ctx.getImageData(0, 0, 1, 1).data; }}; let data = paint(document.body); if (data[3] === 0) data = paint(document.documentElement); return data[0] {check}; }})()"
    )
}

fn assert_chrome_focus(page: &mut Browser) {
    let hash = page.eval("location.hash");
    page.click_selector("a.skip");
    assert_eq!(page.eval("document.activeElement?.id"), json!("main"));
    assert_eq!(
        page.eval("document.getElementById('main')?.tabIndex"),
        json!(-1)
    );
    assert_eq!(page.eval("location.hash"), hash);
    page.show("#agent");
    page.show("#logs");
    assert_eq!(page.eval("document.activeElement?.id"), json!("view-title"));
    assert_eq!(
        page.eval("document.getElementById('view-title')?.textContent"),
        json!("Logs")
    );
    page.click_box("sidebar-toggle");
    assert_eq!(
        page.eval("document.activeElement?.id"),
        json!("sidebar-toggle")
    );
    let mouse = page.eval(
        "(() => { const style = getComputedStyle(document.getElementById('sidebar-toggle')); return style.outlineStyle; })()",
    );
    assert_eq!(mouse, json!("none"));
    page.press_tab();
    let keyboard = page.eval(
        "(() => { const style = getComputedStyle(document.activeElement); return style.outlineStyle + ' ' + style.outlineWidth; })()",
    );
    assert!(
        keyboard.as_str().unwrap_or("").contains("3px"),
        "{keyboard}"
    );
}

fn assert_chrome_storage(page: &mut Browser) {
    let keys = page.eval("Object.keys(localStorage)");
    let allowed = keys.as_array().is_some_and(|items| {
        items
            .iter()
            .all(|item| matches!(item.as_str(), Some("sidebar_state" | "theme")))
    });
    assert!(allowed, "{keys}");
    assert_eq!(page.eval("JSON.stringify(sessionStorage)"), json!("{}"));
    assert_eq!(page.eval("document.cookie"), json!(""));
    let stored = page.eval("JSON.stringify(localStorage)").to_string();
    assert!(!stored.contains("keep-draft"));
    assert!(!stored.contains("bearer"));
}

fn assert_connection_states(page: &mut Browser) {
    page.show("#agent");
    page.fail_network(true);
    page.click("retry");
    page.wait_until("document.getElementById('connection')?.dataset.state === 'offline'");
    for route in ["#logs", "#metrics", "#chat", "#tasks"] {
        page.show(route);
        let visible = page.eval(
            "(() => { const el = document.getElementById('connection'); const box = el.getBoundingClientRect(); return el.getAttribute('aria-live') === 'polite' && el.getAttribute('role') === 'status' && getComputedStyle(el).display !== 'none' && box.height > 8; })()",
        );
        assert_eq!(visible, Value::Bool(true), "{route}");
    }
}

fn chrome_metrics(page: &mut Browser) -> Value {
    page.eval(
        "(() => {
            const header = document.querySelector('header.banner').getBoundingClientRect();
            const brand = document.getElementById('brand').getBoundingClientRect();
            const gap = document.querySelector('.rail-gap').getBoundingClientRect();
            const nav = document.getElementById('primary-nav');
            const navBox = nav.getBoundingClientRect();
            const column = document.querySelector('.shell-main').getBoundingClientRect();
            const shell = document.getElementById('shell').getBoundingClientRect();
            const glyph = document.querySelector('#nav-agent .nav-glyph')?.getBoundingClientRect();
            const connection = document.getElementById('connection');
            const connectionBox = connection.getBoundingClientRect();
            const toggle = document.getElementById('sidebar-toggle').getBoundingClientRect();
            return {
                header: Math.round(header.height),
                brand: Math.round(brand.height),
                align: Math.abs(header.bottom - brand.bottom) < 1.5,
                gap: Math.round(gap.width),
                nav: Math.round(navBox.width),
                navTop: Math.round(navBox.top),
                navLeft: Math.round(navBox.left),
                position: getComputedStyle(nav).position,
                dots: getComputedStyle(nav).backgroundImage.includes('radial-gradient'),
                hidden: nav.hasAttribute('hidden'),
                full: Math.abs(gap.width + column.width - shell.width) < 2,
                glyph: glyph ? Math.round(glyph.width) : 0,
                glyphH: glyph ? Math.round(glyph.height) : 0,
                toggle: document.getElementById('nav-toggle') == null && !document.getElementById('sidebar-toggle').hasAttribute('hidden'),
                toggleWidth: toggle.width,
                oxanium: getComputedStyle(document.getElementById('brand-name')).fontFamily.toLowerCase().includes('oxanium'),
                stripe: getComputedStyle(document.body).backgroundImage,
                extra: !!document.querySelector('footer, [aria-label=\"Breadcrumb\"]') || document.body.innerText.includes('Log out'),
                headings: document.querySelectorAll('header h1, header h2, header h3').length,
                title: document.getElementById('view-title').textContent,
                live: connection.getAttribute('aria-live'),
                role: connection.getAttribute('role'),
                quiet: connection.dataset.state === 'connected' && connectionBox.width <= 1 && getComputedStyle(connection).display !== 'none' && connection.getAttribute('aria-hidden') !== 'true',
                layout: document.documentElement.dataset.layout
            };
        })()",
    )
}
