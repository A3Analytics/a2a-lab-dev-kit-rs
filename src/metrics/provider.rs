//! Metric provider interface.

use crate::error::A2aLabError;
use crate::metrics::{ListMetricsRequest, MetricDescriptor, MetricPoint, QueryMetricRequest};
use crate::page::Page;

/// Lists metrics and reads their samples.
pub trait MetricProvider: Send + Sync {
    /// Returns the metrics this agent can read.
    fn list_metrics(
        &self,
        request: ListMetricsRequest,
    ) -> impl Future<Output = Result<Page<MetricDescriptor>, A2aLabError>> + Send;

    /// Returns samples for one metric in the requested interval.
    fn query(
        &self,
        request: QueryMetricRequest,
    ) -> impl Future<Output = Result<Page<MetricPoint>, A2aLabError>> + Send;
}
