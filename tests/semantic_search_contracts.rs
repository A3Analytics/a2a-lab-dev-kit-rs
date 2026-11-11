//! Contract tests for bounded semantic frame search and coverage.

use a2a_lab_dev_kit::{
    DEFAULT_SEMANTIC_TOP_K, DecisionHistoryPolicy, DecisionOutcome, DecisionSearchFilter,
    DecisionSubject, DvrStoreNamespace, DvrTrackId, EmbeddingMetric,
    FakeSemanticVideoSearchProvider, FrameDescriptionProvenance, FrameRoiTransform,
    FrameVectorIndexAttachment, JevProducerIdentity, LabPredicate, ModelSpaceIdentity, PageRequest,
    ProvenanceGeneration, SavedFrameRef, SearchSemanticFramesRequest,
    SemanticFramePlaybackNeighbors, SemanticIndexingCoverage, SemanticIndexingFreshness,
    SemanticIndexingStateCounts, SemanticProvenanceSelector, SemanticSearchLimits,
    SemanticSearchQuery, SemanticSearchScope, SemanticVideoSearchProvider, Sha256Digest,
    TckMalformedSemanticSearchRequests, TextEmbeddingProvenance, TimeRange, UtcTimestamp,
    VideoSourceId, fake_semantic_hit, semantic_search_unavailable,
};
use a2a_lab_dev_kit::{FrameEmbeddingVector, VectorNormalization};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;

fn source(name: &str) -> VideoSourceId {
    VideoSourceId::new(name).unwrap()
}

fn scope() -> SemanticSearchScope {
    scoped_range(
        TimeRange::new(
            UtcTimestamp::parse("2024-06-01T12:00:00Z").unwrap(),
            UtcTimestamp::parse("2024-06-01T13:00:00Z").unwrap(),
        )
        .unwrap(),
    )
}

fn scoped_range(range: TimeRange) -> SemanticSearchScope {
    SemanticSearchScope::new(source("deck-left"), Some(range)).unwrap()
}

fn unscoped() -> SemanticSearchScope {
    SemanticSearchScope::new(source("deck-left"), None).unwrap()
}

fn frame(payload: &[u8], capture: &str) -> SavedFrameRef {
    SavedFrameRef::from_payload_bytes(
        DvrStoreNamespace::new("chupacabra.dvr").unwrap(),
        source("deck-left"),
        DvrTrackId::new("mjpeg-0").unwrap(),
        UtcTimestamp::parse(capture).unwrap(),
        payload,
        None,
    )
    .unwrap()
}

fn model_space(generation: u64) -> ModelSpaceIdentity {
    ModelSpaceIdentity::new(
        "bedrock",
        "text-embed-v1",
        4,
        VectorNormalization::L2Unit,
        EmbeddingMetric::Cosine,
        ProvenanceGeneration::new(generation).unwrap(),
    )
    .unwrap()
}

fn attachment(generation: u64, text: &[u8]) -> FrameVectorIndexAttachment {
    let desc = FrameDescriptionProvenance::new(
        "bedrock",
        "global.openai.gpt-5.6-luna",
        Sha256Digest::hash_bytes(b"prompt-v1"),
        Sha256Digest::hash_bytes(b"preprocess-v1"),
        Some(FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap()),
        Sha256Digest::hash_bytes(text),
        ProvenanceGeneration::new(generation).unwrap(),
    )
    .unwrap();
    let embed = TextEmbeddingProvenance::new(
        "bedrock",
        "text-embed-v1",
        4,
        VectorNormalization::L2Unit,
        EmbeddingMetric::Cosine,
        ProvenanceGeneration::new(generation).unwrap(),
        Sha256Digest::hash_bytes(text),
    )
    .unwrap();
    let vector = FrameEmbeddingVector::new(vec![0.1, 0.2, 0.3, 0.4], 4).unwrap();
    FrameVectorIndexAttachment::new(model_space(generation), desc, embed, vector).unwrap()
}

