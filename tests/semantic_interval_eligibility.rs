//! Deterministic semantic interval eligibility (TASK-149).

use a2a_lab_dev_kit::{
    DvrStoreNamespace, MemoryVideos, SemanticChunkDvrStore, SemanticIndexingStage,
    SemanticSchedule, SemanticStreamSchedule, SqliteChunkDvrStore, UtcTimestamp, VideoSourceId,
    interval_bucket_start,
};

const FPS_PERIOD_US: i64 = 200_000;
const INTERVAL_US: i64 = 5_000_000;
const ALIGNED_EPOCH_US: i64 = 1_700_000_000_000_000;

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "a2a-lab-devkit-semantic-elig-{name}-{}",
        std::process::id()
    ))
}

fn store_ns() -> DvrStoreNamespace {
    DvrStoreNamespace::new("test.video-dvr").unwrap()
}

fn jpeg(tag: u8) -> Vec<u8> {
    vec![0xFF, 0xD8, tag, 0xD9]
}

fn interval_micros() -> u64 {
    u64::try_from(INTERVAL_US).unwrap()
}

fn interval_schedule(stream: &str) -> SemanticStreamSchedule {
    SemanticStreamSchedule::with_stages(
        stream,
        SemanticSchedule::Interval {
            micros: interval_micros(),
        },
        SemanticSchedule::Interval {
            micros: interval_micros(),
        },
    )
}

fn jpeg_at(index: usize) -> Vec<u8> {
    jpeg(u8::try_from(index).unwrap_or(u8::MAX))
}

fn capture_at(offset_us: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_microseconds(ALIGNED_EPOCH_US + offset_us).unwrap()
}

async fn open_scheduled(name: &str, stream: &str) -> (std::path::PathBuf, SemanticChunkDvrStore) {
    let dir = temp_root(name);
    let _ = std::fs::remove_dir_all(&dir);
    let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    store.set_stream_schedules(vec![interval_schedule(stream)]);
    (dir, store)
}

async fn append_frames(
    store: &SemanticChunkDvrStore,
    source: &VideoSourceId,
    offsets: impl IntoIterator<Item = i64>,
) {
    for (index, offset) in offsets.into_iter().enumerate() {
        store
            .append_mjpeg_frame(
                source,
                capture_at(offset),
                u64::try_from(FPS_PERIOD_US).unwrap(),
                &jpeg_at(index),
            )
            .await
            .unwrap();
    }
}

fn bucket_winners<'a>(
    slots: &'a [a2a_lab_dev_kit::StageEligibilityRecord],
    stage: &str,
) -> Vec<(i64, &'a str)> {
    slots
        .iter()
        .filter(|slot| slot.stage() == stage)
        .map(|slot| (slot.bucket_start_us(), slot.winner_frame_id()))
        .collect()
}

#[test]
fn epoch_alignment_matches_5s_buckets() {
    assert_eq!(
        interval_bucket_start(ALIGNED_EPOCH_US, INTERVAL_US),
        ALIGNED_EPOCH_US
    );
    assert_eq!(
        interval_bucket_start(ALIGNED_EPOCH_US + 4_999_999, INTERVAL_US),
        ALIGNED_EPOCH_US
    );
    assert_eq!(
        interval_bucket_start(ALIGNED_EPOCH_US + 5_000_000, INTERVAL_US),
        ALIGNED_EPOCH_US + 5_000_000
    );
}

