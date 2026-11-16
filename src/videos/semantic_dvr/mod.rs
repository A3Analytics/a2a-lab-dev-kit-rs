//! Generic semantic persistence paired with [`crate::videos::SqliteChunkDvrStore`].

mod coverage;
mod db;
mod decision_tombstone;
mod eligibility;
mod eligibility_types;
mod engine;
mod faults;
mod indexing_jobs;
mod indexing_types;
mod manifest;
mod manifest_types;
mod media;
mod persist_hook;
mod registration;

pub use coverage::neighbor_window;
pub use eligibility_types::{
    StageEligibilityRecord, interval_bucket_start, stage_eligibility_job_id,
};
pub use engine::SemanticChunkDvrStore;
pub use faults::SaveFaults;
pub use indexing_types::{IndexingJobRecord, IndexingJobState, IndexingWorkerQueueStats};
pub use manifest_types::{
    IndexingCheckpoint, ListManifestFramesRequest, ManifestPageRequest, ManifestRetainedState,
    ManifestSearchCandidateFilter, ManifestSearchCandidateScan, MediaPayloadStatus,
    RegisterManifestOutcome, SemanticFrameManifestRecord, StoredFrameDescription,
    StoredFrameEmbedding,
};
pub use registration::{
    DEFAULT_RECONCILE_BATCH, INDEXING_PRIORITY_BACKFILL, INDEXING_PRIORITY_LIVE,
    IndexingReconcileReport, IndexingWorkerLane, indexing_job_id,
};
