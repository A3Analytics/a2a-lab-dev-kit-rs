//! Scoped pending/ready/failed/skipped/expired coverage for semantic search.

use sqlx::Row;

use crate::error::A2aLabError;
use crate::page::Page;
use crate::time::{TimeRange, UtcTimestamp};
use crate::videos::saved_frame::SavedFrameRef;
use crate::videos::semantic_search::{
    SemanticIndexingFreshness, SemanticIndexingStateCounts, SemanticSearchScope,
};

use super::db::db_error;
use super::engine::SemanticChunkDvrStore;
use super::manifest::saved_frame_from_row;

impl SemanticChunkDvrStore {
    /// Counts in-scope revisions by exclusive coverage class.
    pub async fn semantic_indexing_coverage_snapshot(
        &self,
        scope: &SemanticSearchScope,
    ) -> Result<(SemanticIndexingStateCounts, SemanticIndexingFreshness), A2aLabError> {
        let (start_us, end_us) = scope_bounds(scope);
        let rows = sqlx::query(
            "SELECT rev.start_us, rev.retained_state, rev.description_checkpoint,
                    rev.embedding_checkpoint, r.job_state
             FROM semantic_frame_revisions rev
             JOIN semantic_stage_eligibility e ON e.winner_frame_id = rev.frame_id
             LEFT JOIN semantic_indexing_jobs j
               ON j.frame_id = rev.frame_id AND j.job_kind = 'embeddings'
             LEFT JOIN semantic_indexing_job_runtime r ON r.job_id = j.job_id
             WHERE rev.source_id = ?1 AND rev.start_us >= ?2 AND rev.start_us < ?3
             GROUP BY rev.frame_id",
        )
        .bind(scope.source_id().as_str())
        .bind(start_us)
        .bind(end_us)
        .fetch_all(self.semantic_pool())
        .await
        .map_err(db_error)?;
        let mut pending = 0;
        let mut ready = 0;
        let mut failed = 0;
        let mut expired = 0;
        let mut skipped = 0;
        let mut oldest_pending: Option<i64> = None;
        let mut newest_ready: Option<i64> = None;
        let mut newest_failed: Option<i64> = None;
        for row in rows {
            let start_us: i64 = row.try_get("start_us").map_err(db_error)?;
            let retained: String = row.try_get("retained_state").map_err(db_error)?;
            let description: String = row.try_get("description_checkpoint").map_err(db_error)?;
            let embedding: String = row.try_get("embedding_checkpoint").map_err(db_error)?;
            let job_state: Option<String> = row.try_get("job_state").map_err(db_error)?;
            match classify_coverage(&retained, job_state.as_deref(), &description, &embedding) {
                CoverageClass::Pending => {
                    pending += 1;
                    oldest_pending =
                        Some(oldest_pending.map_or(start_us, |value| value.min(start_us)));
                }
                CoverageClass::Ready => {
                    ready += 1;
                    newest_ready = Some(newest_ready.map_or(start_us, |value| value.max(start_us)));
                }
                CoverageClass::Failed => {
                    failed += 1;
                    newest_failed =
                        Some(newest_failed.map_or(start_us, |value| value.max(start_us)));
                }
                CoverageClass::Expired => expired += 1,
                CoverageClass::Skipped => skipped += 1,
            }
        }
        Ok((
            SemanticIndexingStateCounts::new(pending, ready, failed, expired).with_skipped(skipped),
            SemanticIndexingFreshness::new(
                timestamp_opt(oldest_pending)?,
                timestamp_opt(newest_ready)?,
                timestamp_opt(newest_failed)?,
            ),
        ))
    }

    /// Bounded neighbor revisions around one capture instant.
    pub async fn list_semantic_playback_neighbors(
        &self,
        source_id: &str,
        captured_at_us: i64,
        max_frames: u32,
    ) -> Result<Page<SavedFrameRef>, A2aLabError> {
        let fetch = i64::from(max_frames.max(1));
        let rows = sqlx::query(
            "SELECT frame_id, store_namespace, source_id, track_id, start_us, end_us,
                    payload_sha256, payload_crc32, capture_group, retained_state, search_current,
                    revision_seq, description_checkpoint, embedding_checkpoint
             FROM semantic_frame_revisions
             WHERE source_id = ?1
               AND search_current = 1
               AND retained_state = 'retained'
             ORDER BY ABS(start_us - ?2) ASC, start_us ASC, frame_id ASC
             LIMIT ?3",
        )
        .bind(source_id)
        .bind(captured_at_us)
        .bind(fetch)
        .fetch_all(self.semantic_pool())
        .await
        .map_err(db_error)?;
        let mut frames = Vec::new();
        for row in rows {
            frames.push(saved_frame_from_row(&row)?);
        }
        frames.sort_by_key(|frame| {
            (
                frame.captured_at().as_micros(),
                frame.frame_id().as_str().to_owned(),
            )
        });
        Ok(Page::new(frames, None::<String>))
    }
}

#[derive(Clone, Copy)]
enum CoverageClass {
    Pending,
    Ready,
    Failed,
    Expired,
    Skipped,
}

fn classify_coverage(
    retained: &str,
    job_state: Option<&str>,
    description: &str,
    embedding: &str,
) -> CoverageClass {
    if retained != "retained" || job_state == Some("expired") {
        return CoverageClass::Expired;
    }
    if job_state == Some("skipped") {
        return CoverageClass::Skipped;
    }
    if description == "failed" || embedding == "failed" || job_state == Some("terminal_failed") {
        return CoverageClass::Failed;
    }
    if description == "ready" && embedding == "ready" {
        return CoverageClass::Ready;
    }
    CoverageClass::Pending
}

fn scope_bounds(scope: &SemanticSearchScope) -> (i64, i64) {
    match scope.range() {
        Some(range) => (range.start().as_micros(), range.end().as_micros()),
        None => (i64::MIN / 2, i64::MAX / 2),
    }
}

fn timestamp_opt(value: Option<i64>) -> Result<Option<UtcTimestamp>, A2aLabError> {
    value.map(UtcTimestamp::from_unix_microseconds).transpose()
}

#[must_use]
pub fn neighbor_window(frames: &[SavedFrameRef], captured_at: UtcTimestamp) -> TimeRange {
    let start = frames
        .first()
        .map_or(captured_at, SavedFrameRef::captured_at);
    let last = frames
        .last()
        .map_or(captured_at, SavedFrameRef::captured_at);
    let end = if last > start {
        last.saturating_add_micros(1)
    } else {
        start.saturating_add_micros(1)
    };
    TimeRange::new(start, end).unwrap_or_else(|_| {
        TimeRange::new(captured_at, captured_at.saturating_add_micros(1)).expect("window")
    })
}
