//! Startup tail recovery for chunk DVR.

use std::path::Path;

use sqlx::SqlitePool;
use tokio::task;

use crate::error::A2aLabError;

use super::chunk::truncate_chunk_files;
use super::db::{db_error, join_error};

pub(crate) async fn recover_partials(pool: &SqlitePool, root: &Path) -> Result<(), A2aLabError> {
    let root_owned = root.to_path_buf();
    task::spawn_blocking(move || remove_stale_legacy_tmp(&root_owned))
        .await
        .map_err(join_error)??;
    sqlx::query(
        "DELETE FROM dvr_segments
         WHERE payload_ready = 0 OR payload_relpath = ''",
    )
    .execute(pool)
    .await
    .map_err(db_error)?;
    sqlx::query("DELETE FROM dvr_frames WHERE payload_ready = 0")
        .execute(pool)
        .await
        .map_err(db_error)?;
    let indexed = sqlx::query(
        "SELECT chunk_relpath, MAX(payload_offset + payload_length) AS chunk_end
         FROM dvr_frames
         WHERE payload_ready = 1 AND chunk_relpath != ''
         GROUP BY chunk_relpath",
    )
    .fetch_all(pool)
    .await
    .map_err(db_error)?;
    let mut truncate_to = Vec::new();
    for row in indexed {
        let relpath: String = row.try_get("chunk_relpath").map_err(db_error)?;
        let end: i64 = row.try_get("chunk_end").map_err(db_error)?;
        truncate_to.push((relpath, u64::try_from(end.max(0)).unwrap_or(0)));
    }
    let truncate_root = root.to_path_buf();
    task::spawn_blocking(move || truncate_chunk_files(&truncate_root, &truncate_to))
        .await
        .map_err(join_error)??;
    sqlx::query(
        "DELETE FROM dvr_chunks
         WHERE chunk_ready = 0
           AND NOT EXISTS (
             SELECT 1 FROM dvr_frames
             WHERE dvr_frames.source_id = dvr_chunks.source_id
               AND dvr_frames.track_id = dvr_chunks.track_id
               AND dvr_frames.minute_start_us = dvr_chunks.minute_start_us
               AND dvr_frames.payload_ready = 1
           )",
    )
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(())
}

fn remove_stale_legacy_tmp(root: &Path) -> Result<(), A2aLabError> {
    if !root.is_dir() {
        return Ok(());
    }
    walk_tmp_files(root, &mut |path| {
        let _ = std::fs::remove_file(path);
        Ok(())
    })
}

fn walk_tmp_files(
    root: &Path,
    remove: &mut dyn FnMut(&Path) -> Result<(), A2aLabError>,
) -> Result<(), A2aLabError> {
    if !root.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)
        .map_err(|error| A2aLabError::unavailable(format!("read {}: {error}", root.display())))?
    {
        let entry = entry.map_err(|error| {
            A2aLabError::unavailable(format!("read {}: {error}", root.display()))
        })?;
        let path = entry.path();
        if path.is_dir() {
            walk_tmp_files(&path, remove)?;
        } else if path.extension().is_some_and(|ext| ext == "tmp") {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".chunk.tmp"))
            {
                continue;
            }
            remove(&path)?;
        }
    }
    Ok(())
}

use sqlx::Row;
