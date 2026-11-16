//! Generic semantic rebuild and search (TASK-151).

use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aLabConfig, ChunkDvrSemanticSearchProvider, DescriptionStageWorker, DvrStoreNamespace,
    EmbeddingMetric, EmbeddingStageWorker, FakeEmbeddingBehavior, FakeTextEmbeddingProvider,
    GetSemanticIndexingCoverageRequest, IndexingJobState, IndexingWorkerFaults, IndexingWorkerLane,
    MemoryVideos, ModelSpaceIdentity, PageRequest, ProvenanceGeneration,
    SearchSemanticFramesRequest, SemanticChunkDvrStore, SemanticFrameMediaAvailability,
    SemanticIndexingConfig, SemanticProvenanceSelector, SemanticSchedule, SemanticSearchLimits,
    SemanticSearchQuery, SemanticSearchScope, SemanticStreamSchedule, SemanticVideoSearchProvider,
    SqliteChunkDvrStore, TextEmbeddingLimits, TextEmbeddingProviderConfig, UtcTimestamp,
    VectorNormalization, VideoSourceId, chunk_relpath, minute_start_us,
};

const FPS_PERIOD_US: i64 = 200_000;
const INTERVAL_US: u64 = 5_000_000;
const ALIGNED_EPOCH_US: i64 = 1_700_000_000_000_000;

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "a2a-lab-devkit-semantic-rebuild-{name}-{}",
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
    A2aLabConfig::parse(BASE)
        .unwrap()
        .semantic_indexing
        .unwrap()
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

fn with_schedules(semantic: SemanticIndexingConfig, stream: &str) -> SemanticIndexingConfig {
    semantic.with_schedules(vec![interval_schedule(stream)])
}

fn capture_at(offset_us: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_microseconds(ALIGNED_EPOCH_US + offset_us).unwrap()
}

async fn append_offsets(
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

async fn run_workers(
    store: &SemanticChunkDvrStore,
    semantic: &SemanticIndexingConfig,
    rounds: u32,
) {
    let desc_live = DescriptionStageWorker::deterministic(
        semantic,
        "desc-live",
        IndexingWorkerFaults::new(),
        IndexingWorkerLane::Live,
    )
    .unwrap();
    let desc_hist = DescriptionStageWorker::deterministic(
        semantic,
        "desc-hist",
        IndexingWorkerFaults::new(),
        IndexingWorkerLane::Backfill,
    )
    .unwrap();
    let embed_live = EmbeddingStageWorker::deterministic(
        semantic,
        "embed-live",
        IndexingWorkerFaults::new(),
        IndexingWorkerLane::Live,
    )
    .unwrap();
    let embed_hist = EmbeddingStageWorker::deterministic(
        semantic,
        "embed-hist",
        IndexingWorkerFaults::new(),
        IndexingWorkerLane::Backfill,
    )
    .unwrap();
    for _ in 0..rounds {
        let _ = desc_live.run_batch(store, semantic).await.unwrap();
        let _ = desc_hist.run_batch(store, semantic).await.unwrap();
        let _ = embed_live.run_batch(store, semantic).await.unwrap();
        let _ = embed_hist.run_batch(store, semantic).await.unwrap();
    }
}

async fn embedding_count(store: &SemanticChunkDvrStore) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM semantic_frame_embeddings")
        .fetch_one(store.semantic_pool())
        .await
        .unwrap()
}

fn search_request(
    source: &str,
    text: &str,
    provenance: SemanticProvenanceSelector,
) -> SearchSemanticFramesRequest {
    SearchSemanticFramesRequest::new(
        SemanticSearchScope::new(VideoSourceId::new(source).unwrap(), None).unwrap(),
        SemanticSearchQuery::Text {
            text: text.to_owned(),
        },
        provenance,
        SemanticSearchLimits::pilot(),
        PageRequest::new(None, 5).unwrap(),
        None,
    )
    .unwrap()
}

