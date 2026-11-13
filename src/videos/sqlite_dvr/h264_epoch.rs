//! Immutable H.264 DVR epochs bound to decoder initialization and picture size.

use crate::error::A2aLabError;
use crate::id::{DvrTrackId, VideoSourceId};
use crate::time::{TimeRange, UtcTimestamp};
use crate::videos::codec::VideoCodec;
use crate::videos::dvr_store::{DvrMediaTrack, DvrPictureProperties};
use sqlx::{Row, SqlitePool};

use super::db::db_error;

pub(crate) async fn ensure_legacy_epoch_column(pool: &SqlitePool) -> Result<(), A2aLabError> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM pragma_table_info('dvr_frames') WHERE name = 'h264_epoch_id' LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(db_error)?;
    if exists.is_some() {
        return Ok(());
    }
    sqlx::query(
        "ALTER TABLE dvr_frames ADD COLUMN h264_epoch_id INTEGER REFERENCES dvr_h264_epochs(epoch_id)",
    )
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn epoch_id_from_track(track_id: &DvrTrackId) -> Option<i64> {
    track_id
        .as_str()
        .strip_prefix("h264-")
        .and_then(|suffix| suffix.parse::<i64>().ok())
}

pub(crate) async fn resolve_epoch_id(
    pool: &SqlitePool,
    source_id: &VideoSourceId,
    codec_init: &[u8],
    opened_at: UtcTimestamp,
) -> Result<i64, A2aLabError> {
    let picture = declared_picture_properties(codec_init)?;
    let width = i64::from(picture.width());
    let height = i64::from(picture.height());
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT epoch_id FROM dvr_h264_epochs
         WHERE source_id = ?1 AND codec_init = ?2 AND width = ?3 AND height = ?4",
    )
    .bind(source_id.as_str())
    .bind(codec_init)
    .bind(width)
    .bind(height)
    .fetch_optional(pool)
    .await
    .map_err(db_error)?;
    if let Some(epoch_id) = existing {
        return Ok(epoch_id);
    }
    let result = sqlx::query(
        "INSERT INTO dvr_h264_epochs (source_id, codec_init, width, height, opened_at_us)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(source_id.as_str())
    .bind(codec_init)
    .bind(width)
    .bind(height)
    .bind(opened_at.as_micros())
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(result.last_insert_rowid())
}

pub(crate) async fn list_h264_epoch_tracks(
    pool: &SqlitePool,
    source_id: &VideoSourceId,
) -> Result<Vec<DvrMediaTrack>, A2aLabError> {
    let rows = sqlx::query(
        "SELECT e.epoch_id, e.codec_init, e.width, e.height,
                MIN(f.start_us) AS min_start, MAX(f.end_us) AS max_end
         FROM dvr_h264_epochs e
         JOIN dvr_frames f ON f.h264_epoch_id = e.epoch_id AND f.payload_ready = 1
         WHERE e.source_id = ?1
         GROUP BY e.epoch_id",
    )
    .bind(source_id.as_str())
    .fetch_all(pool)
    .await
    .map_err(db_error)?;
    let mut tracks = Vec::new();
    for row in rows {
        let epoch_id: i64 = row.try_get("epoch_id").map_err(db_error)?;
        let track_id = DvrTrackId::new(format!("h264-{epoch_id}"))?;
        let init: Vec<u8> = row.try_get("codec_init").map_err(db_error)?;
        let width: i64 = row.try_get("width").map_err(db_error)?;
        let height: i64 = row.try_get("height").map_err(db_error)?;
        let start =
            UtcTimestamp::from_unix_microseconds(row.try_get("min_start").map_err(db_error)?)?;
        let end = UtcTimestamp::from_unix_microseconds(row.try_get("max_end").map_err(db_error)?)?;
        let picture = DvrPictureProperties::new(
            u32::try_from(width.max(0)).unwrap_or(640),
            u32::try_from(height.max(0)).unwrap_or(480),
        )?;
        tracks.push(DvrMediaTrack::with_picture(
            track_id,
            VideoCodec::H264,
            init,
            TimeRange::new(start, end)?,
            picture,
        )?);
    }
    Ok(tracks)
}

pub(crate) async fn prune_unreferenced_epochs(
    pool: &SqlitePool,
    source_id: &str,
) -> Result<(), A2aLabError> {
    sqlx::query(
        "DELETE FROM dvr_h264_epochs
         WHERE source_id = ?1
           AND epoch_id NOT IN (
             SELECT h264_epoch_id FROM dvr_frames
             WHERE source_id = ?1 AND h264_epoch_id IS NOT NULL AND payload_ready = 1
           )",
    )
    .bind(source_id)
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) async fn count_epochs(pool: &SqlitePool, source_id: &str) -> Result<i64, A2aLabError> {
    sqlx::query_scalar("SELECT COUNT(*) FROM dvr_h264_epochs WHERE source_id = ?1")
        .bind(source_id)
        .fetch_one(pool)
        .await
        .map_err(db_error)
}

fn declared_picture_properties(codec_init: &[u8]) -> Result<DvrPictureProperties, A2aLabError> {
    let (width, height) = crate::videos::cmaf::codec_init_picture_size(codec_init);
    DvrPictureProperties::new(width, height)
}
