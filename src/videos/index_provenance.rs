//! Description and text-embedding provenance separate from vector payloads.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::videos::digest::Sha256Digest;
use crate::videos::model_space::{
    EmbeddingMetric, ModelSpaceIdentity, ProvenanceGeneration, VectorNormalization,
};

/// Normalized ROI `[x_min, y_min, x_max, y_max]` on the oriented saved frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameRoiTransform {
    bounds: [f32; 4],
}

impl FrameRoiTransform {
    /// Full oriented frame (`[0, 0, 1, 1]`).
    #[must_use]
    pub fn full_frame() -> Self {
        Self {
            bounds: [0.0, 0.0, 1.0, 1.0],
        }
    }

    /// Validates normalized bounds.
    pub fn new(bounds: [f32; 4]) -> Result<Self, A2aLabError> {
        let [x_min, y_min, x_max, y_max] = bounds;
        if !(0.0..=1.0).contains(&x_min)
            || !(0.0..=1.0).contains(&y_min)
            || !(0.0..=1.0).contains(&x_max)
            || !(0.0..=1.0).contains(&y_max)
            || x_min >= x_max
            || y_min >= y_max
        {
            return Err(A2aLabError::invalid(
                "roi",
                "bounds must be normalized with min strictly before max",
            ));
        }
        Ok(Self { bounds })
    }

    /// Normalized bounds.
    #[must_use]
    pub const fn bounds(self) -> [f32; 4] {
        self.bounds
    }
}

/// Luna (or configured) frame description provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDescriptionProvenance {
    provider: String,
    resolved_model: String,
    prompt_digest: Sha256Digest,
    preprocessing_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    frame_roi: Option<FrameRoiTransform>,
    text_hash: Sha256Digest,
    generation: ProvenanceGeneration,
}

impl FrameDescriptionProvenance {
    /// Validates description-side provenance.
    pub fn new(
        provider: impl AsRef<str>,
        resolved_model: impl AsRef<str>,
        prompt_digest: Sha256Digest,
        preprocessing_digest: Sha256Digest,
        frame_roi: Option<FrameRoiTransform>,
        text_hash: Sha256Digest,
        generation: ProvenanceGeneration,
    ) -> Result<Self, A2aLabError> {
        if provider.as_ref().trim().is_empty() {
            return Err(A2aLabError::invalid(
                "provider",
                "description provider must not be empty",
            ));
        }
        if resolved_model.as_ref().trim().is_empty() {
            return Err(A2aLabError::invalid(
                "resolved_model",
                "description model must not be empty",
            ));
        }
        Ok(Self {
            provider: provider.as_ref().to_owned(),
            resolved_model: resolved_model.as_ref().to_owned(),
            prompt_digest,
            preprocessing_digest,
            frame_roi,
            text_hash,
            generation,
        })
    }

    /// Description provider id.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Resolved description model.
    #[must_use]
    pub fn resolved_model(&self) -> &str {
        &self.resolved_model
    }

    /// Versioned prompt digest.
    #[must_use]
    pub const fn prompt_digest(&self) -> &Sha256Digest {
        &self.prompt_digest
    }

    /// Digest of bounded preprocessing applied before description.
    #[must_use]
    pub const fn preprocessing_digest(&self) -> &Sha256Digest {
        &self.preprocessing_digest
    }

    /// Optional frame/ROI transform applied before description.
    #[must_use]
    pub const fn frame_roi(&self) -> Option<FrameRoiTransform> {
        self.frame_roi
    }

    /// Hash of persisted description text.
    #[must_use]
    pub const fn text_hash(&self) -> &Sha256Digest {
        &self.text_hash
    }

    /// Description generation for this revision.
    #[must_use]
    pub const fn generation(&self) -> ProvenanceGeneration {
        self.generation
    }
}

/// Text-embedding provenance independent from description generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextEmbeddingProvenance {
    provider: String,
    model: String,
    dimensions: u32,
    normalization: VectorNormalization,
    metric: EmbeddingMetric,
    generation: ProvenanceGeneration,
    source_text_hash: Sha256Digest,
}

