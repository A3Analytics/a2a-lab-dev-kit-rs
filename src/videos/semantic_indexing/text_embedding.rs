//! Bounded text-embedding provider contract for description and query text.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::videos::digest::Sha256Digest;
use crate::videos::index_provenance::{FrameEmbeddingVector, TextEmbeddingProvenance};
use crate::videos::model_space::{EmbeddingMetric, ModelSpaceIdentity, VectorNormalization};
use crate::videos::semantic_indexing::failure::SemanticIndexingFailure;
use crate::videos::semantic_indexing::frame_description::FrameDescriptionKey;

/// Resolved text-embedding settings independent from chat and Jev.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextEmbeddingProviderConfig {
    provider: String,
    model: String,
    model_space: ModelSpaceIdentity,
}

impl TextEmbeddingProviderConfig {
    /// Validates embedding provider configuration.
    pub fn new(
        provider: impl AsRef<str>,
        model: impl AsRef<str>,
        model_space: ModelSpaceIdentity,
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
        Ok(Self {
            provider: provider.as_ref().to_owned(),
            model: model.as_ref().to_owned(),
            model_space,
        })
    }

    /// Configured provider id.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Resolved embedding model.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Versioned retrieval space for vectors from this provider.
    #[must_use]
    pub const fn model_space(&self) -> &ModelSpaceIdentity {
        &self.model_space
    }
}

/// Finite bounds for text-embedding batches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextEmbeddingLimits {
    max_batch_items: u32,
    max_input_bytes: usize,
    max_input_tokens: u32,
    deadline: Duration,
}

impl TextEmbeddingLimits {
    /// Validates embedding limits.
    pub fn new(
        max_batch_items: u32,
        max_input_bytes: usize,
        max_input_tokens: u32,
        deadline: Duration,
    ) -> Result<Self, A2aLabError> {
        if max_batch_items == 0 {
            return Err(A2aLabError::invalid("max_batch_items", "must be positive"));
        }
        if max_input_bytes == 0 {
            return Err(A2aLabError::invalid("max_input_bytes", "must be positive"));
        }
        if max_input_tokens == 0 {
            return Err(A2aLabError::invalid("max_input_tokens", "must be positive"));
        }
        if deadline.is_zero() {
            return Err(A2aLabError::invalid("deadline", "must be positive"));
        }
        Ok(Self {
            max_batch_items,
            max_input_bytes,
            max_input_tokens,
            deadline,
        })
    }

    /// Conservative defaults for lab indexing workers.
    #[must_use]
    pub fn pilot() -> Self {
        Self {
            max_batch_items: 32,
            max_input_bytes: 32 * 1024,
            max_input_tokens: 512,
            deadline: Duration::from_secs(15),
        }
    }

    /// Maximum items per batch.
    #[must_use]
    pub const fn max_batch_items(self) -> u32 {
        self.max_batch_items
    }

    /// Maximum UTF-8 bytes per input string.
    #[must_use]
    pub const fn max_input_bytes(self) -> usize {
        self.max_input_bytes
    }

    /// Maximum declared tokens per input string.
    #[must_use]
    pub const fn max_input_tokens(self) -> u32 {
        self.max_input_tokens
    }

    /// Per-batch deadline.
    #[must_use]
    pub const fn deadline(self) -> Duration {
        self.deadline
    }
}

/// Stable key for one embedding output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum TextEmbeddingKey {
    /// Embedding of one persisted frame description.
    PersistedDescription(FrameDescriptionKey),
    /// Embedding of one search query string.
    SearchQuery(Sha256Digest),
}

/// One text input to embed.
#[derive(Debug, Clone, PartialEq)]
pub struct TextEmbeddingRequest {
    key: TextEmbeddingKey,
    text: String,
    token_count: u32,
}

