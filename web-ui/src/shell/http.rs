//! Same-origin fetch. Credentials are attached only as an Authorization header.

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Headers, Request, RequestCache, RequestCredentials, RequestInit, RequestMode, Response,
};

use crate::codec::Reply;

#[derive(Clone)]
pub struct Transport {
    origin: String,
}

impl Transport {
    pub fn new(origin: String) -> Self {
        Self { origin }
    }

    pub async fn get_card(&self) -> Result<Reply, ()> {
        self.request("GET", "/.well-known/agent-card.json", None, None)
            .await
    }

    pub async fn send(&self, body: &str, token: Option<&str>) -> Result<Reply, ()> {
        self.request("POST", "/message:send", Some(body), token)
            .await
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
        token: Option<&str>,
    ) -> Result<Reply, ()> {
        let window = web_sys::window().ok_or(())?;
        if !window.navigator().on_line() {
            return Err(());
        }
        let url = format!("{}{path}", self.origin.trim_end_matches('/'));
        let request = build_request(&url, method, body, token)?;
        let value = JsFuture::from(window.fetch_with_request(&request))
            .await
            .map_err(|_| ())?;
        let response: Response = value.dyn_into().map_err(|_| ())?;
        let status = response.status();
        let text = match response.text() {
            Ok(promise) => JsFuture::from(promise)
                .await
                .ok()
                .and_then(|value| value.as_string())
                .unwrap_or_default(),
            Err(_) => String::new(),
        };
        Ok(Reply { status, body: text })
    }
}

fn build_request(
    url: &str,
    method: &str,
    body: Option<&str>,
    token: Option<&str>,
) -> Result<Request, ()> {
    let headers = Headers::new().map_err(|_| ())?;
    headers.set("A2A-Version", "1.0").map_err(|_| ())?;
    if body.is_some() {
        headers
            .set("Content-Type", "application/a2a+json")
            .map_err(|_| ())?;
    }
    if let Some(token) = token.filter(|token| !token.is_empty()) {
        headers
            .set("Authorization", &format!("Bearer {token}"))
            .map_err(|_| ())?;
    }
    let init = RequestInit::new();
    init.set_method(method);
    init.set_mode(RequestMode::SameOrigin);
    init.set_credentials(RequestCredentials::Omit);
    init.set_cache(RequestCache::NoStore);
    init.set_headers(headers.as_ref());
    if let Some(body) = body {
        init.set_body(&JsValue::from_str(body));
    }
    Request::new_with_str_and_init(url, &init).map_err(|_| ())
}
