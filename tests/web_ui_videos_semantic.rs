//! Videos semantic search UI: capability gates, metadata/decision separation, and fixtures.

#![cfg(feature = "web-ui")]

use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aLabCommand, A2aLabResult, A2aLabService, A2aServer, DecisionEvaluationCoverage,
    DecisionEvidenceItem, DecisionIdempotencyKey, DecisionOutcome, DecisionSubject,
    DvrStoreNamespace, DvrTrackId, EmbeddingMetric, FakeFrameDecisionLedgerProvider,
    FakeSemanticVideoSearchProvider, FrameDecisionEvaluatorLimits,
    FrameDecisionEvaluatorProviderConfig, FrameDecisionRecord, FrameDescriptionProvenance,
    FrameRoiTransform, FrameVectorIndexAttachment, GetVideoTimelineRequest, JevEvaluationId,
    JevEvaluationProvenance, JevProducerIdentity, JevStructuredOutput, LabPredicate,
    MJPEG_MEDIA_TYPE, MemoryLogs, MemoryMetrics, MemoryTasks, MemoryVideos, ModelSpaceIdentity,
    PipetteChannelSubject, ProvenanceGeneration, SavedFrameRef, SemanticFramePlaybackNeighbors,
    SemanticIndexingCoverage, SemanticIndexingFreshness, SemanticIndexingStateCounts,
    SemanticSearchScope, SemanticVideoSearchProvider, Sha256Digest, TextEmbeddingProvenance,
    TimeRange, UtcTimestamp, VideoProvider, VideoSource, VideoSourceId, bind_local,
    fake_semantic_hit,
};
use a2a_lab_dev_kit::{FrameDecisionLedgerProvider, FrameEmbeddingVector, VectorNormalization};

#[path = "harness/browser.rs"]
#[allow(dead_code)]
mod browser;

use browser::Browser;

const CONNECTED: &str = "document.getElementById('connection')?.dataset.state === 'connected'";
const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
];

#[tokio::test]
async fn browser_videos_semantic_absent_keeps_legacy_browsing() {
    let base = legacy_lab().await;
    tokio::task::spawn_blocking(move || legacy_flow(&base))
        .await
        .unwrap();
}

#[tokio::test]
async fn browser_videos_semantic_hits_navigate_and_separate_metadata() {
    let base = semantic_lab().await;
    tokio::task::spawn_blocking(move || semantic_flow(&base))
        .await
        .unwrap();
}

async fn legacy_lab() -> String {
    let videos = filled_videos().await;
    serve(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_videos(videos)
            .share(),
    )
    .await
}

async fn semantic_lab() -> String {
    let videos = filled_videos().await;
    insert_source(&videos, "cam-b", "Camera B").await;
    for at in [
        "2024-01-01T00:00:03Z",
        "2024-01-01T00:00:04Z",
        "2024-01-01T00:00:05Z",
        "2024-01-01T00:00:06Z",
        "2024-01-01T00:00:07Z",
    ] {
        insert_frame(&videos, "cam-a", at).await;
    }
    let semantic = paged_semantic_provider();
    let decisions = decision_provider(&frame_payload()).unwrap();
    serve(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_videos(videos)
            .with_semantic_search(semantic)
            .with_frame_decisions(decisions)
            .share(),
    )
    .await
}

fn legacy_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#videos");
    page.click("video-search-tab");
    page.wait_until("document.getElementById('video-interval-0')");
    page.wait_until(
        "document.getElementById('video-semantic-capability')?.dataset.state === 'absent' || document.getElementById('video-semantic-capability')?.dataset.state === 'ready'",
    );
    let state = page
        .eval("(() => document.getElementById('video-semantic-capability')?.dataset.state ?? '')()")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert_eq!(state, "absent", "legacy browsing should report absent APIs");
    assert_eq!(
        page.eval("document.getElementById('video-browse-tab')?.getAttribute('aria-selected')"),
        serde_json::json!("false"),
    );
    page.click("video-browse-tab");
    page.wait_until("!document.getElementById('video-browse-panel')?.hidden");
    page.wait_until(
        "document.getElementById('video-status')?.dataset.state === 'playing' || document.getElementById('video-player')",
    );
}

