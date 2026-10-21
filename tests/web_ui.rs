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

fn assert_boot_script(html: &str) -> &str {
    let boot_path = script_src(html, "/ui/assets/boot-");
    assert!(
        !html.contains("/ui/assets/chart-"),
        "packaged HTML must not reference Chart.js"
    );
    assert!(
        !html.contains("chart-bridge"),
        "packaged HTML must not reference the chart bridge"
    );
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

fn font_asset_paths(css: &str) -> Vec<&str> {
    let mut paths = Vec::new();
    let mut rest = css;
    while let Some(start) = rest.find("url(\"/ui/assets/") {
        let after = &rest[start + 5..];
        let end = after
            .find('"')
            .unwrap_or_else(|| panic!("unterminated font url"));
        paths.push(&after[..end]);
        rest = &after[end + 1..];
    }
    assert!(
        !paths.is_empty(),
        "stylesheet does not reference packaged fonts"
    );
    paths
}

fn assert_style_contract(css_body: &str) {
    assert!(!css_body.contains("fonts.googleapis.com"));
    assert!(!css_body.contains("fonts.gstatic.com"));
    assert!(!css_body.contains("http://"));
    assert!(!css_body.contains("https://"));
    let mut search_from = 0;
    for marker in [
        "fragment:tokens",
        "fragment:shell",
        "fragment:controls",
        "fragment:loading",
        "fragment:logs",
        "fragment:metrics",
        "fragment:tasks",
        "fragment:chat",
        "fragment:inventory",
        "fragment:workbench",
    ] {
        let found = css_body[search_from..]
            .find(marker)
            .unwrap_or_else(|| panic!("missing {marker}"));
        search_from += found + marker.len();
    }
    for index in 1..=12 {
        let token = format!("--chart-{index}:");
        assert!(
            css_body.matches(&token).count() >= 2,
            "missing light and dark {token}"
        );
    }
    for token in [
        "[data-theme=\"dark\"]",
        "prefers-color-scheme: dark",
        ":not([data-theme=\"light\"])",
        "--a3-nav-glyph-mute-bg",
        "--a3-nav-glyph-active-fg",
        "--elevation-2xs",
        "--elevation-2xl",
        "--density-control-height",
        "[data-density=\"compact\"]",
        "--motion-normal:",
        "font-family: Inter",
        "font-family: Oxanium",
        "--font-oxanium:",
    ] {
        assert!(css_body.contains(token), "missing {token}");
    }
    let reduced = css_body
        .split("prefers-reduced-motion: reduce")
        .nth(1)
        .expect("missing reduced-motion block");
    for token in [
        "--motion-instant: 0ms",
        "--motion-fast: 0ms",
        "--motion-normal: 0ms",
        "--motion-slow: 0ms",
        "--motion-slower: 0ms",
        "--motion-feedback-duration: 0ms",
        "--motion-small-enter-duration: 0ms",
        "--motion-small-exit-duration: 0ms",
        "--motion-medium-enter-duration: 0ms",
        "--motion-medium-exit-duration: 0ms",
        "--motion-alert-enter-duration: 0ms",
        "--motion-large-enter-duration: 0ms",
        "--motion-large-exit-duration: 0ms",
    ] {
        assert!(reduced.contains(token), "missing {token}");
    }
}

fn assert_font_licenses() {
    let fonts = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web-ui/static/fonts");
    let inter = std::fs::read_to_string(fonts.join("inter/LICENSE.txt")).unwrap();
    assert!(inter.contains("Copyright (c) 2016 The Inter Project Authors"));
    assert!(inter.contains("SIL OPEN FONT LICENSE"));
    let oxanium = std::fs::read_to_string(fonts.join("oxanium/OFL.txt")).unwrap();
    assert!(oxanium.contains("Copyright 2019 The Oxanium Project Authors"));
    assert!(oxanium.contains("SIL OPEN FONT LICENSE"));
}

fn is_woff2_path(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("woff2"))
}

async fn assert_packaged_styles(base: &str, css_body: &str) {
    assert_style_contract(css_body);
    assert_font_licenses();
    let font_paths = font_asset_paths(css_body);
    assert!(font_paths.iter().any(|path| path.contains("inter-")));
    assert!(font_paths.iter().any(|path| path.contains("oxanium-")));
    for path in font_paths {
        assert!(path.starts_with("/ui/assets/"), "{path}");
        assert!(is_woff2_path(path), "{path}");
        let font = get(&format!("{base}{path}"), None).await;
        assert_eq!(font.status(), reqwest::StatusCode::OK);
        assert_eq!(header(&font, "content-type"), "font/woff2");
        assert!(header(&font, "content-security-policy").contains("font-src 'self'"));
        let bytes = font.bytes().await.unwrap();
        assert!(bytes.starts_with(b"wOF2"), "{path}");
    }
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
    assert!(policy.contains("font-src 'self'"));
    assert!(!policy.contains("fonts.googleapis.com"));
    assert!(!policy.contains("fonts.gstatic.com"));
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
    let boot_path = assert_boot_script(&html);
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
    assert_packaged_styles(&base, &css_body).await;
    let boot = get(&format!("{base}{boot_path}"), None).await;
    assert_eq!(boot.status(), reqwest::StatusCode::OK);
    assert_eq!(
        header(&boot, "content-type"),
        "text/javascript; charset=utf-8"
    );
    let boot_js = boot.text().await.unwrap();
    assert!(boot_js.contains("import init"));
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
