//! Deterministic provider construction from validated semantic configuration.

use crate::config::SemanticIndexingConfig;
use crate::error::A2aLabError;
use crate::videos::digest::Sha256Digest;
use crate::videos::semantic_indexing::fakes::{
    FakeDescriptionBehavior, FakeEmbeddingBehavior, FakeFrameDescriptionProvider,
    FakeTextEmbeddingProvider,
};
use crate::videos::semantic_indexing::frame_description::FrameDescriptionProviderConfig;
use crate::videos::semantic_indexing::queue::SemanticIndexingProviders;
use crate::videos::semantic_indexing::text_embedding::TextEmbeddingProviderConfig;
use crate::videos::semantic_indexing::worker::{frame_description_limits, text_embedding_limits};

/// Builds fake description and embedding adapters from one config identity.
pub fn deterministic_indexing_providers(
    semantic: &SemanticIndexingConfig,
) -> Result<
    SemanticIndexingProviders<FakeFrameDescriptionProvider, FakeTextEmbeddingProvider>,
    A2aLabError,
> {
    let description = FrameDescriptionProviderConfig::new(
        semantic.description.backend(),
        Some(semantic.description.model()),
        Sha256Digest::hash_bytes(b"devkit-description-prompt"),
        Sha256Digest::hash_bytes(b"devkit-description-preprocess"),
    )?;
    let embeddings = semantic.embeddings.as_ref().ok_or_else(|| {
        A2aLabError::invalid(
            "semantic_indexing.embeddings",
            "embeddings are required for deterministic indexing providers",
        )
    })?;
    let embedding = TextEmbeddingProviderConfig::new(
        embeddings.provider(),
        embeddings.model(),
        embeddings.model_space_identity()?,
    )?;
    Ok(SemanticIndexingProviders::new(
        FakeFrameDescriptionProvider::new(
            description,
            frame_description_limits(&semantic.limits),
            FakeDescriptionBehavior::Ok,
        ),
        FakeTextEmbeddingProvider::new(
            embedding,
            text_embedding_limits(&semantic.limits),
            FakeEmbeddingBehavior::Ok,
        ),
    ))
}
