//! Bounded semantic indexing provider contracts (description and text embeddings).

mod description_worker;
mod embedding_worker;
mod failure;
mod fakes;
mod frame_description;
mod providers;
mod queue;
mod text_embedding;
mod worker;

pub use description_worker::DescriptionStageWorker;
pub use embedding_worker::EmbeddingStageWorker;
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
pub use providers::deterministic_indexing_providers;
pub use queue::{IndexingWorkerBatchReport, IndexingWorkerFaults, SemanticIndexingProviders};
pub use text_embedding::{
    KeyedTextEmbedding, TextEmbeddingKey, TextEmbeddingLimits, TextEmbeddingProvider,
    TextEmbeddingProviderConfig, TextEmbeddingRequest, validate_embedding_batch,
};
pub use worker::{
    SemanticIndexingStage, assemble_stream_schedules, attach_stream_schedules,
    frame_description_limits, indexing_claim_batch_size, text_embedding_limits,
};
