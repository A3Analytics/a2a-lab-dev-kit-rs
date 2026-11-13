//! Retention cleanup for chunk DVR.

use std::path::Path;

use sqlx::SqlitePool;
use tokio::task;

use crate::error::A2aLabError;
use crate::id::VideoSourceId;
use crate::time::UtcTimestamp;

use super::db::{db_error, join_error};
use super::h264_epoch;

pub(crate) async fn cleanup_source(
    pool: &SqlitePool,
    root: &Path,
    source_id: &VideoSourceId,
    retain_us: i64,
    now: UtcTimestamp,
) -> Result<(), A2aLabError> {
    let cutoff = now.saturating_add_micros(-retain_us);
    cleanup_source_id(pool, root, source_id.as_str(), cutoff).await
}

pub(crate) async fn cleanup_at(
    pool: &SqlitePool,
    root: &Path,
    retain_us: i64,
    now: UtcTimestamp,
) -> Result<(), A2aLabError> {
    let sources = sqlx::query(
        "SELECT DISTINCT source_id FROM dvr_segments
         UNION
         SELECT DISTINCT source_id FROM dvr_frames",
    )
    .fetch_all(pool)
    .await
    .map_err(db_error)?;
    for row in sources {
        let source_id: String = row.try_get("source_id").map_err(db_error)?;
        let cutoff = now.saturating_add_micros(-retain_us);
        cleanup_source_id(pool, root, &source_id, cutoff).await?;
    }
    Ok(())
}

async fn cleanup_source_id(
    pool: &SqlitePool,
    root: &Path,
    source_id: &str,
    cutoff: UtcTimestamp,
) -> Result<(), A2aLabError> {
    let rows = sqlx::query(
        "SELECT track_id, start_us, payload_relpath FROM dvr_segments
         WHERE source_id = ?1 AND start_us < ?2",
    )
    .bind(source_id)
    .bind(cutoff.as_micros())
    .fetch_all(pool)
    .await
    .map_err(db_error)?;
    let root = root.to_path_buf();
    for row in rows {
        let relpath: String = row.try_get("payload_relpath").map_err(db_error)?;
        let path = root.join(relpath);
        let _ = task::spawn_blocking(move || std::fs::remove_file(path))
            .await
            .map_err(join_error)?;
    }
    sqlx::query("DELETE FROM dvr_segments WHERE source_id = ?1 AND start_us < ?2")
        .bind(source_id)
        .bind(cutoff.as_micros())
        .execute(pool)
        .await
        .map_err(db_error)?;
    let frame_rows = sqlx::query(
        "SELECT DISTINCT chunk_relpath FROM dvr_frames
         WHERE source_id = ?1 AND start_us < ?2 AND chunk_relpath != ''",
    )
    .bind(source_id)
    .bind(cutoff.as_micros())
    .fetch_all(pool)
    .await
    .map_err(db_error)?;
    sqlx::query("DELETE FROM dvr_frames WHERE source_id = ?1 AND start_us < ?2")
        .bind(source_id)
        .bind(cutoff.as_micros())
        .execute(pool)
        .await
        .map_err(db_error)?;
    for row in frame_rows {
        let relpath: String = row.try_get("chunk_relpath").map_err(db_error)?;
        remove_chunk_if_empty(pool, &root, relpath).await?;
    }
    h264_epoch::prune_unreferenced_epochs(pool, source_id).await?;
    Ok(())
}

async fn remove_chunk_if_empty(
    pool: &SqlitePool,
    root: &Path,
    relpath: String,
) -> Result<(), A2aLabError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM dvr_frames WHERE chunk_relpath = ?1 AND payload_ready = 1",
    )
    .bind(&relpath)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;
    if count != 0 {
        return Ok(());
    }
    sqlx::query("DELETE FROM dvr_chunks WHERE chunk_relpath = ?1")
        .bind(&relpath)
        .execute(pool)
        .await
        .map_err(db_error)?;
    let path = root.join(&relpath);
    let tmp = root.join(format!("{relpath}.tmp"));
    task::spawn_blocking(move || {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(tmp);
    })
    .await
    .map_err(join_error)?;
    Ok(())
}

use sqlx::Row;
