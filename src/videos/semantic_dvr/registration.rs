//! Durable indexing intent at the saved-frame metadata commit boundary.

use crate::{
    A2aLabError, DvrStoreNamespace, DvrTrackId, SavedFrameRef, UtcTimestamp, VideoSourceId,
};
use sqlx::Row;

use super::db::db_error;
use super::engine::SemanticChunkDvrStore;
use super::manifest_types::RegisterManifestOutcome;

const FRAME_INDEXING_JOB_KIND: &str = "frame_indexing";
pub const DEFAULT_RECONCILE_BATCH: u32 = 256;

/// Claimed before historical backfill jobs (`INDEXING_PRIORITY_BACKFILL`).
pub const INDEXING_PRIORITY_LIVE: i32 = 0;
/// Drained after live capture jobs; `created_at_us` is the frame capture instant.
pub const INDEXING_PRIORITY_BACKFILL: i32 = 1;

/// Durable queue lane processed by one dedicated annotation worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndexingWorkerLane {
    /// Newly committed capture frames (`INDEXING_PRIORITY_LIVE`).
    #[default]
    Live,
    /// Historical retained discovery jobs (`INDEXING_PRIORITY_BACKFILL`).
    Backfill,
}

impl IndexingWorkerLane {
    #[must_use]
    pub const fn enqueue_priority(self) -> i32 {
        match self {
            Self::Live => INDEXING_PRIORITY_LIVE,
            Self::Backfill => INDEXING_PRIORITY_BACKFILL,
        }
    }

    #[must_use]
    pub const fn id_suffix(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Backfill => "backfill",
        }
    }
}

/// Outcome of one bounded reconciliation pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexingReconcileReport {
    examined: u32,
    registered: u32,
    jobs_created: u32,
    idempotent: u32,
    missing_payload: u32,
    corrupt_payload: u32,
    has_more: bool,
}

impl IndexingReconcileReport {
    #[must_use]
    pub const fn examined(&self) -> u32 {
        self.examined
    }

    #[must_use]
    pub const fn registered(&self) -> u32 {
        self.registered
    }

    #[must_use]
    pub const fn jobs_created(&self) -> u32 {
        self.jobs_created
    }

    #[must_use]
    pub const fn idempotent(&self) -> u32 {
        self.idempotent
    }

    #[must_use]
    pub const fn missing_payload(&self) -> u32 {
        self.missing_payload
    }

    #[must_use]
    pub const fn corrupt_payload(&self) -> u32 {
        self.corrupt_payload
    }

    #[must_use]
    pub const fn has_more(&self) -> bool {
        self.has_more
    }

    fn absorb(&mut self, batch: &Self) {
        self.examined = self.examined.saturating_add(batch.examined);
        self.registered = self.registered.saturating_add(batch.registered);
        self.jobs_created = self.jobs_created.saturating_add(batch.jobs_created);
        self.idempotent = self.idempotent.saturating_add(batch.idempotent);
        self.missing_payload = self.missing_payload.saturating_add(batch.missing_payload);
        self.corrupt_payload = self.corrupt_payload.saturating_add(batch.corrupt_payload);
        self.has_more = batch.has_more;
    }
}

impl SemanticChunkDvrStore {
    /// Registers manifest revision and indexing job for one committed DVR key.
    pub async fn ensure_indexing_at_dvr_key(
        &self,
        store: &DvrStoreNamespace,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        start_us: UtcTimestamp,
    ) -> Result<(SavedFrameRef, RegisterManifestOutcome, bool), A2aLabError> {
        let (frame, outcome) = self
            .register_manifest_at_dvr_key(store, source_id, track_id, start_us)
            .await?;
        let created = if Self::permits_indexing_source(source_id.as_str()) {
            self.materialize_frame_eligibility(
                source_id.as_str(),
                track_id.as_str(),
                frame.frame_id().as_str(),
                start_us.as_micros(),
                INDEXING_PRIORITY_LIVE,
                start_us.as_micros(),
            )
            .await?
        } else {
            0_u32
        };
        Ok((frame, outcome, created > 0))
    }

