//! Bounded semantic indexing provider contracts (description and text embeddings).

mod failure;
mod fakes;
mod frame_description;
mod text_embedding;
mod worker;

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
pub use worker::{
    SemanticIndexingStage, assemble_stream_schedules, attach_stream_schedules,
    frame_description_limits, indexing_claim_batch_size, text_embedding_limits,
};
