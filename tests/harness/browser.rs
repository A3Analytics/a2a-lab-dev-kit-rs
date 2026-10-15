//! Headless Chrome session for the packaged shell.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

pub struct Browser {
    child: Child,
    socket: Socket,
    profile: PathBuf,
}

#[allow(dead_code)]
impl Browser {
    pub fn open(url: &str) -> Self {
        let (child, port, profile) = launch();
        let mut browser = Self {
            socket: Socket::connect(&page_socket(port)),
            child,
            profile,
        };
        browser.call("Page.enable", &json!({}));
        browser.call("Network.enable", &json!({}));
        browser.call("Runtime.enable", &json!({}));
        browser.goto(url);
        browser
    }

    pub fn goto(&mut self, url: &str) {
        self.call("Page.navigate", &json!({ "url": url }));
        self.wait_until("document.readyState === 'complete'");
    }

    pub fn move_mouse(&mut self, x: f64, y: f64) {
        self.call(
            "Input.dispatchMouseEvent",
            &json!({ "type": "mouseMoved", "x": x, "y": y }),
        );
    }

    pub fn click(&mut self, id: &str) {
        self.click_selector(&format!("#{id}"));
    }

    pub fn click_selector(&mut self, selector: &str) {
        let selector = serde_json::to_string(selector).unwrap_or_default();
        let value = self.eval(&format!(
            "(() => {{ const el = document.querySelector({selector}); if (!el) return false; el.click(); return true; }})()"
        ));
        assert_eq!(value, Value::Bool(true), "missing control");
    }

    pub fn focus(&mut self, id: &str) {
        let id = serde_json::to_string(id).unwrap_or_default();
        let focused = self.eval(&format!(
            "(() => {{ const el = document.getElementById({id}); if (!el) return false; el.focus(); return document.activeElement === el; }})()"
        ));
        assert_eq!(focused, Value::Bool(true), "control did not take focus");
    }

    pub fn press_enter(&mut self) {
        self.press_key("Enter", "Enter", 13, Some("\r"));
    }

    pub fn press_tab(&mut self) {
        self.press_key("Tab", "Tab", 9, None);
    }

    fn press_key(&mut self, key: &str, code: &str, key_code: u16, text: Option<&str>) {
        self.call(
            "Emulation.setFocusEmulationEnabled",
            &json!({ "enabled": true }),
        );
        let mut down = json!({
            "type": "keyDown",
            "key": key,
            "code": code,
            "windowsVirtualKeyCode": key_code,
            "nativeVirtualKeyCode": key_code
        });
        if let Some(text) = text {
            down["text"] = Value::String(text.to_owned());
        }
        self.call("Input.dispatchKeyEvent", &down);
        self.call(
            "Input.dispatchKeyEvent",
            &json!({
                "type": "keyUp",
                "key": key,
                "code": code,
                "windowsVirtualKeyCode": key_code,
                "nativeVirtualKeyCode": key_code
            }),
        );
    }

    pub fn set_value(&mut self, id: &str, value: &str) {
        let id = serde_json::to_string(id).unwrap_or_default();
        let value = serde_json::to_string(value).unwrap_or_default();
        let entered = self.eval(&format!(
            "(() => {{ const el = document.getElementById({id}); if (!el) return false; el.value = {value}; return el.value === {value}; }})()"
        ));
        assert_eq!(entered, Value::Bool(true), "control did not keep its value");
    }

