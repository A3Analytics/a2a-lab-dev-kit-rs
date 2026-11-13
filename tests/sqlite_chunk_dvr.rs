//! Focused SQLite chunk DVR tests (TASK-141).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use a2a_lab_dev_kit::{
    DvrCommittedFrame, DvrFrameAtRequest, DvrMediaPayload, DvrMediaStore, DvrPersistHook,
    MemoryVideos, SqliteChunkDvrStore, UtcTimestamp, VideoSourceId,
};
use async_trait::async_trait;

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "a2a-lab-devkit-sqlite-dvr-{name}-{}",
        std::process::id()
    ))
}

fn ts(text: &str) -> UtcTimestamp {
    UtcTimestamp::parse(text).unwrap()
}

fn recent(offset_micros: i64) -> UtcTimestamp {
    UtcTimestamp::now().saturating_add_micros(offset_micros)
}

fn jpeg() -> Vec<u8> {
    vec![0xFF, 0xD8, 0xFF, 0xD9]
}

const MINUTE_US: i64 = 60_000_000;

const INIT_A: &[u8] = &[
    0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, 0x9A, 0x74, 0x03, 0xC0, 0x11, 0x00, 0x00, 0x03,
    0x00, 0x01, 0x00, 0x00, 0x03, 0x00, 0x3C, 0x0F, 0x16, 0x2D, 0x96, 0x00, 0x00, 0x00, 0x01, 0x68,
    0xCE, 0x3C, 0x80,
];

const INIT_B: &[u8] = &[
    0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x28, 0x9A, 0x74, 0x03, 0xC0, 0x11, 0x00, 0x00, 0x03,
    0x00, 0x01, 0x00, 0x00, 0x03, 0x00, 0x3C, 0x0F, 0x16, 0x2D, 0x96, 0x00, 0x00, 0x00, 0x01, 0x68,
    0xCE, 0x3C, 0x80,
];

fn sample(tag: u8) -> Vec<u8> {
    vec![0x00, 0x00, 0x00, 0x01, 0x65, tag]
}

