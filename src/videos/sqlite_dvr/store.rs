//! SQLite-backed recoverable minute-chunk DVR store.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{SqlitePool, Transaction};
use tokio::task;

use crate::error::A2aLabError;
use crate::id::{DvrTrackId, VideoSourceId};
use crate::memory::MemoryVideos;
use crate::time::UtcTimestamp;
use crate::videos::codec::VideoCodec;

use super::chunk::{
    append_payload, chunk_relpath, chunk_tmp_relpath, finalize_chunk, minute_start_us,
};
use super::committed_frame::DvrCommittedFrame;
use super::db::{codec_label, db_error, join_error};
use super::dvr_persist_hook::DvrPersistHook;
use super::h264_epoch;
use super::recovery::recover_partials;
use super::retention;

const SCHEMA: &str = include_str!("dvr_schema.sql");

/// Default on-disk retention (three days).
pub const DEFAULT_DISK_VIDEO_RETENTION_SECS: u64 = 3 * 24 * 60 * 60;

/// Restart-safe indexed DVR with SQLite metadata and atomic minute chunks.
#[derive(Clone)]
pub struct SqliteChunkDvrStore {
    root: PathBuf,
    pool: SqlitePool,
    retain_us: i64,
    hooks: Arc<[Arc<dyn DvrPersistHook>]>,
}

impl SqliteChunkDvrStore {
    /// Opens or creates a DVR store at `root`.
    pub async fn open(root: impl AsRef<Path>, retain_secs: u64) -> Result<Self, A2aLabError> {
        Self::open_with_hooks(root, retain_secs, Vec::new()).await
    }

    /// Opens a store with post-commit hooks.
    pub async fn open_with_hooks(
        root: impl AsRef<Path>,
        retain_secs: u64,
        hooks: Vec<Arc<dyn DvrPersistHook>>,
    ) -> Result<Self, A2aLabError> {
        let root = root.as_ref().to_path_buf();
        if let Some(parent) = root.parent() {
            task::spawn_blocking({
                let parent = parent.to_path_buf();
                move || std::fs::create_dir_all(&parent)
            })
            .await
            .map_err(join_error)?
            .map_err(|error| {
                A2aLabError::unavailable(format!("create {}: {error}", parent.display()))
            })?;
        }
        task::spawn_blocking({
            let root = root.clone();
            move || std::fs::create_dir_all(&root)
        })
        .await
        .map_err(join_error)?
        .map_err(|error| A2aLabError::unavailable(format!("create {}: {error}", root.display())))?;
        let db_path = root.join("index.sqlite");
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .map_err(db_error)?;
        for statement in SCHEMA.split(';') {
            let statement = statement.trim();
            if statement.is_empty() {
                continue;
            }
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(db_error)?;
        }
        h264_epoch::ensure_legacy_epoch_column(&pool).await?;
        let store = Self {
            root,
            pool,
            retain_us: i64::try_from(retain_secs.saturating_mul(1_000_000)).unwrap_or(i64::MAX),
            hooks: hooks.into(),
        };
        store.finish_opening().await?;
        Ok(store)
    }

    async fn finish_opening(&self) -> Result<(), A2aLabError> {
        recover_partials(&self.pool, &self.root).await?;
        retention::cleanup_at(&self.pool, &self.root, self.retain_us, UtcTimestamp::now()).await?;
        recover_partials(&self.pool, &self.root).await?;
        Ok(())
    }

    /// Shared SQLite pool for OT-2 semantic tables.
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Payload root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Indexes one MJPEG frame for `source_id` on the implicit track.
    pub async fn append_mjpeg_frame(
        &self,
        source_id: &VideoSourceId,
        captured_at: UtcTimestamp,
        frame_period_us: u64,
        jpeg: &[u8],
    ) -> Result<(), A2aLabError> {
        let track_id = MemoryVideos::mjpeg_track_id(source_id)?;
        self.ensure_track(
            source_id,
            &track_id,
            VideoCodec::Mjpeg,
            &[],
            frame_period_us,
        )
        .await?;
        self.write_chunk_frame(
            source_id,
            &track_id,
            captured_at,
            frame_period_us,
            true,
            jpeg,
            None,
        )
        .await
    }

