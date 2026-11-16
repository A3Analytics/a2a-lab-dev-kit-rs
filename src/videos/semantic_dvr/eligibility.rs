//! Deterministic stage eligibility before indexing work enters a queue.

use sqlx::{Row, Sqlite, Transaction};

use crate::config::{SemanticSchedule, SemanticStreamSchedule};
use crate::error::A2aLabError;
use crate::videos::semantic_indexing::SemanticIndexingStage;

use super::db::db_error;
use super::eligibility_types::{
    StageEligibilityRecord, eligibility_winner_is_better, interval_bucket_start,
    stage_eligibility_job_id,
};
use super::engine::SemanticChunkDvrStore;
use super::indexing_types::ensure_job_runtime_tx;

const STAGES: [SemanticIndexingStage; 2] = [
    SemanticIndexingStage::Description,
    SemanticIndexingStage::Embeddings,
];

#[derive(Clone, Copy)]
struct StageEligibilityWrite<'a> {
    source_id: &'a str,
    track_id: &'a str,
    stage: &'a str,
    frame_id: &'a str,
    start_us: i64,
    schedule: SemanticSchedule,
    enqueue_priority: i32,
    now_us: i64,
}

impl SemanticChunkDvrStore {
    /// Replaces in-memory stream schedules used by live ingest and reconciliation.
    pub fn set_stream_schedules(&self, schedules: Vec<SemanticStreamSchedule>) {
        *self
            .schedules
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = schedules;
    }

    /// Materializes description and embedding eligibility for one retained frame.
    pub async fn materialize_frame_eligibility(
        &self,
        source_id: &str,
        track_id: &str,
        frame_id: &str,
        start_us: i64,
        enqueue_priority: i32,
        now_us: i64,
    ) -> Result<u32, A2aLabError> {
        let mut tx = self.semantic_pool().begin().await.map_err(db_error)?;
        let created = self
            .materialize_frame_eligibility_tx(
                &mut tx,
                source_id,
                track_id,
                frame_id,
                start_us,
                enqueue_priority,
                now_us,
            )
            .await?;
        tx.commit().await.map_err(db_error)?;
        Ok(created)
    }

    pub(crate) async fn materialize_frame_eligibility_tx(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        source_id: &str,
        track_id: &str,
        frame_id: &str,
        start_us: i64,
        enqueue_priority: i32,
        now_us: i64,
    ) -> Result<u32, A2aLabError> {
        let schedule = self.schedule_for_source(source_id);
        let mut created = 0_u32;
        for stage in STAGES {
            let added: u32 = upsert_stage_eligibility(
                tx,
                &StageEligibilityWrite {
                    source_id,
                    track_id,
                    stage: stage.id(),
                    frame_id,
                    start_us,
                    schedule: schedule.schedule_for_stage(stage.id()),
                    enqueue_priority,
                    now_us,
                },
            )
            .await?;
            created = created.saturating_add(added);
        }
        sqlx::query(
            "INSERT INTO semantic_reconcile_cursors (source_id, track_id, last_start_us)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(source_id, track_id) DO UPDATE SET last_start_us = excluded.last_start_us",
        )
        .bind(source_id)
        .bind(track_id)
        .bind(start_us)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
        Ok(created)
    }

    /// Durable eligibility slots, ordered by identity then bucket.
    pub async fn list_stage_eligibility(&self) -> Result<Vec<StageEligibilityRecord>, A2aLabError> {
        let rows = sqlx::query(
            "SELECT source_id, track_id, stage, bucket_start_us, interval_us,
                    winner_frame_id, winner_start_us
             FROM semantic_stage_eligibility
             ORDER BY source_id, track_id, stage, bucket_start_us",
        )
        .fetch_all(self.semantic_pool())
        .await
        .map_err(db_error)?;
        rows.iter().map(record_from_row).collect()
    }

    pub async fn count_stage_eligibility(&self) -> Result<u64, A2aLabError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM semantic_stage_eligibility")
            .fetch_one(self.semantic_pool())
            .await
            .map_err(db_error)?;
        Ok(u64::try_from(count.max(0)).unwrap_or(0))
    }

    fn schedule_for_source(&self, source_id: &str) -> SemanticStreamSchedule {
        let schedules = self
            .schedules
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        schedules
            .iter()
            .find(|schedule| schedule.stream() == source_id)
            .cloned()
            .unwrap_or_else(|| SemanticStreamSchedule::every_saved_frame(source_id))
    }
}

