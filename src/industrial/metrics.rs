//! Catalog-backed metric provider.

use std::sync::Arc;

use crate::catalog::{AssetCatalogProvider, BindingRole, ListBindingsRequest};
use crate::error::SdkError;
use crate::id::MetricId;
use crate::industrial::LiveSource;
use crate::industrial::logs::find_binding;
use crate::metrics::{
    ListMetricsRequest, MetricDescriptor, MetricPoint, MetricProvider, QueryMetricRequest,
};
use crate::page::{Page, PageRequest};

/// Lists metrics from the catalog and reads samples from the bound protocol.
pub struct IndustrialMetrics<C, L> {
    catalog: Arc<C>,
    live: Arc<L>,
}

impl<C, L> IndustrialMetrics<C, L> {
    /// Creates a provider over a shared catalog and live source.
    #[must_use]
    pub const fn new(catalog: Arc<C>, live: Arc<L>) -> Self {
        Self { catalog, live }
    }
}

impl<C, L> MetricProvider for IndustrialMetrics<C, L>
where
    C: AssetCatalogProvider,
    L: LiveSource,
{
    async fn list_metrics(
        &self,
        request: ListMetricsRequest,
    ) -> Result<Page<MetricDescriptor>, SdkError> {
        let bindings = self
            .catalog
            .list_bindings(ListBindingsRequest {
                page: PageRequest::new(None, crate::page::MAX_PAGE_LIMIT)?,
            })
            .await?;
        let mut metrics = Vec::new();
        for binding in bindings.items() {
            if binding.role() != BindingRole::Metric {
                continue;
            }
            metrics.push(MetricDescriptor {
                id: MetricId::new(binding.lab_id())?,
                name: binding.lab_id().to_owned(),
                description: binding.semantic_id().as_str().to_owned(),
                unit: String::new(),
                asset_id: Some(binding.asset_key().as_str().to_owned()),
                semantic_id: Some(binding.semantic_id().as_str().to_owned()),
            });
        }
        crate::page::slice_page(&metrics, &request.page)
    }

    async fn query(&self, request: QueryMetricRequest) -> Result<Page<MetricPoint>, SdkError> {
        let binding = find_binding(
            &*self.catalog,
            request.metric_id.as_str(),
            BindingRole::Metric,
        )
        .await?;
        self.live
            .query_metrics(binding.endpoint(), request.range, request.page)
            .await
    }
}