impl TextEmbeddingProvenance {
    /// Validates embedding-side provenance.
    pub fn new(
        provider: impl AsRef<str>,
        model: impl AsRef<str>,
        dimensions: u32,
        normalization: VectorNormalization,
        metric: EmbeddingMetric,
        generation: ProvenanceGeneration,
        source_text_hash: Sha256Digest,
    ) -> Result<Self, A2aLabError> {
        if provider.as_ref().trim().is_empty() {
            return Err(A2aLabError::invalid(
                "provider",
                "embedding provider must not be empty",
            ));
        }
        if model.as_ref().trim().is_empty() {
            return Err(A2aLabError::invalid(
                "model",
                "embedding model must not be empty",
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
            source_text_hash,
        })
    }

    /// Embedding provider id.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Resolved embedding model.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Output dimensionality.
    #[must_use]
    pub fn dimensions(&self) -> u32 {
        self.dimensions
    }

    /// Stored normalization.
    #[must_use]
    pub fn normalization(&self) -> VectorNormalization {
        self.normalization
    }

    /// Index metric.
    #[must_use]
    pub fn metric(&self) -> EmbeddingMetric {
        self.metric
    }

    /// Embedding generation counter.
    #[must_use]
    pub fn generation(&self) -> ProvenanceGeneration {
        self.generation
    }

    /// Hash of the embedded text.
    #[must_use]
    pub const fn source_text_hash(&self) -> &Sha256Digest {
        &self.source_text_hash
    }

    /// Checks compatibility with the declared model space.
    #[must_use]
    pub fn matches_model_space(&self, space: &ModelSpaceIdentity) -> bool {
        self.provider == space.provider()
            && self.model == space.model()
            && self.dimensions == space.dimensions()
            && self.normalization == space.normalization()
            && self.metric == space.metric()
    }
}

/// One validated vector aligned to a model space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameEmbeddingVector {
    values: Vec<f32>,
}

impl FrameEmbeddingVector {
    /// Validates a vector for `dimensions`.
    pub fn new(values: Vec<f32>, dimensions: u32) -> Result<Self, A2aLabError> {
        validate_vector_values(&values, dimensions)?;
        Ok(Self { values })
    }

    /// Stored components.
    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

/// Vector index attachment binding frame provenance and embeddings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameVectorIndexAttachment {
    model_space: ModelSpaceIdentity,
    description: FrameDescriptionProvenance,
    embedding: TextEmbeddingProvenance,
    vector: FrameEmbeddingVector,
}

impl FrameVectorIndexAttachment {
    /// Creates an attachment after full cross-field validation.
    pub fn new(
        model_space: ModelSpaceIdentity,
        description: FrameDescriptionProvenance,
        embedding: TextEmbeddingProvenance,
        vector: FrameEmbeddingVector,
    ) -> Result<Self, A2aLabError> {
        let candidate = Self {
            model_space,
            description,
            embedding,
            vector,
        };
        candidate.validate()?;
        Ok(candidate)
    }

    /// Validates provenance separation, dimensions, and generation coherence.
    pub fn validate(&self) -> Result<(), A2aLabError> {
        if !self.embedding.matches_model_space(&self.model_space) {
            return Err(A2aLabError::invalid(
                "embedding",
                "provenance does not match the declared model space",
            ));
        }
        if self.embedding.generation() != self.model_space.generation() {
            return Err(A2aLabError::invalid(
                "generation",
                "embedding generation must match model-space generation",
            ));
        }
        if self.embedding.source_text_hash() != self.description.text_hash() {
            return Err(A2aLabError::invalid(
                "source_text_hash",
                "embedding source text must match description text hash",
            ));
        }
        validate_vector_values(self.vector.values(), self.model_space.dimensions())?;
        Ok(())
    }

    /// Target index namespace.
    #[must_use]
    pub const fn model_space(&self) -> &ModelSpaceIdentity {
        &self.model_space
    }

    /// Description provenance carried with the vector.
    #[must_use]
    pub const fn description(&self) -> &FrameDescriptionProvenance {
        &self.description
    }

    /// Text-embedding provenance.
    #[must_use]
    pub const fn embedding(&self) -> &TextEmbeddingProvenance {
        &self.embedding
    }

    /// Stored vector payload.
    #[must_use]
    pub const fn vector(&self) -> &FrameEmbeddingVector {
        &self.vector
    }
}

fn validate_vector_values(values: &[f32], dimensions: u32) -> Result<(), A2aLabError> {
    let expected = usize::try_from(dimensions).map_err(|_| {
        A2aLabError::invalid("dimensions", "vector dimension count overflowed usize")
    })?;
    if values.len() != expected {
        return Err(A2aLabError::invalid(
            "vector",
            format!("length must be {expected}"),
        ));
    }
    if values.iter().copied().any(|value| !value.is_finite()) {
        return Err(A2aLabError::invalid(
            "vector",
            "components must be finite numbers",
        ));
    }
    Ok(())
}