    pub fn eval(&mut self, expression: &str) -> Value {
        let result = self.call(
            "Runtime.evaluate",
            &json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": true
            }),
        );
        assert!(
            result.get("exceptionDetails").is_none(),
            "browser expression failed: {result}"
        );
        result["result"]
            .get("value")
            .cloned()
            .unwrap_or(Value::Null)
    }

    pub fn wait_until(&mut self, expression: &str) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(15) {
            if self.eval(&format!("!!({expression})")) == Value::Bool(true) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let text = self.eval("document.body ? document.body.innerText : ''");
        panic!("timed out waiting for {expression}: {text}");
    }

    pub fn snapshot(&mut self) -> Value {
        self.eval(
            "({
                text: document.body.innerText,
                html: document.documentElement.outerHTML,
                state: document.getElementById('connection')?.dataset.state ?? '',
                access: document.getElementById('security')?.dataset.access ?? '',
                alert: document.getElementById('alert')?.textContent ?? '',
                output: document.getElementById('output')?.textContent ?? '',
                name: document.getElementById('agent-name')?.textContent ?? '',
                bearer: !!document.getElementById('bearer-token'),
                bearerValue: document.getElementById('bearer-token')?.value ?? '',
                context: document.getElementById('context-id')?.value ?? '',
                nav: !!document.querySelector('nav[aria-label=\"Lab sections\"]'),
                skip: !!document.querySelector('a[href=\"#main\"]'),
                live: document.getElementById('connection')?.getAttribute('aria-live') ?? '',
                alertRole: document.getElementById('alert')?.getAttribute('role') ?? '',
                local: Object.keys(localStorage),
                session: Object.keys(sessionStorage),
                cookie: document.cookie,
                href: location.href,
                xss: window.__xss === 1,
                xssName: !!document.getElementById('xss-name'),
                xssSkill: !!document.getElementById('xss-skill'),
                xssPayload: !!document.getElementById('xss-payload'),
                inlineScripts: [...document.scripts].filter(script => !script.src).length,
                images: [...document.images].filter(img => !img.closest('#brand')).length,
                unlabeled: [...document.querySelectorAll('input, textarea')].filter(el => !document.querySelector(`label[for=\"${CSS.escape(el.id)}\"]`)).map(el => el.id),
                blankButtons: [...document.querySelectorAll('button')].filter(el => !el.textContent.trim()).length,
                wasm: performance.getEntriesByType('resource').map(entry => entry.name).filter(name => name.endsWith('.wasm')),
                scroll: document.documentElement.scrollWidth,
                width: window.innerWidth,
                buttonWidth: document.querySelector('a.skip')?.getBoundingClientRect().width ?? 0,
                focus: (() => { const el = document.querySelector('a.skip'); el.focus(); const style = getComputedStyle(el); return style.outlineStyle + ' ' + style.outlineWidth; })()
            })",
        )
    }

    pub fn urls(&self) -> Vec<String> {
        self.socket
            .events
            .iter()
            .filter(|event| event["method"] == "Network.requestWillBeSent")
            .filter_map(|event| {
                event["params"]["request"]["url"]
                    .as_str()
                    .map(str::to_owned)
            })
            .collect()
    }

    pub fn requests(&self, base: &str) -> Vec<Value> {
        self.socket
            .events
            .iter()
            .filter(|event| event["method"] == "Network.requestWillBeSent")
            .filter(|event| {
                event["params"]["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.starts_with(base))
            })
            .cloned()
            .collect()
    }

    pub fn console_text(&self) -> String {
        self.socket
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event["method"].as_str(),
                    Some("Runtime.consoleAPICalled" | "Runtime.exceptionThrown")
                )
            })
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn show(&mut self, hash: &str) {
        let target = if hash.starts_with('#') {
            hash.to_owned()
        } else {
            format!("#{hash}")
        };
        let literal = serde_json::to_string(&target).unwrap_or_default();
        self.eval(&format!("location.hash = {literal}"));
        let hash = target.trim_start_matches('#').trim_matches('/');
        let view = match hash {
            "chat" => "view-chat",
            "logs" | "inventory" | "inventory/logs" | "workbench" => "view-logs",
            "metrics" | "inventory/metrics" => "view-metrics",
            "tasks" | "inventory/tasks" => "view-tasks",
            "images" | "inventory/images" => "view-images",
            _ => "view-agent",
        };
        self.wait_until(&format!(
            "location.hash === {literal} && document.getElementById('{view}')?.parentElement?.id === 'main' && document.querySelectorAll('#main > .view').length === 1"
        ));
    }

    pub fn viewport(&mut self, width: u32, height: u32) {
        self.call(
            "Emulation.setDeviceMetricsOverride",
            &json!({
                "width": width,
                "height": height,
                "deviceScaleFactor": 1,
                "mobile": width < 600
            }),
        );
        self.eval("window.dispatchEvent(new Event('resize'))");
        let layout = if width < 768 { "mobile" } else { "desktop" };
        self.wait_until(&format!(
            "document.documentElement.dataset.layout === '{layout}'"
        ));
    }

    pub fn fail_network(&mut self, fail: bool) {
        if fail && !self.socket.fail_fetches {
            self.call(
                "Fetch.enable",
                &json!({ "patterns": [{ "urlPattern": "*", "requestStage": "Request" }] }),
            );
        }
        if !fail && self.socket.fail_fetches {
            self.call("Fetch.disable", &json!({}));
        }
        self.socket.fail_fetches = fail;
    }

    pub fn reload(&mut self) {
        self.call("Page.reload", &json!({ "ignoreCache": true }));
        self.wait_until("document.readyState === 'complete'");
    }

    fn call(&mut self, method: &str, params: &Value) -> Value {
        self.socket.call(method, params)
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let pid = self.child.id();
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{pid}")])
            .output();
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

pub fn header<'a>(request: &'a Value, name: &str) -> Option<&'a str> {
    request["params"]["request"]["headers"]
        .as_object()?
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.as_str())
}