fn semantic_flow(base: &str) {
    let mut page = Browser::open(&format!("{base}/ui/"));
    page.wait_until(CONNECTED);
    page.show("#videos");
    page.click("video-search-tab");
    page.wait_until(
        "document.querySelector('#video-search-panel:not([hidden]) #video-search-browser[data-source-id=\"cam-a\"]')",
    );
    page.wait_until(
        "document.getElementById('video-semantic-capability')?.dataset.state === 'ready'",
    );
    assert_eq!(
        page.eval("document.getElementById('video-browse-panel')?.hidden"),
        serde_json::json!(true),
    );
    assert_eq!(
        page.eval("document.getElementById('video-search-context')"),
        serde_json::json!(null),
    );
    assert_eq!(
        page.eval("document.getElementById('video-search-browser')?.dataset.sourceId"),
        serde_json::json!("cam-a"),
    );
    assert_eq!(
        page.eval(
            "document.getElementById('video-search-source-all')?.getAttribute('aria-pressed')"
        ),
        serde_json::json!("true"),
    );
    page.wait_until("document.getElementById('video-search-range-start')?.value.includes('UTC')");
    let player_source = page.eval("document.getElementById('video-source')?.value");
    page.click("video-search-source-0");
    page.click("video-search-source-1");
    page.wait_until(
        "document.getElementById('video-search-browser')?.dataset.searchSources === 'cam-a,cam-b'",
    );
    assert_eq!(
        page.eval(
            "document.getElementById('video-search-source-all')?.getAttribute('aria-pressed')"
        ),
        serde_json::json!("false"),
    );
    assert_eq!(
        page.eval("document.getElementById('video-source')?.value"),
        player_source,
    );
    assert_eq!(
        page.eval("document.getElementById('video-browse-panel')?.hidden"),
        serde_json::json!(true),
    );
    page.click("video-search-source-all");
    page.wait_until(
        "document.getElementById('video-search-browser')?.dataset.searchSources === 'all'",
    );
    page.wait_until(
        "document.getElementById('video-semantic-capability')?.dataset.state === 'ready'",
    );
    page.click("video-search-clear");
    page.wait_until(
        "document.getElementById('video-search-browser')?.dataset.start?.startsWith('2024-01-01T00:00:00')",
    );
    page.click("video-search-local-time");
    page.wait_until(
        "document.getElementById('video-search-local-time')?.getAttribute('aria-pressed') === 'true'",
    );
    assert_eq!(
        page.eval("document.getElementById('video-search-range-start')?.placeholder"),
        serde_json::json!("2024-01-01 00:00:00"),
    );
    page.click("video-search-local-time");
    page.wait_until(
        "document.getElementById('video-search-local-time')?.getAttribute('aria-pressed') === 'false' && document.getElementById('video-search-range-start')?.value.includes('UTC')",
    );
    let generation_before = page.eval("document.getElementById('video-view')?.dataset.gen ?? ''");
    assert_eq!(
        page.eval(
            r#"(() => {
                const start = document.getElementById('video-search-range-start');
                const end = document.getElementById('video-search-range-end');
                start.value = '2024-01-01 00:00:01 UTC';
                end.value = '2024-01-01 00:00:05 UTC';
                start.dispatchEvent(new Event('input', { bubbles: true }));
                end.dispatchEvent(new Event('input', { bubbles: true }));
                return start.value === '2024-01-01 00:00:01 UTC' && end.value === '2024-01-01 00:00:05 UTC';
            })()"#
        ),
        serde_json::json!(true),
    );
    page.wait_until(
        "document.getElementById('video-search-browser')?.dataset.start?.startsWith('2024-01-01T00:00:01')",
    );
    assert_eq!(
        page.eval("document.getElementById('video-view')?.dataset.gen ?? ''"),
        generation_before.clone(),
    );
    page.click("video-search-clear");
    page.wait_until(
        "document.getElementById('video-search-browser')?.dataset.start?.startsWith('2024-01-01T00:00:00')",
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-query')?.value"),
        serde_json::json!(""),
    );
    assert_eq!(
        page.eval("document.getElementById('video-view')?.dataset.gen ?? ''"),
        generation_before,
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-label')?.textContent.trim()"),
        serde_json::json!("Search"),
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-filters')?.getAttribute('aria-label')"),
        serde_json::json!("Search"),
    );
    page.set_value("video-semantic-query", "pipette");
    page.click_selector("#video-semantic-submit");
    page.wait_until("document.querySelector('#video-semantic-hits .video-semantic-hit')");
    page.wait_until("document.querySelector('#video-semantic-hits [data-source=\"cam-b\"]')");
    page.wait_until("document.getElementById('video-semantic-pagination')");
    page.click("video-semantic-next");
    page.wait_until(
        "document.getElementById('video-semantic-page-status')?.textContent.includes('Page 2')",
    );
    assert_eq!(
        page.eval("document.getElementById('video-semantic-hit-0')"),
        serde_json::json!(null),
    );
    page.click("video-semantic-previous");
    page.wait_until("document.getElementById('video-semantic-hit-0')");
    page.wait_until("document.querySelector('[data-chip=\"search\"]')");
    page.click("video-semantic-hit-0");
    page.wait_until("document.getElementById('video-semantic-detail')");
    let html = page
        .eval("(() => document.getElementById('video-semantic-detail')?.innerHTML ?? '')()")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(html.contains("Search metadata (Luna)"));
    assert!(html.contains("Frame decisions (Jev)"));
    assert!(html.contains("not a verified lab decision"));
    assert!(!html.contains("chupacabra.dvr"));
    assert!(!html.contains("Bearer "));
    let index_state = page
        .eval(
            "(() => document.getElementById('video-semantic-index-state')?.dataset.state ?? '')()",
        )
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert_eq!(index_state, "ready");
    page.wait_until(
        "document.querySelector('#video-decision-row-0')?.dataset.outcome === 'present'",
    );
    let decision_outcome = page
        .eval("(() => document.querySelector('#video-decision-row-0')?.dataset.outcome ?? '')()")
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert_eq!(decision_outcome, "present");
}

