//! Catalog provider interface.

use crate::catalog::{Asset, AssetKey, Binding, ListAssetsRequest, ListBindingsRequest};
use crate::error::SdkError;
use crate::page::Page;

/// Reads the authoritative asset and binding catalog.
pub trait AssetCatalogProvider: Send + Sync {
    /// Returns a page of assets.
    fn list_assets(
        &self,
        request: ListAssetsRequest,
    ) -> impl Future<Output = Result<Page<Asset>, SdkError>> + Send;

    /// Returns one asset.
    fn get_asset(&self, key: &AssetKey) -> impl Future<Output = Result<Asset, SdkError>> + Send;

    /// Returns a page of bindings across all assets.
    fn list_bindings(
        &self,
        request: ListBindingsRequest,
    ) -> impl Future<Output = Result<Page<Binding>, SdkError>> + Send;
}
