//! Hybrid lexical and vector semantic search (TASK-156 / TASK-159).

use a2a_lab_dev_kit::{
    A2aLabConfig, ChunkDvrSemanticSearchProvider, DescriptionStageWorker, DvrStoreNamespace,
    EmbeddingMetric, EmbeddingStageWorker, IndexingWorkerFaults, IndexingWorkerLane, PageRequest,
    SearchSemanticFramesRequest, SemanticChunkDvrStore, SemanticFrameMediaAvailability,
    SemanticIndexingConfig, SemanticProvenanceSelector, SemanticSchedule, SemanticSearchLimits,
    SemanticSearchQuery, SemanticSearchScope, SemanticStreamSchedule, SemanticVideoSearchProvider,
    Sha256Digest, TextEmbeddingKey, TextEmbeddingProvider, TextEmbeddingRequest, UtcTimestamp,
    VideoSourceId, deterministic_indexing_providers, text_embedding_limits,
};

const FPS_PERIOD_US: i64 = 200_000;
const INTERVAL_US: u64 = 5_000_000;
const ALIGNED_EPOCH_US: i64 = 1_700_000_000_000_000;

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "a2a-lab-devkit-semantic-hybrid-{name}-{}",
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

fn high_floor_semantic() -> SemanticIndexingConfig {
    let mut semantic = semantic_config().with_schedules(vec![interval_schedule("cam-a")]);
    "0.99".clone_into(&mut semantic.limits.search.score_floor);
    semantic
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

struct HybridIndex {
    dir: std::path::PathBuf,
    store: SemanticChunkDvrStore,
    search: ChunkDvrSemanticSearchProvider<a2a_lab_dev_kit::FakeTextEmbeddingProvider>,
    provenance: SemanticProvenanceSelector,
    ids: Vec<String>,
    semantic: SemanticIndexingConfig,
}

async fn index_frames(name: &str, offsets: &[i64]) -> HybridIndex {
    let dir = temp_root(name);
    let _ = std::fs::remove_dir_all(&dir);
    let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    store.set_stream_schedules(vec![interval_schedule("cam-a")]);
    let source = VideoSourceId::new("cam-a").unwrap();
    append_offsets(&store, &source, offsets.iter().copied()).await;
    store
        .reconcile_retained_indexing_until_idle(&store_ns())
        .await
        .unwrap();
    let semantic = high_floor_semantic();
    run_workers(&store, &semantic, 8).await;
    let search = ChunkDvrSemanticSearchProvider::deterministic(store.clone(), semantic.clone())
        .await
        .unwrap();
    let provenance = search.active_provenance().unwrap();
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT e.frame_id FROM semantic_frame_embeddings e
         JOIN semantic_frame_revisions r ON r.frame_id = e.frame_id
         ORDER BY r.start_us, e.frame_id",
    )
    .fetch_all(store.semantic_pool())
    .await
    .unwrap();
    HybridIndex {
        dir,
        store,
        search,
        provenance,
        ids,
        semantic,
    }
}

fn text_search(
    text: &str,
    provenance: SemanticProvenanceSelector,
    top_k: u32,
    page_limit: u32,
    cursor: Option<String>,
) -> SearchSemanticFramesRequest {
    text_search_limited(text, provenance, top_k, 1_000, page_limit, cursor)
}

fn text_search_limited(
    text: &str,
    provenance: SemanticProvenanceSelector,
    top_k: u32,
    candidate_limit: u32,
    page_limit: u32,
    cursor: Option<String>,
) -> SearchSemanticFramesRequest {
    SearchSemanticFramesRequest::new(
        SemanticSearchScope::new(VideoSourceId::new("cam-a").unwrap(), None).unwrap(),
        SemanticSearchQuery::Text {
            text: text.to_owned(),
        },
        provenance,
        SemanticSearchLimits::new(top_k, candidate_limit, 5).unwrap(),
        PageRequest::new(cursor, page_limit).unwrap(),
        None,
    )
    .unwrap()
}

async fn set_description(store: &SemanticChunkDvrStore, frame_id: &str, text: &str) {
    sqlx::query("UPDATE semantic_frame_descriptions SET description_text = ?1 WHERE frame_id = ?2")
        .bind(text)
        .bind(frame_id)
        .execute(store.semantic_pool())
        .await
        .unwrap();
}

