//! Retained DVR payload reads for manifest correlation.

use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::error::A2aLabError;
use crate::id::{DvrTrackId, VideoSourceId};
use crate::time::UtcTimestamp;
use crate::videos::sqlite_dvr::{crc32, load_payload_bytes_from_row};

use super::db::db_error;
use super::engine::SemanticChunkDvrStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetainedStorageKind {
    LegacySegment,
    ChunkFrame,
}

fn retained_payload_fallback_allowed(error: &A2aLabError) -> bool {
    let message = error.to_string();
    message.contains("checksum mismatch")
        || message.contains("not found")
        || message.contains("read ")
        || message.contains("open ")
        || message.contains("seek ")
}

impl SemanticChunkDvrStore {
    pub(crate) async fn load_retained_payload_at_key(
        &self,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        start_us: UtcTimestamp,
    ) -> Result<(Vec<u8>, u32, UtcTimestamp), A2aLabError> {
        let key = start_us.as_micros();
        let segment = sqlx::query(
            "SELECT end_us, payload_relpath, '' AS chunk_relpath, NULL AS payload_offset,
                    NULL AS payload_length, NULL AS payload_crc32
             FROM dvr_segments
             WHERE source_id = ?1 AND track_id = ?2 AND start_us = ?3 AND payload_ready = 1",
        )
        .bind(source_id.as_str())
        .bind(track_id.as_str())
        .bind(key)
        .fetch_optional(self.media_pool())
        .await
        .map_err(db_error)?;
        let frame = sqlx::query(
            "SELECT end_us, '' AS payload_relpath, chunk_relpath, payload_offset, payload_length,
                    payload_crc32
             FROM dvr_frames
             WHERE source_id = ?1 AND track_id = ?2 AND start_us = ?3 AND payload_ready = 1",
        )
        .bind(source_id.as_str())
        .bind(track_id.as_str())
        .bind(key)
        .fetch_optional(self.media_pool())
        .await
        .map_err(db_error)?;
        let mut candidates = Vec::with_capacity(2);
        if segment.is_some() {
            candidates.push(RetainedStorageKind::LegacySegment);
        }
        if frame.is_some() {
            candidates.push(RetainedStorageKind::ChunkFrame);
        }
        if candidates.is_empty() {
            return Err(A2aLabError::not_found(
                "DVR frame",
                format!("{}/{}/{}", source_id.as_str(), track_id.as_str(), key),
            ));
        }
        let mut last_err: Option<A2aLabError> = None;
        for kind in candidates {
            let row = match kind {
                RetainedStorageKind::LegacySegment => segment.as_ref(),
                RetainedStorageKind::ChunkFrame => frame.as_ref(),
            };
            let Some(row) = row else {
                continue;
            };
            match self.payload_tuple_from_retained_row(row).await {
                Ok(tuple) => return Ok(tuple),
                Err(error) if retained_payload_fallback_allowed(&error) => {
                    last_err = Some(error);
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            A2aLabError::not_found(
                "DVR frame",
                format!("{}/{}/{}", source_id.as_str(), track_id.as_str(), key),
            )
        }))
    }

    async fn payload_tuple_from_retained_row(
        &self,
        row: &SqliteRow,
    ) -> Result<(Vec<u8>, u32, UtcTimestamp), A2aLabError> {
        let end_us: i64 = row.try_get("end_us").map_err(db_error)?;
        let bytes = load_payload_bytes_from_row(self.chunk().root(), row).await?;
        let crc = crc32(&bytes);
        Ok((bytes, crc, UtcTimestamp::from_unix_microseconds(end_us)?))
    }

    pub(crate) async fn chunk_frame_row(
        &self,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        start_us: UtcTimestamp,
    ) -> Result<SqliteRow, A2aLabError> {
        let key = start_us.as_micros();
        let frame = sqlx::query(
            "SELECT end_us, '' AS payload_relpath, chunk_relpath, payload_offset, payload_length,
                    payload_crc32
             FROM dvr_frames
             WHERE source_id = ?1 AND track_id = ?2 AND start_us = ?3 AND payload_ready = 1",
        )
        .bind(source_id.as_str())
        .bind(track_id.as_str())
        .bind(key)
        .fetch_optional(self.media_pool())
        .await
        .map_err(db_error)?;
        frame.ok_or_else(|| {
            A2aLabError::not_found(
                "DVR frame",
                format!("{}/{}/{}", source_id.as_str(), track_id.as_str(), key),
            )
        })
    }
}
