//! Metric catalog and sample types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::SdkError;
use crate::id::MetricId;
use crate::page::PageRequest;
use crate::time::{TimeRange, UtcTimestamp};

/// Description of a metric the agent can query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MetricDescriptor {
    /// Stable metric identifier.
    pub id: MetricId,
    /// Human-readable name.
    pub name: String,
    /// What the metric measures.
    pub description: String,
    /// Unit of `value`, such as `ms` or `count`.
    pub unit: String,
}

/// One finite sample in a time series.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, JsonSchema)]
pub struct MetricPoint {
    /// When the sample was taken.
    pub timestamp: UtcTimestamp,
    /// Finite numeric value.
    pub value: f64,
}

impl<'de> Deserialize<'de> for MetricPoint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            timestamp: UtcTimestamp,
            value: f64,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.timestamp, raw.value).map_err(serde::de::Error::custom)
    }
}

impl MetricPoint {
    /// Creates a sample when `value` is finite.
    pub fn new(timestamp: UtcTimestamp, value: f64) -> Result<Self, SdkError> {
        if !value.is_finite() {
            return Err(SdkError::invalid("value", "must be finite"));
        }
        Ok(Self { timestamp, value })
    }
}

/// Request for a page of metric descriptors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListMetricsRequest {
    /// Page bounds.
    pub page: PageRequest,
}

/// Request for samples of one metric in a half-open time range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct QueryMetricRequest {
    /// Metric to read.
    pub metric_id: MetricId,
    /// Half-open UTC interval.
    pub range: TimeRange,
    /// Page bounds.
    pub page: PageRequest,
}