async fn query_embedding_blob(semantic: &SemanticIndexingConfig, text: &str) -> Vec<u8> {
    let providers = deterministic_indexing_providers(semantic).unwrap();
    let limits = text_embedding_limits(&semantic.limits);
    let key = TextEmbeddingKey::SearchQuery(Sha256Digest::hash_bytes(text.as_bytes()));
    let item = TextEmbeddingRequest::new(
        limits,
        key,
        text.to_owned(),
        u32::try_from(text.len()).unwrap(),
    )
    .unwrap();
    let responses = providers.embedding.embed_texts(vec![item]).await.unwrap();
    responses[0]
        .vector()
        .values()
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

async fn set_embedding(store: &SemanticChunkDvrStore, frame_id: &str, blob: &[u8]) {
    sqlx::query("UPDATE semantic_frame_embeddings SET embedding_vector = ?1 WHERE frame_id = ?2")
        .bind(blob)
        .bind(frame_id)
        .execute(store.semantic_pool())
        .await
        .unwrap();
}

#[tokio::test]
async fn literal_red_below_floor_is_returned_monkey_is_omitted() {
    let fixture = index_frames("red", &[0]).await;
    let frame_id = &fixture.ids[0];
    set_description(&fixture.store, frame_id, "A red pipette on the deck").await;
    let red = fixture
        .search
        .search_semantic_frames(text_search("red", fixture.provenance.clone(), 10, 10, None))
        .await
        .unwrap();
    assert_eq!(red.hits().items().len(), 1);
    assert_eq!(red.hits().items()[0].frame().frame_id().as_str(), frame_id);
    assert!(red.hits().items()[0].retrieval().value() < 0.99);
    let monkey = fixture
        .search
        .search_semantic_frames(text_search(
            "monkey",
            fixture.provenance.clone(),
            10,
            10,
            None,
        ))
        .await
        .unwrap();
    assert!(monkey.hits().items().is_empty());
    let _ = std::fs::remove_dir_all(&fixture.dir);
}

#[tokio::test]
async fn paraphrase_vector_hit_has_no_required_token_overlap() {
    let fixture = index_frames("para", &[0]).await;
    let frame_id = &fixture.ids[0];
    set_description(
        &fixture.store,
        frame_id,
        "clear plastic labware on the deck",
    )
    .await;
    let query = "transparent container";
    let blob = query_embedding_blob(&fixture.semantic, query).await;
    set_embedding(&fixture.store, frame_id, &blob).await;
    let result = fixture
        .search
        .search_semantic_frames(text_search(query, fixture.provenance.clone(), 10, 10, None))
        .await
        .unwrap();
    assert_eq!(result.hits().items().len(), 1);
    assert_eq!(
        result.hits().items()[0].frame().frame_id().as_str(),
        frame_id
    );
    assert!(
        result.hits().items()[0].retrieval().value() >= 0.99,
        "paraphrase retrieval {}",
        result.hits().items()[0].retrieval().value()
    );
    let _ = std::fs::remove_dir_all(&fixture.dir);
}

#[tokio::test]
async fn hybrid_rank_boost_orders_lexical_ahead_then_captured_at() {
    let fixture = index_frames("order", &[0, 5_000_000]).await;
    assert_eq!(fixture.ids.len(), 2);
    let early = &fixture.ids[0];
    let late = &fixture.ids[1];
    set_description(&fixture.store, early, "clear plastic labware").await;
    set_description(&fixture.store, late, "A red pipette on the deck").await;
    let blob = query_embedding_blob(&fixture.semantic, "red").await;
    set_embedding(&fixture.store, early, &blob).await;
    let result = fixture
        .search
        .search_semantic_frames(text_search("red", fixture.provenance.clone(), 10, 10, None))
        .await
        .unwrap();
    let ids: Vec<&str> = result
        .hits()
        .items()
        .iter()
        .map(|hit| hit.frame().frame_id().as_str())
        .collect();
    assert_eq!(ids, vec![late.as_str(), early.as_str()]);
    let late_score = result.hits().items()[0].retrieval().value();
    let early_score = result.hits().items()[1].retrieval().value();
    assert!(late_score < early_score);
    assert!((early_score - 1.0).abs() < 1e-5);
    let _ = std::fs::remove_dir_all(&fixture.dir);
}

#[tokio::test]
async fn older_red_outside_newest_candidate_limit_is_returned() {
    let fixture = index_frames("older-red", &[0, 5_000_000]).await;
    assert_eq!(fixture.ids.len(), 2);
    let older = &fixture.ids[0];
    let newer = &fixture.ids[1];
    set_description(&fixture.store, older, "A red pipette on the deck").await;
    set_description(&fixture.store, newer, "clear plastic labware").await;
    let result = fixture
        .search
        .search_semantic_frames(text_search_limited(
            "red",
            fixture.provenance.clone(),
            10,
            1,
            10,
            None,
        ))
        .await
        .unwrap();
    let ids: Vec<&str> = result
        .hits()
        .items()
        .iter()
        .map(|hit| hit.frame().frame_id().as_str())
        .collect();
    assert!(ids.contains(&older.as_str()), "older red missing: {ids:?}");
    assert!(!ids.contains(&newer.as_str()));
    let _ = std::fs::remove_dir_all(&fixture.dir);
}

#[tokio::test]
async fn lexical_and_vector_union_dedupes_by_frame_id() {
    let fixture = index_frames("union", &[0, 5_000_000]).await;
    let early = &fixture.ids[0];
    let late = &fixture.ids[1];
    set_description(&fixture.store, early, "A red pipette on the deck").await;
    set_description(&fixture.store, late, "A red pipette on the deck").await;
    let blob = query_embedding_blob(&fixture.semantic, "red").await;
    set_embedding(&fixture.store, late, &blob).await;
    let result = fixture
        .search
        .search_semantic_frames(text_search("red", fixture.provenance.clone(), 10, 10, None))
        .await
        .unwrap();
    let ids: Vec<&str> = result
        .hits()
        .items()
        .iter()
        .map(|hit| hit.frame().frame_id().as_str())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(
        ids.len(),
        ids.iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len()
    );
    assert!(ids.contains(&early.as_str()));
    assert!(ids.contains(&late.as_str()));
    let _ = std::fs::remove_dir_all(&fixture.dir);
}

async fn red_hit_count_after_sql(name: &str, sql: &str) -> usize {
    let fixture = index_frames(name, &[0]).await;
    set_description(&fixture.store, &fixture.ids[0], "A red pipette on the deck").await;
    sqlx::query(sql)
        .execute(fixture.store.semantic_pool())
        .await
        .unwrap();
    let count = fixture
        .search
        .search_semantic_frames(text_search("red", fixture.provenance.clone(), 10, 10, None))
        .await
        .unwrap()
        .hits()
        .items()
        .len();
    let _ = std::fs::remove_dir_all(&fixture.dir);
    count
}

#[tokio::test]
async fn lexical_omits_expired_superseded_non_current_and_wrong_provenance() {
    assert_eq!(
        red_hit_count_after_sql(
            "lex-expired",
            "UPDATE semantic_frame_revisions SET retained_state = 'expired'"
        )
        .await,
        0
    );
    assert_eq!(
        red_hit_count_after_sql(
            "lex-superseded",
            "UPDATE semantic_frame_revisions SET search_current = 0, retained_state = 'superseded'"
        )
        .await,
        0
    );
    assert_eq!(
        red_hit_count_after_sql(
            "lex-non-current",
            "UPDATE semantic_frame_revisions SET search_current = 0"
        )
        .await,
        0
    );
    assert_eq!(
        red_hit_count_after_sql(
            "lex-prov",
            "UPDATE semantic_frame_embeddings SET embedding_provider = 'other'"
        )
        .await,
        0
    );
    assert_eq!(
        red_hit_count_after_sql(
            "lex-generation",
            "UPDATE semantic_frame_descriptions SET description_generation = description_generation + 1"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn unplayable_lexical_match_is_omitted() {
    let fixture = index_frames("avail", &[0]).await;
    let frame_id = &fixture.ids[0];
    set_description(&fixture.store, frame_id, "A red pipette on the deck").await;
    sqlx::query("UPDATE dvr_frames SET payload_ready = 0")
        .execute(fixture.store.media_pool())
        .await
        .unwrap();
    let result = fixture
        .search
        .search_semantic_frames(text_search("red", fixture.provenance.clone(), 10, 10, None))
        .await
        .unwrap();
    assert!(result.hits().items().is_empty());
    let _ = std::fs::remove_dir_all(&fixture.dir);
}

#[tokio::test]
async fn top_k_and_page_apply_after_hybrid_union() {
    let fixture = index_frames("page", &[0, 5_000_000, 10_000_000]).await;
    assert_eq!(fixture.ids.len(), 3);
    for id in &fixture.ids {
        set_description(&fixture.store, id, "A red pipette on the deck").await;
    }
    let first = fixture
        .search
        .search_semantic_frames(text_search("red", fixture.provenance.clone(), 2, 1, None))
        .await
        .unwrap();
    assert_eq!(first.hits().items().len(), 1);
    let cursor = first.hits().next_cursor().map(ToOwned::to_owned);
    assert!(cursor.is_some());
    let second = fixture
        .search
        .search_semantic_frames(text_search("red", fixture.provenance.clone(), 2, 1, cursor))
        .await
        .unwrap();
    assert_eq!(second.hits().items().len(), 1);
    assert_ne!(
        first.hits().items()[0].frame().frame_id(),
        second.hits().items()[0].frame().frame_id()
    );
    assert!(second.hits().next_cursor().is_none());
    let _ = std::fs::remove_dir_all(&fixture.dir);
}

#[tokio::test]
async fn blank_and_token_empty_and_reference_frame_stay_vector_only() {
    let fixture = index_frames("query", &[0, 5_000_000]).await;
    let lexical = &fixture.ids[0];
    let vector = &fixture.ids[1];
    set_description(&fixture.store, lexical, "A red pipette on the deck").await;
    set_description(&fixture.store, vector, "clear plastic labware").await;
    let punctuation = "!!!";
    let blob = query_embedding_blob(&fixture.semantic, punctuation).await;
    set_embedding(&fixture.store, vector, &blob).await;
    let blank = SearchSemanticFramesRequest::new(
        SemanticSearchScope::new(VideoSourceId::new("cam-a").unwrap(), None).unwrap(),
        SemanticSearchQuery::Text {
            text: "   ".to_owned(),
        },
        fixture.provenance.clone(),
        SemanticSearchLimits::pilot(),
        PageRequest::new(None, 5).unwrap(),
        None,
    );
    assert!(blank.unwrap_err().to_string().contains("nonblank"));
    let token_empty = fixture
        .search
        .search_semantic_frames(text_search(
            punctuation,
            fixture.provenance.clone(),
            10,
            10,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(token_empty.hits().items().len(), 1);
    assert_eq!(
        token_empty.hits().items()[0].frame().frame_id().as_str(),
        vector
    );
    let reference = fixture.store.get_manifest_frame(vector).await.unwrap();
    let by_frame = fixture
        .search
        .search_semantic_frames(
            SearchSemanticFramesRequest::new(
                SemanticSearchScope::new(VideoSourceId::new("cam-a").unwrap(), None).unwrap(),
                SemanticSearchQuery::ReferenceFrame {
                    frame: reference.frame().clone(),
                },
                fixture.provenance.clone(),
                SemanticSearchLimits::new(10, 1_000, 5).unwrap(),
                PageRequest::new(None, 10).unwrap(),
                None,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let ids: Vec<&str> = by_frame
        .hits()
        .items()
        .iter()
        .map(|hit| hit.frame().frame_id().as_str())
        .collect();
    assert_eq!(ids, vec![vector.as_str()]);
    assert!(matches!(
        by_frame.hits().items()[0].media_availability(),
        SemanticFrameMediaAvailability::Ready
    ));
    assert_eq!(
        by_frame.hits().items()[0].retrieval().metric(),
        EmbeddingMetric::Cosine
    );
    let _ = std::fs::remove_dir_all(&fixture.dir);
}

#[tokio::test]
async fn exact_substring_before_token_before_vector_only() {
    let fixture = index_frames("priority", &[0, 5_000_000, 10_000_000]).await;
    assert_eq!(fixture.ids.len(), 3);
    let substring = &fixture.ids[0];
    let token = &fixture.ids[1];
    let vector = &fixture.ids[2];
    set_description(&fixture.store, substring, "A red pipette on the deck").await;
    set_description(&fixture.store, token, "sterile pipette rack").await;
    set_description(&fixture.store, vector, "clear plastic labware").await;
    let query = "red pipette";
    let blob = query_embedding_blob(&fixture.semantic, query).await;
    set_embedding(&fixture.store, vector, &blob).await;
    let result = fixture
        .search
        .search_semantic_frames(text_search(query, fixture.provenance.clone(), 10, 10, None))
        .await
        .unwrap();
    let ids: Vec<&str> = result
        .hits()
        .items()
        .iter()
        .map(|hit| hit.frame().frame_id().as_str())
        .collect();
    assert_eq!(
        ids,
        vec![substring.as_str(), token.as_str(), vector.as_str()]
    );
    let vector_score = result.hits().items()[2].retrieval();
    assert_eq!(vector_score.metric(), EmbeddingMetric::Cosine);
    assert!((vector_score.value() - 1.0).abs() < 1e-5);
    for hit in result.hits().items() {
        assert_eq!(hit.retrieval().metric(), EmbeddingMetric::Cosine);
        assert!(
            hit.retrieval().value() > 0.05,
            "public score must stay metric similarity, not RRF {}",
            hit.retrieval().value()
        );
        assert!(matches!(
            hit.media_availability(),
            SemanticFrameMediaAvailability::Ready
        ));
    }
    let _ = std::fs::remove_dir_all(&fixture.dir);
}