    /// Discovers ready DVR rows missing manifest/job pairs up to `batch` keys.
    pub async fn reconcile_indexing_jobs(
        &self,
        store: &DvrStoreNamespace,
        batch: u32,
    ) -> Result<IndexingReconcileReport, A2aLabError> {
        let limit = i64::from(batch.max(1));
        let rows = self.reconcile_indexing_job_candidates(limit).await?;
        let has_more = rows.len() > usize::try_from(limit).unwrap_or(usize::MAX);
        let mut report = IndexingReconcileReport {
            examined: 0,
            registered: 0,
            jobs_created: 0,
            idempotent: 0,
            missing_payload: 0,
            corrupt_payload: 0,
            has_more,
        };
        let mut last_cursor: Option<(String, String, i64)> = None;
        for row in rows
            .iter()
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
        {
            self.reconcile_indexing_jobs_apply_row(store, row, &mut report, &mut last_cursor)
                .await?;
        }
        if let Some((source_id, track_id, start_us)) = last_cursor {
            self.reconcile_cursor_save(&source_id, &track_id, start_us)
                .await?;
        }
        Ok(report)
    }

    async fn reconcile_indexing_job_candidates(
        &self,
        limit: i64,
    ) -> Result<Vec<sqlx::sqlite::SqliteRow>, A2aLabError> {
        let media = self.list_ready_media_keys(limit + 1).await?;
        let indexed = self.current_semantic_revision_keys(&media).await?;
        let missing: Vec<_> = media
            .into_iter()
            .filter(|key| !indexed.contains(key))
            .collect();
        if missing.is_empty() {
            return Ok(Vec::new());
        }
        let mut rows = Vec::new();
        for (source_id, track_id, start_us) in missing {
            let row = sqlx::query("SELECT ?1 AS source_id, ?2 AS track_id, ?3 AS start_us")
                .bind(source_id)
                .bind(track_id)
                .bind(start_us)
                .fetch_one(self.semantic_pool())
                .await
                .map_err(db_error)?;
            rows.push(row);
        }
        Ok(rows)
    }

    async fn reconcile_indexing_jobs_apply_row(
        &self,
        store: &DvrStoreNamespace,
        row: &sqlx::sqlite::SqliteRow,
        report: &mut IndexingReconcileReport,
        last_cursor: &mut Option<(String, String, i64)>,
    ) -> Result<(), A2aLabError> {
        report.examined = report.examined.saturating_add(1);
        let source_id =
            VideoSourceId::new(row.try_get::<String, _>("source_id").map_err(db_error)?)?;
        if !Self::permits_indexing_source(source_id.as_str()) {
            return Ok(());
        }
        let track_id = DvrTrackId::new(row.try_get::<String, _>("track_id").map_err(db_error)?)?;
        let start_us = UtcTimestamp::from_unix_microseconds(
            row.try_get::<i64, _>("start_us").map_err(db_error)?,
        )?;
        *last_cursor = Some((
            source_id.as_str().to_owned(),
            track_id.as_str().to_owned(),
            start_us.as_micros(),
        ));
        let register = self
            .register_manifest_at_dvr_key(store, &source_id, &track_id, start_us)
            .await;
        match register {
            Ok((frame, outcome)) => {
                self.reconcile_indexing_jobs_on_register(
                    report, &source_id, &track_id, start_us, &frame, outcome,
                )
                .await
            }
            Err(error) => Self::reconcile_indexing_jobs_on_register_error(report, error),
        }
    }

    async fn reconcile_indexing_jobs_on_register(
        &self,
        report: &mut IndexingReconcileReport,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        start_us: UtcTimestamp,
        frame: &SavedFrameRef,
        outcome: RegisterManifestOutcome,
    ) -> Result<(), A2aLabError> {
        let created = if Self::permits_indexing_source(source_id.as_str()) {
            self.materialize_frame_eligibility(
                source_id.as_str(),
                track_id.as_str(),
                frame.frame_id().as_str(),
                start_us.as_micros(),
                INDEXING_PRIORITY_BACKFILL,
                start_us.as_micros(),
            )
            .await?
        } else {
            0_u32
        };
        report.jobs_created = report.jobs_created.saturating_add(created);
        match outcome {
            RegisterManifestOutcome::Registered | RegisterManifestOutcome::ReplacedPrior => {
                report.registered = report.registered.saturating_add(1);
            }
            RegisterManifestOutcome::Idempotent => {
                report.idempotent = report.idempotent.saturating_add(1);
            }
        }
        if let Ok(record) = self.get_manifest_frame(frame.frame_id().as_str()).await {
            match record.payload_status() {
                super::manifest_types::MediaPayloadStatus::Missing => {
                    report.missing_payload = report.missing_payload.saturating_add(1);
                }
                super::manifest_types::MediaPayloadStatus::Corrupt => {
                    report.corrupt_payload = report.corrupt_payload.saturating_add(1);
                }
                super::manifest_types::MediaPayloadStatus::Available => {}
            }
        }
        Ok(())
    }