fn selector(generation: u64) -> SemanticProvenanceSelector {
    SemanticProvenanceSelector::new(
        model_space(generation),
        ProvenanceGeneration::new(generation).unwrap(),
    )
    .unwrap()
}

fn neighbors(saved: &SavedFrameRef) -> SemanticFramePlaybackNeighbors {
    let start = saved.captured_at();
    let end = start.saturating_add_micros(5_000_000);
    SemanticFramePlaybackNeighbors::new(TimeRange::new(start, end).unwrap(), vec![saved.clone()], 5)
        .unwrap()
}

fn text_search(query: &str, generation: u64) -> SearchSemanticFramesRequest {
    SearchSemanticFramesRequest::new(
        scope(),
        SemanticSearchQuery::Text {
            text: query.to_owned(),
        },
        selector(generation),
        SemanticSearchLimits::pilot(),
        PageRequest::new(None, 5).unwrap(),
        None,
    )
    .unwrap()
}

#[test]
fn request_rejects_blank_text() {
    let err = SearchSemanticFramesRequest::new(
        scope(),
        SemanticSearchQuery::Text {
            text: " ".to_owned(),
        },
        selector(1),
        SemanticSearchLimits::pilot(),
        PageRequest::default(),
        None,
    )
    .unwrap_err();
    assert!(err.to_string().contains("nonblank"));
}

#[test]
fn request_accepts_reference_frame_query() {
    let saved = frame(b"a", "2024-06-01T12:00:10Z");
    SearchSemanticFramesRequest::new(
        scope(),
        SemanticSearchQuery::ReferenceFrame { frame: saved },
        selector(1),
        SemanticSearchLimits::pilot(),
        PageRequest::new(None, DEFAULT_SEMANTIC_TOP_K).unwrap(),
        None,
    )
    .unwrap();
}

#[test]
fn decision_filter_requires_authorized_jev_producer() {
    let filter = DecisionSearchFilter::new(
        LabPredicate::pipette_tip_attached_v1(),
        Some(DecisionSubject::PipetteChannel(
            a2a_lab_dev_kit::PipetteChannelSubject::new("left", "ch1", None).unwrap(),
        )),
        Some(DecisionOutcome::Present),
        JevProducerIdentity::default_production(),
        DecisionHistoryPolicy::LatestAuthorizedProduction,
    )
    .unwrap();
    assert_eq!(filter.producer().producer_id(), "typesafe-jev");
}

#[test]
fn malformed_fixture_rejects_zero_page_limit() {
    let request = TckMalformedSemanticSearchRequests::search_zero_page_limit(
        scope(),
        SemanticSearchQuery::Text {
            text: "pipette".to_owned(),
        },
        selector(1),
    );
    assert!(request.check().is_err());
}

#[test]
fn malformed_fixture_rejects_page_above_top_k() {
    let request = TckMalformedSemanticSearchRequests::search_page_exceeds_top_k(
        scope(),
        SemanticSearchQuery::Text {
            text: "pipette".to_owned(),
        },
        selector(1),
    );
    assert!(request.check().is_err());
}

#[tokio::test]
async fn round_trip_text_search_with_fake_provider() {
    let saved = frame(b"pipette-tip", "2024-06-01T12:00:10Z");
    let attachment = attachment(1, b"A pipette tip is visible on channel one.");
    let neighbors = neighbors(&saved);
    let prepared = fake_semantic_hit(
        saved,
        attachment,
        "A pipette tip is visible on channel one.",
        0.92,
        neighbors,
    )
    .unwrap();
    let mut provider = FakeSemanticVideoSearchProvider::available();
    provider.insert_hit(prepared);
    provider.set_coverage(SemanticIndexingCoverage::new(
        scope(),
        model_space(1),
        SemanticIndexingStateCounts::new(0, 1, 0, 0),
        SemanticIndexingFreshness::new(
            None,
            Some(UtcTimestamp::parse("2024-06-01T12:00:10Z").unwrap()),
            None,
        ),
        true,
    ));
    let request = text_search("pipette", 1);
    let result = provider.search_semantic_frames(request).await.unwrap();
    assert_eq!(result.hits().items().len(), 1);
    assert!(!result.incomplete_indexing());
    let hit = &result.hits().items()[0];
    assert_eq!(
        hit.generated_description(),
        "A pipette tip is visible on channel one."
    );
    assert_eq!(hit.retrieval().metric(), EmbeddingMetric::Cosine);
}

