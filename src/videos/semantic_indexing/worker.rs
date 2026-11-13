//! Semantic stage identities, schedule assembly, and worker limit mapping.

use std::time::Duration;

use crate::config::{
    SemanticIndexingConfig, SemanticIndexingLimits, SemanticStreamSchedule, SemanticWorkerLimits,
    VideoSourceConfig,
};
use crate::videos::semantic_indexing::{FrameDescriptionLimits, TextEmbeddingLimits};

/// One independent semantic indexing stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticIndexingStage {
    /// Luna frame description.
    Description,
    /// Text embedding of persisted descriptions.
    Embeddings,
}

impl SemanticIndexingStage {
    /// Stable stage id used in schedule and worker routing.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Description => "description",
            Self::Embeddings => "embeddings",
        }
    }
}

/// Collects per-source stage schedules from validated video sources.
#[must_use]
pub fn assemble_stream_schedules(sources: &[VideoSourceConfig]) -> Vec<SemanticStreamSchedule> {
    sources
        .iter()
        .map(|source| {
            source
                .semantic_indexing
                .clone()
                .unwrap_or_else(|| SemanticStreamSchedule::every_saved_frame(source.id.as_str()))
        })
        .collect()
}

/// Attaches assembled source schedules to a parsed semantic-indexing block.
#[must_use]
pub fn attach_stream_schedules(
    config: SemanticIndexingConfig,
    sources: &[VideoSourceConfig],
) -> SemanticIndexingConfig {
    config.with_schedules(assemble_stream_schedules(sources))
}

/// Durable claim width: one batch may fill up to `max_batch_items × max_concurrency` leases.
#[must_use]
pub fn indexing_claim_batch_size(limits: &SemanticWorkerLimits) -> u32 {
    limits
        .max_batch_items
        .saturating_mul(limits.max_concurrency)
        .max(limits.max_batch_items)
}

/// Frame-description limits from validated v2 semantic configuration.
#[must_use]
pub fn frame_description_limits(limits: &SemanticIndexingLimits) -> FrameDescriptionLimits {
    let worker = &limits.worker;
    FrameDescriptionLimits::new(
        worker.max_batch_items,
        16 * 1024 * 1024,
        u64::from(limits.roi.max_edge_pixels) * u64::from(limits.roi.max_edge_pixels),
        16 * 1024,
        512,
        Duration::from_secs(worker.job_deadline.as_secs()),
    )
    .unwrap_or_else(|_| FrameDescriptionLimits::pilot())
}

/// Text-embedding limits from validated v2 semantic configuration.
#[must_use]
pub fn text_embedding_limits(limits: &SemanticIndexingLimits) -> TextEmbeddingLimits {
    let worker = &limits.worker;
    TextEmbeddingLimits::new(
        worker.max_batch_items,
        32 * 1024,
        512,
        Duration::from_secs(worker.job_deadline.as_secs()),
    )
    .unwrap_or_else(|_| TextEmbeddingLimits::pilot())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SemanticSchedule;
    use crate::id::VideoSourceId;

    #[test]
    fn claim_batch_scales_with_concurrency() {
        let limits = SemanticWorkerLimits {
            max_batch_items: 2,
            max_concurrency: 4,
            max_queue_backlog: 100,
            max_retry_attempts: 3,
            job_deadline: crate::config::ConfigDuration::parse("test", "30s").expect("duration"),
            retry_backoff: crate::config::ConfigDuration::parse("test", "5s").expect("duration"),
        };
        assert_eq!(indexing_claim_batch_size(&limits), 8);
    }

    #[test]
    fn assemble_schedules_default_every_saved_frame() {
        let source = VideoSourceConfig {
            id: VideoSourceId::new("cam").expect("id"),
            name: None,
            description: None,
            format: crate::config::VideoInputFormat::Mjpeg,
            camera_id: None,
            width: None,
            height: None,
            capture_rate: None,
            semantic_indexing: None,
        };
        let schedules = assemble_stream_schedules(std::slice::from_ref(&source));
        assert_eq!(schedules.len(), 1);
        assert_eq!(schedules[0].stream(), "cam");
        assert_eq!(
            schedules[0].description(),
            SemanticSchedule::EverySavedFrame
        );
    }
}
