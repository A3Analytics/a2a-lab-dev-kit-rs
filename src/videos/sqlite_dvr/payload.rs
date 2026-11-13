//! On-disk payload reads for chunk-indexed frames.

use std::path::Path;

use sqlx::Row;
use sqlx::sqlite::SqliteRow;
use tokio::task;

use crate::error::A2aLabError;

use super::chunk::read_payload_range;
use super::db::{db_error, join_error};

pub(crate) async fn load_chunk_payload(
    root: &Path,
    relpath: &str,
    offset: u64,
    length: u64,
    expected_crc32: u32,
) -> Result<Vec<u8>, A2aLabError> {
    let root = root.to_path_buf();
    let final_path = root.join(relpath);
    let tmp_path = root.join(format!("{relpath}.tmp"));
    let path = if tmp_path.exists() {
        tmp_path
    } else {
        final_path
    };
    read_payload_range(&path, offset, length, expected_crc32).await
}

pub(crate) async fn load_payload_bytes_from_row(
    root: &Path,
    row: &SqliteRow,
) -> Result<Vec<u8>, A2aLabError> {
    let chunk_relpath: String = row.try_get("chunk_relpath").map_err(db_error)?;
    if chunk_relpath.is_empty() {
        let relpath: String = row.try_get("payload_relpath").map_err(db_error)?;
        return load_legacy_payload(root, &relpath).await;
    }
    let offset: i64 = row.try_get("payload_offset").map_err(db_error)?;
    let length: i64 = row.try_get("payload_length").map_err(db_error)?;
    let crc32: i64 = row.try_get("payload_crc32").map_err(db_error)?;
    load_chunk_payload(
        root,
        &chunk_relpath,
        u64::try_from(offset.max(0)).unwrap_or(0),
        u64::try_from(length.max(0)).unwrap_or(0),
        u32::try_from(crc32.max(0)).unwrap_or(0),
    )
    .await
}

async fn load_legacy_payload(root: &Path, relpath: &str) -> Result<Vec<u8>, A2aLabError> {
    let path = root.join(relpath);
    let display = path.display().to_string();
    task::spawn_blocking(move || std::fs::read(path))
        .await
        .map_err(join_error)?
        .map_err(|error| A2aLabError::unavailable(format!("read {display}: {error}")))
}
