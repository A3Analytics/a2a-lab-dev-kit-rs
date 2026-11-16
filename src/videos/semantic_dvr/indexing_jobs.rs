//! Indexing job leases, retries, and completion.

use std::fmt::Write;

use sqlx::Row;

use crate::config::{SemanticSchedule, SemanticWorkerLimits};
use crate::error::A2aLabError;
use crate::videos::model_space::ModelSpaceIdentity;
use crate::videos::semantic_indexing::SemanticIndexingStage;

use super::db::db_error;
use super::engine::SemanticChunkDvrStore;
use super::indexing_types::{IndexingJobRecord, IndexingJobState, IndexingWorkerQueueStats};
use super::manifest::{metric_label, normalization_label};
impl SemanticChunkDvrStore {
    /// True when `frame_id` is the durable winner for its stage bucket.
    /// Interval lookup never advances a live-edge cursor.
    pub async fn select_semantic_stage_frame(
        &self,
        source_id: &str,
        track_id: &str,
        stage: &str,
        frame_id: &str,
        start_us: i64,
        schedule: SemanticSchedule,
    ) -> Result<bool, A2aLabError> {
        let Some(interval_us) = schedule.interval_micros() else {
            return Ok(true);
        };
        let interval_us = i64::try_from(interval_us).map_err(|_| {
            A2aLabError::invalid("semantic_indexing.schedules", "interval is too large")
        })?;
        let bucket = super::eligibility_types::interval_bucket_start(start_us, interval_us);
        let winner: Option<String> = sqlx::query_scalar(
            "SELECT winner_frame_id FROM semantic_stage_eligibility
             WHERE source_id = ?1 AND track_id = ?2 AND stage = ?3 AND bucket_start_us = ?4",
        )
        .bind(source_id)
        .bind(track_id)
        .bind(stage)
        .bind(bucket)
        .fetch_optional(self.semantic_pool())
        .await
        .map_err(db_error)?;
        Ok(winner.as_deref() == Some(frame_id))
    }

    /// Terminates an ineligible leased slot; never restores pending or described.
    pub async fn skip_ineligible_indexing_job(
        &self,
        job_id: &str,
        worker_id: &str,
        now_us: i64,
        reason: &str,
    ) -> Result<(), A2aLabError> {
        sqlx::query(
            "UPDATE semantic_indexing_job_runtime
             SET job_state = 'skipped',
                 lease_owner = NULL,
                 lease_expires_at_us = NULL,
                 completed_at_us = ?3,
                 last_retry_reason = ?4
             WHERE job_id = ?1 AND job_state = 'leased' AND lease_owner = ?2",
        )
        .bind(job_id)
        .bind(worker_id)
        .bind(now_us)
        .bind(reason)
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        Ok(())
    }

    /// Alias for [`Self::skip_ineligible_indexing_job`] with a stable skip reason.
    pub async fn release_indexing_job(
        &self,
        job_id: &str,
        worker_id: &str,
    ) -> Result<(), A2aLabError> {
        self.skip_ineligible_indexing_job(job_id, worker_id, 0, "ineligible")
            .await
    }

    /// Loads one job with runtime state when present.
    pub async fn get_indexing_job(&self, job_id: &str) -> Result<IndexingJobRecord, A2aLabError> {
        let row = sqlx::query(
            "SELECT j.job_id, j.frame_id, j.job_kind, COALESCE(r.job_state, 'pending') AS job_state,
                    COALESCE(r.revision_generation, 0) AS revision_generation,
                    r.lease_owner, r.lease_expires_at_us, r.description_generation,
                    COALESCE(r.description_attempts, 0) AS description_attempts,
                    COALESCE(r.embedding_attempts, 0) AS embedding_attempts,
                    r.last_retry_reason, COALESCE(r.next_retry_at_us, 0) AS next_retry_at_us,
                    r.last_error_at_us, r.completed_at_us
             FROM semantic_indexing_jobs j
             LEFT JOIN semantic_indexing_job_runtime r ON r.job_id = j.job_id
             WHERE j.job_id = ?1",
        )
        .bind(job_id)
        .fetch_optional(self.semantic_pool())
        .await
        .map_err(db_error)?;
        let Some(row) = row else {
            return Err(A2aLabError::not_found("indexing job", job_id));
        };
        record_from_row(&row)
    }