pub fn request_url(request: &Value) -> &str {
    request["params"]["request"]["url"].as_str().unwrap_or("")
}

pub fn request_method(request: &Value) -> &str {
    request["params"]["request"]["method"]
        .as_str()
        .unwrap_or("")
}

struct Socket {
    stream: TcpStream,
    buffer: Vec<u8>,
    events: Vec<Value>,
    next_id: u64,
    fail_fetches: bool,
}

impl Socket {
    fn connect(url: &str) -> Self {
        let (host, port, path) = split_ws(url);
        let mut stream =
            TcpStream::connect((host, port)).unwrap_or_else(|_| panic!("connect {url}"));
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap_or_else(|_| panic!("timeout {url}"));
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .unwrap_or_else(|_| panic!("handshake {url}"));
        let mut header = Vec::new();
        loop {
            let mut byte = [0; 1];
            stream
                .read_exact(&mut byte)
                .unwrap_or_else(|_| panic!("handshake header {url}"));
            header.push(byte[0]);
            if header.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&header);
        assert!(text.contains("101"), "websocket upgrade failed: {text}");
        Self {
            stream,
            buffer: Vec::new(),
            events: Vec::new(),
            next_id: 0,
            fail_fetches: false,
        }
    }

    fn call(&mut self, method: &str, params: &Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send_text(&json!({ "id": id, "method": method, "params": params }).to_string());
        loop {
            let value = self.read_json();
            if value["method"] == "Fetch.requestPaused" && self.fail_fetches {
                self.fail_paused(&value);
                continue;
            }
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = value.get("error") {
                    panic!("cdp {method} failed: {error}");
                }
                return value.get("result").cloned().unwrap_or(Value::Null);
            }
            self.events.push(value);
        }
    }

    fn send_text(&mut self, text: &str) {
        let payload = text.as_bytes();
        let mut frame = vec![0x81];
        let mask = [1, 2, 3, 4];
        if payload.len() < 126 {
            frame.push(0x80 | u8::try_from(payload.len()).unwrap_or(125));
        } else {
            frame.push(0x80 | 0x7e);
            let len =
                u16::try_from(payload.len()).unwrap_or_else(|_| panic!("cdp frame is too large"));
            frame.extend_from_slice(&len.to_be_bytes());
        }
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ mask[index % 4]),
        );
        self.stream
            .write_all(&frame)
            .unwrap_or_else(|_| panic!("send cdp frame"));
    }

    fn read_json(&mut self) -> Value {
        let mut message = Vec::new();
        loop {
            let (opcode, fin, payload) = self.read_frame();
            match opcode {
                0x1 | 0x0 => message.extend(payload),
                0x8 => panic!("browser closed the debugging socket"),
                0x9 => self.send_pong(),
                _ => {}
            }
            if fin && opcode != 0x9 {
                break;
            }
        }
        serde_json::from_slice(&message).unwrap_or_else(|_| panic!("cdp frame was not json"))
    }

    fn read_frame(&mut self) -> (u8, bool, Vec<u8>) {
        let first = self.read_exact(2);
        let opcode = first[0] & 0x0f;
        let fin = first[0] & 0x80 != 0;
        let masked = first[1] & 0x80 != 0;
        let length = match first[1] & 0x7f {
            126 => u64::from(u16::from_be_bytes(
                self.read_exact(2).try_into().unwrap_or([0, 0]),
            )),
            127 => u64::from_be_bytes(self.read_exact(8).try_into().unwrap_or([0; 8])),
            short => u64::from(short),
        };
        let mask = masked.then(|| self.read_exact(4));
        let mut payload = self.read_exact(usize::try_from(length).unwrap_or(0));
        if let Some(mask) = mask {
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
        }
        (opcode, fin, payload)
    }

    fn read_exact(&mut self, size: usize) -> Vec<u8> {
        while self.buffer.len() < size {
            let mut chunk = [0; 8192];
            let read = self
                .stream
                .read(&mut chunk)
                .unwrap_or_else(|_| panic!("debugging socket closed"));
            assert!(read > 0, "debugging socket closed");
            self.buffer.extend_from_slice(&chunk[..read]);
        }
        self.buffer.drain(..size).collect()
    }

    fn send_pong(&mut self) {
        let frame = [0x8A, 0x80, 9, 8, 7, 6];
        let _ = self.stream.write_all(&frame);
    }

    fn fail_paused(&mut self, value: &Value) {
        let request_id = value["params"]["requestId"].as_str().unwrap_or("");
        self.next_id += 1;
        let id = self.next_id;
        self.send_text(
            &json!({
                "id": id,
                "method": "Fetch.failRequest",
                "params": { "requestId": request_id, "errorReason": "InternetDisconnected" }
            })
            .to_string(),
        );
    }
}