    /// Indexes one H.264 access unit on the `h264` track.
    pub async fn append_h264_sample(
        &self,
        source_id: &VideoSourceId,
        captured_at: UtcTimestamp,
        frame_period_us: u64,
        keyframe: bool,
        codec_init: &[u8],
        sample: &[u8],
    ) -> Result<(), A2aLabError> {
        let track_id = DvrTrackId::new("h264")?;
        self.ensure_track(source_id, &track_id, VideoCodec::H264, &[], frame_period_us)
            .await?;
        let epoch_id =
            h264_epoch::resolve_epoch_id(&self.pool, source_id, codec_init, captured_at).await?;
        self.write_chunk_frame(
            source_id,
            &track_id,
            captured_at,
            frame_period_us,
            keyframe,
            sample,
            Some(epoch_id),
        )
        .await
    }

    /// Removes expired rows and payloads for one source.
    pub async fn cleanup_source(
        &self,
        source_id: &VideoSourceId,
        now: UtcTimestamp,
    ) -> Result<(), A2aLabError> {
        retention::cleanup_source(&self.pool, &self.root, source_id, self.retain_us, now).await?;
        recover_partials(&self.pool, &self.root).await
    }

    /// Global retention cleanup across every source.
    pub async fn cleanup_at(&self, now: UtcTimestamp) -> Result<(), A2aLabError> {
        retention::cleanup_at(&self.pool, &self.root, self.retain_us, now).await?;
        recover_partials(&self.pool, &self.root).await
    }