#[tokio::test]
async fn startup_rejects_index_query_identity_mismatch() {
    let dir = temp_root("identity");
    let _ = std::fs::remove_dir_all(&dir);
    let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    let semantic = with_schedules(semantic_config(), "cam-a");
    let space = ModelSpaceIdentity::new(
        "other",
        "other-model",
        8,
        VectorNormalization::L2Unit,
        EmbeddingMetric::Cosine,
        ProvenanceGeneration::new(1).unwrap(),
    )
    .unwrap();
    let embedder = FakeTextEmbeddingProvider::new(
        TextEmbeddingProviderConfig::new("other", "other-model", space).unwrap(),
        TextEmbeddingLimits::pilot(),
        FakeEmbeddingBehavior::Ok,
    );
    let Err(error) =
        ChunkDvrSemanticSearchProvider::new(store.clone(), semantic.clone(), embedder).await
    else {
        panic!("expected identity mismatch");
    };
    assert!(error.to_string().contains("semantic_indexing.embeddings"));

    let configured = semantic
        .embeddings
        .as_ref()
        .unwrap()
        .model_space_identity()
        .unwrap();
    store
        .persist_embedding_config_identity(&configured, 1)
        .await
        .unwrap();
    let mismatch = ModelSpaceIdentity::new(
        configured.provider(),
        configured.model(),
        configured.dimensions(),
        configured.normalization(),
        configured.metric(),
        ProvenanceGeneration::new(2).unwrap(),
    )
    .unwrap();
    let persist_err = store
        .persist_embedding_config_identity(&mismatch, 2)
        .await
        .unwrap_err();
    assert!(
        persist_err
            .to_string()
            .contains("semantic_indexing.embeddings")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn fresh_rebuild_from_dvr_returns_readable_ranked_hit() {
    let dir = temp_root("rebuild");
    let _ = std::fs::remove_dir_all(&dir);
    let source = VideoSourceId::new("cam-a").unwrap();
    let chunk = SqliteChunkDvrStore::open(&dir, 86_400).await.unwrap();
    for (index, offset) in [0_i64, 200_000, 5_000_000].into_iter().enumerate() {
        chunk
            .append_mjpeg_frame(
                &source,
                capture_at(offset),
                u64::try_from(FPS_PERIOD_US).unwrap(),
                &jpeg(u8::try_from(index).unwrap()),
            )
            .await
            .unwrap();
    }
    let slot = Arc::new(std::sync::OnceLock::new());
    let store = SemanticChunkDvrStore::adopt_opened(chunk, slot, store_ns())
        .await
        .unwrap();
    store.set_stream_schedules(vec![interval_schedule("cam-a")]);
    store
        .reconcile_retained_indexing_until_idle(&store_ns())
        .await
        .unwrap();
    let semantic = with_schedules(semantic_config(), "cam-a");
    run_workers(&store, &semantic, 8).await;
    assert!(embedding_count(&store).await >= 2);

    let search = ChunkDvrSemanticSearchProvider::deterministic(store.clone(), semantic.clone())
        .await
        .unwrap();
    let provenance = search.active_provenance().unwrap();
    let first = sqlx::query_scalar::<_, String>(
        "SELECT frame_id FROM semantic_frame_embeddings ORDER BY frame_id LIMIT 1",
    )
    .fetch_one(store.semantic_pool())
    .await
    .unwrap();
    let result = search
        .search_semantic_frames(search_request(
            "cam-a",
            &format!("desc:{first}"),
            provenance.clone(),
        ))
        .await
        .unwrap();
    let hit = &result.hits().items()[0];
    assert_eq!(hit.generated_description(), format!("desc:{first}"));
    assert_eq!(hit.frame().frame_id().as_str(), first);
    assert!(hit.retrieval().value().is_finite());
    assert_eq!(hit.model_space(), provenance.model_space());
    assert!(matches!(
        hit.media_availability(),
        SemanticFrameMediaAvailability::Ready
    ));

    let coverage = search
        .get_semantic_indexing_coverage(
            GetSemanticIndexingCoverageRequest::new(
                SemanticSearchScope::new(source, None).unwrap(),
                provenance,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(coverage.counts().ready() >= 2);
    assert_eq!(coverage.counts().skipped(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn restart_during_rebuild_keeps_skips_and_live_progress() {
    let dir = temp_root("restart");
    let _ = std::fs::remove_dir_all(&dir);
    let source = VideoSourceId::new("cam-a").unwrap();
    let semantic = with_schedules(semantic_config(), "cam-a");
    let skipped_job;
    let eligibility_before;
    {
        let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
            .await
            .unwrap();
        store.set_stream_schedules(vec![interval_schedule("cam-a")]);
        append_offsets(&store, &source, [0, 5_000_000, 10_000_000]).await;
        store
            .reconcile_retained_indexing_until_idle(&store_ns())
            .await
            .unwrap();
        run_workers(&store, &semantic, 1).await;
        skipped_job = sqlx::query_scalar::<_, String>(
            "SELECT job_id FROM semantic_indexing_jobs WHERE job_kind = 'embeddings'
             ORDER BY created_at_us ASC, job_id ASC LIMIT 1",
        )
        .fetch_one(store.semantic_pool())
        .await
        .unwrap();
        sqlx::query(
            "UPDATE semantic_indexing_job_runtime SET job_state = 'skipped',
                 lease_owner = NULL, lease_expires_at_us = NULL
             WHERE job_id = ?1",
        )
        .bind(&skipped_job)
        .execute(store.semantic_pool())
        .await
        .unwrap();
        eligibility_before = store.count_stage_eligibility().await.unwrap();
    }

    let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    store.set_stream_schedules(vec![interval_schedule("cam-a")]);
    assert_eq!(
        store.count_stage_eligibility().await.unwrap(),
        eligibility_before
    );
    let skipped = store.get_indexing_job(&skipped_job).await.unwrap();
    assert_eq!(skipped.state(), IndexingJobState::Skipped);
    append_offsets(&store, &source, [40_000_000]).await;
    run_workers(&store, &semantic, 8).await;
    let skipped_again = store.get_indexing_job(&skipped_job).await.unwrap();
    assert_eq!(skipped_again.state(), IndexingJobState::Skipped);
    let unique: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM (SELECT DISTINCT frame_id FROM semantic_frame_embeddings)",
    )
    .fetch_one(store.semantic_pool())
    .await
    .unwrap();
    assert_eq!(unique, embedding_count(&store).await);
    assert!(unique >= 1);

    let search = ChunkDvrSemanticSearchProvider::deterministic(store.clone(), semantic)
        .await
        .unwrap();
    let provenance = search.active_provenance().unwrap();
    let coverage = search
        .get_semantic_indexing_coverage(
            GetSemanticIndexingCoverageRequest::new(
                SemanticSearchScope::new(source, None).unwrap(),
                provenance.clone(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(coverage.counts().skipped() >= 1);
    assert!(coverage.counts().ready() >= 1);

    let other = SemanticProvenanceSelector::new(
        ModelSpaceIdentity::new(
            provenance.model_space().provider(),
            provenance.model_space().model(),
            provenance.model_space().dimensions(),
            provenance.model_space().normalization(),
            provenance.model_space().metric(),
            ProvenanceGeneration::new(9).unwrap(),
        )
        .unwrap(),
        provenance.description_generation(),
    )
    .unwrap();
    let mismatch = search
        .search_semantic_frames(search_request("cam-a", "desc", other))
        .await
        .unwrap_err();
    assert!(mismatch.to_string().contains("provenance"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn ranking_is_deterministic_for_repeated_queries() {
    let dir = temp_root("rank");
    let _ = std::fs::remove_dir_all(&dir);
    let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    store.set_stream_schedules(vec![interval_schedule("cam-a")]);
    let source = VideoSourceId::new("cam-a").unwrap();
    append_offsets(&store, &source, [0, 5_000_000]).await;
    let semantic = with_schedules(semantic_config(), "cam-a");
    run_workers(&store, &semantic, 8).await;
    let search = ChunkDvrSemanticSearchProvider::deterministic(store.clone(), semantic)
        .await
        .unwrap();
    let provenance = search.active_provenance().unwrap();
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT frame_id FROM semantic_frame_embeddings ORDER BY frame_id")
            .fetch_all(store.semantic_pool())
            .await
            .unwrap();
    assert!(ids.len() >= 2);
    let query = format!("desc:{}", ids[0]);
    let first = search
        .search_semantic_frames(search_request("cam-a", &query, provenance.clone()))
        .await
        .unwrap();
    let second = search
        .search_semantic_frames(search_request("cam-a", &query, provenance))
        .await
        .unwrap();
    assert_eq!(
        first.hits().items()[0].frame(),
        second.hits().items()[0].frame()
    );
    assert_eq!(first.hits().items()[0].frame().frame_id().as_str(), ids[0]);
    let _ = std::fs::remove_dir_all(&dir);
}

struct IndexedSearch {
    dir: std::path::PathBuf,
    store: SemanticChunkDvrStore,
    search: ChunkDvrSemanticSearchProvider<FakeTextEmbeddingProvider>,
    provenance: SemanticProvenanceSelector,
    frame_id: String,
    capture: UtcTimestamp,
    source: VideoSourceId,
}

async fn index_one_searchable(name: &str) -> IndexedSearch {
    let dir = temp_root(name);
    let _ = std::fs::remove_dir_all(&dir);
    let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    store.set_stream_schedules(vec![interval_schedule("cam-a")]);
    let source = VideoSourceId::new("cam-a").unwrap();
    append_offsets(&store, &source, [0]).await;
    store
        .reconcile_retained_indexing_until_idle(&store_ns())
        .await
        .unwrap();
    let semantic = with_schedules(semantic_config(), "cam-a");
    run_workers(&store, &semantic, 8).await;
    let search = ChunkDvrSemanticSearchProvider::deterministic(store.clone(), semantic)
        .await
        .unwrap();
    let provenance = search.active_provenance().unwrap();
    let frame_id: String = sqlx::query_scalar(
        "SELECT frame_id FROM semantic_frame_embeddings ORDER BY frame_id LIMIT 1",
    )
    .fetch_one(store.semantic_pool())
    .await
    .unwrap();
    let capture_us: i64 =
        sqlx::query_scalar("SELECT start_us FROM semantic_frame_revisions WHERE frame_id = ?1")
            .bind(&frame_id)
            .fetch_one(store.semantic_pool())
            .await
            .unwrap();
    IndexedSearch {
        dir,
        store,
        search,
        provenance,
        frame_id,
        capture: UtcTimestamp::from_unix_microseconds(capture_us).unwrap(),
        source,
    }
}

async fn search_hits(fixture: &IndexedSearch) -> usize {
    fixture
        .search
        .search_semantic_frames(search_request(
            "cam-a",
            &format!("desc:{}", fixture.frame_id),
            fixture.provenance.clone(),
        ))
        .await
        .unwrap()
        .hits()
        .items()
        .len()
}

#[tokio::test]
async fn retained_digest_valid_payload_is_ready_available_hit() {
    let fixture = index_one_searchable("avail").await;
    let result = fixture
        .search
        .search_semantic_frames(search_request(
            "cam-a",
            &format!("desc:{}", fixture.frame_id),
            fixture.provenance.clone(),
        ))
        .await
        .unwrap();
    let hit = &result.hits().items()[0];
    assert_eq!(hit.frame().frame_id().as_str(), fixture.frame_id);
    assert!(matches!(
        hit.media_availability(),
        SemanticFrameMediaAvailability::Ready
    ));
    let _ = std::fs::remove_dir_all(&fixture.dir);
}

#[tokio::test]
async fn pending_not_ready_payload_is_omitted() {
    let fixture = index_one_searchable("pending").await;
    sqlx::query("UPDATE dvr_frames SET payload_ready = 0")
        .execute(fixture.store.media_pool())
        .await
        .unwrap();
    assert_eq!(search_hits(&fixture).await, 0);
    let _ = std::fs::remove_dir_all(&fixture.dir);
}

#[tokio::test]
async fn missing_payload_is_omitted() {
    let fixture = index_one_searchable("missing").await;
    sqlx::query("DELETE FROM dvr_frames")
        .execute(fixture.store.media_pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM dvr_segments")
        .execute(fixture.store.media_pool())
        .await
        .unwrap();
    assert_eq!(search_hits(&fixture).await, 0);
    let _ = std::fs::remove_dir_all(&fixture.dir);
}

#[tokio::test]
async fn corrupt_digest_mismatch_payload_is_omitted() {
    let fixture = index_one_searchable("corrupt").await;
    let track = MemoryVideos::mjpeg_track_id(&fixture.source).unwrap();
    let relpath = chunk_relpath(
        fixture.source.as_str(),
        track.as_str(),
        minute_start_us(fixture.capture),
    );
    let path = fixture.store.root().join(&relpath);
    let path = if path.exists() {
        path
    } else {
        fixture.store.root().join(format!("{relpath}.tmp"))
    };
    let mut bytes = std::fs::read(&path).unwrap();
    if let Some(byte) = bytes.first_mut() {
        *byte ^= 0xFF;
    }
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(search_hits(&fixture).await, 0);
    let _ = std::fs::remove_dir_all(&fixture.dir);
}

#[tokio::test]
async fn expired_or_non_current_retained_state_is_omitted() {
    let expired = index_one_searchable("expired").await;
    sqlx::query("UPDATE semantic_frame_revisions SET retained_state = 'expired'")
        .execute(expired.store.semantic_pool())
        .await
        .unwrap();
    assert_eq!(search_hits(&expired).await, 0);
    let _ = std::fs::remove_dir_all(&expired.dir);

    let superseded = index_one_searchable("superseded").await;
    sqlx::query(
        "UPDATE semantic_frame_revisions SET search_current = 0, retained_state = 'superseded'",
    )
    .execute(superseded.store.semantic_pool())
    .await
    .unwrap();
    assert_eq!(search_hits(&superseded).await, 0);
    let _ = std::fs::remove_dir_all(&superseded.dir);
}

#[tokio::test]
async fn stale_pending_checkpoints_with_retained_media_remain_searchable() {
    let fixture = index_one_searchable("stale-pending").await;
    sqlx::query(
        "UPDATE semantic_frame_revisions
         SET description_checkpoint = 'pending', embedding_checkpoint = 'pending'",
    )
    .execute(fixture.store.semantic_pool())
    .await
    .unwrap();
    let result = fixture
        .search
        .search_semantic_frames(search_request(
            "cam-a",
            &format!("desc:{}", fixture.frame_id),
            fixture.provenance.clone(),
        ))
        .await
        .unwrap();
    let hit = &result.hits().items()[0];
    assert_eq!(hit.frame().frame_id().as_str(), fixture.frame_id);
    assert!(matches!(
        hit.media_availability(),
        SemanticFrameMediaAvailability::Ready
    ));
    let _ = std::fs::remove_dir_all(&fixture.dir);
}

#[tokio::test]
async fn reopen_and_reconcile_keeps_playable_media_and_excludes_unplayable() {
    let dir = temp_root("reopen-avail");
    let _ = std::fs::remove_dir_all(&dir);
    let source = VideoSourceId::new("cam-a").unwrap();
    let semantic = with_schedules(semantic_config(), "cam-a");
    let frame_id;
    {
        let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
            .await
            .unwrap();
        store.set_stream_schedules(vec![interval_schedule("cam-a")]);
        let now_us = UtcTimestamp::now().as_micros();
        let aligned = now_us - now_us.rem_euclid(i64::try_from(INTERVAL_US).unwrap());
        store
            .append_mjpeg_frame(
                &source,
                UtcTimestamp::from_unix_microseconds(aligned).unwrap(),
                u64::try_from(FPS_PERIOD_US).unwrap(),
                &jpeg(1),
            )
            .await
            .unwrap();
        store
            .reconcile_retained_indexing_until_idle(&store_ns())
            .await
            .unwrap();
        run_workers(&store, &semantic, 8).await;
        frame_id = sqlx::query_scalar::<_, String>(
            "SELECT frame_id FROM semantic_frame_embeddings ORDER BY frame_id LIMIT 1",
        )
        .fetch_one(store.semantic_pool())
        .await
        .unwrap();
        sqlx::query(
            "UPDATE semantic_frame_revisions
             SET description_checkpoint = 'pending', embedding_checkpoint = 'pending'",
        )
        .execute(store.semantic_pool())
        .await
        .unwrap();
    }
    let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    store.set_stream_schedules(vec![interval_schedule("cam-a")]);
    store
        .reconcile_retained_indexing_until_idle(&store_ns())
        .await
        .unwrap();
    let search = ChunkDvrSemanticSearchProvider::deterministic(store.clone(), semantic)
        .await
        .unwrap();
    let provenance = search.active_provenance().unwrap();
    let result = search
        .search_semantic_frames(search_request(
            "cam-a",
            &format!("desc:{frame_id}"),
            provenance.clone(),
        ))
        .await
        .unwrap();
    assert!(matches!(
        result.hits().items()[0].media_availability(),
        SemanticFrameMediaAvailability::Ready
    ));
    sqlx::query("UPDATE dvr_frames SET payload_ready = 0")
        .execute(store.media_pool())
        .await
        .unwrap();
    let empty = search
        .search_semantic_frames(search_request(
            "cam-a",
            &format!("desc:{frame_id}"),
            provenance,
        ))
        .await
        .unwrap();
    assert!(empty.hits().items().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
