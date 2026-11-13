//! [`DvrMediaStore`] for [`super::SqliteChunkDvrStore`].

use futures_util::TryStreamExt;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::error::A2aLabError;
use crate::id::{DvrTrackId, VideoSourceId};
use crate::page::{Page, PageRequest};
use crate::time::{TimeRange, UtcTimestamp};
use crate::videos::codec::VideoCodec;
use crate::videos::dvr_store::{
    DvrFrameAtRequest, DvrFrameAtResult, DvrMediaCoverage, DvrMediaPayload, DvrMediaSegment,
    DvrMediaStore, DvrMediaTrack, DvrRetentionBounds, ReadDvrMediaWindowRequest,
};

use super::SqliteChunkDvrStore;
use super::coverage::IntervalFold;
use super::db::{db_error, parse_codec};
use super::h264_epoch;
use super::payload::load_payload_bytes_from_row;

impl DvrMediaStore for SqliteChunkDvrStore {
    async fn retention_bounds(
        &self,
        source_id: &VideoSourceId,
    ) -> Result<DvrRetentionBounds, A2aLabError> {
        self.require_source(source_id).await?;
        let oldest: i64 = sqlx::query_scalar(
            "SELECT MIN(start_us) FROM (
                SELECT start_us FROM dvr_segments WHERE source_id = ?1 AND payload_ready = 1
                UNION ALL
                SELECT start_us FROM dvr_frames WHERE source_id = ?1 AND payload_ready = 1
             )",
        )
        .bind(source_id.as_str())
        .fetch_one(self.pool())
        .await
        .map_err(db_error)?;
        let newest: i64 = sqlx::query_scalar(
            "SELECT MAX(end_us) FROM (
                SELECT end_us FROM dvr_segments WHERE source_id = ?1 AND payload_ready = 1
                UNION ALL
                SELECT end_us FROM dvr_frames WHERE source_id = ?1 AND payload_ready = 1
             )",
        )
        .bind(source_id.as_str())
        .fetch_one(self.pool())
        .await
        .map_err(db_error)?;
        DvrRetentionBounds::new(
            UtcTimestamp::from_unix_microseconds(oldest)?,
            UtcTimestamp::from_unix_microseconds(newest)?,
        )
    }

    async fn media_coverage(
        &self,
        source_id: &VideoSourceId,
    ) -> Result<DvrMediaCoverage, A2aLabError> {
        self.require_source(source_id).await?;
        let period = self.frame_period_us(source_id).await?;
        let mut fold = IntervalFold::new(period);
        self.fold_source_starts(source_id, &mut fold).await?;
        let intervals = fold.finish()?;
        if intervals.is_empty() {
            return Err(A2aLabError::no_recording(
                "source has no retained DVR media",
            ));
        }
        DvrMediaCoverage::new(intervals)
    }

    async fn list_tracks(
        &self,
        source_id: &VideoSourceId,
    ) -> Result<Vec<DvrMediaTrack>, A2aLabError> {
        self.require_source(source_id).await?;
        let rows = sqlx::query(
            "SELECT track_id, codec, codec_init, MIN(start_us) AS min_start, MAX(end_us) AS max_end
             FROM (
                SELECT source_id, track_id, start_us, end_us, codec, codec_init
                FROM dvr_segments
                JOIN dvr_tracks USING (source_id, track_id)
                WHERE source_id = ?1 AND payload_ready = 1 AND track_id != 'h264'
                UNION ALL
                SELECT source_id, track_id, start_us, end_us, codec, codec_init
                FROM dvr_frames
                JOIN dvr_tracks USING (source_id, track_id)
                WHERE source_id = ?1 AND payload_ready = 1 AND track_id != 'h264'
             )
             GROUP BY track_id, codec, codec_init",
        )
        .bind(source_id.as_str())
        .fetch_all(self.pool())
        .await
        .map_err(db_error)?;
        let mut tracks: Vec<DvrMediaTrack> = rows
            .into_iter()
            .map(|row| track_from_row(&row))
            .collect::<Result<_, _>>()?;
        tracks.extend(h264_epoch::list_h264_epoch_tracks(self.pool(), source_id).await?);
        tracks.sort_by_key(|track| track.bounds().start());
        Ok(tracks)
    }

    async fn read_media_window(
        &self,
        request: ReadDvrMediaWindowRequest,
    ) -> Result<Page<DvrMediaSegment>, A2aLabError> {
        self.require_source(request.source_id()).await?;
        if let Some(epoch_id) = h264_epoch::epoch_id_from_track(request.track_id()) {
            return self.read_h264_epoch_window(request, epoch_id).await;
        }
        let window = request.window();
        let page = request.page();
        page.check()?;
        let offset = page_cursor_offset(page)?;
        let limit = i64::from(page.limit());
        let rows = sqlx::query(
            "SELECT track_id, start_us, end_us, keyframe, '' AS payload_relpath, chunk_relpath,
                    payload_offset, payload_length, payload_crc32, codec
             FROM dvr_frames
             JOIN dvr_tracks USING (source_id, track_id)
             WHERE source_id = ?1 AND track_id = ?2
               AND start_us < ?4 AND end_us > ?3
               AND payload_ready = 1
             ORDER BY start_us
             LIMIT ?5 OFFSET ?6",
        )
        .bind(request.source_id().as_str())
        .bind(request.track_id().as_str())
        .bind(window.start().as_micros())
        .bind(window.end().as_micros())
        .bind(limit + 1)
        .bind(offset)
        .fetch_all(self.pool())
        .await
        .map_err(db_error)?;
        let mut items = Vec::new();
        for row in rows
            .iter()
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
        {
            items.push(segment_from_row(self, row).await?);
        }
        let limit_usize = usize::try_from(limit).unwrap_or(usize::MAX);
        let next_cursor = (rows.len() > limit_usize).then(|| (offset + limit).to_string());
        Ok(Page::new(items, next_cursor))
    }

    async fn frame_at(&self, request: DvrFrameAtRequest) -> Result<DvrFrameAtResult, A2aLabError> {
        self.require_source(request.source_id()).await?;
        let at = request.at();
        let row = self
            .containing_frame(request.source_id(), request.track_id(), at.as_micros())
            .await?;
        let Some(row) = row else {
            return self.missing_frame(request.source_id(), at).await;
        };
        let start_us: i64 = row.try_get("start_us").map_err(db_error)?;
        let actual = UtcTimestamp::from_unix_microseconds(start_us)?;
        if let Some(limit) = request.max_delta_us()
            && actual.as_micros().abs_diff(at.as_micros()) > limit
        {
            return Err(A2aLabError::not_found("DVR frame", at.to_string()));
        }
        let segment = segment_from_row(self, &row).await?;
        Ok(DvrFrameAtResult::new(at, actual, segment.payload().clone()))
    }
}