#[tokio::test]
async fn capability_absent_returns_unavailable() {
    let provider = FakeSemanticVideoSearchProvider::unavailable();
    let err = provider
        .search_semantic_frames(text_search("tip", 1))
        .await
        .unwrap_err();
    assert_eq!(err, semantic_search_unavailable());
}

#[tokio::test]
async fn pagination_slices_ranked_hits() {
    let mut provider = FakeSemanticVideoSearchProvider::available();
    let base = UtcTimestamp::parse("2024-06-01T12:00:00Z").unwrap();
    let payloads: [&[u8]; 3] = [b"one", b"two", b"three"];
    let scores: [f32; 3] = [0.9, 0.8, 0.7];
    for ((idx, payload), score) in payloads.into_iter().enumerate().zip(scores) {
        let capture = base.saturating_add_micros(i64::try_from(idx).unwrap() * 1_000_000);
        let saved = SavedFrameRef::from_payload_bytes(
            DvrStoreNamespace::new("chupacabra.dvr").unwrap(),
            source("deck-left"),
            DvrTrackId::new("mjpeg-0").unwrap(),
            capture,
            payload,
            None,
        )
        .unwrap();
        let attachment = attachment(1, format!("match {idx}").as_bytes());
        let neighbors = neighbors(&saved);
        let prepared =
            fake_semantic_hit(saved, attachment, &format!("match {idx}"), score, neighbors)
                .unwrap();
        provider.insert_hit(prepared);
    }
    let page0 = provider
        .search_semantic_frames(
            SearchSemanticFramesRequest::new(
                scope(),
                SemanticSearchQuery::Text {
                    text: "match".to_owned(),
                },
                selector(1),
                SemanticSearchLimits::new(3, 100, 3).unwrap(),
                PageRequest::new(None, 2).unwrap(),
                None,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page0.hits().items().len(), 2);
    let cursor = page0.hits().next_cursor().expect("cursor");
    let page1 = provider
        .search_semantic_frames(
            SearchSemanticFramesRequest::new(
                scope(),
                SemanticSearchQuery::Text {
                    text: "match".to_owned(),
                },
                selector(1),
                SemanticSearchLimits::new(3, 100, 3).unwrap(),
                PageRequest::new(Some(cursor.to_owned()), 2).unwrap(),
                None,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page1.hits().items().len(), 1);
}

#[tokio::test]
async fn mixed_space_attachment_is_filtered_out() {
    let saved = frame(b"mix", "2024-06-01T12:00:05Z");
    let attachment = attachment(1, b"plate alignment off");
    let neighbors = neighbors(&saved);
    let prepared =
        fake_semantic_hit(saved, attachment, "plate alignment off", 0.8, neighbors).unwrap();
    let mut provider = FakeSemanticVideoSearchProvider::available();
    provider.insert_hit(prepared);
    let result = provider
        .search_semantic_frames(text_search("plate", 2))
        .await
        .unwrap();
    assert!(result.hits().items().is_empty());
}

#[test]
fn scope_accepts_omitted_range() {
    let scope: SemanticSearchScope =
        serde_json::from_value(serde_json::json!({ "source_id": "deck-left" })).unwrap();
    assert!(scope.range().is_none());
}

#[tokio::test]
async fn omitted_range_keeps_out_of_window_candidates() {
    let mut provider = FakeSemanticVideoSearchProvider::available();
    let early = frame(b"early", "2024-06-01T11:00:05Z");
    let early_attachment = attachment(1, b"early pipette");
    let early_neighbors = neighbors(&early);
    provider.insert_hit(
        fake_semantic_hit(
            early,
            early_attachment,
            "early pipette",
            0.9,
            early_neighbors,
        )
        .unwrap(),
    );
    let late = frame(b"late", "2024-06-01T12:00:05Z");
    let late_frame_id = late.frame_id().clone();
    let late_attachment = attachment(1, b"late pipette");
    let late_neighbors = neighbors(&late);
    provider.insert_hit(
        fake_semantic_hit(late, late_attachment, "late pipette", 0.95, late_neighbors).unwrap(),
    );
    let bounded = SearchSemanticFramesRequest::new(
        scoped_range(
            TimeRange::new(
                UtcTimestamp::parse("2024-06-01T12:00:00Z").unwrap(),
                UtcTimestamp::parse("2024-06-01T13:00:00Z").unwrap(),
            )
            .unwrap(),
        ),
        SemanticSearchQuery::Text {
            text: "pipette".to_owned(),
        },
        selector(1),
        SemanticSearchLimits::pilot(),
        PageRequest::new(None, 5).unwrap(),
        None,
    )
    .unwrap();
    let bounded = provider.search_semantic_frames(bounded).await.unwrap();
    assert_eq!(bounded.hits().items().len(), 1);
    assert_eq!(bounded.hits().items()[0].frame().frame_id(), &late_frame_id);
    let unbounded = SearchSemanticFramesRequest::new(
        unscoped(),
        SemanticSearchQuery::Text {
            text: "pipette".to_owned(),
        },
        selector(1),
        SemanticSearchLimits::pilot(),
        PageRequest::new(None, 5).unwrap(),
        None,
    )
    .unwrap();
    let unbounded = provider.search_semantic_frames(unbounded).await.unwrap();
    assert_eq!(unbounded.hits().items().len(), 2);
}

#[tokio::test]
async fn incomplete_coverage_surfaces_on_empty_page() {
    let mut provider = FakeSemanticVideoSearchProvider::available();
    provider.set_coverage(SemanticIndexingCoverage::new(
        scope(),
        model_space(1),
        SemanticIndexingStateCounts::new(12, 0, 3, 1),
        SemanticIndexingFreshness::new(
            Some(UtcTimestamp::parse("2024-06-01T12:00:01Z").unwrap()),
            None,
            Some(UtcTimestamp::parse("2024-06-01T12:00:30Z").unwrap()),
        ),
        false,
    ));
    let result = provider
        .search_semantic_frames(text_search("anything", 1))
        .await
        .unwrap();
    assert!(result.hits().items().is_empty());
    assert!(result.incomplete_indexing());
}

#[test]
fn hit_json_schema_excludes_decision_fields() {
    fn property_names<T: JsonSchema + DeserializeOwned>() -> Vec<String> {
        let value = serde_json::to_value(schemars::schema_for!(T)).unwrap();
        value
            .pointer("/properties")
            .and_then(|properties| properties.as_object())
            .map(|properties| properties.keys().cloned().collect())
            .unwrap_or_default()
    }
    let names = property_names::<a2a_lab_dev_kit::SemanticFrameHit>();
    for forbidden in [
        "outcome",
        "predicate",
        "probability",
        "jev",
        "decision",
        "confidence",
    ] {
        assert!(
            !names.iter().any(|name| name.contains(forbidden)),
            "SemanticFrameHit must not expose decision-like field {forbidden}"
        );
    }
}

#[tokio::test]
async fn video_provider_still_lists_sources_without_semantic_search() {
    use a2a_lab_dev_kit::{ListVideoSourcesRequest, MemoryVideos, VideoProvider, VideoSource};
    let videos = MemoryVideos::new();
    let deck = VideoSource::new_mjpeg(
        source("deck-left"),
        "Deck",
        "Left camera",
        None,
        None,
        640,
        480,
        5,
    )
    .unwrap();
    videos.insert_source(deck).await.unwrap();
    let page = videos
        .list_video_sources(ListVideoSourcesRequest::new(PageRequest::default()).unwrap())
        .await
        .unwrap();
    assert_eq!(page.items().len(), 1);
}