    fn reconcile_indexing_jobs_on_register_error(
        report: &mut IndexingReconcileReport,
        error: A2aLabError,
    ) -> Result<(), A2aLabError> {
        let message = error.to_string();
        if message.contains("not found") || message.contains("checksum mismatch") {
            report.missing_payload = report.missing_payload.saturating_add(1);
            Ok(())
        } else if message.contains("checksum") {
            report.corrupt_payload = report.corrupt_payload.saturating_add(1);
            Ok(())
        } else {
            Err(error)
        }
    }

    /// Registers every retained DVR key lacking manifest/job pairs before returning.
    pub async fn reconcile_retained_indexing_until_idle(
        &self,
        store: &DvrStoreNamespace,
    ) -> Result<IndexingReconcileReport, A2aLabError> {
        let mut total = IndexingReconcileReport {
            examined: 0,
            registered: 0,
            jobs_created: 0,
            idempotent: 0,
            missing_payload: 0,
            corrupt_payload: 0,
            has_more: false,
        };
        loop {
            let batch = self
                .reconcile_indexing_jobs(store, DEFAULT_RECONCILE_BATCH)
                .await?;
            total.absorb(&batch);
            if !batch.has_more() {
                break;
            }
        }
        self.repair_ready_index_checkpoints().await?;
        total.has_more = false;
        Ok(total)
    }

    #[doc(hidden)]
    pub async fn indexing_job_enqueue_priority(&self, job_id: &str) -> Result<i32, A2aLabError> {
        let priority: i64 = sqlx::query_scalar(
            "SELECT enqueue_priority FROM semantic_indexing_jobs WHERE job_id = ?1",
        )
        .bind(job_id)
        .fetch_optional(self.semantic_pool())
        .await
        .map_err(db_error)?
        .unwrap_or(INDEXING_PRIORITY_BACKFILL.into());
        Ok(i32::try_from(priority).unwrap_or(INDEXING_PRIORITY_BACKFILL))
    }

    pub async fn count_indexing_jobs(&self) -> Result<u64, A2aLabError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM semantic_indexing_jobs")
            .fetch_one(self.semantic_pool())
            .await
            .map_err(db_error)?;
        Ok(u64::try_from(count.max(0)).unwrap_or(0))
    }

    pub async fn indexing_job_exists(&self, frame_id: &str) -> Result<bool, A2aLabError> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM semantic_indexing_jobs WHERE frame_id = ?1")
                .bind(frame_id)
                .fetch_one(self.semantic_pool())
                .await
                .map_err(db_error)?;
        Ok(count > 0)
    }

    async fn repair_ready_index_checkpoints(&self) -> Result<(), A2aLabError> {
        sqlx::query(
            "UPDATE semantic_frame_revisions
             SET description_checkpoint = 'ready'
             WHERE description_checkpoint != 'failed'
               AND EXISTS (
                 SELECT 1 FROM semantic_frame_descriptions
                 WHERE semantic_frame_descriptions.frame_id = semantic_frame_revisions.frame_id
               )",
        )
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        sqlx::query(
            "UPDATE semantic_frame_revisions
             SET embedding_checkpoint = 'ready'
             WHERE embedding_checkpoint != 'failed'
               AND EXISTS (
                 SELECT 1 FROM semantic_frame_embeddings
                 WHERE semantic_frame_embeddings.frame_id = semantic_frame_revisions.frame_id
                   AND semantic_frame_embeddings.embedding_vector IS NOT NULL
                   AND length(semantic_frame_embeddings.embedding_vector) > 0
               )",
        )
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        Ok(())
    }
}

#[must_use]
pub fn indexing_job_id(frame_id: &str) -> String {
    format!("{frame_id}:{FRAME_INDEXING_JOB_KIND}")
}
