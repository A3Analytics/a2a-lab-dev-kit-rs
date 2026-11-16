//! Embedding-stage worker claiming one durable live or backfill lane.

use std::sync::Arc;

use crate::config::{SemanticIndexingConfig, SemanticWorkerLimits};
use crate::error::A2aLabError;
use crate::time::UtcTimestamp;
use crate::videos::semantic_dvr::{
    IndexingCheckpoint, IndexingJobRecord, IndexingWorkerLane, ManifestRetainedState,
    MediaPayloadStatus, SemanticChunkDvrStore,
};
use crate::videos::semantic_indexing::failure::SemanticIndexingFailure;
use crate::videos::semantic_indexing::fakes::FakeTextEmbeddingProvider;
use crate::videos::semantic_indexing::frame_description::FrameDescriptionKey;
use crate::videos::semantic_indexing::providers::deterministic_indexing_providers;
use crate::videos::semantic_indexing::queue::{
    IndexingWorkerBatchReport, IndexingWorkerFaults, StageJobOutcome, map_indexing_failure,
    prepare_lane_claim, schedule_indexing_retry,
};
use crate::videos::semantic_indexing::text_embedding::{
    TextEmbeddingKey, TextEmbeddingLimits, TextEmbeddingProvider, TextEmbeddingRequest,
    validate_embedding_batch,
};
use crate::videos::semantic_indexing::worker::{SemanticIndexingStage, text_embedding_limits};

/// Embedding-only worker bound to one claim lane.
pub struct EmbeddingStageWorker<E> {
    provider: E,
    limits: SemanticWorkerLimits,
    worker_id: String,
    lane: IndexingWorkerLane,
    faults: Arc<IndexingWorkerFaults>,
}

