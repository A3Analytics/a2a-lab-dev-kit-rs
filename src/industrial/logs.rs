//! Catalog-backed log provider.

use std::sync::Arc;

use crate::catalog::{AssetCatalogProvider, BindingRole, ListBindingsRequest};
use crate::error::A2aLabError;
use crate::id::SourceId;
use crate::industrial::LiveSource;
use crate::logs::{ListLogSourcesRequest, LogProvider, LogRecord, LogSource, QueryLogsRequest};
use crate::page::{Page, PageRequest};

/// Lists log sources from the catalog and reads them from the bound protocol.
pub struct IndustrialLogs<C, L> {
    catalog: Arc<C>,
    live: Arc<L>,
}

impl<C, L> IndustrialLogs<C, L> {
    /// Creates a provider over a shared catalog and live source.
    #[must_use]
    pub const fn new(catalog: Arc<C>, live: Arc<L>) -> Self {
        Self { catalog, live }
    }
}

impl<C, L> LogProvider for IndustrialLogs<C, L>
where
    C: AssetCatalogProvider,
    L: LiveSource,
{
    async fn list_sources(
        &self,
        request: ListLogSourcesRequest,
    ) -> Result<Page<LogSource>, A2aLabError> {
        let bindings = self
            .catalog
            .list_bindings(ListBindingsRequest {
                page: PageRequest::new(None, crate::page::MAX_PAGE_LIMIT)?,
            })
            .await?;
        let mut sources = Vec::new();
        for binding in bindings.items() {
            if binding.role() != BindingRole::LogSource {
                continue;
            }
            sources.push(LogSource {
                id: SourceId::new(binding.lab_id())?,
                name: binding.lab_id().to_owned(),
                description: binding.semantic_id().as_str().to_owned(),
                asset_id: Some(binding.asset_key().as_str().to_owned()),
                semantic_id: Some(binding.semantic_id().as_str().to_owned()),
            });
        }
        crate::page::slice_page(&sources, &request.page)
    }

    async fn query(&self, request: QueryLogsRequest) -> Result<Page<LogRecord>, A2aLabError> {
        let binding = find_binding(
            &*self.catalog,
            request.source_id.as_str(),
            BindingRole::LogSource,
        )
        .await?;
        self.live
            .query_logs(binding.endpoint(), request.range, request.page)
            .await
    }
}

pub(crate) async fn find_binding<C: AssetCatalogProvider>(
    catalog: &C,
    lab_id: &str,
    role: BindingRole,
) -> Result<crate::catalog::Binding, A2aLabError> {
    let bindings = catalog
        .list_bindings(ListBindingsRequest {
            page: PageRequest::new(None, crate::page::MAX_PAGE_LIMIT)?,
        })
        .await?;
    bindings
        .items()
        .iter()
        .find(|binding| binding.lab_id() == lab_id && binding.role() == role)
        .cloned()
        .ok_or_else(|| A2aLabError::not_found("binding", lab_id))
}
