//! Local [`MediaAssetProvider`] over [`A2aLabApi`] and optional packaged HLS sessions.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::error::A2aLabError;
use crate::id::VideoClipId;
use crate::service::A2aLabApi;
use crate::videos::dvr_store::DvrMediaStore;
use crate::videos::media_asset::{
    ClipAssetRequest, LiveAssetRequest, MediaAssetProvider, ResolvedMediaAsset,
    normalize_media_asset_path,
};
use crate::videos::packager::{
    DvrMediaPackager, DvrPackagerConfig, PackagedHlsSession, PackagedMediaAsset,
};
use crate::videos::{HLS_INIT_RELATIVE_PATH, HLS_PLAYLIST_RELATIVE_PATH, video_clip_path};

struct RegisteredHlsClip {
    default_path: String,
    assets: HashMap<String, PackagedMediaAsset>,
}

/// In-process packaged assets and MJPEG streams for prepared sessions.
pub struct LocalMediaAssetProvider {
    lab: Arc<dyn A2aLabApi>,
    packager: DvrMediaPackager,
    hls_clips: Mutex<HashMap<VideoClipId, RegisteredHlsClip>>,
}

impl LocalMediaAssetProvider {
    /// Creates a provider backed by `lab` with the default packager window.
    #[must_use]
    pub fn new(lab: Arc<dyn A2aLabApi>) -> Self {
        Self {
            lab,
            packager: DvrMediaPackager::new(DvrPackagerConfig::default()),
            hls_clips: Mutex::new(HashMap::new()),
        }
    }

    /// Registers a bounded HLS session for a prepared clip id.
    ///
    /// Packaged assets are loaded eagerly from `store` so the gateway never needs
    /// upstream locations at request time. The session root must match [`video_clip_path`]
    /// for `id`.
    pub async fn register_hls_clip(
        &self,
        id: VideoClipId,
        session: PackagedHlsSession,
        store: &impl DvrMediaStore,
    ) -> Result<(), A2aLabError> {
        if session.session_root() != video_clip_path(&id) {
            return Err(A2aLabError::invalid(
                "session_root",
                "must match the clip gateway root",
            ));
        }
        let mut assets = HashMap::new();
        assets.insert(
            HLS_INIT_RELATIVE_PATH.to_owned(),
            self.packager
                .load_hls_asset(store, &session, HLS_INIT_RELATIVE_PATH)
                .await?,
        );
        assets.insert(
            HLS_PLAYLIST_RELATIVE_PATH.to_owned(),
            self.packager
                .load_hls_asset(store, &session, HLS_PLAYLIST_RELATIVE_PATH)
                .await?,
        );
        for segment in session.segments() {
            assets.insert(
                segment.relative_path().to_owned(),
                self.packager
                    .load_hls_asset(store, &session, segment.relative_path())
                    .await?,
            );
        }
        if let Some(trick) = session.trick_8x() {
            assets.insert(
                trick.playlist_relative_path().to_owned(),
                self.packager
                    .load_hls_asset(store, &session, trick.playlist_relative_path())
                    .await?,
            );
            for segment in trick.segments() {
                assets.insert(
                    segment.relative_path().to_owned(),
                    self.packager
                        .load_hls_asset(store, &session, segment.relative_path())
                        .await?,
                );
            }
        }
        let entry = RegisteredHlsClip {
            default_path: HLS_PLAYLIST_RELATIVE_PATH.to_owned(),
            assets,
        };
        self.hls_clips
            .lock()
            .map_err(|_| A2aLabError::unavailable("media session registry is unavailable"))?
            .insert(id, entry);
        Ok(())
    }

    fn registered_hls(&self, id: &VideoClipId) -> Result<Option<RegisteredHlsClip>, A2aLabError> {
        let guard = self
            .hls_clips
            .lock()
            .map_err(|_| A2aLabError::unavailable("media session registry is unavailable"))?;
        Ok(guard.get(id).map(|entry| RegisteredHlsClip {
            default_path: entry.default_path.clone(),
            assets: entry.assets.clone(),
        }))
    }
}

#[async_trait]
impl MediaAssetProvider for LocalMediaAssetProvider {
    async fn resolve_clip_asset<'life0, 'life1>(
        &'life0 self,
        request: ClipAssetRequest<'life1>,
    ) -> Result<ResolvedMediaAsset, A2aLabError>
    where
        'life0: 'life1,
        'life1: 'life0,
    {
        let relative = normalize_media_asset_path(request.relative_path)?;
        if let Some(entry) = self.registered_hls(request.id)? {
            let path = relative.as_deref().unwrap_or(entry.default_path.as_str());
            let asset = entry
                .assets
                .get(path)
                .cloned()
                .ok_or_else(|| A2aLabError::not_found("packaged asset", path.to_owned()))?;
            return Ok(ResolvedMediaAsset::Bytes(asset));
        }
        if relative.is_some() {
            return Err(A2aLabError::not_found(
                "packaged asset",
                relative.unwrap_or_default(),
            ));
        }
        let stream = self.lab.open_video_clip(request.id).await?;
        Ok(ResolvedMediaAsset::ClipStream(stream))
    }

    async fn resolve_live_asset<'life0, 'life1>(
        &'life0 self,
        request: LiveAssetRequest<'life1>,
    ) -> Result<ResolvedMediaAsset, A2aLabError>
    where
        'life0: 'life1,
        'life1: 'life0,
    {
        let relative = normalize_media_asset_path(request.relative_path)?;
        match self
            .lab
            .resolve_live_playback_asset(request.id, relative.as_deref())
            .await
        {
            Ok(asset) => Ok(asset),
            Err(error) if relative.is_none() && error.code() == "not_found" => {
                let stream = self.lab.open_video_follow(request.id).await?;
                Ok(ResolvedMediaAsset::LiveStream(stream))
            }
            Err(error) => Err(error),
        }
    }
}