    /// Counts jobs that are not terminal, for backlog telemetry.
    pub async fn count_active_indexing_jobs(&self) -> Result<u64, A2aLabError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM semantic_indexing_job_runtime r
             JOIN semantic_indexing_jobs j ON j.job_id = r.job_id
             JOIN semantic_frame_revisions rev ON rev.frame_id = j.frame_id
             WHERE rev.search_current = 1
               AND rev.retained_state = 'retained'
               AND r.job_state NOT IN ('terminal_failed', 'expired', 'skipped', 'ready')
               AND NOT (
                 r.job_state = 'ready'
                 AND rev.embedding_checkpoint = 'ready'
                 AND EXISTS (
                   SELECT 1 FROM semantic_frame_embeddings emb
                   WHERE emb.frame_id = j.frame_id
                     AND emb.model_space_generation = COALESCE(
                       (SELECT model_space_generation FROM semantic_embedding_config_identity WHERE singleton_id = 1),
                       0
                     )
                     AND emb.embedding_vector IS NOT NULL
                     AND length(emb.embedding_vector) > 0
                 )
               )",
        )
        .fetch_one(self.semantic_pool())
        .await
        .map_err(db_error)?;
        Ok(u64::try_from(count.max(0)).unwrap_or(0))
    }

    pub async fn indexing_worker_queue_stats(
        &self,
        limits: &SemanticWorkerLimits,
    ) -> Result<IndexingWorkerQueueStats, A2aLabError> {
        let active = self.count_active_indexing_jobs().await?;
        let backlog = u64::from(limits.max_queue_backlog);
        let backlog_over_limit =
            active >= backlog.saturating_add(u64::from(limits.max_batch_items));
        Ok(IndexingWorkerQueueStats {
            active_jobs: active,
            backlog_over_limit,
        })
    }

    /// Clears legacy `materialization_paused` worker meta left from pre-TASK-77 backpressure.
    pub async fn clear_legacy_materialization_pause(&self) -> Result<(), A2aLabError> {
        self.set_worker_meta_flag("materialization_paused", false)
            .await
    }

    /// Marks claimable jobs whose retained manifest keys have no DVR media as expired.
    pub async fn classify_expired_indexing_jobs_missing_retained_media(
        &self,
        now_us: i64,
        batch: u32,
    ) -> Result<u32, A2aLabError> {
        let limit = i64::from(batch.max(1));
        let rows = sqlx::query(
            "SELECT r.job_id, f.source_id, f.track_id, f.start_us
             FROM semantic_indexing_job_runtime r
             JOIN semantic_indexing_jobs j ON j.job_id = r.job_id
             JOIN semantic_frame_revisions f ON f.frame_id = j.frame_id
             WHERE f.search_current = 1
               AND f.retained_state = 'retained'
               AND r.job_state IN ('pending', 'retryable_failed', 'described')
               AND (r.lease_expires_at_us IS NULL OR r.lease_expires_at_us < ?1)
               AND r.next_retry_at_us <= ?1
             ORDER BY j.created_at_us ASC, j.job_id ASC
             LIMIT ?2",
        )
        .bind(now_us)
        .bind(limit)
        .fetch_all(self.semantic_pool())
        .await
        .map_err(db_error)?;
        let keys = rows
            .iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("source_id").map_err(db_error)?,
                    row.try_get::<String, _>("track_id").map_err(db_error)?,
                    row.try_get::<i64, _>("start_us").map_err(db_error)?,
                ))
            })
            .collect::<Result<Vec<_>, A2aLabError>>()?;
        let retained = self.retained_media_keys(&keys).await?;
        let missing: Vec<String> = rows
            .into_iter()
            .filter_map(|row| {
                let key = (
                    row.try_get::<String, _>("source_id").ok()?,
                    row.try_get::<String, _>("track_id").ok()?,
                    row.try_get::<i64, _>("start_us").ok()?,
                );
                if retained.contains(&key) {
                    None
                } else {
                    row.try_get("job_id").ok()
                }
            })
            .collect();
        if missing.is_empty() {
            return Ok(0);
        }
        let mut sql = String::from(
            "UPDATE semantic_indexing_job_runtime
             SET job_state = 'expired',
                 lease_owner = NULL,
                 lease_expires_at_us = NULL,
                 completed_at_us = ?1,
                 last_retry_reason = 'frame_expired'
             WHERE job_id IN (",
        );
        for index in 0..missing.len() {
            if index > 0 {
                sql.push(',');
            }
            let _ = write!(sql, "?{}", index + 2);
        }
        sql.push(')');
        let mut query = sqlx::query(&sql).bind(now_us);
        for job_id in &missing {
            query = query.bind(job_id);
        }
        let result = query
            .execute(self.semantic_pool())
            .await
            .map_err(db_error)?;
        Ok(u32::try_from(result.rows_affected()).unwrap_or(0))
    }

    /// Marks claimable jobs that are superseded, obsolete, or unretained as skipped.
    pub async fn classify_ineligible_indexing_jobs(
        &self,
        now_us: i64,
        batch: u32,
    ) -> Result<u32, A2aLabError> {
        let limit = i64::from(batch.max(1));
        let result = sqlx::query(
            "UPDATE semantic_indexing_job_runtime
             SET job_state = 'skipped',
                 lease_owner = NULL,
                 lease_expires_at_us = NULL,
                 completed_at_us = ?1,
                 last_retry_reason = 'ineligible'
             WHERE job_id IN (
               SELECT job_id FROM (
                 SELECT r.job_id
                 FROM semantic_indexing_job_runtime r
                 JOIN semantic_indexing_jobs j ON j.job_id = r.job_id
                 JOIN semantic_frame_revisions f ON f.frame_id = j.frame_id
                 WHERE r.job_state IN ('pending', 'retryable_failed', 'described')
                   AND (r.lease_expires_at_us IS NULL OR r.lease_expires_at_us < ?1)
                   AND (
                     f.search_current = 0
                     OR f.retained_state != 'retained'
                     OR NOT EXISTS (
                       SELECT 1 FROM semantic_stage_eligibility e
                       WHERE e.winner_frame_id = j.frame_id
                         AND e.stage = j.job_kind
                         AND e.source_id = f.source_id
                         AND e.track_id = f.track_id
                     )
                   )
                 ORDER BY j.created_at_us ASC, j.job_id ASC
                 LIMIT ?2
               )
             )",
        )
        .bind(now_us)
        .bind(limit)
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        Ok(u32::try_from(result.rows_affected()).unwrap_or(0))
    }

    /// Reclaims leases past their deadline back to a claimable state.
    pub async fn reclaim_expired_indexing_leases(&self, now_us: i64) -> Result<u32, A2aLabError> {
        let result = sqlx::query(
            "UPDATE semantic_indexing_job_runtime
             SET job_state = CASE
                   WHEN job_state = 'leased' AND description_generation IS NOT NULL THEN 'described'
                   ELSE 'pending'
                 END,
                 lease_owner = NULL,
                 lease_expires_at_us = NULL,
                 last_retry_reason = COALESCE(last_retry_reason, 'lease_expired'),
                 last_error_at_us = ?1
             WHERE job_state = 'leased'
               AND lease_expires_at_us IS NOT NULL
               AND lease_expires_at_us <= ?1",
        )
        .bind(now_us)
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        Ok(u32::try_from(result.rows_affected()).unwrap_or(0))
    }

    /// Claims up to `batch` jobs from one enqueue-priority lane and stage.
    pub async fn claim_indexing_jobs_for_lane(
        &self,
        worker_id: &str,
        batch: u32,
        lease_deadline_us: i64,
        now_us: i64,
        lane: super::registration::IndexingWorkerLane,
        stage: SemanticIndexingStage,
    ) -> Result<Vec<IndexingJobRecord>, A2aLabError> {
        let batch_usize = usize::try_from(batch.max(1)).unwrap_or(1);
        let candidates = self
            .indexing_job_claim_candidates(
                lane.enqueue_priority(),
                Some(stage),
                batch_usize,
                now_us,
            )
            .await?;
        let mut claimed = Vec::new();
        for (job_id, frame_id) in candidates {
            if let Some(record) = self
                .try_claim_one_job(&job_id, &frame_id, worker_id, lease_deadline_us, now_us)
                .await?
            {
                claimed.push(record);
            }
        }
        Ok(claimed)
    }

    /// Claims up to `batch` jobs for one worker id, preferring live with one backfill slot.
    pub async fn claim_indexing_jobs(
        &self,
        worker_id: &str,
        batch: u32,
        lease_deadline_us: i64,
        now_us: i64,
    ) -> Result<Vec<IndexingJobRecord>, A2aLabError> {
        let batch = batch.max(1);
        let backfill_reserve = usize::from(batch > 1);
        let batch_usize = usize::try_from(batch).unwrap_or(usize::MAX);
        let live_budget = batch_usize.saturating_sub(backfill_reserve);
        let live_candidates = self
            .indexing_job_claim_candidates(
                super::registration::INDEXING_PRIORITY_LIVE,
                None,
                live_budget,
                now_us,
            )
            .await?;
        let backfill_budget = batch_usize.saturating_sub(live_candidates.len());
        let backfill_candidates = if backfill_budget == 0 {
            Vec::new()
        } else {
            self.indexing_job_claim_candidates(
                super::registration::INDEXING_PRIORITY_BACKFILL,
                None,
                backfill_budget,
                now_us,
            )
            .await?
        };
        let candidates: Vec<(String, String)> = live_candidates
            .into_iter()
            .chain(backfill_candidates)
            .collect();

        let mut claimed = Vec::new();
        for (job_id, frame_id) in candidates {
            if let Some(record) = self
                .try_claim_one_job(&job_id, &frame_id, worker_id, lease_deadline_us, now_us)
                .await?
            {
                claimed.push(record);
            }
        }
        Ok(claimed)
    }

    async fn indexing_job_claim_candidates(
        &self,
        enqueue_priority: i32,
        stage: Option<SemanticIndexingStage>,
        limit: usize,
        now_us: i64,
    ) -> Result<Vec<(String, String)>, A2aLabError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let stage_id = stage.map(SemanticIndexingStage::id);
        let require_description = stage == Some(SemanticIndexingStage::Embeddings);
        let rows = sqlx::query(
            "SELECT j.job_id, j.frame_id, f.source_id, f.track_id, f.start_us
             FROM semantic_indexing_jobs j
             JOIN semantic_indexing_job_runtime r ON r.job_id = j.job_id
             JOIN semantic_frame_revisions f ON f.frame_id = j.frame_id
             WHERE f.search_current = 1
               AND f.retained_state = 'retained'
               AND j.enqueue_priority = ?3
               AND (?4 IS NULL OR j.job_kind = ?4)
               AND r.job_state IN ('pending', 'retryable_failed', 'described')
               AND r.next_retry_at_us <= ?1
               AND (r.lease_expires_at_us IS NULL OR r.lease_expires_at_us < ?1)
               AND EXISTS (
                 SELECT 1 FROM semantic_stage_eligibility e
                 WHERE e.winner_frame_id = j.frame_id
                   AND e.stage = j.job_kind
                   AND e.source_id = f.source_id
                   AND e.track_id = f.track_id
               )
               AND (
                 ?5 = 0
                 OR EXISTS (
                   SELECT 1 FROM semantic_frame_descriptions d WHERE d.frame_id = j.frame_id
                 )
               )
             ORDER BY j.created_at_us ASC, j.job_id ASC
             LIMIT ?2",
        )
        .bind(now_us)
        .bind(limit_i64.saturating_mul(2).max(limit_i64))
        .bind(i64::from(enqueue_priority))
        .bind(stage_id)
        .bind(i64::from(require_description))
        .fetch_all(self.semantic_pool())
        .await
        .map_err(db_error)?;
        let keys = rows
            .iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("source_id").map_err(db_error)?,
                    row.try_get::<String, _>("track_id").map_err(db_error)?,
                    row.try_get::<i64, _>("start_us").map_err(db_error)?,
                ))
            })
            .collect::<Result<Vec<_>, A2aLabError>>()?;
        let retained = self.retained_media_keys(&keys).await?;
        let mut ranked = rows
            .into_iter()
            .map(|row| {
                let key = (
                    row.try_get::<String, _>("source_id").map_err(db_error)?,
                    row.try_get::<String, _>("track_id").map_err(db_error)?,
                    row.try_get::<i64, _>("start_us").map_err(db_error)?,
                );
                let job_id: String = row.try_get("job_id").map_err(db_error)?;
                let frame_id: String = row.try_get("frame_id").map_err(db_error)?;
                Ok((usize::from(!retained.contains(&key)), job_id, frame_id))
            })
            .collect::<Result<Vec<_>, A2aLabError>>()?;
        ranked.sort_by_key(|(missing, _, _)| *missing);
        Ok(ranked
            .into_iter()
            .take(limit)
            .map(|(_, job_id, frame_id)| (job_id, frame_id))
            .collect())
    }

    pub(crate) async fn try_claim_one_job(
        &self,
        job_id: &str,
        frame_id: &str,
        worker_id: &str,
        lease_deadline_us: i64,
        now_us: i64,
    ) -> Result<Option<IndexingJobRecord>, A2aLabError> {
        let revision: i64 = sqlx::query_scalar(
            "SELECT revision_seq FROM semantic_frame_revisions WHERE frame_id = ?1",
        )
        .bind(frame_id)
        .fetch_optional(self.semantic_pool())
        .await
        .map_err(db_error)?
        .unwrap_or(0);
        let result = sqlx::query(
            "UPDATE semantic_indexing_job_runtime
             SET job_state = 'leased',
                 lease_owner = ?2,
                 lease_expires_at_us = ?3,
                 revision_generation = ?4,
                 description_attempts = description_attempts + CASE
                   WHEN job_state IN ('pending', 'retryable_failed') THEN 1 ELSE 0 END,
                 embedding_attempts = embedding_attempts + CASE
                   WHEN job_state = 'described' THEN 1 ELSE 0 END
             WHERE job_id = ?1
               AND job_state IN ('pending', 'retryable_failed', 'described')
               AND (lease_expires_at_us IS NULL OR lease_expires_at_us < ?5)
               AND next_retry_at_us <= ?5",
        )
        .bind(job_id)
        .bind(worker_id)
        .bind(lease_deadline_us)
        .bind(revision)
        .bind(now_us)
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_indexing_job(job_id).await.map(Some)
    }

    /// Marks description committed with lease ownership and revision generation checks.
    pub async fn complete_indexing_description_cas(
        &self,
        job_id: &str,
        worker_id: &str,
        revision_generation: u32,
        description_generation: u64,
        now_us: i64,
    ) -> Result<bool, A2aLabError> {
        let result = sqlx::query(
            "UPDATE semantic_indexing_job_runtime
             SET job_state = 'ready',
                 lease_owner = NULL,
                 lease_expires_at_us = NULL,
                 description_generation = ?4,
                 last_retry_reason = NULL
             WHERE job_id = ?1
               AND job_state = 'leased'
               AND lease_owner = ?2
               AND revision_generation = ?3
               AND EXISTS (
                 SELECT 1 FROM semantic_indexing_jobs j
                 JOIN semantic_frame_revisions f ON f.frame_id = j.frame_id
                 WHERE j.job_id = ?1
                   AND f.revision_seq = ?3
                   AND f.search_current = 1
               )",
        )
        .bind(job_id)
        .bind(worker_id)
        .bind(i64::from(revision_generation))
        .bind(i64::try_from(description_generation).unwrap_or(i64::MAX))
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        if result.rows_affected() == 0 {
            return Ok(false);
        }
        let _ = now_us;
        Ok(true)
    }

    /// Marks embedding provenance and vector payload committed; transitions to ready.
    pub async fn complete_indexing_embedding_cas(
        &self,
        job_id: &str,
        worker_id: &str,
        revision_generation: u32,
        now_us: i64,
    ) -> Result<bool, A2aLabError> {
        let result = sqlx::query(
            "UPDATE semantic_indexing_job_runtime
             SET job_state = 'ready',
                 lease_owner = NULL,
                 lease_expires_at_us = NULL,
                 completed_at_us = ?4,
                 last_retry_reason = NULL
             WHERE job_id = ?1
               AND job_state = 'leased'
               AND lease_owner = ?2
               AND revision_generation = ?3
               AND EXISTS (
                 SELECT 1 FROM semantic_indexing_jobs j
                 JOIN semantic_frame_revisions f ON f.frame_id = j.frame_id
                 WHERE j.job_id = ?1
                   AND f.revision_seq = ?3
                   AND f.search_current = 1
               )",
        )
        .bind(job_id)
        .bind(worker_id)
        .bind(i64::from(revision_generation))
        .bind(now_us)
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn fail_indexing_job_lease(
        &self,
        job_id: &str,
        worker_id: &str,
        revision_generation: u32,
        reason: &str,
        retryable: bool,
        next_retry_at_us: i64,
        now_us: i64,
        limits: &SemanticWorkerLimits,
    ) -> Result<(), A2aLabError> {
        let attempts: i64 = sqlx::query_scalar(
            "SELECT description_attempts + embedding_attempts
             FROM semantic_indexing_job_runtime WHERE job_id = ?1",
        )
        .bind(job_id)
        .fetch_optional(self.semantic_pool())
        .await
        .map_err(db_error)?
        .unwrap_or(0);
        let terminal =
            !retryable || attempts >= i64::from(limits.max_retry_attempts.saturating_add(1));
        let next_state = if terminal {
            IndexingJobState::TerminalFailed.as_str()
        } else {
            IndexingJobState::RetryableFailed.as_str()
        };
        sqlx::query(
            "UPDATE semantic_indexing_job_runtime
             SET job_state = ?4,
                 lease_owner = NULL,
                 lease_expires_at_us = NULL,
                 last_retry_reason = ?5,
                 next_retry_at_us = ?6,
                 last_error_at_us = ?7
             WHERE job_id = ?1
               AND lease_owner = ?2
               AND revision_generation = ?3",
        )
        .bind(job_id)
        .bind(worker_id)
        .bind(i64::from(revision_generation))
        .bind(next_state)
        .bind(reason)
        .bind(next_retry_at_us)
        .bind(now_us)
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        Ok(())
    }

    pub async fn expire_indexing_job(&self, job_id: &str, now_us: i64) -> Result<(), A2aLabError> {
        sqlx::query(
            "UPDATE semantic_indexing_job_runtime
             SET job_state = 'expired',
                 lease_owner = NULL,
                 lease_expires_at_us = NULL,
                 completed_at_us = ?2,
                 last_retry_reason = 'frame_expired'
             WHERE job_id = ?1",
        )
        .bind(job_id)
        .bind(now_us)
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        Ok(())
    }

    /// Loads the singleton configured embedding identity when present.
    pub async fn load_embedding_config_identity(
        &self,
    ) -> Result<Option<ModelSpaceIdentity>, A2aLabError> {
        let row = sqlx::query(
            "SELECT provider, model, dimensions, normalization, metric, model_space_generation
             FROM semantic_embedding_config_identity WHERE singleton_id = 1",
        )
        .fetch_optional(self.semantic_pool())
        .await
        .map_err(db_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(ModelSpaceIdentity::new(
            row.try_get::<String, _>("provider").map_err(db_error)?,
            row.try_get::<String, _>("model").map_err(db_error)?,
            u32::try_from(
                row.try_get::<i64, _>("dimensions")
                    .map_err(db_error)?
                    .max(0),
            )
            .unwrap_or(0),
            super::manifest::parse_normalization(
                &row.try_get::<String, _>("normalization")
                    .map_err(db_error)?,
            )?,
            super::manifest::parse_metric(&row.try_get::<String, _>("metric").map_err(db_error)?)?,
            crate::ProvenanceGeneration::new(
                u64::try_from(
                    row.try_get::<i64, _>("model_space_generation")
                        .map_err(db_error)?
                        .max(0),
                )
                .unwrap_or(0),
            )?,
        )?))
    }

    /// Records the configured embedding identity, rejecting a different model space.
    pub async fn persist_embedding_config_identity(
        &self,
        space: &ModelSpaceIdentity,
        now_us: i64,
    ) -> Result<(), A2aLabError> {
        if let Some(existing) = self.load_embedding_config_identity().await? {
            if &existing != space {
                return Err(A2aLabError::invalid(
                    "semantic_indexing.embeddings",
                    "index and query embedding identity must match the configured model space",
                ));
            }
            return Ok(());
        }
        let digest = format!(
            "{}|{}|{}|{}|{}|{}",
            space.provider(),
            space.model(),
            space.dimensions(),
            normalization_label(space.normalization()),
            metric_label(space.metric()),
            space.generation().get()
        );
        sqlx::query(
            "INSERT INTO semantic_embedding_config_identity (
                singleton_id, provider, model, dimensions, normalization, metric,
                config_digest, model_space_generation, updated_at_us
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(space.provider())
        .bind(space.model())
        .bind(i64::from(space.dimensions()))
        .bind(normalization_label(space.normalization()))
        .bind(metric_label(space.metric()))
        .bind(digest)
        .bind(i64::try_from(space.generation().get()).unwrap_or(i64::MAX))
        .bind(now_us)
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn set_worker_meta_flag(&self, key: &str, enabled: bool) -> Result<(), A2aLabError> {
        let value = if enabled { "true" } else { "false" };
        sqlx::query(
            "INSERT INTO semantic_indexing_worker_meta (meta_key, meta_value)
             VALUES (?1, ?2)
             ON CONFLICT(meta_key) DO UPDATE SET meta_value = excluded.meta_value",
        )
        .bind(key)
        .bind(value)
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        Ok(())
    }

    pub(crate) async fn reconcile_cursor_save(
        &self,
        source_id: &str,
        track_id: &str,
        start_us: i64,
    ) -> Result<(), A2aLabError> {
        sqlx::query(
            "INSERT INTO semantic_reconcile_cursors (source_id, track_id, last_start_us)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(source_id, track_id) DO UPDATE SET last_start_us = excluded.last_start_us",
        )
        .bind(source_id)
        .bind(track_id)
        .bind(start_us)
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        let cursor = format!("{source_id}|{track_id}|{start_us}");
        self.set_worker_meta_string("reconcile_cursor", &cursor)
            .await
    }

    /// Last reconcile cursor for bounded historical discovery batches.
    pub async fn reconcile_cursor(&self) -> Result<Option<(String, String, i64)>, A2aLabError> {
        if let Some(row) = sqlx::query(
            "SELECT source_id, track_id, last_start_us FROM semantic_reconcile_cursors
             ORDER BY source_id, track_id LIMIT 1",
        )
        .fetch_optional(self.semantic_pool())
        .await
        .map_err(db_error)?
        {
            return Ok(Some((
                row.try_get("source_id").map_err(db_error)?,
                row.try_get("track_id").map_err(db_error)?,
                row.try_get("last_start_us").map_err(db_error)?,
            )));
        }
        let value = self.worker_meta_string("reconcile_cursor").await?;
        let Some(value) = value else {
            return Ok(None);
        };
        let mut parts = value.split('|');
        let source = parts
            .next()
            .ok_or_else(|| A2aLabError::invalid("reconcile_cursor", "missing source_id"))?
            .to_owned();
        let track = parts
            .next()
            .ok_or_else(|| A2aLabError::invalid("reconcile_cursor", "missing track_id"))?
            .to_owned();
        let start = parts
            .next()
            .ok_or_else(|| A2aLabError::invalid("reconcile_cursor", "missing start_us"))?
            .parse()
            .map_err(|_| A2aLabError::invalid("reconcile_cursor", "invalid start_us"))?;
        Ok(Some((source, track, start)))
    }

    /// Per-source/track reconcile cursors persisted in the semantic database.
    pub async fn list_reconcile_cursors(&self) -> Result<Vec<(String, String, i64)>, A2aLabError> {
        let rows = sqlx::query(
            "SELECT source_id, track_id, last_start_us FROM semantic_reconcile_cursors
             ORDER BY source_id, track_id",
        )
        .fetch_all(self.semantic_pool())
        .await
        .map_err(db_error)?;
        rows.into_iter()
            .map(|row| {
                Ok((
                    row.try_get("source_id").map_err(db_error)?,
                    row.try_get("track_id").map_err(db_error)?,
                    row.try_get("last_start_us").map_err(db_error)?,
                ))
            })
            .collect()
    }

    async fn worker_meta_string(&self, key: &str) -> Result<Option<String>, A2aLabError> {
        sqlx::query_scalar(
            "SELECT meta_value FROM semantic_indexing_worker_meta WHERE meta_key = ?1",
        )
        .bind(key)
        .fetch_optional(self.semantic_pool())
        .await
        .map_err(db_error)
    }

    async fn set_worker_meta_string(&self, key: &str, value: &str) -> Result<(), A2aLabError> {
        sqlx::query(
            "INSERT INTO semantic_indexing_worker_meta (meta_key, meta_value)
             VALUES (?1, ?2)
             ON CONFLICT(meta_key) DO UPDATE SET meta_value = excluded.meta_value",
        )
        .bind(key)
        .bind(value)
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        Ok(())
    }
}

fn record_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<IndexingJobRecord, A2aLabError> {
    let job_state: String = row.try_get("job_state").map_err(db_error)?;
    let description_generation: Option<i64> =
        row.try_get("description_generation").map_err(db_error)?;
    Ok(IndexingJobRecord {
        job_id: row.try_get("job_id").map_err(db_error)?,
        frame_id: row.try_get("frame_id").map_err(db_error)?,
        job_kind: row.try_get("job_kind").map_err(db_error)?,
        state: IndexingJobState::parse(&job_state)?,
        revision_generation: u32::try_from(
            row.try_get::<i64, _>("revision_generation")
                .map_err(db_error)?
                .max(0),
        )
        .unwrap_or(0),
        lease_owner: row.try_get("lease_owner").map_err(db_error)?,
        lease_expires_at_us: row.try_get("lease_expires_at_us").map_err(db_error)?,
        description_generation: description_generation.and_then(|value| u64::try_from(value).ok()),
        description_attempts: u32::try_from(
            row.try_get::<i64, _>("description_attempts")
                .map_err(db_error)?
                .max(0),
        )
        .unwrap_or(0),
        embedding_attempts: u32::try_from(
            row.try_get::<i64, _>("embedding_attempts")
                .map_err(db_error)?
                .max(0),
        )
        .unwrap_or(0),
        last_retry_reason: row.try_get("last_retry_reason").map_err(db_error)?,
        next_retry_at_us: row
            .try_get::<i64, _>("next_retry_at_us")
            .map_err(db_error)?,
        last_error_at_us: row.try_get("last_error_at_us").map_err(db_error)?,
        completed_at_us: row.try_get("completed_at_us").map_err(db_error)?,
    })
}
