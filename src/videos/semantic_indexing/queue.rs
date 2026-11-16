//! Shared worker-lane report, faults, and retry helpers.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::SemanticWorkerLimits;
use crate::error::A2aLabError;
use crate::videos::semantic_dvr::{
    DEFAULT_RECONCILE_BATCH, IndexingJobRecord, IndexingWorkerLane, SemanticChunkDvrStore,
};
use crate::videos::semantic_indexing::failure::SemanticIndexingFailure;
use crate::videos::semantic_indexing::worker::{SemanticIndexingStage, indexing_claim_batch_size};
use crate::videos::semantic_indexing::{FrameDescriptionProvider, TextEmbeddingProvider};

/// Outcome of one worker batch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexingWorkerBatchReport {
    reclaimed_leases: u32,
    claimed: u32,
    descriptions_committed: u32,
    embeddings_committed: u32,
    failures: u32,
    skipped: u32,
    backlog_over_limit: bool,
    lane: IndexingWorkerLane,
}

impl IndexingWorkerBatchReport {
    #[must_use]
    pub const fn reclaimed_leases(&self) -> u32 {
        self.reclaimed_leases
    }

    #[must_use]
    pub const fn claimed(&self) -> u32 {
        self.claimed
    }

    #[must_use]
    pub const fn descriptions_committed(&self) -> u32 {
        self.descriptions_committed
    }

    #[must_use]
    pub const fn embeddings_committed(&self) -> u32 {
        self.embeddings_committed
    }

    #[must_use]
    pub const fn failures(&self) -> u32 {
        self.failures
    }

    #[must_use]
    pub const fn skipped(&self) -> u32 {
        self.skipped
    }

    #[must_use]
    pub const fn backlog_over_limit(&self) -> bool {
        self.backlog_over_limit
    }

    #[must_use]
    pub const fn lane(&self) -> IndexingWorkerLane {
        self.lane
    }

    pub(crate) fn record_description_committed(&mut self) {
        self.descriptions_committed = self.descriptions_committed.saturating_add(1);
    }

    pub(crate) fn record_embedding_committed(&mut self) {
        self.embeddings_committed = self.embeddings_committed.saturating_add(1);
    }

    pub(crate) fn record_failure(&mut self) {
        self.failures = self.failures.saturating_add(1);
    }

    pub(crate) fn record_skipped(&mut self) {
        self.skipped = self.skipped.saturating_add(1);
    }

    pub(crate) fn after_claim(
        reclaimed_leases: u32,
        claimed: u32,
        backlog_over_limit: bool,
        lane: IndexingWorkerLane,
    ) -> Self {
        Self {
            reclaimed_leases,
            claimed,
            backlog_over_limit,
            lane,
            ..Self::default()
        }
    }
}

/// Injectable faults for hardware-free worker tests.
#[derive(Debug, Default)]
pub struct IndexingWorkerFaults {
    crash_before_description_commit: AtomicBool,
    crash_before_embedding_commit: AtomicBool,
    force_embedding_failure: AtomicBool,
}

impl IndexingWorkerFaults {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_crash_before_description_commit(&self, value: bool) {
        self.crash_before_description_commit
            .store(value, Ordering::Relaxed);
    }

    #[must_use]
    pub fn crash_before_description_commit(&self) -> bool {
        self.crash_before_description_commit.load(Ordering::Relaxed)
    }

    pub fn set_crash_before_embedding_commit(&self, value: bool) {
        self.crash_before_embedding_commit
            .store(value, Ordering::Relaxed);
    }

    #[must_use]
    pub fn crash_before_embedding_commit(&self) -> bool {
        self.crash_before_embedding_commit.load(Ordering::Relaxed)
    }

    pub fn set_force_embedding_failure(&self, value: bool) {
        self.force_embedding_failure.store(value, Ordering::Relaxed);
    }

    #[must_use]
    pub fn force_embedding_failure(&self) -> bool {
        self.force_embedding_failure.load(Ordering::Relaxed)
    }
}

/// Injected description and embedding adapters for one worker.
pub struct SemanticIndexingProviders<D, E> {
    /// Frame-description adapter.
    pub description: D,
    /// Text-embedding adapter.
    pub embedding: E,
}

impl<D, E> SemanticIndexingProviders<D, E>
where
    D: FrameDescriptionProvider,
    E: TextEmbeddingProvider,
{
    #[must_use]
    pub const fn new(description: D, embedding: E) -> Self {
        Self {
            description,
            embedding,
        }
    }
}

pub(crate) fn map_indexing_failure(failure: SemanticIndexingFailure) -> A2aLabError {
    failure.into_lab_error()
}

pub(crate) async fn schedule_indexing_retry(
    store: &SemanticChunkDvrStore,
    worker_id: &str,
    job: &IndexingJobRecord,
    limits: &SemanticWorkerLimits,
    reason: &str,
    retryable: bool,
    now_us: i64,
) -> Result<(), A2aLabError> {
    let backoff = i64::try_from(limits.retry_backoff.as_secs())
        .unwrap_or(0)
        .saturating_mul(1_000_000);
    store
        .fail_indexing_job_lease(
            job.job_id(),
            worker_id,
            job.revision_generation(),
            reason,
            retryable,
            now_us.saturating_add(backoff),
            now_us,
            limits,
        )
        .await
}

pub(crate) enum StageJobOutcome {
    Committed,
    Skipped,
    Failed,
}

pub(crate) async fn prepare_lane_claim(
    store: &SemanticChunkDvrStore,
    worker_id: &str,
    limits: &SemanticWorkerLimits,
    lane: IndexingWorkerLane,
    stage: SemanticIndexingStage,
    now_us: i64,
) -> Result<(IndexingWorkerBatchReport, Vec<IndexingJobRecord>), A2aLabError> {
    let reclaimed = store.reclaim_expired_indexing_leases(now_us).await?;
    let queue = store.indexing_worker_queue_stats(limits).await?;
    store.clear_legacy_materialization_pause().await?;
    let deadline = now_us.saturating_add(
        i64::try_from(limits.job_deadline.as_secs())
            .unwrap_or(i64::MAX)
            .saturating_mul(1_000_000),
    );
    let claim_batch = indexing_claim_batch_size(limits);
    let classify_batch = claim_batch.saturating_mul(4).max(DEFAULT_RECONCILE_BATCH);
    let _ = store
        .classify_expired_indexing_jobs_missing_retained_media(now_us, classify_batch)
        .await?;
    let _ = store
        .classify_ineligible_indexing_jobs(now_us, classify_batch)
        .await?;
    let claimed = store
        .claim_indexing_jobs_for_lane(worker_id, claim_batch, deadline, now_us, lane, stage)
        .await?;
    let report = IndexingWorkerBatchReport::after_claim(
        reclaimed,
        u32::try_from(claimed.len()).unwrap_or(u32::MAX),
        queue.backlog_over_limit,
        lane,
    );
    Ok((report, claimed))
}
