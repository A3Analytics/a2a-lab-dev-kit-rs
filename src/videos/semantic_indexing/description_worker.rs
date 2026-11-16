//! Description-stage worker claiming one durable live or backfill lane.

use std::sync::Arc;

use crate::config::{SemanticIndexingConfig, SemanticWorkerLimits};
use crate::error::A2aLabError;
use crate::time::UtcTimestamp;
use crate::videos::model_space::ProvenanceGeneration;
use crate::videos::semantic_dvr::{
    IndexingCheckpoint, IndexingJobRecord, IndexingWorkerLane, ManifestRetainedState,
    MediaPayloadStatus, SemanticChunkDvrStore,
};
use crate::videos::semantic_indexing::failure::SemanticIndexingFailure;
use crate::videos::semantic_indexing::fakes::FakeFrameDescriptionProvider;
use crate::videos::semantic_indexing::frame_description::{
    FrameDescriptionKey, FrameDescriptionLimits, FrameDescriptionProvider, FrameDescriptionRequest,
    KeyedFrameDescription, validate_description_batch,
};
use crate::videos::semantic_indexing::providers::deterministic_indexing_providers;
use crate::videos::semantic_indexing::queue::{
    IndexingWorkerBatchReport, IndexingWorkerFaults, StageJobOutcome, map_indexing_failure,
    prepare_lane_claim, schedule_indexing_retry,
};
use crate::videos::semantic_indexing::worker::{SemanticIndexingStage, frame_description_limits};

/// Description-only worker bound to one claim lane.
pub struct DescriptionStageWorker<D> {
    provider: D,
    limits: SemanticWorkerLimits,
    worker_id: String,
    lane: IndexingWorkerLane,
    faults: Arc<IndexingWorkerFaults>,
}

impl<D> DescriptionStageWorker<D>
where
    D: FrameDescriptionProvider,
{
    #[must_use]
    pub fn new(
        provider: D,
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

    /// Reclaims, classifies, claims, and describes one bounded batch.
    pub async fn run_batch(
        &self,
        store: &SemanticChunkDvrStore,
        semantic: &SemanticIndexingConfig,
    ) -> Result<IndexingWorkerBatchReport, A2aLabError> {
        let now_us = UtcTimestamp::now().as_micros();
        let description_limits = frame_description_limits(&semantic.limits);
        let (mut report, claimed) = prepare_lane_claim(
            store,
            &self.worker_id,
            &self.limits,
            self.lane,
            SemanticIndexingStage::Description,
            now_us,
        )
        .await?;
        for job in claimed {
            match self
                .process_job(store, semantic, &job, now_us, description_limits)
                .await
            {
                Ok(StageJobOutcome::Committed) => report.record_description_committed(),
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
        limits: FrameDescriptionLimits,
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
                SemanticIndexingStage::Description.id(),
                frame.frame_id().as_str(),
                frame.captured_at().as_micros(),
                schedule.description(),
            )
            .await?
        {
            store
                .skip_ineligible_indexing_job(job.job_id(), &self.worker_id, now_us, "ineligible")
                .await?;
            return Ok(StageJobOutcome::Skipped);
        }
        if manifest.description_checkpoint() == IndexingCheckpoint::Ready
            && let Some(stored) = manifest.description()
        {
            let committed = store
                .complete_indexing_description_cas(
                    job.job_id(),
                    &self.worker_id,
                    job.revision_generation(),
                    stored.provenance().generation().get(),
                    now_us,
                )
                .await?;
            return Ok(if committed {
                StageJobOutcome::Committed
            } else {
                StageJobOutcome::Failed
            });
        }
        let generation = ProvenanceGeneration::new(1)
            .map_err(|error| A2aLabError::invalid("generation", error.to_string()))?;
        let key = FrameDescriptionKey::new(frame.clone(), None, generation);
        let request = FrameDescriptionRequest::new(
            limits,
            key,
            limits.max_input_bytes().min(16_384),
            limits.max_pixels().min(640 * 480),
        )
        .map_err(map_indexing_failure)?;
        let responses = match self.provider.describe_frames(vec![request.clone()]).await {
            Ok(responses) => responses,
            Err(failure) => {
                return self.fail_job(store, job, &failure, now_us).await;
            }
        };
        let validated =
            match validate_description_batch(limits, std::slice::from_ref(&request), &responses) {
                Ok(validated) => validated,
                Err(failure) => {
                    return self.fail_job(store, job, &failure, now_us).await;
                }
            };
        let item = validated
            .first()
            .ok_or_else(|| A2aLabError::unavailable("empty description batch"))?;
        self.commit_description(store, job, item, now_us).await
    }

    async fn commit_description(
        &self,
        store: &SemanticChunkDvrStore,
        job: &IndexingJobRecord,
        description: &KeyedFrameDescription,
        now_us: i64,
    ) -> Result<StageJobOutcome, A2aLabError> {
        if self.faults.crash_before_description_commit() {
            let failure = SemanticIndexingFailure::ProviderUnavailable {
                message: "injected crash before description commit".to_owned(),
            };
            return self.fail_job(store, job, &failure, now_us).await;
        }
        let provenance = description.to_provenance()?;
        store
            .upsert_frame_description(description.key().frame(), description.text(), &provenance)
            .await?;
        let committed = store
            .complete_indexing_description_cas(
                job.job_id(),
                &self.worker_id,
                job.revision_generation(),
                provenance.generation().get(),
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

impl DescriptionStageWorker<FakeFrameDescriptionProvider> {
    /// Deterministic description worker from validated semantic configuration.
    pub fn deterministic(
        semantic: &SemanticIndexingConfig,
        worker_id: impl Into<String>,
        faults: Arc<IndexingWorkerFaults>,
        lane: IndexingWorkerLane,
    ) -> Result<Self, A2aLabError> {
        let providers = deterministic_indexing_providers(semantic)?;
        Ok(Self::new(
            providers.description,
            semantic.limits.worker.clone(),
            worker_id,
            faults,
        )
        .with_lane(lane))
    }
}
