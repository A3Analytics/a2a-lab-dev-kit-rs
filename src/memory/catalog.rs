//! In-memory asset catalog.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::catalog::{
    Asset, AssetCatalogProvider, AssetKey, Binding, ListAssetsRequest, ListBindingsRequest,
};
use crate::error::SdkError;
use crate::page::{Page, slice_page};

#[derive(Default)]
struct CatalogState {
    assets: Vec<Asset>,
}

/// In-memory [`AssetCatalogProvider`].
#[derive(Clone, Default)]
pub struct MemoryCatalog {
    inner: Arc<Mutex<CatalogState>>,
}

impl MemoryCatalog {
    /// Creates an empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an asset. A second insert with the same key replaces it.
    pub async fn insert(&self, asset: Asset) {
        let mut state = self.inner.lock().await;
        if let Some(existing) = state
            .assets
            .iter_mut()
            .find(|current| current.key() == asset.key())
        {
            *existing = asset;
        } else {
            state.assets.push(asset);
        }
    }
}

impl AssetCatalogProvider for MemoryCatalog {
    async fn list_assets(&self, request: ListAssetsRequest) -> Result<Page<Asset>, SdkError> {
        let state = self.inner.lock().await;
        let mut assets = state.assets.clone();
        assets.sort_by(|left, right| left.key().as_str().cmp(right.key().as_str()));
        slice_page(&assets, &request.page)
    }

    async fn get_asset(&self, key: &AssetKey) -> Result<Asset, SdkError> {
        self.inner
            .lock()
            .await
            .assets
            .iter()
            .find(|asset| asset.key() == key)
            .cloned()
            .ok_or_else(|| SdkError::not_found("asset", key.to_string()))
    }

    async fn list_bindings(&self, request: ListBindingsRequest) -> Result<Page<Binding>, SdkError> {
        let state = self.inner.lock().await;
        let mut bindings: Vec<_> = state
            .assets
            .iter()
            .flat_map(|asset| asset.bindings().iter().cloned())
            .collect();
        bindings.sort_by(|left, right| left.lab_id().cmp(right.lab_id()));
        slice_page(&bindings, &request.page)
    }
}
