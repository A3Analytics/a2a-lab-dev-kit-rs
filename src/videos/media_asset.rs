//! Session-relative media resolution for the authenticated HTTP gateway.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::A2aLabError;
use crate::id::{VideoClipId, VideoFollowId};
use crate::videos::packager::PackagedMediaAsset;
use crate::videos::{VideoClipStream, VideoLiveStream};

/// Resolved gateway payload: finite bytes or a frame stream.
pub enum ResolvedMediaAsset {
    /// One complete response body.
    Bytes(PackagedMediaAsset),
    /// Finite MJPEG multipart stream for a prepared clip.
    ClipStream(VideoClipStream),
    /// Live MJPEG multipart stream for a prepared follow handle.
    LiveStream(VideoLiveStream),
}

/// Clip session lookup under `/media/video-clips/{id}/…`.
pub struct ClipAssetRequest<'a> {
    /// Prepared clip id from the gateway path.
    pub id: &'a VideoClipId,
    /// Session-relative asset (`init.mp4`, `index.m3u8`, segments). `None` is the session root.
    pub relative_path: Option<&'a str>,
}

/// Live session lookup under `/media/video-live/{id}/…`.
pub struct LiveAssetRequest<'a> {
    /// Prepared follow id from the gateway path.
    pub id: &'a VideoFollowId,
    /// Session-relative asset when live packaging is used. `None` is the session root.
    pub relative_path: Option<&'a str>,
}

/// Serves opaque session-relative assets from local packaged data or a proxied backend.
#[async_trait]
pub trait MediaAssetProvider: Send + Sync {
    /// Resolves one clip asset or stream.
    async fn resolve_clip_asset<'life0, 'life1>(
        &'life0 self,
        request: ClipAssetRequest<'life1>,
    ) -> Result<ResolvedMediaAsset, A2aLabError>
    where
        'life0: 'life1,
        'life1: 'life0;

    /// Resolves one live follow asset or stream.
    async fn resolve_live_asset<'life0, 'life1>(
        &'life0 self,
        request: LiveAssetRequest<'life1>,
    ) -> Result<ResolvedMediaAsset, A2aLabError>
    where
        'life0: 'life1,
        'life1: 'life0;
}

/// Shared provider handle for HTTP routers.
pub type SharedMediaAssetProvider = Arc<dyn MediaAssetProvider>;

/// Rejects traversal, absolute paths, credentials in paths, and unknown encodings.
pub fn validate_media_asset_path(relative: &str) -> Result<(), A2aLabError> {
    if relative.is_empty() {
        return Ok(());
    }
    if relative.contains("..")
        || relative.contains("://")
        || relative.contains('\\')
        || relative.starts_with('/')
        || relative.contains('@')
        || relative.contains('?')
        || relative.contains('#')
    {
        return Err(A2aLabError::invalid(
            "path",
            "must be a session-relative asset path",
        ));
    }
    let decoded = percent_decode_path(relative);
    if decoded.contains("..") || decoded.starts_with('/') {
        return Err(A2aLabError::invalid(
            "path",
            "must be a session-relative asset path",
        ));
    }
    Ok(())
}

/// Normalizes an optional nested asset path after gateway prefix stripping.
pub fn normalize_media_asset_path(relative: Option<&str>) -> Result<Option<String>, A2aLabError> {
    match relative.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(path) => {
            validate_media_asset_path(path)?;
            Ok(Some(path.to_owned()))
        }
    }
}

fn percent_decode_path(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &value[index + 1..index + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                decoded.push(byte);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

const CREDENTIAL_QUERY_KEYS: &[&str] = &[
    "access_token",
    "token",
    "api_key",
    "apikey",
    "key",
    "authorization",
    "bearer",
];

/// Returns true when a query string carries credential material.
#[must_use]
pub fn media_query_carries_credential(query: Option<&str>) -> bool {
    let Some(query) = query else {
        return false;
    };
    query.split('&').any(|pair| {
        let (key, _) = pair.split_once('=').unwrap_or((pair, ""));
        CREDENTIAL_QUERY_KEYS
            .iter()
            .any(|name| key.eq_ignore_ascii_case(name))
    })
}
