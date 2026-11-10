//! A2A and MCP parity for optional semantic frame search and coverage APIs.

use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabCommand, A2aLabError, A2aLabResult, A2aLabService,
    DvrStoreNamespace, DvrTrackId, EmbeddingMetric, FakeSemanticVideoSearchProvider,
    FrameDescriptionProvenance, FrameRoiTransform, FrameVectorIndexAttachment,
    GetSemanticIndexingCoverageRequest, GetSemanticSearchCapabilitiesRequest,
    ListVideoSourcesRequest, McpLab, McpServer, MemoryLogs, MemoryMetrics, MemoryTasks,
    MemoryVideos, ModelSpaceIdentity, PageRequest, ProvenanceGeneration, SavedFrameRef,
    SearchSemanticFramesRequest, SemanticFrameIndexState, SemanticFrameMediaAvailability,
    SemanticFramePlaybackNeighbors, SemanticFrameSearchResult, SemanticIndexingCoverage,
    SemanticIndexingFreshness, SemanticIndexingStateCounts, SemanticProvenanceSelector,
    SemanticSearchLimits, SemanticSearchOperation, SemanticSearchQuery, SemanticSearchScope,
    SemanticVideoSearchProvider, Sha256Digest, TckMalformedSemanticSearchRequests,
    TextEmbeddingProvenance, TimeRange, UtcTimestamp, VideoSourceId, bind_local, fake_semantic_hit,
};
use a2a_lab_dev_kit::{FrameEmbeddingVector, VectorNormalization};
use rmcp::handler::server::common::schema_for_input;
use rmcp::handler::server::wrapper::Parameters;

fn source(name: &str) -> VideoSourceId {
    VideoSourceId::new(name).unwrap()
}

