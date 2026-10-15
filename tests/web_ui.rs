#![cfg(feature = "web-ui")]

use std::collections::HashMap;
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, HttpAuthSecurityScheme, MemoryLogs, MemoryMetrics, MemoryTasks,
    SecurityScheme, TaskDefinition, TaskId, WEB_UI_PATH, bind_local,
};
use a2a_types::SecurityRequirement;

const AGENT_MARKER: &str = "web-ui-agent-secret";
const BEARER_SECRET: &str = "web-ui-bearer-secret";

async fn serve(web_ui: bool, secured: bool) -> String {
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("build").unwrap(),
            name: "Build".to_owned(),
            description: AGENT_MARKER.to_owned(),
            asset_id: None,
            semantic_id: None,
            input_schema: None,
            output_schema: None,
        })
        .await;
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), tasks).share();
    let (listener, address) = bind_local().await.unwrap();
    let mut server = A2aServer::new(&service);
    if web_ui {
        server = server.with_web_ui();
    }
    if secured {
        let schemes: HashMap<String, SecurityScheme> = [(
            "bearerAuth".to_owned(),
            SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                scheme: "bearer".to_owned(),
                description: None,
                bearer_format: None,
            }),
        )]
        .into();
        let requirements: Vec<SecurityRequirement> =
            vec![[("bearerAuth".to_owned(), Vec::new())].into()];
        server = server.with_security(schemes, requirements);
    }
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

async fn get(url: &str, token: Option<&str>) -> reqwest::Response {
    let mut request = http().get(url);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().await.unwrap()
}

fn header<'a>(response: &'a reqwest::Response, name: &str) -> &'a str {
    response
        .headers()
        .get(name)
        .unwrap_or_else(|| panic!("missing {name}"))
        .to_str()
        .unwrap()
}

fn has_extension(value: &str, extension: &str) -> bool {
    std::path::Path::new(value)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
}

async fn assert_chart_scripts<'a>(base: &str, html: &'a str) -> &'a str {
    let chart_path = script_src(html, "/ui/assets/chart-");
    let bridge_path = script_src(html, "/ui/assets/chart-bridge-");
    let boot_path = script_src(html, "/ui/assets/boot-");
    assert!(!chart_path.contains("bridge"), "chart asset was the bridge");
    assert!(html.find(chart_path).unwrap() < html.find(bridge_path).unwrap());
    assert!(html.find(bridge_path).unwrap() < html.find(boot_path).unwrap());
    assert_script(base, chart_path, "Chart.js v4.5.1").await;
    assert_script(base, bridge_path, "renderMetricChart").await;
    boot_path
}

fn script_src<'a>(html: &'a str, prefix: &str) -> &'a str {
    let mut rest = html;
    while let Some(start) = rest.find("src=\"") {
        let after = &rest[start + 5..];
        let end = after
            .find('"')
            .unwrap_or_else(|| panic!("unterminated script src"));
        let value = &after[..end];
        if value.starts_with(prefix) {
            return value;
        }
        rest = &after[end + 1..];
    }
    panic!("missing script {prefix}");
}

async fn assert_script(base: &str, path: &str, marker: &str) {
    let response = get(&format!("{base}{path}"), None).await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        header(&response, "cache-control"),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(header(&response, "x-content-type-options"), "nosniff");
    assert!(
        header(&response, "content-security-policy")
            .contains("script-src 'self' 'wasm-unsafe-eval'")
    );
    assert!(!header(&response, "content-security-policy").contains("unsafe-inline"));
    let body = response.text().await.unwrap();
    assert!(body.contains(marker), "script missing {marker}");
    assert!(!body.contains("eval("));
    assert!(!body.contains("new Function"));
    assert!(!body.contains(AGENT_MARKER));
    assert!(!body.contains(BEARER_SECRET));
}