async fn filled_videos() -> MemoryVideos {
    let videos = MemoryVideos::new();
    insert_source(&videos, "cam-a", "Camera A").await;
    for at in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:02Z",
    ] {
        insert_frame(&videos, "cam-a", at).await;
    }
    videos
}

async fn insert_source(videos: &MemoryVideos, id: &str, name: &str) {
    videos
        .insert_source(
            VideoSource::new(
                VideoSourceId::new(id).unwrap(),
                name,
                format!("{name} recording"),
                None,
                None,
                640,
                360,
                30,
                MJPEG_MEDIA_TYPE,
            )
            .unwrap(),
        )
        .await
        .unwrap();
}

async fn insert_frame(videos: &MemoryVideos, source: &str, at: &str) {
    videos
        .insert_frame(
            VideoSourceId::new(source).unwrap(),
            UtcTimestamp::parse(at).unwrap(),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
}

async fn serve(service: Arc<dyn a2a_lab_dev_kit::A2aLabApi>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    tokio::spawn(async move {
        A2aServer::new(&service)
            .with_web_ui()
            .listen(listener)
            .await
            .unwrap();
    });
    format!("http://{address}")
}

fn semantic_provider() -> Arc<dyn SemanticVideoSearchProvider> {
    let saved = frame_payload();
    let mut provider = FakeSemanticVideoSearchProvider::available();
    let attachment = attachment(1, b"A pipette tip is visible on channel one.");
    let playback = neighbors(&saved);
    let prepared = fake_semantic_hit(
        saved,
        attachment,
        "A pipette tip is visible on channel one.",
        0.92,
        playback,
    )
    .unwrap();
    provider.insert_hit(prepared);
    provider.set_coverage(SemanticIndexingCoverage::new(
        scope(),
        model_space(1),
        SemanticIndexingStateCounts::new(0, 1, 0, 0),
        SemanticIndexingFreshness::new(
            None,
            Some(UtcTimestamp::parse("2024-01-01T00:00:02Z").unwrap()),
            None,
        ),
        true,
    ));
    Arc::new(provider)
}

fn paged_semantic_provider() -> Arc<dyn SemanticVideoSearchProvider> {
    let saved = frame_payload();
    let mut provider = FakeSemanticVideoSearchProvider::available();
    let attachment = attachment(1, b"A pipette tip is visible on channel one.");
    let prepared = fake_semantic_hit(
        saved.clone(),
        attachment.clone(),
        "A pipette tip is visible on channel one.",
        0.92,
        neighbors(&saved),
    )
    .unwrap();
    provider.insert_hit(prepared);
    let other = frame_for("cam-b", "2024-01-01T00:00:01Z");
    let other_hit = fake_semantic_hit(
        other.clone(),
        attachment.clone(),
        "A pipette tip is visible on the external camera.",
        0.91,
        neighbors(&other),
    )
    .unwrap();
    provider.insert_hit(other_hit);
    for (offset, at) in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:02Z",
        "2024-01-01T00:00:03Z",
        "2024-01-01T00:00:04Z",
        "2024-01-01T00:00:05Z",
        "2024-01-01T00:00:06Z",
    ]
    .into_iter()
    .enumerate()
    {
        let frame = frame_at(at);
        let extra = fake_semantic_hit(
            frame.clone(),
            attachment.clone(),
            "A pipette tip is visible on channel one.",
            0.8 - (offset as f32) * 0.05,
            neighbors(&frame),
        )
        .unwrap();
        provider.insert_hit(extra);
    }
    provider.set_coverage(SemanticIndexingCoverage::new(
        scope(),
        model_space(1),
        SemanticIndexingStateCounts::new(0, 1, 0, 0),
        SemanticIndexingFreshness::new(
            None,
            Some(UtcTimestamp::parse("2024-01-01T00:00:02Z").unwrap()),
            None,
        ),
        true,
    ));
    Arc::new(provider)
}

