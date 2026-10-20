//! Packaged browser shell served on the A2A HTTP listener.

use axum::Router;
use axum::extract::Path;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::Response;
use axum::routing::get;

/// Documented path of the packaged web interface.
pub const WEB_UI_PATH: &str = "/ui/";

const ENTRY_CACHE: &str = "no-cache";
const ASSET_CACHE: &str = "public, max-age=31536000, immutable";
const POLICY: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; font-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'";

mod packed {
    include!(concat!(env!("OUT_DIR"), "/web_ui_assets.rs"));
}

#[cfg(all(test, feature = "web-ui"))]
#[path = "../../web-ui/build/freshness.rs"]
mod freshness;

pub(crate) fn router() -> Router {
    Router::new()
        .route("/ui", get(entry))
        .route("/ui/", get(entry))
        .route("/ui/index.html", get(entry))
        .route("/ui/assets/{*path}", get(asset))
}

#[must_use]
pub(crate) fn is_public_route(method: &Method, path: &str) -> bool {
    if *method != Method::GET {
        return false;
    }
    match path {
        "/ui" | "/ui/" | "/ui/index.html" => true,
        _ => asset_name(path).is_some_and(|name| packed_asset(name).is_some()),
    }
}

async fn entry() -> Response {
    shell(packed::ENTRY_HTML, "text/html; charset=utf-8", ENTRY_CACHE)
}

async fn asset(Path(path): Path<String>) -> Response {
    match packed_asset(&path) {
        Some(asset) => shell(asset.body, asset.content_type, ASSET_CACHE),
        None => not_found(),
    }
}

fn asset_name(path: &str) -> Option<&str> {
    let name = path.strip_prefix("/ui/assets/")?;
    if name.is_empty() || name.contains("..") {
        return None;
    }
    name.chars()
        .all(|character| {
            character.is_ascii_alphanumeric()
                || character == '-'
                || character == '_'
                || character == '.'
                || character == '/'
        })
        .then_some(name)
}

fn packed_asset(name: &str) -> Option<&'static packed::PackedAsset> {
    packed::ASSETS.iter().find(|asset| asset.name == name)
}

fn shell(body: &'static [u8], content_type: &'static str, cache_control: &'static str) -> Response {
    let mut response = Response::new(axum::body::Body::from(body));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache_control));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(POLICY));
    response
}

fn not_found() -> Response {
    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::NOT_FOUND;
    let headers = response.headers_mut();
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::freshness::{fingerprint_tree, package_status};
    use super::packed::SOURCE_FINGERPRINT;

    #[test]
    fn packaged_assets_match_the_browser_sources() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web-ui");
        assert_eq!(fingerprint_tree(&root), SOURCE_FINGERPRINT);
        assert_eq!(
            package_status(SOURCE_FINGERPRINT, Some(SOURCE_FINGERPRINT), true),
            super::freshness::PackageStatus::Fresh
        );
    }
}
