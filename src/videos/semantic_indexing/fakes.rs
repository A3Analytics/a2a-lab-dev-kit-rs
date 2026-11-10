//! Deterministic provider doubles for semantic indexing contract tests.

use crate::videos::digest::Sha256Digest;
use crate::videos::index_provenance::{FrameEmbeddingVector, TextEmbeddingProvenance};
use crate::videos::semantic_indexing::failure::SemanticIndexingFailure;
use crate::videos::semantic_indexing::frame_description::{
    FrameDescriptionProvider, FrameDescriptionProviderConfig, FrameDescriptionRequest,
    KeyedFrameDescription,
};
use crate::videos::semantic_indexing::text_embedding::{
    KeyedTextEmbedding, TextEmbeddingProvider, TextEmbeddingProviderConfig, TextEmbeddingRequest,
};

/// Scripted failure modes for [`FakeFrameDescriptionProvider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeDescriptionBehavior {
    /// Returns one keyed description per request.
    Ok,
    /// Omits the final keyed result.
    Partial,
    /// Swaps keys between the first two responses when the batch has at least two items.
    ShuffledKeys,
    /// Returns whitespace-only text.
    Empty,
    /// Returns text larger than configured output limits.
    Oversized,
    /// Returns [`SemanticIndexingFailure::DeadlineExceeded`].
    Timeout,
}

/// Deterministic frame-description double with no decision authority.
#[derive(Debug, Clone)]
pub struct FakeFrameDescriptionProvider {
    config: FrameDescriptionProviderConfig,
    limits: crate::videos::semantic_indexing::frame_description::FrameDescriptionLimits,
    behavior: FakeDescriptionBehavior,
    text_template: String,
}

impl FakeFrameDescriptionProvider {
    /// Creates a fake that emits `desc:{frame_id}` text by default.
    #[must_use]
    pub fn new(
        config: FrameDescriptionProviderConfig,
        limits: crate::videos::semantic_indexing::frame_description::FrameDescriptionLimits,
        behavior: FakeDescriptionBehavior,
    ) -> Self {
        Self {
            config,
            limits,
            behavior,
            text_template: "desc".to_owned(),
        }
    }

    /// Overrides the description prefix used for successful responses.
    #[must_use]
    pub fn with_text_template(mut self, text_template: impl Into<String>) -> Self {
        self.text_template = text_template.into();
        self
    }
}

#[allow(clippy::unused_async_trait_impl)]
impl FrameDescriptionProvider for FakeFrameDescriptionProvider {
    fn config(&self) -> &FrameDescriptionProviderConfig {
        &self.config
    }

    fn limits(
        &self,
    ) -> crate::videos::semantic_indexing::frame_description::FrameDescriptionLimits {
        self.limits
    }

    async fn describe_frames(
        &self,
        requests: Vec<FrameDescriptionRequest>,
    ) -> Result<Vec<KeyedFrameDescription>, SemanticIndexingFailure> {
        match self.behavior {
            FakeDescriptionBehavior::Timeout => Err(SemanticIndexingFailure::DeadlineExceeded),
            FakeDescriptionBehavior::Partial => {
                if requests.is_empty() {
                    return Ok(Vec::new());
                }
                let only = Self::ok_item(self, &requests[0])?;
                Ok(vec![only])
            }
            FakeDescriptionBehavior::ShuffledKeys if requests.len() >= 2 => {
                let first = Self::ok_item(self, &requests[0])?;
                let miskeyed = KeyedFrameDescription::unchecked(
                    self.config(),
                    requests[0].key().clone(),
                    format!("desc:{}", requests[1].key().frame().frame_id()),
                );
                Ok(vec![first, miskeyed])
            }
            FakeDescriptionBehavior::Empty => {
                let key = requests
                    .first()
                    .ok_or(SemanticIndexingFailure::PartialResponse {
                        message: "empty batch".to_owned(),
                    })?
                    .key()
                    .clone();
                Ok(vec![KeyedFrameDescription::unchecked(
                    self.config(),
                    key,
                    "   ".to_owned(),
                )])
            }
            FakeDescriptionBehavior::Oversized => {
                let key = requests
                    .first()
                    .ok_or(SemanticIndexingFailure::PartialResponse {
                        message: "empty batch".to_owned(),
                    })?
                    .key()
                    .clone();
                let text = "x".repeat(self.limits.max_output_bytes() + 1);
                Ok(vec![KeyedFrameDescription::unchecked(
                    self.config(),
                    key,
                    text,
                )])
            }
            FakeDescriptionBehavior::Ok | FakeDescriptionBehavior::ShuffledKeys => requests
                .iter()
                .map(|request| Self::ok_item(self, request))
                .collect(),
        }
    }
}

