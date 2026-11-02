//! Fake remote [`MediaAssetProvider`] for gateway seam tests.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use a2a_lab_dev_kit::{
    A2aLabError, ClipAssetRequest, LiveAssetRequest, MediaAssetProvider, PackagedMediaAsset,
    ResolvedMediaAsset, VideoClipId, VideoFollowId, video_clip_path, video_live_path,
};

const UPSTREAM_SECRET: &str = "https://vault.internal:9999/private/";

/// One proxied asset keyed by session-relative path.
#[derive(Clone)]
pub struct FakeRemoteAsset {
    content_type: String,
    bytes: Vec<u8>,
}

impl FakeRemoteAsset {
    /// Creates a non-empty remote asset.
    pub fn new(content_type: impl Into<String>, bytes: Vec<u8>) -> Result<Self, A2aLabError> {
        if bytes.is_empty() {
            return Err(A2aLabError::invalid("bytes", "must not be empty"));
        }
        Ok(Self {
            content_type: content_type.into(),
            bytes,
        })
    }
}

struct RemoteClipBinding {
    assets: HashMap<String, FakeRemoteAsset>,
}

struct RemoteLiveBinding {
    assets: HashMap<String, FakeRemoteAsset>,
}

/// In-memory remote backend stand-in. Upstream URLs never appear in responses.
pub struct FakeRemoteMediaAssetProvider {
    clips: Mutex<HashMap<VideoClipId, RemoteClipBinding>>,
    live: Mutex<HashMap<VideoFollowId, RemoteLiveBinding>>,
}

impl FakeRemoteMediaAssetProvider {
    /// Creates an empty remote provider.
    #[must_use]
    pub fn new() -> Self {
        Self {
            clips: Mutex::new(HashMap::new()),
            live: Mutex::new(HashMap::new()),
        }
    }

    /// Binds proxied clip assets for `id`.
    pub fn bind_clip(
        &self,
        id: VideoClipId,
        assets: HashMap<String, FakeRemoteAsset>,
    ) -> Result<(), A2aLabError> {
        if assets.is_empty() {
            return Err(A2aLabError::invalid("assets", "must not be empty"));
        }
        let _root = video_clip_path(&id);
        self.clips
            .lock()
            .map_err(|_| A2aLabError::unavailable("remote clip registry is unavailable"))?
            .insert(id, RemoteClipBinding { assets });
        Ok(())
    }

    /// Binds proxied live assets for `id`.
    pub fn bind_live(
        &self,
        id: VideoFollowId,
        assets: HashMap<String, FakeRemoteAsset>,
    ) -> Result<(), A2aLabError> {
        if assets.is_empty() {
            return Err(A2aLabError::invalid("assets", "must not be empty"));
        }
        let _root = video_live_path(&id);
        self.live
            .lock()
            .map_err(|_| A2aLabError::unavailable("remote live registry is unavailable"))?
            .insert(id, RemoteLiveBinding { assets });
        Ok(())
    }

    fn proxy_clip(
        binding: &RemoteClipBinding,
        relative: Option<&str>,
    ) -> Result<PackagedMediaAsset, A2aLabError> {
        let path = relative.unwrap_or("index.m3u8");
        let asset = binding
            .assets
            .get(path)
            .ok_or_else(|| A2aLabError::not_found("packaged asset", path.to_owned()))?;
        // Simulate an upstream fetch without exposing the upstream location.
        let _upstream = format!("{UPSTREAM_SECRET}{path}");
        PackagedMediaAsset::new(asset.content_type.clone(), asset.bytes.clone())
    }

    fn proxy_live(
        binding: &RemoteLiveBinding,
        relative: Option<&str>,
    ) -> Result<PackagedMediaAsset, A2aLabError> {
        let path = relative.unwrap_or("index.m3u8");
        let asset = binding
            .assets
            .get(path)
            .ok_or_else(|| A2aLabError::not_found("packaged asset", path.to_owned()))?;
        let _upstream = format!("{UPSTREAM_SECRET}{path}");
        PackagedMediaAsset::new(asset.content_type.clone(), asset.bytes.clone())
    }
}

impl Default for FakeRemoteMediaAssetProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MediaAssetProvider for FakeRemoteMediaAssetProvider {
    async fn resolve_clip_asset<'life0, 'life1>(
        &'life0 self,
        request: ClipAssetRequest<'life1>,
    ) -> Result<ResolvedMediaAsset, A2aLabError>
    where
        'life0: 'life1,
        'life1: 'life0,
    {
        let guard = self
            .clips
            .lock()
            .map_err(|_| A2aLabError::unavailable("remote clip registry is unavailable"))?;
        let binding = guard
            .get(request.id)
            .ok_or_else(|| A2aLabError::not_found("video clip", request.id.to_string()))?;
        let asset = Self::proxy_clip(binding, request.relative_path)?;
        Ok(ResolvedMediaAsset::Bytes(asset))
    }

    async fn resolve_live_asset<'life0, 'life1>(
        &'life0 self,
        request: LiveAssetRequest<'life1>,
    ) -> Result<ResolvedMediaAsset, A2aLabError>
    where
        'life0: 'life1,
        'life1: 'life0,
    {
        let guard = self
            .live
            .lock()
            .map_err(|_| A2aLabError::unavailable("remote live registry is unavailable"))?;
        let binding = guard
            .get(request.id)
            .ok_or_else(|| A2aLabError::not_found("video follow", request.id.to_string()))?;
        let asset = Self::proxy_live(binding, request.relative_path)?;
        Ok(ResolvedMediaAsset::Bytes(asset))
    }
}