fn quoted_attr<'a>(html: &'a str, marker: &str) -> &'a str {
    let start = html
        .find(marker)
        .unwrap_or_else(|| panic!("missing {marker}"))
        + marker.len();
    let rest = &html[start..];
    let end = rest.find('"').unwrap();
    &rest[..end]
}

async fn send_message(base: &str, token: Option<&str>) -> reqwest::StatusCode {
    let mut request = http()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({
            "message": {
                "messageId": "web-ui",
                "role": "ROLE_USER",
                "parts": [{"text": "hello"}]
            }
        }));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().await.unwrap().status()
}

#[tokio::test]
async fn web_ui_is_absent_until_opt_in() {
    let base = serve(false, false).await;
    let shell = get(&format!("{base}{WEB_UI_PATH}"), None).await;
    assert_eq!(shell.status(), reqwest::StatusCode::NOT_FOUND);
    let card = get(&format!("{base}/.well-known/agent-card.json"), None).await;
    assert_eq!(card.status(), reqwest::StatusCode::OK);
    assert!(card.text().await.unwrap().contains("list-tasks"));
}

#[tokio::test]
async fn web_ui_opt_in_serves_shell_and_hashed_assets() {
    let base = serve(true, false).await;
    let shell = get(&format!("{base}{WEB_UI_PATH}"), None).await;
    assert_eq!(shell.status(), reqwest::StatusCode::OK);
    assert_eq!(header(&shell, "content-type"), "text/html; charset=utf-8");
    assert_eq!(header(&shell, "cache-control"), "no-cache");
    assert_eq!(header(&shell, "x-content-type-options"), "nosniff");
    let policy = header(&shell, "content-security-policy").to_owned();
    assert!(policy.contains("default-src 'none'"));
    assert!(policy.contains("script-src 'self' 'wasm-unsafe-eval'"));
    assert!(policy.contains("style-src 'self'"));
    assert!(policy.contains("img-src 'self' data:"));
    assert!(policy.contains("connect-src 'self'"));
    assert!(!policy.contains("unsafe-inline"));
    assert!(!policy.contains('*'));
    assert!(!policy.contains("http:"));
    let html = shell.text().await.unwrap();
    assert!(html.contains("id=\"app\""));
    assert!(html.contains("type=\"module\""));
    assert!(!html.contains(AGENT_MARKER));
    let logo_path = quoted_attr(&html, "data-logo=\"");
    assert!(logo_path.starts_with("/ui/assets/logo-"));
    assert!(has_extension(logo_path, "svg"));
    let logo = get(&format!("{base}{logo_path}"), None).await;
    assert_eq!(logo.status(), reqwest::StatusCode::OK);
    assert_eq!(header(&logo, "content-type"), "image/svg+xml");
    assert!(header(&logo, "cache-control").contains("immutable"));
    assert!(logo.text().await.unwrap().contains("<svg"));
    let css_path = quoted_attr(&html, "href=\"");
    let boot_path = assert_chart_scripts(&base, &html).await;
    assert!(css_path.starts_with("/ui/assets/app-"));
    assert!(has_extension(css_path, "css"));
    assert!(boot_path.starts_with("/ui/assets/boot-"));
    let css = get(&format!("{base}{css_path}"), None).await;
    assert_eq!(css.status(), reqwest::StatusCode::OK);
    assert_eq!(header(&css, "content-type"), "text/css; charset=utf-8");
    assert_eq!(
        header(&css, "cache-control"),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(header(&css, "x-content-type-options"), "nosniff");
    assert_eq!(header(&css, "content-security-policy"), policy);
    let css_body = css.text().await.unwrap();
    assert!(!css_body.contains(AGENT_MARKER));
    let boot = get(&format!("{base}{boot_path}"), None).await;
    assert_eq!(boot.status(), reqwest::StatusCode::OK);
    assert_eq!(
        header(&boot, "content-type"),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        header(&boot, "cache-control"),
        "public, max-age=31536000, immutable"
    );
    let boot_js = boot.text().await.unwrap();
    assert!(!boot_js.contains(AGENT_MARKER));
    let module = boot_js
        .split('"')
        .find(|part| has_extension(part, "js") && part.starts_with("./app-"))
        .unwrap();
    let javascript = get(
        &format!("{base}/ui/assets/{}", module.trim_start_matches("./")),
        None,
    )
    .await;
    assert_eq!(javascript.status(), reqwest::StatusCode::OK);
    assert_eq!(
        header(&javascript, "content-type"),
        "text/javascript; charset=utf-8"
    );
    assert!(header(&javascript, "cache-control").contains("immutable"));
    let glue = javascript.text().await.unwrap();
    assert!(!glue.contains(AGENT_MARKER));
    assert!(!glue.contains(BEARER_SECRET));
    let wasm_name = glue
        .split(['\'', '"'])
        .find(|part| part.starts_with("app-") && has_extension(part, "wasm"))
        .unwrap_or_else(|| panic!("generated javascript does not name its wasm asset"));
    let wasm = get(&format!("{base}/ui/assets/{wasm_name}"), None).await;
    assert_eq!(wasm.status(), reqwest::StatusCode::OK);
    assert_eq!(header(&wasm, "content-type"), "application/wasm");
    assert!(header(&wasm, "cache-control").contains("immutable"));
    assert_eq!(header(&wasm, "x-content-type-options"), "nosniff");
    let bytes = wasm.bytes().await.unwrap();
    assert!(bytes.starts_with(b"\0asm"));
    assert!(
        !bytes
            .windows(AGENT_MARKER.len())
            .any(|window| window == AGENT_MARKER.as_bytes())
    );
    let index = get(&format!("{base}/ui"), None).await;
    assert_eq!(index.status(), reqwest::StatusCode::OK);
    assert_eq!(header(&index, "cache-control"), "no-cache");
    let unknown = get(&format!("{base}/ui/assets/not-packaged.js"), None).await;
    assert_eq!(unknown.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(header(&unknown, "x-content-type-options"), "nosniff");
    assert_eq!(header(&unknown, "cache-control"), "no-store");
}

#[tokio::test]
async fn web_ui_assets_stay_public_and_protocol_routes_stay_gated() {
    let base = serve(true, true).await;
    let shell = get(&format!("{base}{WEB_UI_PATH}"), Some(BEARER_SECRET)).await;
    assert_eq!(shell.status(), reqwest::StatusCode::OK);
    let html = shell.text().await.unwrap();
    assert!(!html.contains(AGENT_MARKER));
    assert!(!html.contains(BEARER_SECRET));
    assert!(!html.contains("Authorization"));
    let anonymous = get(&format!("{base}{WEB_UI_PATH}"), None).await;
    assert_eq!(anonymous.status(), reqwest::StatusCode::OK);
    let card = get(&format!("{base}/.well-known/agent-card.json"), None).await;
    assert_eq!(card.status(), reqwest::StatusCode::OK);
    assert!(!card.text().await.unwrap().is_empty());
    assert_eq!(
        send_message(&base, None).await,
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert_ne!(
        send_message(&base, Some(BEARER_SECRET)).await,
        reqwest::StatusCode::UNAUTHORIZED
    );
    let posted = http()
        .post(format!("{base}{WEB_UI_PATH}"))
        .send()
        .await
        .unwrap();
    assert_eq!(posted.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn web_ui_without_opt_in_does_not_bypass_security() {
    let base = serve(false, true).await;
    let shell = get(&format!("{base}{WEB_UI_PATH}"), None).await;
    assert_eq!(shell.status(), reqwest::StatusCode::UNAUTHORIZED);
    let asset = get(&format!("{base}/ui/assets/app.js"), None).await;
    assert_eq!(asset.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(
        send_message(&base, None).await,
        reqwest::StatusCode::UNAUTHORIZED
    );
    let card = get(&format!("{base}/.well-known/agent-card.json"), None).await;
    assert_eq!(card.status(), reqwest::StatusCode::OK);
}