#[tokio::test]
async fn open_exposes_pool_and_schema() {
    let dir = temp_root("schema");
    let _ = std::fs::remove_dir_all(&dir);
    let store = SqliteChunkDvrStore::open(&dir, 86_400).await.unwrap();
    let tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN
         ('dvr_frames', 'dvr_chunks', 'dvr_h264_epochs')",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(tables, 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn crash_recovery_truncates_unindexed_chunk_tail() {
    let dir = temp_root("crc");
    let _ = std::fs::remove_dir_all(&dir);
    let source = VideoSourceId::new("cam-a").unwrap();
    let at = recent(0);
    let track = MemoryVideos::mjpeg_track_id(&source).unwrap();
    let minute = at.as_micros() - at.as_micros().rem_euclid(MINUTE_US);
    let relpath = format!("{}/{track}/{minute}.chunk", source.as_str());
    let tmp_path = dir.join(format!("{relpath}.tmp"));
    {
        let store = SqliteChunkDvrStore::open(&dir, 86_400).await.unwrap();
        store
            .append_mjpeg_frame(&source, at, 1_000_000, &jpeg())
            .await
            .unwrap();
    }
    let mut bytes = std::fs::read(&tmp_path).unwrap();
    bytes.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    std::fs::write(&tmp_path, bytes).unwrap();
    let store = SqliteChunkDvrStore::open(&dir, 86_400).await.unwrap();
    let frame = store
        .frame_at(DvrFrameAtRequest::new(source, track, at, None))
        .await
        .unwrap();
    assert!(matches!(
        frame.payload(),
        DvrMediaPayload::MjpegFrame(bytes) if bytes == &jpeg()
    ));
    assert_eq!(
        std::fs::metadata(&tmp_path).unwrap().len(),
        jpeg().len() as u64
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn h264_epochs_stay_distinct_and_queryable() {
    let dir = temp_root("epochs");
    let _ = std::fs::remove_dir_all(&dir);
    let store = SqliteChunkDvrStore::open(&dir, 86_400).await.unwrap();
    let source = VideoSourceId::new("cam").unwrap();
    store
        .append_h264_sample(
            &source,
            ts("2024-06-01T00:00:00Z"),
            200_000,
            true,
            INIT_A,
            &sample(1),
        )
        .await
        .unwrap();
    store
        .append_h264_sample(
            &source,
            ts("2024-06-01T00:00:01Z"),
            200_000,
            true,
            INIT_A,
            &sample(2),
        )
        .await
        .unwrap();
    store
        .append_h264_sample(
            &source,
            ts("2024-06-01T00:00:02Z"),
            200_000,
            true,
            INIT_B,
            &sample(3),
        )
        .await
        .unwrap();
    let epochs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM dvr_h264_epochs WHERE source_id = ?1")
            .bind(source.as_str())
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(epochs, 2);
    let tracks = store.list_tracks(&source).await.unwrap();
    assert_eq!(tracks.len(), 2);
    let update = sqlx::query("UPDATE dvr_h264_epochs SET width = 1 WHERE source_id = ?1")
        .bind(source.as_str())
        .execute(store.pool())
        .await
        .unwrap();
    assert!(update.rows_affected() > 0);
    let distinct_init: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT codec_init) FROM dvr_h264_epochs WHERE source_id = ?1",
    )
    .bind(source.as_str())
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(distinct_init, 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn retention_drops_chunks_but_keeps_referenced_epochs() {
    let dir = temp_root("retention");
    let _ = std::fs::remove_dir_all(&dir);
    let store = SqliteChunkDvrStore::open(&dir, 120).await.unwrap();
    let source = VideoSourceId::new("cam").unwrap();
    store
        .append_h264_sample(
            &source,
            recent(-1_000_000_000),
            200_000,
            true,
            INIT_A,
            &sample(1),
        )
        .await
        .unwrap();
    store
        .append_h264_sample(
            &source,
            recent(-10_000_000),
            200_000,
            true,
            INIT_A,
            &sample(2),
        )
        .await
        .unwrap();
    store
        .cleanup_source(&source, UtcTimestamp::now())
        .await
        .unwrap();
    let epochs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM dvr_h264_epochs WHERE source_id = ?1")
            .bind(source.as_str())
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(epochs, 1);
    let frames: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM dvr_frames WHERE source_id = ?1 AND payload_ready = 1",
    )
    .bind(source.as_str())
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(frames, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

struct CountingHook {
    calls: Arc<AtomicUsize>,
    fail: bool,
}

#[async_trait]
impl DvrPersistHook for CountingHook {
    async fn after_frame_committed(
        &self,
        _frame: &DvrCommittedFrame,
    ) -> Result<(), a2a_lab_dev_kit::A2aLabError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(a2a_lab_dev_kit::A2aLabError::unavailable("hook fault"));
        }
        Ok(())
    }
}

#[tokio::test]
async fn hook_failures_do_not_rollback_commits() {
    let dir = temp_root("hook");
    let _ = std::fs::remove_dir_all(&dir);
    let calls = Arc::new(AtomicUsize::new(0));
    let hook = Arc::new(CountingHook {
        calls: Arc::clone(&calls),
        fail: true,
    });
    let store = SqliteChunkDvrStore::open_with_hooks(&dir, 86_400, vec![hook])
        .await
        .unwrap();
    let source = VideoSourceId::new("cam").unwrap();
    let track = MemoryVideos::mjpeg_track_id(&source).unwrap();
    store
        .append_mjpeg_frame(&source, ts("2024-06-01T00:00:00Z"), 1_000_000, &jpeg())
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dvr_frames WHERE source_id = ?1")
        .bind(source.as_str())
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
    let _ = store
        .frame_at(DvrFrameAtRequest::new(
            source,
            track,
            ts("2024-06-01T00:00:00Z"),
            None,
        ))
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}