#[tokio::test]
async fn interval_schedule_one_slot_per_bucket_not_per_frame() {
    let (dir, store) = open_scheduled("interval-25", "cam-a").await;
    let source = VideoSourceId::new("cam-a").unwrap();
    let offsets = (0..50).map(|index| i64::from(index) * FPS_PERIOD_US);
    append_frames(&store, &source, offsets).await;
    let slots = store.list_stage_eligibility().await.unwrap();
    assert_eq!(slots.len(), 4);
    assert_eq!(
        slots
            .iter()
            .filter(|slot| slot.stage() == SemanticIndexingStage::Description.id())
            .count(),
        2
    );
    assert_eq!(
        slots
            .iter()
            .filter(|slot| slot.stage() == SemanticIndexingStage::Embeddings.id())
            .count(),
        2
    );
    assert_eq!(store.count_indexing_jobs().await.unwrap(), 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn every_saved_frame_remains_per_frame() {
    let dir = temp_root("every-frame");
    let _ = std::fs::remove_dir_all(&dir);
    let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    store.set_stream_schedules(vec![SemanticStreamSchedule::every_saved_frame("cam-a")]);
    let source = VideoSourceId::new("cam-a").unwrap();
    append_frames(
        &store,
        &source,
        (0..25).map(|index| i64::from(index) * FPS_PERIOD_US),
    )
    .await;
    assert_eq!(store.count_stage_eligibility().await.unwrap(), 50);
    assert_eq!(store.count_indexing_jobs().await.unwrap(), 50);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn out_of_order_frame_updates_unprocessed_winner() {
    let (dir, store) = open_scheduled("ooo", "cam-a").await;
    let source = VideoSourceId::new("cam-a").unwrap();
    append_frames(&store, &source, [400_000, 200_000, 0]).await;
    let slots = store.list_stage_eligibility().await.unwrap();
    assert_eq!(slots.len(), 2);
    let track = MemoryVideos::mjpeg_track_id(&source).unwrap();
    let (first, _) = store
        .register_manifest_at_dvr_key(&store_ns(), &source, &track, capture_at(0))
        .await
        .unwrap();
    for slot in &slots {
        assert_eq!(slot.winner_frame_id(), first.frame_id().as_str());
        assert_eq!(slot.winner_start_us(), ALIGNED_EPOCH_US);
        assert!(
            store
                .select_semantic_stage_frame(
                    source.as_str(),
                    track.as_str(),
                    slot.stage(),
                    first.frame_id().as_str(),
                    ALIGNED_EPOCH_US,
                    SemanticSchedule::Interval {
                        micros: interval_micros(),
                    },
                )
                .await
                .unwrap()
        );
    }
    assert_eq!(store.count_indexing_jobs().await.unwrap(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn processed_slot_ignores_later_better_frame() {
    let (dir, store) = open_scheduled("processed", "cam-a").await;
    let source = VideoSourceId::new("cam-a").unwrap();
    append_frames(&store, &source, [200_000]).await;
    let claimed = store
        .claim_indexing_jobs("worker-a", 8, ALIGNED_EPOCH_US + 10, ALIGNED_EPOCH_US)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 2);
    let before = store.list_stage_eligibility().await.unwrap();
    append_frames(&store, &source, [0]).await;
    let after = store.list_stage_eligibility().await.unwrap();
    assert_eq!(
        before
            .iter()
            .map(|slot| slot.winner_frame_id().to_owned())
            .collect::<Vec<_>>(),
        after
            .iter()
            .map(|slot| slot.winner_frame_id().to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(store.count_indexing_jobs().await.unwrap(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn newer_bucket_does_not_skip_older_bucket() {
    let (dir, store) = open_scheduled("no-skip", "cam-a").await;
    let source = VideoSourceId::new("cam-a").unwrap();
    append_frames(&store, &source, [5_000_000, 0]).await;
    let slots = store.list_stage_eligibility().await.unwrap();
    let desc = bucket_winners(&slots, SemanticIndexingStage::Description.id());
    assert_eq!(desc.len(), 2);
    assert_eq!(desc[0].0, ALIGNED_EPOCH_US);
    assert_eq!(desc[1].0, ALIGNED_EPOCH_US + 5_000_000);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn distinct_sources_tracks_and_stages() {
    let dir = temp_root("identities");
    let _ = std::fs::remove_dir_all(&dir);
    let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    store.set_stream_schedules(vec![
        interval_schedule("cam-a"),
        SemanticStreamSchedule::with_stages(
            "cam-b",
            SemanticSchedule::Interval {
                micros: interval_micros(),
            },
            SemanticSchedule::EverySavedFrame,
        ),
    ]);
    let cam_a = VideoSourceId::new("cam-a").unwrap();
    let cam_b = VideoSourceId::new("cam-b").unwrap();
    append_frames(&store, &cam_a, [0, 200_000]).await;
    append_frames(&store, &cam_b, [0, 200_000]).await;
    let slots = store.list_stage_eligibility().await.unwrap();
    let cam_a_desc = slots
        .iter()
        .filter(|slot| {
            slot.source_id() == "cam-a" && slot.stage() == SemanticIndexingStage::Description.id()
        })
        .count();
    let cam_b_embed = slots
        .iter()
        .filter(|slot| {
            slot.source_id() == "cam-b" && slot.stage() == SemanticIndexingStage::Embeddings.id()
        })
        .count();
    assert_eq!(cam_a_desc, 1);
    assert_eq!(cam_b_embed, 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn live_and_history_share_bucket_winners() {
    let source = VideoSourceId::new("cam-a").unwrap();
    let offsets = [400_000, 0, 200_000, 5_200_000, 5_000_000];
    let live_dir = temp_root("parity-live");
    let _ = std::fs::remove_dir_all(&live_dir);
    let live = SemanticChunkDvrStore::open(&live_dir, 86_400, store_ns())
        .await
        .unwrap();
    live.set_stream_schedules(vec![interval_schedule("cam-a")]);
    append_frames(&live, &source, offsets).await;
    let live_slots = live.list_stage_eligibility().await.unwrap();

    let hist_dir = temp_root("parity-hist");
    let _ = std::fs::remove_dir_all(&hist_dir);
    let chunk = SqliteChunkDvrStore::open(&hist_dir, 86_400).await.unwrap();
    for (index, offset) in offsets.into_iter().enumerate() {
        chunk
            .append_mjpeg_frame(
                &source,
                capture_at(offset),
                u64::try_from(FPS_PERIOD_US).unwrap(),
                &jpeg_at(index),
            )
            .await
            .unwrap();
    }
    let slot = std::sync::Arc::new(std::sync::OnceLock::new());
    let history = SemanticChunkDvrStore::adopt_opened(chunk, slot, store_ns())
        .await
        .unwrap();
    history.set_stream_schedules(vec![interval_schedule("cam-a")]);
    history
        .reconcile_retained_indexing_until_idle(&store_ns())
        .await
        .unwrap();
    let history_slots = history.list_stage_eligibility().await.unwrap();
    assert_eq!(live_slots.len(), history_slots.len());
    for (live_slot, hist_slot) in live_slots.iter().zip(history_slots.iter()) {
        assert_eq!(live_slot.stage(), hist_slot.stage());
        assert_eq!(live_slot.bucket_start_us(), hist_slot.bucket_start_us());
        assert_eq!(live_slot.winner_start_us(), hist_slot.winner_start_us());
    }
    let _ = std::fs::remove_dir_all(&live_dir);
    let _ = std::fs::remove_dir_all(&hist_dir);
}

#[tokio::test]
async fn eligibility_and_cursors_survive_reopen() {
    let dir = temp_root("reopen");
    let _ = std::fs::remove_dir_all(&dir);
    let source = VideoSourceId::new("cam-a").unwrap();
    {
        let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
            .await
            .unwrap();
        store.set_stream_schedules(vec![interval_schedule("cam-a")]);
        append_frames(&store, &source, [5_000_000, 0, 200_000]).await;
        store.reconcile_indexing_jobs(&store_ns(), 8).await.unwrap();
        assert_eq!(store.count_stage_eligibility().await.unwrap(), 4);
    }
    let reopened = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    assert_eq!(reopened.count_stage_eligibility().await.unwrap(), 4);
    assert_eq!(reopened.count_indexing_jobs().await.unwrap(), 4);
    let cursors = reopened.list_reconcile_cursors().await.unwrap();
    assert!(!cursors.is_empty());
    reopened.set_stream_schedules(vec![interval_schedule("cam-a")]);
    append_frames(&reopened, &source, [5_400_000]).await;
    assert_eq!(reopened.count_stage_eligibility().await.unwrap(), 4);
    let _ = std::fs::remove_dir_all(&dir);
}
