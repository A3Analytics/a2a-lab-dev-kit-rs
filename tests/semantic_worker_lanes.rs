//! Durable live/backfill semantic workers (TASK-150).

use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aLabConfig, DescriptionStageWorker, DvrStoreNamespace, EmbeddingStageWorker,
    FakeDescriptionBehavior, FakeFrameDescriptionProvider, FrameDescriptionProviderConfig,
    IndexingJobState, IndexingWorkerFaults, IndexingWorkerLane, MemoryVideos,
    SemanticChunkDvrStore, SemanticIndexingConfig, SemanticIndexingStage, SemanticSchedule,
    SemanticStreamSchedule, Sha256Digest, UtcTimestamp, VideoSourceId, frame_description_limits,
    stage_eligibility_job_id,
};

const FPS_PERIOD_US: i64 = 200_000;
const INTERVAL_US: u64 = 5_000_000;
const ALIGNED_EPOCH_US: i64 = 1_700_000_000_000_000;

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "a2a-lab-devkit-semantic-workers-{name}-{}",
        std::process::id()
    ))
}

fn store_ns() -> DvrStoreNamespace {
    DvrStoreNamespace::new("test.video-dvr").unwrap()
}

fn jpeg(tag: u8) -> Vec<u8> {
    vec![0xFF, 0xD8, tag, 0xD9]
}

fn semantic_config() -> SemanticIndexingConfig {
    const BASE: &str = include_str!("fixtures/a2a-lab-v2-semantic-valid.json");
    let mut config = A2aLabConfig::parse(BASE).unwrap();
    let semantic = config.semantic_indexing.as_mut().unwrap();
    semantic.limits.worker.max_batch_items = 4;
    semantic.limits.worker.max_concurrency = 2;
    semantic.limits.worker.max_queue_backlog = 64;
    semantic.limits.worker.max_retry_attempts = 2;
    semantic.limits.worker.retry_backoff =
        a2a_lab_dev_kit::ConfigDuration::parse("test", "1s").unwrap();
    semantic.clone()
}

fn interval_schedule(stream: &str) -> SemanticStreamSchedule {
    SemanticStreamSchedule::with_stages(
        stream,
        SemanticSchedule::Interval {
            micros: INTERVAL_US,
        },
        SemanticSchedule::Interval {
            micros: INTERVAL_US,
        },
    )
}

fn capture_at(offset_us: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_microseconds(ALIGNED_EPOCH_US + offset_us).unwrap()
}

async fn open_store(name: &str, stream: &str) -> (std::path::PathBuf, SemanticChunkDvrStore) {
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
                &jpeg(u8::try_from(index).unwrap_or(u8::MAX)),
            )
            .await
            .unwrap();
    }
}

fn with_schedules(semantic: SemanticIndexingConfig, stream: &str) -> SemanticIndexingConfig {
    semantic.with_schedules(vec![interval_schedule(stream)])
}

async fn job_attempts(store: &SemanticChunkDvrStore, job_id: &str) -> (u32, u32, IndexingJobState) {
    let job = store.get_indexing_job(job_id).await.unwrap();
    (
        job.description_attempts(),
        job.embedding_attempts(),
        job.state(),
    )
}