fn launch() -> (Child, u16, PathBuf) {
    let profile = std::env::temp_dir().join(format!(
        "web-ui-browser-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&profile).unwrap_or_else(|_| panic!("create browser profile"));
    let log_path = profile.join("chrome.log");
    let log = std::fs::File::create(&log_path).unwrap_or_else(|_| panic!("create browser log"));
    let mut command = Command::new(browser_bin());
    command
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-extensions")
        .arg("--remote-debugging-port=0")
        .arg("--remote-allow-origins=*")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            log.try_clone()
                .unwrap_or_else(|_| panic!("clone browser log")),
        ))
        .stderr(Stdio::from(log));
    detach(&mut command);
    let child = command
        .spawn()
        .unwrap_or_else(|_| panic!("start headless browser"));
    let port = wait_port(&log_path);
    (child, port, profile)
}

fn detach(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

fn browser_bin() -> PathBuf {
    if let Some(path) = std::env::var_os("WEB_UI_BROWSER") {
        return PathBuf::from(path);
    }
    for candidate in [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return path;
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        for cache in [
            PathBuf::from(&home).join("Library/Caches/ms-playwright"),
            PathBuf::from(&home).join(".cache/ms-playwright"),
        ] {
            if let Some(path) = find_shell(&cache) {
                return path;
            }
        }
    }
    panic!("set WEB_UI_BROWSER to a Chrome or Chromium binary");
}

fn find_shell(dir: &Path) -> Option<PathBuf> {
    let mut found = Vec::new();
    collect_shells(dir, &mut found);
    found.into_iter().find(|path| {
        path.file_name().and_then(|name| name.to_str()) == Some("chrome-headless-shell")
    })
}

fn collect_shells(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_shells(&path, found);
        } else if path.file_name().and_then(|name| name.to_str()) == Some("chrome-headless-shell") {
            found.push(path);
        }
    }
}

fn wait_port(log: &Path) -> u16 {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if let Ok(text) = std::fs::read_to_string(log)
            && let Some(port) = parse_port(&text)
        {
            return port;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let text = std::fs::read_to_string(log).unwrap_or_default();
    panic!("headless browser did not open a debugging port: {text}");
}

fn parse_port(text: &str) -> Option<u16> {
    for token in text.split_whitespace() {
        let Some(rest) = token
            .strip_prefix("ws://127.0.0.1:")
            .or_else(|| token.strip_prefix("ws://[::1]:"))
        else {
            continue;
        };
        let port = rest.split(['/', ' ']).next()?;
        if let Ok(port) = port.parse() {
            return Some(port);
        }
    }
    None
}

fn page_socket(port: u16) -> String {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if let Some(url) = page_target(port) {
            return url;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("headless browser did not expose a page target");
}

fn page_target(port: u16) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let request =
        format!("GET /json/list HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;
    let mut raw = String::new();
    let _ = stream.read_to_string(&mut raw);
    let start = raw.find('[')?;
    let end = raw.rfind(']')?;
    let list: Vec<Value> = serde_json::from_str(&raw[start..=end]).ok()?;
    list.iter()
        .find(|item| item["type"] == "page")
        .and_then(|item| item["webSocketDebuggerUrl"].as_str())
        .map(str::to_owned)
}

fn split_ws(url: &str) -> (&str, u16, &str) {
    let rest = url.strip_prefix("ws://").unwrap_or(url);
    let (authority, _) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = authority.rsplit_once(':').unwrap_or((authority, "80"));
    let port = port.parse().unwrap_or(80);
    let path = url
        .find(authority)
        .map(|index| &url[index + authority.len()..])
        .filter(|path| !path.is_empty())
        .unwrap_or("/");
    (host, port, path)
}

#[cfg(test)]
mod path_is_used {
    #[test]
    fn split_keeps_the_socket_path() {
        let (host, port, path) = super::split_ws("ws://127.0.0.1:9/devtools/page/abc");
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 9);
        assert_eq!(path, "/devtools/page/abc");
    }
}
