//! Bounded semantic indexing provider contracts (description and text embeddings).

mod failure;
mod fakes;
mod frame_description;
mod text_embedding;

pub use failure::SemanticIndexingFailure;
pub use fakes::{
    FakeDescriptionBehavior, FakeEmbeddingBehavior, FakeFrameDescriptionProvider,
    FakeTextEmbeddingProvider,
};
pub use frame_description::{
    DEFAULT_BEDROCK_DESCRIPTION_MODEL, DEFAULT_OPENAI_DESCRIPTION_MODEL, FrameDescriptionBackend,
    FrameDescriptionKey, FrameDescriptionLimits, FrameDescriptionProvider,
    FrameDescriptionProviderConfig, FrameDescriptionRequest, KeyedFrameDescription,
    validate_description_batch,
};
pub use text_embedding::{
    KeyedTextEmbedding, TextEmbeddingKey, TextEmbeddingLimits, TextEmbeddingProvider,
    TextEmbeddingProviderConfig, TextEmbeddingRequest, validate_embedding_batch,
};