impl TextEmbeddingRequest {
    /// Builds a request after limit checks.
    pub fn new(
        limits: TextEmbeddingLimits,
        key: TextEmbeddingKey,
        text: impl Into<String>,
        token_count: u32,
    ) -> Result<Self, SemanticIndexingFailure> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(SemanticIndexingFailure::EmptyDescription);
        }
        if text.len() > limits.max_input_bytes() {
            return Err(SemanticIndexingFailure::LimitExceeded {
                limit: "max_input_bytes".to_owned(),
                message: format!(
                    "text length {} exceeds {}",
                    text.len(),
                    limits.max_input_bytes()
                ),
            });
        }
        if token_count > limits.max_input_tokens() {
            return Err(SemanticIndexingFailure::LimitExceeded {
                limit: "max_input_tokens".to_owned(),
                message: format!(
                    "token_count {token_count} exceeds {}",
                    limits.max_input_tokens()
                ),
            });
        }
        Ok(Self {
            key,
            text,
            token_count,
        })
    }

    /// Output key.
    #[must_use]
    pub const fn key(&self) -> &TextEmbeddingKey {
        &self.key
    }

    /// Source text to embed.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Declared token count for accounting.
    #[must_use]
    pub fn token_count(self) -> u32 {
        self.token_count
    }
}

/// Provider output for one keyed text embedding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KeyedTextEmbedding {
    key: TextEmbeddingKey,
    model_space: ModelSpaceIdentity,
    provenance: TextEmbeddingProvenance,
    vector: FrameEmbeddingVector,
}

impl KeyedTextEmbedding {
    /// Builds a response without model-space checks (contract tests only).
    pub(crate) fn unchecked(
        key: TextEmbeddingKey,
        model_space: ModelSpaceIdentity,
        provenance: TextEmbeddingProvenance,
        vector: FrameEmbeddingVector,
    ) -> Self {
        Self {
            key,
            model_space,
            provenance,
            vector,
        }
    }

    /// Validates vector, provenance, and model-space alignment.
    pub fn new(
        config: &TextEmbeddingProviderConfig,
        key: TextEmbeddingKey,
        source_text: &str,
        values: Vec<f32>,
    ) -> Result<Self, SemanticIndexingFailure> {
        let space = config.model_space().clone();
        if values.len() != usize::try_from(space.dimensions()).unwrap_or(usize::MAX) {
            return Err(SemanticIndexingFailure::InvalidVector {
                message: format!(
                    "vector length {} does not match dimensions {}",
                    values.len(),
                    space.dimensions()
                ),
            });
        }
        validate_vector_metric(&values, space.normalization(), space.metric())?;
        let provenance = TextEmbeddingProvenance::new(
            config.provider(),
            config.model(),
            space.dimensions(),
            space.normalization(),
            space.metric(),
            space.generation(),
            Sha256Digest::hash_bytes(source_text.as_bytes()),
        )
        .map_err(|error| SemanticIndexingFailure::InvalidVector {
            message: error.to_string(),
        })?;
        if !provenance.matches_model_space(&space) {
            return Err(SemanticIndexingFailure::IncompatibleModelSpace {
                message: "embedding provenance does not match configured model space".to_owned(),
            });
        }
        let vector = FrameEmbeddingVector::new(values, space.dimensions()).map_err(|error| {
            SemanticIndexingFailure::InvalidVector {
                message: error.to_string(),
            }
        })?;
        Ok(Self {
            key,
            model_space: space,
            provenance,
            vector,
        })
    }

    /// Output key.
    #[must_use]
    pub const fn key(&self) -> &TextEmbeddingKey {
        &self.key
    }

    /// Declared retrieval space.
    #[must_use]
    pub const fn model_space(&self) -> &ModelSpaceIdentity {
        &self.model_space
    }

    /// Embedding provenance separate from description generation.
    #[must_use]
    pub const fn provenance(&self) -> &TextEmbeddingProvenance {
        &self.provenance
    }

    /// Stored vector components.
    #[must_use]
    pub const fn vector(&self) -> &FrameEmbeddingVector {
        &self.vector
    }
}