impl SqliteChunkDvrStore {
    async fn read_h264_epoch_window(
        &self,
        request: ReadDvrMediaWindowRequest,
        epoch_id: i64,
    ) -> Result<Page<DvrMediaSegment>, A2aLabError> {
        let window = request.window();
        let page = request.page();
        page.check()?;
        let offset = page_cursor_offset(page)?;
        let limit = i64::from(page.limit());
        let rows = sqlx::query(
            "SELECT ?7 AS track_id, start_us, end_us, keyframe, '' AS payload_relpath, chunk_relpath,
                    payload_offset, payload_length, payload_crc32, codec
             FROM dvr_frames
             JOIN dvr_tracks USING (source_id, track_id)
             WHERE source_id = ?1 AND track_id = 'h264' AND h264_epoch_id = ?2
               AND start_us < ?4 AND end_us > ?3
               AND payload_ready = 1
             ORDER BY start_us
             LIMIT ?5 OFFSET ?6",
        )
        .bind(request.source_id().as_str())
        .bind(epoch_id)
        .bind(window.start().as_micros())
        .bind(window.end().as_micros())
        .bind(limit + 1)
        .bind(offset)
        .bind(request.track_id().as_str())
        .fetch_all(self.pool())
        .await
        .map_err(db_error)?;
        let mut items = Vec::new();
        for row in rows
            .iter()
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
        {
            items.push(segment_from_row(self, row).await?);
        }
        let limit_usize = usize::try_from(limit).unwrap_or(usize::MAX);
        let next_cursor = (rows.len() > limit_usize).then(|| (offset + limit).to_string());
        Ok(Page::new(items, next_cursor))
    }