fn decision_provider(
    saved: &SavedFrameRef,
) -> Result<Arc<dyn FrameDecisionLedgerProvider>, a2a_lab_dev_kit::A2aLabError> {
    let config = FrameDecisionEvaluatorProviderConfig::pilot(
        Sha256Digest::hash_bytes(b"eval-config"),
        Sha256Digest::hash_bytes(b"threshold"),
    )?;
    let provider = Arc::new(FakeFrameDecisionLedgerProvider::read_only_consumer(
        &config,
        FrameDecisionEvaluatorLimits::pilot(),
    )?);
    provider.seed(tip_record(saved, DecisionOutcome::Present))?;
    Ok(provider)
}

fn frame_payload() -> SavedFrameRef {
    frame_at("2024-01-01T00:00:01Z")
}

fn frame_at(at: &str) -> SavedFrameRef {
    frame_for("cam-a", at)
}

fn frame_for(source: &str, at: &str) -> SavedFrameRef {
    SavedFrameRef::from_payload_bytes(
        DvrStoreNamespace::new("chupacabra.dvr").unwrap(),
        VideoSourceId::new(source).unwrap(),
        DvrTrackId::new("mjpeg-0").unwrap(),
        UtcTimestamp::parse(at).unwrap(),
        JPEG,
        None,
    )
    .unwrap()
}

