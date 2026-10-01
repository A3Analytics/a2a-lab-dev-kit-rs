//! Catalog provider interface.

use crate::catalog::{Asset, AssetKey, Binding, ListAssetsRequest, ListBindingsRequest};
use crate::error::A2aLabError;
use crate::page::Page;

/// Reads the authoritative asset and binding catalog.
pub trait AssetCatalogProvider: Send + Sync {
    /// Returns a page of assets.
    fn list_assets(
        &self,
        request: ListAssetsRequest,
    ) -> impl Future<Output = Result<Page<Asset>, A2aLabError>> + Send;

    /// Returns one asset.
    fn get_asset(&self, key: &AssetKey) -> impl Future<Output = Result<Asset, A2aLabError>> + Send;

    /// Returns a page of bindings across all assets.
    fn list_bindings(
        &self,
        request: ListBindingsRequest,
    ) -> impl Future<Output = Result<Page<Binding>, A2aLabError>> + Send;
}
