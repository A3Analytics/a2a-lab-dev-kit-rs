//! Versioned text-embedding model space identity for vector index boundaries.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;

/// Non-zero provenance generation for description or embedding pipelines.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub struct ProvenanceGeneration(u64);

impl ProvenanceGeneration {
    /// Creates a generation identifier.
    pub fn new(value: u64) -> Result<Self, A2aLabError> {
        if value == 0 {
            return Err(A2aLabError::invalid(
                "generation",
                "must be greater than zero",
            ));
        }
        Ok(Self(value))
    }

    /// Generation counter.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Vector normalization applied before indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VectorNormalization {
    /// Values are stored without normalization.
    None,
    /// L2 unit normalization.
    L2Unit,
}

/// Similarity metric for a vector index space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingMetric {
    /// Cosine similarity.
    Cosine,
    /// Inner product on normalized vectors.
    Dot,
    /// Euclidean distance.
    Euclidean,
}

/// Identity of one compatible text-embedding index namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelSpaceIdentity {
    provider: String,
    model: String,
    dimensions: u32,
    normalization: VectorNormalization,
    metric: EmbeddingMetric,
    generation: ProvenanceGeneration,
}

impl ModelSpaceIdentity {
    /// Validates model-space fields.
    pub fn new(
        provider: impl AsRef<str>,
        model: impl AsRef<str>,
        dimensions: u32,
        normalization: VectorNormalization,
        metric: EmbeddingMetric,
        generation: ProvenanceGeneration,
    ) -> Result<Self, A2aLabError> {
        if provider.as_ref().trim().is_empty() {
            return Err(A2aLabError::invalid(
                "provider",
                "model-space provider must not be empty",
            ));
        }
        if model.as_ref().trim().is_empty() {
            return Err(A2aLabError::invalid(
                "model",
                "model-space model must not be empty",
            ));
        }
        if dimensions == 0 || dimensions > 16_384 {
            return Err(A2aLabError::invalid("dimensions", "must be in 1..=16384"));
        }
        Ok(Self {
            provider: provider.as_ref().to_owned(),
            model: model.as_ref().to_owned(),
            dimensions,
            normalization,
            metric,
            generation,
        })
    }

    /// Configured embedding provider.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Resolved embedding model name.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Vector dimensionality for this space.
    #[must_use]
    pub fn dimensions(&self) -> u32 {
        self.dimensions
    }

    /// Stored normalization policy.
    #[must_use]
    pub fn normalization(&self) -> VectorNormalization {
        self.normalization
    }

    /// Index similarity metric.
    #[must_use]
    pub fn metric(&self) -> EmbeddingMetric {
        self.metric
    }

    /// Space generation for cutover and re-embedding.
    #[must_use]
    pub fn generation(&self) -> ProvenanceGeneration {
        self.generation
    }
}