fn scope() -> SemanticSearchScope {
    SemanticSearchScope::new(
        source("deck-left"),
        Some(
            TimeRange::new(
                UtcTimestamp::parse("2024-06-01T12:00:00Z").unwrap(),
                UtcTimestamp::parse("2024-06-01T13:00:00Z").unwrap(),
            )
            .unwrap(),
        ),
    )
    .unwrap()
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

fn search_provider() -> Arc<FakeSemanticVideoSearchProvider> {
    let mut provider = FakeSemanticVideoSearchProvider::available();
    let saved = frame(b"parity-pipette", "2024-06-01T12:00:05Z");
    let attachment = attachment(1, b"A pipette tip is visible on channel one.");
    let prepared = fake_semantic_hit(
        saved,
        attachment,
        "A pipette tip is visible on channel one.",
        0.92,
        neighbors(&frame(b"parity-pipette", "2024-06-01T12:00:05Z")),
    )
    .unwrap();
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
    Arc::new(provider)
}

fn lab_with(provider: Arc<dyn SemanticVideoSearchProvider>) -> Arc<dyn A2aLabApi> {
    A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(MemoryVideos::default())
        .with_semantic_search(provider)
        .share()
}

fn lab_without_semantic() -> Arc<dyn A2aLabApi> {
    A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(MemoryVideos::default())
        .share()
}

struct Endpoints {
    a2a: A2aClient,
    mcp: McpLab,
}

async fn open(lab: Arc<dyn A2aLabApi>) -> Endpoints {
    let a2a_lab = Arc::clone(&lab);
    let (a2a_listener, a2a_address) = bind_local().await.unwrap();
    tokio::spawn(async move {
        a2a_lab_dev_kit::A2aServer::new(&a2a_lab)
            .listen(a2a_listener)
            .await
            .unwrap();
    });
    let (mcp_listener, mcp_address) = bind_local().await.unwrap();
    tokio::spawn(async move {
        McpServer::new(&lab).serve_http(mcp_listener).await.unwrap();
    });
    Endpoints {
        a2a: A2aClient::new(&format!("http://{a2a_address}")).unwrap(),
        mcp: McpLab::connect(&format!("http://{mcp_address}/mcp"))
            .await
            .unwrap(),
    }
}

#[derive(Debug)]
enum Observed {
    Capabilities(a2a_lab_dev_kit::SemanticSearchCapabilities),
    Search(SemanticFrameSearchResult),
    Coverage(SemanticIndexingCoverage),
    Error(A2aLabError),
}

async fn observe_a2a(client: &A2aClient, command: A2aLabCommand) -> Observed {
    match command {
        A2aLabCommand::GetSemanticSearchCapabilities(request) => client
            .get_semantic_search_capabilities(request)
            .await
            .map_or_else(Observed::Error, Observed::Capabilities),
        A2aLabCommand::SearchSemanticFrames(request) => client
            .search_semantic_frames(request)
            .await
            .map_or_else(Observed::Error, Observed::Search),
        A2aLabCommand::GetSemanticIndexingCoverage(request) => client
            .get_semantic_indexing_coverage(request)
            .await
            .map_or_else(Observed::Error, Observed::Coverage),
        _ => Observed::Error(A2aLabError::protocol("unexpected command")),
    }
}

async fn observe_mcp(mcp: &McpLab, command: A2aLabCommand) -> Observed {
    match mcp.execute(command).await {
        Ok(outcome) => match outcome.task.result {
            A2aLabResult::GetSemanticSearchCapabilities(caps) => Observed::Capabilities(caps),
            A2aLabResult::SearchSemanticFrames(search) => Observed::Search(search),
            A2aLabResult::GetSemanticIndexingCoverage(coverage) => Observed::Coverage(coverage),
            other => Observed::Error(A2aLabError::protocol(format!("{other:?}"))),
        },
        Err(error) => Observed::Error(error),
    }
}

fn same(left: &Observed, right: &Observed) -> bool {
    match (left, right) {
        (Observed::Capabilities(left), Observed::Capabilities(right)) => left == right,
        (Observed::Search(left), Observed::Search(right)) => {
            left.hits().items().len() == right.hits().items().len()
                && left.incomplete_indexing() == right.incomplete_indexing()
                && left
                    .hits()
                    .items()
                    .first()
                    .map(|hit| hit.frame().frame_id())
                    == right
                        .hits()
                        .items()
                        .first()
                        .map(|hit| hit.frame().frame_id())
        }
        (Observed::Coverage(left), Observed::Coverage(right)) => {
            left.counts() == right.counts() && left.scope_complete() == right.scope_complete()
        }
        (Observed::Error(left), Observed::Error(right)) => left.code() == right.code(),
        _ => false,
    }
}

async fn compare(endpoints: &Endpoints, name: &str, command: A2aLabCommand) {
    let a2a = observe_a2a(&endpoints.a2a, command.clone()).await;
    let mcp = observe_mcp(&endpoints.mcp, command).await;
    assert!(
        same(&a2a, &mcp),
        "case {name} diverged\nA2A: {a2a:?}\nMCP: {mcp:?}"
    );
}

#[test]
fn capabilities_request_mcp_input_schema_roots_at_object() {
    let schema = schema_for_input::<Parameters<GetSemanticSearchCapabilitiesRequest>>()
        .expect("input schema");
    assert_eq!(
        schema.get("type").and_then(|value| value.as_str()),
        Some("object"),
        "schema root must be object: {schema:?}"
    );
    assert!(
        serde_json::to_value(GetSemanticSearchCapabilitiesRequest::new())
            .unwrap()
            .is_object()
    );
}

#[tokio::test]
async fn registered_capabilities_match_across_transports() {
    let endpoints = open(lab_with(search_provider())).await;
    compare(
        &endpoints,
        "capabilities-registered",
        A2aLabCommand::GetSemanticSearchCapabilities(GetSemanticSearchCapabilitiesRequest::new()),
    )
    .await;
    let a2a = observe_a2a(
        &endpoints.a2a,
        A2aLabCommand::GetSemanticSearchCapabilities(GetSemanticSearchCapabilitiesRequest::new()),
    )
    .await;
    let Observed::Capabilities(caps) = a2a else {
        panic!("expected capabilities");
    };
    assert!(caps.is_registered());
    assert!(caps.schemas().is_some());
    assert!(
        caps.operations()
            .contains(&SemanticSearchOperation::SearchSemanticFrames)
    );
}

#[tokio::test]
async fn unregistered_capabilities_match_without_provider() {
    let endpoints = open(lab_without_semantic()).await;
    compare(
        &endpoints,
        "capabilities-unregistered",
        A2aLabCommand::GetSemanticSearchCapabilities(GetSemanticSearchCapabilitiesRequest::new()),
    )
    .await;
    let a2a = observe_a2a(
        &endpoints.a2a,
        A2aLabCommand::GetSemanticSearchCapabilities(GetSemanticSearchCapabilitiesRequest::new()),
    )
    .await;
    let Observed::Capabilities(caps) = a2a else {
        panic!("expected capabilities");
    };
    assert!(!caps.is_registered());
    assert!(caps.schemas().is_none());
}

#[tokio::test]
async fn text_search_matches_across_transports() {
    let endpoints = open(lab_with(search_provider())).await;
    compare(
        &endpoints,
        "text-search",
        A2aLabCommand::SearchSemanticFrames(text_search("pipette", 1)),
    )
    .await;
}

#[tokio::test]
async fn indexing_coverage_matches_across_transports() {
    let endpoints = open(lab_with(search_provider())).await;
    let request = GetSemanticIndexingCoverageRequest::new(scope(), selector(1)).unwrap();
    compare(
        &endpoints,
        "coverage",
        A2aLabCommand::GetSemanticIndexingCoverage(request),
    )
    .await;
}

#[tokio::test]
async fn absent_provider_search_unavailable_matches() {
    let endpoints = open(lab_without_semantic()).await;
    let command = A2aLabCommand::SearchSemanticFrames(text_search("pipette", 1));
    let a2a = observe_a2a(&endpoints.a2a, command.clone()).await;
    let mcp = observe_mcp(&endpoints.mcp, command).await;
    assert!(same(&a2a, &mcp));
    assert!(matches!(
        (&a2a, &mcp),
        (Observed::Error(left), Observed::Error(right))
            if left.code() == "unavailable" && right.code() == "unavailable"
    ));
}

#[tokio::test]
async fn malformed_search_rejected_equally() {
    let endpoints = open(lab_with(search_provider())).await;
    let command = A2aLabCommand::SearchSemanticFrames(
        TckMalformedSemanticSearchRequests::search_zero_page_limit(
            scope(),
            SemanticSearchQuery::Text {
                text: "pipette".to_owned(),
            },
            selector(1),
        ),
    );
    let a2a = observe_a2a(&endpoints.a2a, command.clone()).await;
    let mcp = observe_mcp(&endpoints.mcp, command).await;
    assert!(same(&a2a, &mcp));
    assert!(matches!(
        (&a2a, &mcp),
        (Observed::Error(left), Observed::Error(right))
            if left.code() == "invalid" && right.code() == "invalid"
    ));
}

#[tokio::test]
async fn bounded_search_result_excludes_inline_bytes() {
    let endpoints = open(lab_with(search_provider())).await;
    let result = endpoints
        .a2a
        .search_semantic_frames(text_search("pipette", 1))
        .await
        .unwrap();
    let json = serde_json::to_string(&result).unwrap();
    assert!(!json.contains("embedding_vector"));
    assert!(!json.contains("image_bytes"));
    let hit = result.hits().items().first().expect("hit");
    assert!(matches!(
        hit.media_availability(),
        SemanticFrameMediaAvailability::Ready
    ));
    assert_eq!(hit.index(), SemanticFrameIndexState::Ready);
}

#[tokio::test]
async fn legacy_list_video_sources_unchanged_with_semantic_provider() {
    let lab = lab_with(search_provider());
    let page = lab
        .execute(A2aLabCommand::ListVideoSources(
            ListVideoSourcesRequest::new(PageRequest::default()).unwrap(),
        ))
        .await
        .unwrap();
    let A2aLabResult::ListVideoSources(page) = page.task.result else {
        panic!("expected sources");
    };
    assert!(page.items().is_empty());
}
