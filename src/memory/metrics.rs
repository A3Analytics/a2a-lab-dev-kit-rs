//! In-memory metric provider.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::error::SdkError;
use crate::metrics::{
    ListMetricsRequest, MetricDescriptor, MetricPoint, MetricProvider, QueryMetricRequest,
};
use crate::page::{Page, slice_page};

#[derive(Default)]
struct MetricState {
    metrics: Vec<MetricDescriptor>,
    points: Vec<(crate::id::MetricId, MetricPoint)>,
    unavailable: Option<String>,
}

/// In-memory [`MetricProvider`] for examples and tests.
#[derive(Clone, Default)]
pub struct MemoryMetrics {
    inner: Arc<Mutex<MetricState>>,
}

impl MemoryMetrics {
    /// Creates an empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a metric descriptor.
    pub async fn insert_metric(&self, metric: MetricDescriptor) {
        self.inner.lock().await.metrics.push(metric);
    }

    /// Adds a sample for an existing metric.
    pub async fn insert_point(
        &self,
        metric_id: crate::id::MetricId,
        point: MetricPoint,
    ) -> Result<(), SdkError> {
        let mut state = self.inner.lock().await;
        if !state.metrics.iter().any(|metric| metric.id == metric_id) {
            return Err(SdkError::not_found("metric", metric_id.to_string()));
        }
        state.points.push((metric_id, point));
        Ok(())
    }

    /// Makes later calls fail until [`Self::clear_unavailable`].
    pub async fn set_unavailable(&self, message: impl Into<String>) {
        self.inner.lock().await.unavailable = Some(message.into());
    }

    /// Clears a simulated provider failure.
    pub async fn clear_unavailable(&self) {
        self.inner.lock().await.unavailable = None;
    }
}

impl MetricProvider for MemoryMetrics {
    async fn list_metrics(
        &self,
        request: ListMetricsRequest,
    ) -> Result<Page<MetricDescriptor>, SdkError> {
        let state = self.inner.lock().await;
        unavailable(state.unavailable.as_deref())?;
        let mut metrics = state.metrics.clone();
        metrics.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        slice_page(&metrics, &request.page)
    }

    async fn query(&self, request: QueryMetricRequest) -> Result<Page<MetricPoint>, SdkError> {
        request.range.check()?;
        let state = self.inner.lock().await;
        unavailable(state.unavailable.as_deref())?;
        if !state
            .metrics
            .iter()
            .any(|metric| metric.id == request.metric_id)
        {
            return Err(SdkError::not_found("metric", request.metric_id.to_string()));
        }
        let mut points: Vec<_> = state
            .points
            .iter()
            .filter(|(metric_id, point)| {
                metric_id == &request.metric_id && request.range.contains(point.timestamp)
            })
            .map(|(_, point)| *point)
            .collect();
        points.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.value.total_cmp(&right.value))
        });
        slice_page(&points, &request.page)
    }
}

fn unavailable(message: Option<&str>) -> Result<(), SdkError> {
    match message {
        Some(message) => Err(SdkError::unavailable(message)),
        None => Ok(()),
    }
}