async fn count_state(store: &SemanticChunkDvrStore, state: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM semantic_indexing_job_runtime WHERE job_state = ?1")
        .bind(state)
        .fetch_one(store.semantic_pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn injected_providers_complete_description_and_embedding_slots() {
    let (dir, store) = open_store("complete", "cam-a").await;
    let source = VideoSourceId::new("cam-a").unwrap();
    append_frames(&store, &source, [0]).await;
    let semantic = with_schedules(semantic_config(), "cam-a");
    let desc = DescriptionStageWorker::deterministic(
        &semantic,
        "desc-live",
        IndexingWorkerFaults::new(),
        IndexingWorkerLane::Live,
    )
    .unwrap();
    let embed = EmbeddingStageWorker::deterministic(
        &semantic,
        "embed-live",
        IndexingWorkerFaults::new(),
        IndexingWorkerLane::Live,
    )
    .unwrap();
    let described = desc.run_batch(&store, &semantic).await.unwrap();
    assert_eq!(described.descriptions_committed(), 1);
    let embedded = embed.run_batch(&store, &semantic).await.unwrap();
    assert_eq!(embedded.embeddings_committed(), 1);
    assert_eq!(count_state(&store, "ready").await, 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn live_progress_continues_under_backfill_backlog() {
    let (dir, store) = open_store("lanes", "cam-a").await;
    let source = VideoSourceId::new("cam-a").unwrap();
    let offsets: Vec<i64> = (0..8).map(|index| i64::from(index) * 5_000_000).collect();
    append_frames(&store, &source, offsets).await;
    sqlx::query("UPDATE semantic_indexing_jobs SET enqueue_priority = 1")
        .execute(store.semantic_pool())
        .await
        .unwrap();
    append_frames(&store, &source, [40_000_000]).await;
    let semantic = with_schedules(semantic_config(), "cam-a");
    let live = DescriptionStageWorker::deterministic(
        &semantic,
        "live",
        IndexingWorkerFaults::new(),
        IndexingWorkerLane::Live,
    )
    .unwrap();
    let backfill = DescriptionStageWorker::deterministic(
        &semantic,
        "backfill",
        IndexingWorkerFaults::new(),
        IndexingWorkerLane::Backfill,
    )
    .unwrap();
    let (live_report, backfill_report) = tokio::join!(
        live.run_batch(&store, &semantic),
        backfill.run_batch(&store, &semantic),
    );
    let live_report = live_report.unwrap();
    let backfill_report = backfill_report.unwrap();
    assert!(live_report.descriptions_committed() >= 1);
    assert!(backfill_report.descriptions_committed() >= 1);
    assert_eq!(live_report.lane(), IndexingWorkerLane::Live);
    assert_eq!(backfill_report.lane(), IndexingWorkerLane::Backfill);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn historical_spin_described_rows_skip_and_stop_claiming() {
    let (dir, store) = open_store("spin", "cam-a").await;
    let source = VideoSourceId::new("cam-a").unwrap();
    let offsets: Vec<i64> = (0..25)
        .map(|index| i64::from(index) * FPS_PERIOD_US)
        .collect();
    append_frames(&store, &source, offsets).await;
    let track = MemoryVideos::mjpeg_track_id(&source).unwrap();
    let winner = stage_eligibility_job_id(
        source.as_str(),
        track.as_str(),
        SemanticIndexingStage::Description.id(),
        ALIGNED_EPOCH_US,
    );
    let extra_frames: Vec<String> = sqlx::query_scalar(
        "SELECT frame_id FROM semantic_frame_revisions
         WHERE frame_id NOT IN (SELECT winner_frame_id FROM semantic_stage_eligibility)
         ORDER BY start_us LIMIT 3",
    )
    .fetch_all(store.semantic_pool())
    .await
    .unwrap();
    assert_eq!(extra_frames.len(), 3);
    for (index, frame_id) in extra_frames.iter().enumerate() {
        let job_id = format!("spin-extra-{index}");
        sqlx::query(
            "INSERT INTO semantic_indexing_jobs (job_id, frame_id, job_kind, created_at_us, enqueue_priority)
             VALUES (?1, ?2, 'description', ?3, 0)",
        )
        .bind(&job_id)
        .bind(frame_id)
        .bind(ALIGNED_EPOCH_US)
        .execute(store.semantic_pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO semantic_indexing_job_runtime
             (job_id, job_state, description_generation, description_attempts)
             VALUES (?1, 'described', 1, 4)",
        )
        .bind(&job_id)
        .execute(store.semantic_pool())
        .await
        .unwrap();
    }
    let semantic = with_schedules(semantic_config(), "cam-a");
    let worker = DescriptionStageWorker::deterministic(
        &semantic,
        "spin",
        IndexingWorkerFaults::new(),
        IndexingWorkerLane::Live,
    )
    .unwrap();
    worker.run_batch(&store, &semantic).await.unwrap();
    let after_first: Vec<(u32, IndexingJobState)> = {
        let mut rows = Vec::new();
        for index in 0..3 {
            let (attempts, _, state) = job_attempts(&store, &format!("spin-extra-{index}")).await;
            rows.push((attempts, state));
        }
        rows
    };
    for (attempts, state) in &after_first {
        assert_eq!(*attempts, 4);
        assert_eq!(*state, IndexingJobState::Skipped);
    }
    worker.run_batch(&store, &semantic).await.unwrap();
    worker.run_batch(&store, &semantic).await.unwrap();
    for index in 0..3 {
        let (attempts, _, state) = job_attempts(&store, &format!("spin-extra-{index}")).await;
        assert_eq!(attempts, 4);
        assert_eq!(state, IndexingJobState::Skipped);
        let claimed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM semantic_indexing_job_runtime
             WHERE job_id = ?1 AND lease_owner IS NOT NULL",
        )
        .bind(format!("spin-extra-{index}"))
        .fetch_one(store.semantic_pool())
        .await
        .unwrap();
        assert_eq!(claimed, 0);
    }
    let winner_job = store.get_indexing_job(&winner).await.unwrap();
    assert_ne!(winner_job.state(), IndexingJobState::Skipped);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn retryable_timeout_then_terminal_empty_description() {
    let (dir, store) = open_store("retry", "cam-a").await;
    let source = VideoSourceId::new("cam-a").unwrap();
    append_frames(&store, &source, [0]).await;
    let semantic = with_schedules(semantic_config(), "cam-a");
    let limits = frame_description_limits(&semantic.limits);
    let config = FrameDescriptionProviderConfig::new(
        semantic.description.backend(),
        Some(semantic.description.model()),
        Sha256Digest::hash_bytes(b"devkit-description-prompt"),
        Sha256Digest::hash_bytes(b"devkit-description-preprocess"),
    )
    .unwrap();
    let timeout = DescriptionStageWorker::new(
        FakeFrameDescriptionProvider::new(config.clone(), limits, FakeDescriptionBehavior::Timeout),
        semantic.limits.worker.clone(),
        "timeout",
        IndexingWorkerFaults::new(),
    )
    .with_lane(IndexingWorkerLane::Live);
    timeout.run_batch(&store, &semantic).await.unwrap();
    let job_id: String = sqlx::query_scalar(
        "SELECT job_id FROM semantic_indexing_jobs WHERE job_kind = 'description'",
    )
    .fetch_one(store.semantic_pool())
    .await
    .unwrap();
    let first = store.get_indexing_job(&job_id).await.unwrap();
    assert_eq!(first.state(), IndexingJobState::RetryableFailed);
    sqlx::query("UPDATE semantic_indexing_job_runtime SET next_retry_at_us = 0 WHERE job_id = ?1")
        .bind(&job_id)
        .execute(store.semantic_pool())
        .await
        .unwrap();
    timeout.run_batch(&store, &semantic).await.unwrap();
    sqlx::query("UPDATE semantic_indexing_job_runtime SET next_retry_at_us = 0 WHERE job_id = ?1")
        .bind(&job_id)
        .execute(store.semantic_pool())
        .await
        .unwrap();
    timeout.run_batch(&store, &semantic).await.unwrap();
    let terminal = store.get_indexing_job(&job_id).await.unwrap();
    assert_eq!(terminal.state(), IndexingJobState::TerminalFailed);

    let (dir2, store2) = open_store("empty", "cam-b").await;
    let source_b = VideoSourceId::new("cam-b").unwrap();
    store2.set_stream_schedules(vec![interval_schedule("cam-b")]);
    append_frames(&store2, &source_b, [0]).await;
    let semantic_b = with_schedules(semantic_config(), "cam-b");
    let empty = DescriptionStageWorker::new(
        FakeFrameDescriptionProvider::new(config, limits, FakeDescriptionBehavior::Empty),
        semantic_b.limits.worker.clone(),
        "empty",
        IndexingWorkerFaults::new(),
    )
    .with_lane(IndexingWorkerLane::Live);
    empty.run_batch(&store2, &semantic_b).await.unwrap();
    let empty_state: String = sqlx::query_scalar(
        "SELECT job_state FROM semantic_indexing_job_runtime r
         JOIN semantic_indexing_jobs j ON j.job_id = r.job_id
         WHERE j.job_kind = 'description'",
    )
    .fetch_one(store2.semantic_pool())
    .await
    .unwrap();
    assert_eq!(empty_state, "terminal_failed");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

#[tokio::test]
async fn expired_lease_recovers_after_restart_and_stale_cas_is_rejected() {
    let dir = temp_root("restart");
    let _ = std::fs::remove_dir_all(&dir);
    let source = VideoSourceId::new("cam-a").unwrap();
    let job_id;
    let capture = UtcTimestamp::now();
    {
        let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
            .await
            .unwrap();
        store.set_stream_schedules(vec![interval_schedule("cam-a")]);
        store
            .append_mjpeg_frame(&source, capture, 1_000_000, &jpeg(1))
            .await
            .unwrap();
        let now_us = capture.as_micros();
        let claimed = store
            .claim_indexing_jobs_for_lane(
                "stale",
                1,
                now_us + 60_000_000,
                now_us,
                IndexingWorkerLane::Live,
                SemanticIndexingStage::Description,
            )
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        job_id = claimed[0].job_id().to_owned();
        sqlx::query(
            "UPDATE semantic_indexing_job_runtime SET lease_expires_at_us = ?1 WHERE job_id = ?2",
        )
        .bind(now_us)
        .bind(&job_id)
        .execute(store.semantic_pool())
        .await
        .unwrap();
    }
    let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    store.set_stream_schedules(vec![interval_schedule("cam-a")]);
    let reclaimed = store
        .reclaim_expired_indexing_leases(capture.as_micros() + 1)
        .await
        .unwrap();
    assert!(reclaimed >= 1);
    let leased = store.get_indexing_job(&job_id).await.unwrap();
    assert!(leased.lease_owner().is_none());
    let semantic = with_schedules(semantic_config(), "cam-a");
    let worker = DescriptionStageWorker::deterministic(
        &semantic,
        "restart",
        IndexingWorkerFaults::new(),
        IndexingWorkerLane::Live,
    )
    .unwrap();
    worker.run_batch(&store, &semantic).await.unwrap();
    let ready = store.get_indexing_job(&job_id).await.unwrap();
    assert_eq!(ready.state(), IndexingJobState::Ready);
    let stale = store
        .complete_indexing_description_cas(&job_id, "other-worker", 1, 1, capture.as_micros())
        .await
        .unwrap();
    assert!(!stale);
    assert_eq!(
        store.get_indexing_job(&job_id).await.unwrap().state(),
        IndexingJobState::Ready
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn crash_before_commit_does_not_duplicate_provider_state() {
    let (dir, store) = open_store("crash", "cam-a").await;
    let source = VideoSourceId::new("cam-a").unwrap();
    append_frames(&store, &source, [0]).await;
    let semantic = with_schedules(semantic_config(), "cam-a");
    let faults = IndexingWorkerFaults::new();
    faults.set_crash_before_description_commit(true);
    let crashing = DescriptionStageWorker::deterministic(
        &semantic,
        "crash",
        Arc::clone(&faults),
        IndexingWorkerLane::Live,
    )
    .unwrap();
    crashing.run_batch(&store, &semantic).await.unwrap();
    let descriptions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM semantic_frame_descriptions")
        .fetch_one(store.semantic_pool())
        .await
        .unwrap();
    assert_eq!(descriptions, 0);
    faults.set_crash_before_description_commit(false);
    sqlx::query("UPDATE semantic_indexing_job_runtime SET next_retry_at_us = 0")
        .execute(store.semantic_pool())
        .await
        .unwrap();
    crashing.run_batch(&store, &semantic).await.unwrap();
    let descriptions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM semantic_frame_descriptions")
        .fetch_one(store.semantic_pool())
        .await
        .unwrap();
    assert_eq!(descriptions, 1);
    let _ = std::fs::remove_dir_all(&dir);
}