impl<E> EmbeddingStageWorker<E>
where
    E: TextEmbeddingProvider,
{
    #[must_use]
    pub fn new(
        provider: E,
        limits: SemanticWorkerLimits,
        worker_id: impl Into<String>,
        faults: Arc<IndexingWorkerFaults>,
    ) -> Self {
        Self {
            provider,
            limits,
            worker_id: worker_id.into(),
            lane: IndexingWorkerLane::Live,
            faults,
        }
    }

    #[must_use]
    pub const fn with_lane(mut self, lane: IndexingWorkerLane) -> Self {
        self.lane = lane;
        self
    }

    /// Reclaims, classifies, claims, and embeds one bounded batch.
    pub async fn run_batch(
        &self,
        store: &SemanticChunkDvrStore,
        semantic: &SemanticIndexingConfig,
    ) -> Result<IndexingWorkerBatchReport, A2aLabError> {
        let now_us = UtcTimestamp::now().as_micros();
        if let Some(embeddings) = semantic.embeddings.as_ref() {
            store
                .persist_embedding_config_identity(&embeddings.model_space_identity()?, now_us)
                .await?;
        }
        let embedding_limits = text_embedding_limits(&semantic.limits);
        let (mut report, claimed) = prepare_lane_claim(
            store,
            &self.worker_id,
            &self.limits,
            self.lane,
            SemanticIndexingStage::Embeddings,
            now_us,
        )
        .await?;
        for job in claimed {
            match self
                .process_job(store, semantic, &job, now_us, embedding_limits)
                .await
            {
                Ok(StageJobOutcome::Committed) => report.record_embedding_committed(),
                Ok(StageJobOutcome::Skipped) => report.record_skipped(),
                Ok(StageJobOutcome::Failed) | Err(_) => report.record_failure(),
            }
        }
        Ok(report)
    }

    async fn process_job(
        &self,
        store: &SemanticChunkDvrStore,
        semantic: &SemanticIndexingConfig,
        job: &IndexingJobRecord,
        now_us: i64,
        limits: TextEmbeddingLimits,
    ) -> Result<StageJobOutcome, A2aLabError> {
        let manifest = store.get_manifest_frame(job.frame_id()).await?;
        if manifest.retained_state() != ManifestRetainedState::Retained
            || !manifest.search_current()
            || manifest.payload_status() != MediaPayloadStatus::Available
        {
            store.expire_indexing_job(job.job_id(), now_us).await?;
            return Ok(StageJobOutcome::Skipped);
        }
        let frame = manifest.frame();
        let schedule = semantic.schedule_for(frame.source_id().as_str());
        if !store
            .select_semantic_stage_frame(
                frame.source_id().as_str(),
                frame.track_id().as_str(),
                SemanticIndexingStage::Embeddings.id(),
                frame.frame_id().as_str(),
                frame.captured_at().as_micros(),
                schedule.embeddings(),
            )
            .await?
        {
            store
                .skip_ineligible_indexing_job(job.job_id(), &self.worker_id, now_us, "ineligible")
                .await?;
            return Ok(StageJobOutcome::Skipped);
        }
        if manifest.embedding_checkpoint() == IndexingCheckpoint::Ready {
            let committed = store
                .complete_indexing_embedding_cas(
                    job.job_id(),
                    &self.worker_id,
                    job.revision_generation(),
                    now_us,
                )
                .await?;
            return Ok(if committed {
                StageJobOutcome::Committed
            } else {
                StageJobOutcome::Failed
            });
        }
        self.embed_eligible_job(store, job, &manifest, now_us, limits)
            .await
    }

    async fn embed_eligible_job(
        &self,
        store: &SemanticChunkDvrStore,
        job: &IndexingJobRecord,
        manifest: &crate::videos::semantic_dvr::SemanticFrameManifestRecord,
        now_us: i64,
        limits: TextEmbeddingLimits,
    ) -> Result<StageJobOutcome, A2aLabError> {
        let frame = manifest.frame();
        let Some(description) = manifest.description() else {
            schedule_indexing_retry(
                store,
                &self.worker_id,
                job,
                &self.limits,
                "description_not_ready",
                true,
                now_us,
            )
            .await?;
            return Ok(StageJobOutcome::Failed);
        };
        if self.faults.force_embedding_failure() {
            schedule_indexing_retry(
                store,
                &self.worker_id,
                job,
                &self.limits,
                "injected_embedding_failure",
                true,
                now_us,
            )
            .await?;
            return Ok(StageJobOutcome::Failed);
        }
        let description_key =
            FrameDescriptionKey::new(frame.clone(), None, description.provenance().generation());
        let request = TextEmbeddingRequest::new(
            limits,
            TextEmbeddingKey::PersistedDescription(description_key),
            description.text(),
            u32::try_from(description.text().len()).unwrap_or(u32::MAX),
        )
        .map_err(map_indexing_failure)?;
        let responses = match self.provider.embed_texts(vec![request.clone()]).await {
            Ok(responses) => responses,
            Err(failure) => {
                return self.fail_job(store, job, &failure, now_us).await;
            }
        };
        let config = self.provider.config().clone();
        let validated = match validate_embedding_batch(
            &config,
            limits,
            std::slice::from_ref(&request),
            &responses,
        ) {
            Ok(validated) => validated,
            Err(failure) => {
                return self.fail_job(store, job, &failure, now_us).await;
            }
        };
        let item = validated
            .first()
            .ok_or_else(|| A2aLabError::unavailable("empty embedding batch"))?;
        if self.faults.crash_before_embedding_commit() {
            let failure = SemanticIndexingFailure::ProviderUnavailable {
                message: "injected crash before embedding commit".to_owned(),
            };
            return self.fail_job(store, job, &failure, now_us).await;
        }
        store
            .upsert_frame_embedding(
                frame,
                item.model_space(),
                item.provenance(),
                item.vector(),
                IndexingCheckpoint::Ready,
            )
            .await?;
        let committed = store
            .complete_indexing_embedding_cas(
                job.job_id(),
                &self.worker_id,
                job.revision_generation(),
                now_us,
            )
            .await?;
        Ok(if committed {
            StageJobOutcome::Committed
        } else {
            StageJobOutcome::Failed
        })
    }

    async fn fail_job(
        &self,
        store: &SemanticChunkDvrStore,
        job: &IndexingJobRecord,
        failure: &SemanticIndexingFailure,
        now_us: i64,
    ) -> Result<StageJobOutcome, A2aLabError> {
        schedule_indexing_retry(
            store,
            &self.worker_id,
            job,
            &self.limits,
            &failure.to_string(),
            failure.is_retryable(),
            now_us,
        )
        .await?;
        Ok(StageJobOutcome::Failed)
    }
}

impl EmbeddingStageWorker<FakeTextEmbeddingProvider> {
    /// Deterministic embedding worker from validated semantic configuration.
    pub fn deterministic(
        semantic: &SemanticIndexingConfig,
        worker_id: impl Into<String>,
        faults: Arc<IndexingWorkerFaults>,
        lane: IndexingWorkerLane,
    ) -> Result<Self, A2aLabError> {
        let providers = deterministic_indexing_providers(semantic)?;
        Ok(Self::new(
            providers.embedding,
            semantic.limits.worker.clone(),
            worker_id,
            faults,
        )
        .with_lane(lane))
    }
}
