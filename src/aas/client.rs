//! IDTA AAS 3.2 HTTP catalog client.

use std::sync::Arc;

use reqwest::Url;
use tokio::sync::Mutex;

use crate::aas::codec::{assets_from_shells, base64url, shells, supports_repository};
use crate::aas::token::AccessTokenSource;
use crate::catalog::{
    Asset, AssetCatalogProvider, AssetKey, Binding, ListAssetsRequest, ListBindingsRequest,
};
use crate::error::SdkError;
use crate::page::{Page, slice_page};

/// Client for an AAS repository, registry descriptors, and submodel repository.
pub struct AasClient<T> {
    http: reqwest::Client,
    base: Url,
    tokens: T,
    cache: Mutex<Option<Arc<[Asset]>>>,
}

impl<T: AccessTokenSource> AasClient<T> {
    /// Creates a client for `base_url`.
    pub fn new(base_url: &str, tokens: T) -> Result<Self, SdkError> {
        let base =
            Url::parse(base_url).map_err(|error| SdkError::invalid("url", error.to_string()))?;
        let http = reqwest::Client::builder()
            .build()
            .map_err(|error| SdkError::transport(error.to_string()))?;
        Ok(Self {
            http,
            base,
            tokens,
            cache: Mutex::new(None),
        })
    }

    async fn assets(&self) -> Result<Arc<[Asset]>, SdkError> {
        if let Some(assets) = self.cache.lock().await.clone() {
            return Ok(assets);
        }
        let description = self.get_json("description").await?;
        if !supports_repository(&description) {
            return Err(SdkError::protocol(
                "AAS repository does not advertise the 3.2 shell repository profile",
            ));
        }
        let document = self.get_json("shells").await?;
        let shell_values = shells(&document);
        let mut submodels = Vec::new();
        for shell in &shell_values {
            for identifier in submodel_identifiers(shell) {
                let path = format!("submodels/{}", base64url(identifier));
                submodels.push(self.get_json(&path).await?);
            }
        }
        let assets = Arc::<[Asset]>::from(assets_from_shells(&shell_values, &submodels)?);
        *self.cache.lock().await = Some(Arc::clone(&assets));
        Ok(assets)
    }

    async fn get_json(&self, path: &str) -> Result<serde_json::Value, SdkError> {
        let mut request = self.http.get(self.url(path)?);
        if let Some(token) = self.tokens.bearer_token().await? {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|error| SdkError::transport(error.to_string()))?;
        let status = response.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(SdkError::protocol(
                "AAS repository rejected the access token",
            ));
        }
        if status.as_u16() == 404 {
            return Err(SdkError::not_found("aas resource", path));
        }
        if !status.is_success() {
            return Err(SdkError::unavailable(format!(
                "AAS repository returned {}",
                status.as_u16()
            )));
        }
        response
            .json()
            .await
            .map_err(|error| SdkError::protocol(error.to_string()))
    }

    fn url(&self, path: &str) -> Result<Url, SdkError> {
        self.base
            .join(path)
            .map_err(|error| SdkError::invalid("url", error.to_string()))
    }
}

impl<T: AccessTokenSource> AssetCatalogProvider for AasClient<T> {
    async fn list_assets(&self, request: ListAssetsRequest) -> Result<Page<Asset>, SdkError> {
        slice_page(&self.assets().await?, &request.page)
    }

    async fn get_asset(&self, key: &AssetKey) -> Result<Asset, SdkError> {
        self.assets()
            .await?
            .iter()
            .find(|asset| asset.key() == key)
            .cloned()
            .ok_or_else(|| SdkError::not_found("asset", key.to_string()))
    }

    async fn list_bindings(&self, request: ListBindingsRequest) -> Result<Page<Binding>, SdkError> {
        let assets = self.assets().await?;
        let bindings: Vec<_> = assets
            .iter()
            .flat_map(|asset| asset.bindings().iter().cloned())
            .collect();
        slice_page(&bindings, &request.page)
    }
}

fn submodel_identifiers(shell: &serde_json::Value) -> Vec<&str> {
    shell
        .get("submodels")
        .and_then(serde_json::Value::as_array)
        .map(|references| {
            references
                .iter()
                .filter_map(|reference| {
                    reference
                        .pointer("/keys/0/value")
                        .and_then(serde_json::Value::as_str)
                })
                .collect()
        })
        .unwrap_or_default()
}