async fn upsert_stage_eligibility(
    tx: &mut Transaction<'_, Sqlite>,
    write: &StageEligibilityWrite<'_>,
) -> Result<u32, A2aLabError> {
    let StageEligibilityWrite {
        source_id,
        track_id,
        stage,
        frame_id,
        start_us,
        schedule,
        enqueue_priority,
        now_us,
    } = *write;
    let (bucket_start_us, interval_us) = match schedule.interval_micros() {
        None => (start_us, None),
        Some(micros) => {
            let interval_us = i64::try_from(micros).map_err(|_| {
                A2aLabError::invalid("semantic_indexing.schedules", "interval is too large")
            })?;
            (
                interval_bucket_start(start_us, interval_us),
                Some(interval_us),
            )
        }
    };
    let job_id = stage_eligibility_job_id(source_id, track_id, stage, bucket_start_us);
    let existing = sqlx::query(
        "SELECT winner_frame_id, winner_start_us,
                EXISTS (
                    SELECT 1 FROM semantic_frame_revisions r
                    WHERE r.frame_id = semantic_stage_eligibility.winner_frame_id
                      AND r.search_current = 1
                      AND r.retained_state = 'retained'
                ) AS winner_retained,
                EXISTS (
                    SELECT 1 FROM semantic_indexing_jobs j
                    JOIN semantic_indexing_job_runtime rt ON rt.job_id = j.job_id
                    WHERE j.job_id = ?5
                      AND rt.job_state NOT IN ('pending', 'retryable_failed')
                ) AS slot_processed
         FROM semantic_stage_eligibility
         WHERE source_id = ?1 AND track_id = ?2 AND stage = ?3 AND bucket_start_us = ?4",
    )
    .bind(source_id)
    .bind(track_id)
    .bind(stage)
    .bind(bucket_start_us)
    .bind(&job_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_error)?;
    let should_write = match existing.as_ref() {
        None => true,
        Some(row) => {
            let processed: i64 = row.try_get("slot_processed").map_err(db_error)?;
            if processed != 0 {
                false
            } else {
                let retained: i64 = row.try_get("winner_retained").map_err(db_error)?;
                let current_frame: String = row.try_get("winner_frame_id").map_err(db_error)?;
                let current_start: i64 = row.try_get("winner_start_us").map_err(db_error)?;
                retained == 0
                    || eligibility_winner_is_better(
                        start_us,
                        frame_id,
                        current_start,
                        &current_frame,
                    )
            }
        }
    };
    if !should_write {
        return Ok(0);
    }
    sqlx::query(
        "INSERT INTO semantic_stage_eligibility (
            source_id, track_id, stage, bucket_start_us, interval_us,
            winner_frame_id, winner_start_us, enqueue_priority, created_at_us, updated_at_us
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
         ON CONFLICT(source_id, track_id, stage, bucket_start_us) DO UPDATE SET
            winner_frame_id = excluded.winner_frame_id,
            winner_start_us = excluded.winner_start_us,
            interval_us = excluded.interval_us,
            updated_at_us = excluded.updated_at_us",
    )
    .bind(source_id)
    .bind(track_id)
    .bind(stage)
    .bind(bucket_start_us)
    .bind(interval_us)
    .bind(frame_id)
    .bind(start_us)
    .bind(i64::from(enqueue_priority))
    .bind(now_us)
    .execute(&mut **tx)
    .await
    .map_err(db_error)?;
    ensure_eligibility_job_tx(tx, &job_id, frame_id, stage, start_us, enqueue_priority).await
}

async fn ensure_eligibility_job_tx(
    tx: &mut Transaction<'_, Sqlite>,
    job_id: &str,
    frame_id: &str,
    stage: &str,
    created_at_us: i64,
    enqueue_priority: i32,
) -> Result<u32, A2aLabError> {
    let existed: Option<String> =
        sqlx::query_scalar("SELECT job_id FROM semantic_indexing_jobs WHERE job_id = ?1")
            .bind(job_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db_error)?;
    sqlx::query(
        "INSERT INTO semantic_indexing_jobs (job_id, frame_id, job_kind, created_at_us, enqueue_priority)
         SELECT ?1, ?2, ?3, ?4, ?5
         WHERE EXISTS (SELECT 1 FROM semantic_frame_revisions WHERE frame_id = ?2)
         ON CONFLICT(job_id) DO UPDATE SET frame_id = excluded.frame_id",
    )
    .bind(job_id)
    .bind(frame_id)
    .bind(stage)
    .bind(created_at_us)
    .bind(i64::from(enqueue_priority))
    .execute(&mut **tx)
    .await
    .map_err(db_error)?;
    ensure_job_runtime_tx(tx, job_id).await?;
    Ok(u32::from(existed.is_none()))
}

fn record_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<StageEligibilityRecord, A2aLabError> {
    Ok(StageEligibilityRecord {
        source_id: row.try_get("source_id").map_err(db_error)?,
        track_id: row.try_get("track_id").map_err(db_error)?,
        stage: row.try_get("stage").map_err(db_error)?,
        bucket_start_us: row.try_get("bucket_start_us").map_err(db_error)?,
        interval_us: row.try_get("interval_us").map_err(db_error)?,
        winner_frame_id: row.try_get("winner_frame_id").map_err(db_error)?,
        winner_start_us: row.try_get("winner_start_us").map_err(db_error)?,
    })
}