    async fn require_source(&self, source_id: &VideoSourceId) -> Result<(), A2aLabError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT (
                (SELECT COUNT(*) FROM dvr_segments WHERE source_id = ?1 AND payload_ready = 1)
              + (SELECT COUNT(*) FROM dvr_frames WHERE source_id = ?1 AND payload_ready = 1)
             )",
        )
        .bind(source_id.as_str())
        .fetch_one(self.pool())
        .await
        .map_err(db_error)?;
        if count == 0 {
            return Err(A2aLabError::not_found(
                "video source",
                source_id.to_string(),
            ));
        }
        Ok(())
    }

    async fn frame_period_us(&self, source_id: &VideoSourceId) -> Result<u64, A2aLabError> {
        let period: i64 = sqlx::query_scalar(
            "SELECT frame_period_us FROM dvr_tracks WHERE source_id = ?1 LIMIT 1",
        )
        .bind(source_id.as_str())
        .fetch_one(self.pool())
        .await
        .map_err(db_error)?;
        Ok(u64::try_from(period.max(1)).unwrap_or(1))
    }

    async fn fold_source_starts(
        &self,
        source_id: &VideoSourceId,
        fold: &mut IntervalFold,
    ) -> Result<(), A2aLabError> {
        let mut segments = sqlx::query(
            "SELECT start_us FROM dvr_segments
             WHERE source_id = ?1 AND payload_ready = 1
             ORDER BY start_us",
        )
        .bind(source_id.as_str())
        .fetch(self.pool());
        let mut frames = sqlx::query(
            "SELECT start_us FROM dvr_frames
             WHERE source_id = ?1 AND payload_ready = 1
             ORDER BY start_us",
        )
        .bind(source_id.as_str())
        .fetch(self.pool());
        let mut segment = next_start(&mut segments).await?;
        let mut frame = next_start(&mut frames).await?;
        loop {
            let next = match (segment, frame) {
                (Some(left), Some(right)) if left <= right => {
                    segment = next_start(&mut segments).await?;
                    left
                }
                (Some(left), None) => {
                    segment = next_start(&mut segments).await?;
                    left
                }
                (Some(_) | None, Some(right)) => {
                    frame = next_start(&mut frames).await?;
                    right
                }
                (None, None) => break,
            };
            fold.push(UtcTimestamp::from_unix_microseconds(next)?)?;
        }
        Ok(())
    }

    async fn containing_frame(
        &self,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        at: i64,
    ) -> Result<Option<SqliteRow>, A2aLabError> {
        sqlx::query(
            "SELECT track_id, start_us, end_us, keyframe, '' AS payload_relpath, chunk_relpath,
                    payload_offset, payload_length, payload_crc32, codec
             FROM dvr_frames
             JOIN dvr_tracks USING (source_id, track_id)
             WHERE source_id = ?1 AND track_id = ?2 AND payload_ready = 1
               AND start_us <= ?3 AND end_us > ?3
             ORDER BY start_us DESC
             LIMIT 1",
        )
        .bind(source_id.as_str())
        .bind(track_id.as_str())
        .bind(at)
        .fetch_optional(self.pool())
        .await
        .map_err(db_error)
    }

    async fn missing_frame(
        &self,
        source_id: &VideoSourceId,
        at: UtcTimestamp,
    ) -> Result<DvrFrameAtResult, A2aLabError> {
        let bounds = self.retention_bounds(source_id).await?;
        if at < bounds.oldest() || at >= bounds.newest() {
            Err(A2aLabError::no_recording(
                "requested instant is not retained",
            ))
        } else {
            Err(A2aLabError::recording_gap("requested instant is in a gap"))
        }
    }
}

fn page_cursor_offset(page: &PageRequest) -> Result<i64, A2aLabError> {
    match page.cursor() {
        None => Ok(0),
        Some(cursor) => cursor
            .parse::<i64>()
            .map_err(|_| A2aLabError::invalid("cursor", "is not a valid page cursor")),
    }
}

async fn next_start(
    stream: &mut (impl futures_util::Stream<Item = Result<SqliteRow, sqlx::Error>> + Unpin),
) -> Result<Option<i64>, A2aLabError> {
    let Some(row) = stream.try_next().await.map_err(db_error)? else {
        return Ok(None);
    };
    Ok(Some(row.try_get("start_us").map_err(db_error)?))
}

fn track_from_row(row: &SqliteRow) -> Result<DvrMediaTrack, A2aLabError> {
    let track_id = DvrTrackId::new(row.try_get::<String, _>("track_id").map_err(db_error)?)?;
    let codec = parse_codec(&row.try_get::<String, _>("codec").map_err(db_error)?)?;
    let init: Vec<u8> = row.try_get("codec_init").map_err(db_error)?;
    let start = UtcTimestamp::from_unix_microseconds(row.try_get("min_start").map_err(db_error)?)?;
    let end = UtcTimestamp::from_unix_microseconds(row.try_get("max_end").map_err(db_error)?)?;
    DvrMediaTrack::new(track_id, codec, init, TimeRange::new(start, end)?)
}

async fn segment_from_row(
    store: &SqliteChunkDvrStore,
    row: &SqliteRow,
) -> Result<DvrMediaSegment, A2aLabError> {
    let track_id = DvrTrackId::new(row.try_get::<String, _>("track_id").map_err(db_error)?)?;
    let start = UtcTimestamp::from_unix_microseconds(row.try_get("start_us").map_err(db_error)?)?;
    let end = UtcTimestamp::from_unix_microseconds(row.try_get("end_us").map_err(db_error)?)?;
    let keyframe: i64 = row.try_get("keyframe").map_err(db_error)?;
    let codec = parse_codec(&row.try_get::<String, _>("codec").map_err(db_error)?)?;
    let bytes = load_payload_bytes_from_row(store.root(), row).await?;
    let payload = match codec {
        VideoCodec::Mjpeg => DvrMediaPayload::MjpegFrame(bytes),
        VideoCodec::H264 | VideoCodec::H265 => DvrMediaPayload::EncodedSample { codec, bytes },
    };
    DvrMediaSegment::new(
        track_id,
        TimeRange::new(start, end)?,
        keyframe != 0,
        payload,
    )
}