    pub(crate) async fn ensure_track(
        &self,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        codec: VideoCodec,
        codec_init: &[u8],
        frame_period_us: u64,
    ) -> Result<(), A2aLabError> {
        sqlx::query(
            "INSERT INTO dvr_tracks (source_id, track_id, codec, codec_init, frame_period_us)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(source_id, track_id) DO NOTHING",
        )
        .bind(source_id.as_str())
        .bind(track_id.as_str())
        .bind(codec_label(codec))
        .bind(codec_init)
        .bind(i64::try_from(frame_period_us).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn write_chunk_frame(
        &self,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        captured_at: UtcTimestamp,
        frame_period_us: u64,
        keyframe: bool,
        payload: &[u8],
        h264_epoch_id: Option<i64>,
    ) -> Result<(), A2aLabError> {
        let minute = minute_start_us(captured_at);
        self.finalize_chunks_before(source_id, track_id, minute)
            .await?;
        let relpath = chunk_relpath(source_id.as_str(), track_id.as_str(), minute);
        let tmp_relpath = chunk_tmp_relpath(source_id.as_str(), track_id.as_str(), minute);
        sqlx::query(
            "INSERT INTO dvr_chunks (source_id, track_id, minute_start_us, chunk_relpath, chunk_ready)
             VALUES (?1, ?2, ?3, ?4, 0)
             ON CONFLICT(source_id, track_id, minute_start_us) DO NOTHING",
        )
        .bind(source_id.as_str())
        .bind(track_id.as_str())
        .bind(minute)
        .bind(&relpath)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        let tmp_path = self.root.join(&tmp_relpath);
        let (offset, checksum) = append_payload(&tmp_path, payload).await?;
        let end =
            captured_at.saturating_add_micros(i64::try_from(frame_period_us.max(1)).unwrap_or(1));
        let mut tx = self.pool.begin().await.map_err(db_error)?;
        insert_frame_tx(
            &mut tx,
            source_id,
            track_id,
            captured_at,
            end,
            keyframe,
            minute,
            &relpath,
            offset,
            payload.len(),
            checksum,
            h264_epoch_id,
        )
        .await?;
        tx.commit().await.map_err(db_error)?;
        let committed = DvrCommittedFrame::new(
            source_id.clone(),
            track_id.clone(),
            captured_at,
            end,
            keyframe,
            relpath,
            checksum,
            h264_epoch_id,
        );
        self.invoke_hooks(committed).await;
        Ok(())
    }

    async fn invoke_hooks(&self, frame: DvrCommittedFrame) {
        for hook in self.hooks.iter() {
            if let Err(error) = hook.after_frame_committed(&frame).await {
                tracing::warn!(%error, "DVR persist hook failed after commit");
            }
        }
    }

    async fn finalize_chunks_before(
        &self,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        minute: i64,
    ) -> Result<(), A2aLabError> {
        let rows = sqlx::query(
            "SELECT minute_start_us, chunk_relpath FROM dvr_chunks
             WHERE source_id = ?1 AND track_id = ?2 AND chunk_ready = 0 AND minute_start_us < ?3",
        )
        .bind(source_id.as_str())
        .bind(track_id.as_str())
        .bind(minute)
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;
        for row in rows {
            let minute_start: i64 = row.try_get("minute_start_us").map_err(db_error)?;
            let relpath: String = row.try_get("chunk_relpath").map_err(db_error)?;
            self.finalize_chunk_minute(source_id, track_id, minute_start, &relpath)
                .await?;
        }
        Ok(())
    }

    async fn finalize_chunk_minute(
        &self,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        minute_start: i64,
        relpath: &str,
    ) -> Result<(), A2aLabError> {
        let tmp_relpath = chunk_tmp_relpath(source_id.as_str(), track_id.as_str(), minute_start);
        let tmp = self.root.join(&tmp_relpath);
        let final_path = self.root.join(relpath);
        finalize_chunk(&tmp, &final_path).await?;
        sqlx::query(
            "UPDATE dvr_chunks SET chunk_ready = 1, chunk_relpath = ?4
             WHERE source_id = ?1 AND track_id = ?2 AND minute_start_us = ?3",
        )
        .bind(source_id.as_str())
        .bind(track_id.as_str())
        .bind(minute_start)
        .bind(relpath)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }
}

async fn insert_frame_tx(
    tx: &mut Transaction<'_, sqlx::Sqlite>,
    source_id: &VideoSourceId,
    track_id: &DvrTrackId,
    captured_at: UtcTimestamp,
    end: UtcTimestamp,
    keyframe: bool,
    minute: i64,
    relpath: &str,
    offset: u64,
    payload_len: usize,
    checksum: u32,
    h264_epoch_id: Option<i64>,
) -> Result<(), A2aLabError> {
    sqlx::query(
        "INSERT INTO dvr_frames
            (source_id, track_id, start_us, end_us, keyframe, minute_start_us, chunk_relpath,
             payload_offset, payload_length, payload_crc32, payload_ready, h264_epoch_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11)
         ON CONFLICT(source_id, track_id, start_us) DO UPDATE SET
            end_us = excluded.end_us,
            keyframe = excluded.keyframe,
            minute_start_us = excluded.minute_start_us,
            chunk_relpath = excluded.chunk_relpath,
            payload_offset = excluded.payload_offset,
            payload_length = excluded.payload_length,
            payload_crc32 = excluded.payload_crc32,
            payload_ready = 1,
            h264_epoch_id = excluded.h264_epoch_id",
    )
    .bind(source_id.as_str())
    .bind(track_id.as_str())
    .bind(captured_at.as_micros())
    .bind(end.as_micros())
    .bind(i64::from(keyframe))
    .bind(minute)
    .bind(relpath)
    .bind(i64::try_from(offset).unwrap_or(i64::MAX))
    .bind(i64::try_from(payload_len).unwrap_or(i64::MAX))
    .bind(i64::from(checksum))
    .bind(h264_epoch_id)
    .execute(&mut **tx)
    .await
    .map_err(db_error)?;
    Ok(())
}

use sqlx::Row;