impl FakeFrameDescriptionProvider {
    fn ok_item(
        provider: &Self,
        request: &FrameDescriptionRequest,
    ) -> Result<KeyedFrameDescription, SemanticIndexingFailure> {
        let frame_id = request.key().frame().frame_id();
        let text = format!("{}:{frame_id}", provider.text_template);
        KeyedFrameDescription::new(
            provider.limits,
            provider.config(),
            request.key().clone(),
            text,
        )
    }
}

/// Scripted failure modes for [`FakeTextEmbeddingProvider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeEmbeddingBehavior {
    /// Deterministic unit vectors from source-text hashes.
    Ok,
    /// Reports a different model space than configured.
    WrongModelSpace,
    /// Returns [`SemanticIndexingFailure::DeadlineExceeded`].
    Timeout,
}

/// Deterministic text-embedding double with no decision authority.
#[derive(Debug, Clone)]
pub struct FakeTextEmbeddingProvider {
    config: TextEmbeddingProviderConfig,
    limits: crate::videos::semantic_indexing::text_embedding::TextEmbeddingLimits,
    behavior: FakeEmbeddingBehavior,
}

impl FakeTextEmbeddingProvider {
    /// Creates a fake embedding provider.
    #[must_use]
    pub fn new(
        config: TextEmbeddingProviderConfig,
        limits: crate::videos::semantic_indexing::text_embedding::TextEmbeddingLimits,
        behavior: FakeEmbeddingBehavior,
    ) -> Self {
        Self {
            config,
            limits,
            behavior,
        }
    }
}

#[allow(clippy::unused_async_trait_impl)]
impl TextEmbeddingProvider for FakeTextEmbeddingProvider {
    fn config(&self) -> &TextEmbeddingProviderConfig {
        &self.config
    }

    fn limits(&self) -> crate::videos::semantic_indexing::text_embedding::TextEmbeddingLimits {
        self.limits
    }

    async fn embed_texts(
        &self,
        requests: Vec<TextEmbeddingRequest>,
    ) -> Result<Vec<KeyedTextEmbedding>, SemanticIndexingFailure> {
        match self.behavior {
            FakeEmbeddingBehavior::Timeout => Err(SemanticIndexingFailure::DeadlineExceeded),
            FakeEmbeddingBehavior::WrongModelSpace => {
                if let Some(request) = requests.first() {
                    let mut values = deterministic_vector(self.config().model_space().dimensions());
                    if let Some(first) = values.first_mut() {
                        *first = 2.0;
                    }
                    let wrong_space = crate::videos::model_space::ModelSpaceIdentity::new(
                        "other-provider",
                        "other-model",
                        self.config().model_space().dimensions(),
                        self.config().model_space().normalization(),
                        self.config().model_space().metric(),
                        self.config().model_space().generation(),
                    )
                    .map_err(|error| {
                        SemanticIndexingFailure::IncompatibleModelSpace {
                            message: error.to_string(),
                        }
                    })?;
                    let provenance = TextEmbeddingProvenance::new(
                        "other-provider",
                        "other-model",
                        self.config().model_space().dimensions(),
                        self.config().model_space().normalization(),
                        self.config().model_space().metric(),
                        self.config().model_space().generation(),
                        Sha256Digest::hash_bytes(request.text().as_bytes()),
                    )
                    .map_err(|error| {
                        SemanticIndexingFailure::IncompatibleModelSpace {
                            message: error.to_string(),
                        }
                    })?;
                    let vector =
                        FrameEmbeddingVector::new(values, self.config().model_space().dimensions())
                            .map_err(|error| SemanticIndexingFailure::InvalidVector {
                                message: error.to_string(),
                            })?;
                    return Ok(vec![KeyedTextEmbedding::unchecked(
                        request.key().clone(),
                        wrong_space,
                        provenance,
                        vector,
                    )]);
                }
                Ok(Vec::new())
            }
            FakeEmbeddingBehavior::Ok => requests
                .iter()
                .map(|request| {
                    let values = deterministic_vector(self.config().model_space().dimensions());
                    KeyedTextEmbedding::new(
                        self.config(),
                        request.key().clone(),
                        request.text(),
                        values,
                    )
                })
                .collect(),
        }
    }
}

fn deterministic_vector(dimensions: u32) -> Vec<f32> {
    let count = usize::try_from(dimensions).unwrap_or(0);
    let mut values = vec![0.0_f32; count];
    if count == 0 {
        return values;
    }
    values[0] = 1.0;
    values
}