/// Rejects partial, duplicate, or cross-space batch responses.
pub fn validate_embedding_batch(
    config: &TextEmbeddingProviderConfig,
    limits: TextEmbeddingLimits,
    requests: &[TextEmbeddingRequest],
    responses: &[KeyedTextEmbedding],
) -> Result<Vec<KeyedTextEmbedding>, SemanticIndexingFailure> {
    if requests.len() > usize::try_from(limits.max_batch_items()).unwrap_or(usize::MAX) {
        return Err(SemanticIndexingFailure::LimitExceeded {
            limit: "max_batch_items".to_owned(),
            message: format!(
                "batch size {} exceeds {}",
                requests.len(),
                limits.max_batch_items()
            ),
        });
    }
    if responses.len() != requests.len() {
        return Err(SemanticIndexingFailure::PartialResponse {
            message: format!(
                "expected {} keyed embeddings, got {}",
                requests.len(),
                responses.len()
            ),
        });
    }

    let mut expected = std::collections::HashSet::new();
    for request in requests {
        let token = embedding_key_token(request.key());
        if !expected.insert(token) {
            return Err(SemanticIndexingFailure::KeyMismatch {
                message: "duplicate request key in embedding batch".to_owned(),
            });
        }
    }

    let mut matched = std::collections::HashSet::new();
    for response in responses {
        if response.model_space() != config.model_space() {
            return Err(SemanticIndexingFailure::IncompatibleModelSpace {
                message: "response model space does not match provider configuration".to_owned(),
            });
        }
        if !response
            .provenance()
            .matches_model_space(config.model_space())
        {
            return Err(SemanticIndexingFailure::IncompatibleModelSpace {
                message: "response provenance does not match configured model space".to_owned(),
            });
        }
        validate_vector_metric(
            response.vector().values(),
            config.model_space().normalization(),
            config.model_space().metric(),
        )?;
        let token = embedding_key_token(response.key());
        if !expected.contains(&token) {
            return Err(SemanticIndexingFailure::KeyMismatch {
                message: "response embedding key was not requested".to_owned(),
            });
        }
        if !matched.insert(token) {
            return Err(SemanticIndexingFailure::KeyMismatch {
                message: "duplicate response key in embedding batch".to_owned(),
            });
        }
    }

    if matched.len() != expected.len() {
        return Err(SemanticIndexingFailure::PartialResponse {
            message: "one or more requested embedding keys are missing".to_owned(),
        });
    }

    Ok(responses.to_vec())
}

fn embedding_key_token(key: &TextEmbeddingKey) -> String {
    match key {
        TextEmbeddingKey::PersistedDescription(description) => {
            format!(
                "desc:{}",
                super::frame_description::description_key_token(description)
            )
        }
        TextEmbeddingKey::SearchQuery(digest) => format!("query:{}", digest.as_str()),
    }
}

fn validate_vector_metric(
    values: &[f32],
    normalization: VectorNormalization,
    metric: EmbeddingMetric,
) -> Result<(), SemanticIndexingFailure> {
    if values.iter().copied().any(|value| !value.is_finite()) {
        return Err(SemanticIndexingFailure::InvalidVector {
            message: "components must be finite".to_owned(),
        });
    }
    match normalization {
        VectorNormalization::None => Ok(()),
        VectorNormalization::L2Unit => {
            let norm_sq: f32 = values.iter().map(|value| value * value).sum();
            let norm = norm_sq.sqrt();
            if norm == 0.0 {
                return Err(SemanticIndexingFailure::InvalidVector {
                    message: "l2_unit vector must not be zero".to_owned(),
                });
            }
            if (norm - 1.0).abs() > 1e-4 {
                return Err(SemanticIndexingFailure::InvalidVector {
                    message: format!("l2_unit norm {norm} is not unit length"),
                });
            }
            match metric {
                EmbeddingMetric::Cosine | EmbeddingMetric::Dot | EmbeddingMetric::Euclidean => {
                    Ok(())
                }
            }
        }
    }
}

/// Encodes persisted descriptions and query strings in one retrieval space.
///
/// This contract is text-only. It does not encode saved images and does not
/// append Jev decisions.
pub trait TextEmbeddingProvider: Send + Sync {
    /// Resolved configuration for this provider instance.
    fn config(&self) -> &TextEmbeddingProviderConfig;

    /// Active limits for this provider instance.
    fn limits(&self) -> TextEmbeddingLimits;

    /// Embeds persisted description text and search query strings.
    fn embed_texts(
        &self,
        requests: Vec<TextEmbeddingRequest>,
    ) -> impl std::future::Future<Output = Result<Vec<KeyedTextEmbedding>, SemanticIndexingFailure>> + Send;
}