fn scope() -> SemanticSearchScope {
    SemanticSearchScope::new(
        VideoSourceId::new("cam-a").unwrap(),
        TimeRange::new(
            UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
            UtcTimestamp::parse("2024-01-01T00:00:10Z").unwrap(),
        )
        .unwrap(),
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

fn neighbors(saved: &SavedFrameRef) -> SemanticFramePlaybackNeighbors {
    let start = saved.captured_at();
    let end = start.saturating_add_micros(5_000_000);
    SemanticFramePlaybackNeighbors::new(TimeRange::new(start, end).unwrap(), vec![saved.clone()], 5)
        .unwrap()
}

fn tip_record(saved: &SavedFrameRef, outcome: DecisionOutcome) -> FrameDecisionRecord {
    let key = DecisionIdempotencyKey::new(
        "jev-ui-fixture-001:lab.pipette_tip_attached:left-pipetter-channel-1",
    )
    .unwrap();
    let subject = DecisionSubject::PipetteChannel(
        PipetteChannelSubject::new(
            "left-pipetter",
            "channel-1",
            Some(FrameRoiTransform::new([0.2, 0.3, 0.5, 0.7]).unwrap()),
        )
        .unwrap(),
    );
    let evidence = vec![
        DecisionEvidenceItem::saved_frame_revision(
            saved.frame_id().clone(),
            FrameRoiTransform::new([0.2, 0.3, 0.5, 0.7]).unwrap(),
        )
        .unwrap(),
    ];
    let provenance = JevEvaluationProvenance::new(
        saved.captured_at(),
        UtcTimestamp::parse("2024-01-01T00:00:05Z").unwrap(),
        JevProducerIdentity::default_production(),
        JevEvaluationId::new("jev-ui-fixture-001").unwrap(),
        Sha256Digest::hash_bytes(b"config-v1"),
        Sha256Digest::hash_bytes(b"prompt-v1"),
        Sha256Digest::hash_bytes(b"criteria-v1"),
        Sha256Digest::hash_bytes(b"threshold-v1"),
        FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
        Sha256Digest::hash_bytes(b"jev-response"),
        DecisionEvaluationCoverage::Evaluated,
        Option::<&str>::None,
    )
    .unwrap();
    FrameDecisionRecord::new(
        key,
        saved.clone(),
        LabPredicate::pipette_tip_attached_v1(),
        subject,
        evidence,
        outcome,
        JevStructuredOutput::new(Some(0.97), None, None, None, Some(0.88), None, None).unwrap(),
        provenance,
        None,
    )
    .unwrap()
}

#[tokio::test]
async fn ui_shaped_search_command_returns_fixture_hit() {
    let videos = filled_videos().await;
    let timeline = videos
        .get_video_timeline(GetVideoTimelineRequest::new(
            VideoSourceId::new("cam-a").unwrap(),
        ))
        .await
        .unwrap();
    let interval = timeline.intervals().first().expect("retained interval");
    let last = timeline.intervals().last().expect("last retained interval");
    let lab = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos)
        .with_semantic_search(semantic_provider())
        .share();
    let command: A2aLabCommand = serde_json::from_value(serde_json::json!({
        "operation": "search_semantic_frames",
        "params": {
            "scope": {
                "source_id": "cam-a",
                "range": {
                    "start": interval.start().to_rfc3339(),
                    "end": last.end().to_rfc3339()
                }
            },
            "query": { "kind": "text", "text": "pipette" },
            "provenance": {
                "model_space": {
                    "provider": "bedrock",
                    "model": "text-embed-v1",
                    "dimensions": 4,
                    "normalization": "l2_unit",
                    "metric": "cosine",
                    "generation": 1
                },
                "description_generation": 1
            },
            "limits": {
                "top_k": 10,
                "candidate_limit": 1000,
                "max_neighbor_frames": 5
            },
            "page": { "limit": 10 }
        }
    }))
    .expect("ui-shaped command");
    let outcome = lab.execute(command).await.expect("execute");
    let A2aLabResult::SearchSemanticFrames(result) = outcome.task.result else {
        panic!("search result");
    };
    assert_eq!(result.hits().items().len(), 1);
}
