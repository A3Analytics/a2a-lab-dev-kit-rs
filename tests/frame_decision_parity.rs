//! A2A and MCP parity for authorized Jev decision append and history APIs.

use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabCommand, A2aLabError, A2aLabResult, A2aLabService,
    AppendFrameDecisionRequest, AppendFrameDecisionResult, DecisionEvaluationCoverage,
    DecisionEvidenceItem, DecisionIdempotencyKey, DecisionOutcome, DecisionSubject,
    DvrStoreNamespace, DvrTrackId, FakeFrameDecisionEvaluator, FakeFrameDecisionLedgerProvider,
    FakeJevBehavior, FrameDecisionEvaluationOwnership, FrameDecisionEvaluationRequest,
    FrameDecisionEvaluationRouter, FrameDecisionEvaluatorLimits,
    FrameDecisionEvaluatorProviderConfig, FrameDecisionLedgerProvider, FrameDecisionRecord,
    FrameRoiTransform, GetFrameDecisionCapabilitiesRequest, JevEvaluationId,
    JevEvaluationProvenance, JevProducerIdentity, JevStructuredOutput, LabPredicate, McpLab,
    McpServer, MemoryLogs, MemoryMetrics, MemoryTasks, MemoryVideos, OperatorDecisionCriteria,
    OperatorReferenceImage, PipetteChannelSubject, Sha256Digest, TckMalformedFrameDecisionRequests,
    TimeRange, UtcTimestamp, VideoSourceId, bind_local,
};
use rmcp::handler::server::common::schema_for_input;
use rmcp::handler::server::wrapper::Parameters;

fn evaluator_config() -> FrameDecisionEvaluatorProviderConfig {
    FrameDecisionEvaluatorProviderConfig::pilot(
        Sha256Digest::hash_bytes(b"eval-config"),
        Sha256Digest::hash_bytes(b"threshold"),
    )
    .unwrap()
}

fn frame(payload: &[u8]) -> a2a_lab_dev_kit::SavedFrameRef {
    a2a_lab_dev_kit::SavedFrameRef::from_payload_bytes(
        DvrStoreNamespace::new("chupacabra.dvr").unwrap(),
        VideoSourceId::new("deck-left").unwrap(),
        DvrTrackId::new("mjpeg-0").unwrap(),
        UtcTimestamp::parse("2024-06-01T12:00:00Z").unwrap(),
        payload,
        None,
    )
    .unwrap()
}

fn evaluation_id() -> JevEvaluationId {
    JevEvaluationId::new("jev-req-parity-001").unwrap()
}

fn idempotency_key() -> DecisionIdempotencyKey {
    DecisionIdempotencyKey::new("jev-req-parity-001:lab.pipette_tip_attached:left-1").unwrap()
}

fn provenance() -> JevEvaluationProvenance {
    JevEvaluationProvenance::new(
        UtcTimestamp::parse("2024-06-01T12:00:00Z").unwrap(),
        UtcTimestamp::parse("2024-06-01T12:00:02Z").unwrap(),
        JevProducerIdentity::default_production(),
        evaluation_id(),
        Sha256Digest::hash_bytes(b"config-v1"),
        Sha256Digest::hash_bytes(b"prompt-v1"),
        Sha256Digest::hash_bytes(b"criteria-v1"),
        Sha256Digest::hash_bytes(b"threshold-v1"),
        FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
        Sha256Digest::hash_bytes(b"jev-response-bytes"),
        DecisionEvaluationCoverage::Evaluated,
        Option::<&str>::None,
    )
    .unwrap()
}

fn tip_subject() -> DecisionSubject {
    DecisionSubject::PipetteChannel(
        PipetteChannelSubject::new(
            "left-pipetter",
            "channel-1",
            Some(FrameRoiTransform::new([0.2, 0.3, 0.5, 0.7]).unwrap()),
        )
        .unwrap(),
    )
}

fn tip_evidence(saved: &a2a_lab_dev_kit::SavedFrameRef) -> Vec<DecisionEvidenceItem> {
    vec![
        DecisionEvidenceItem::saved_frame_revision(
            saved.frame_id().clone(),
            FrameRoiTransform::new([0.2, 0.3, 0.5, 0.7]).unwrap(),
        )
        .unwrap(),
    ]
}

fn tip_record(
    saved: &a2a_lab_dev_kit::SavedFrameRef,
    outcome: DecisionOutcome,
) -> FrameDecisionRecord {
    FrameDecisionRecord::new(
        idempotency_key(),
        saved.clone(),
        LabPredicate::pipette_tip_attached_v1(),
        tip_subject(),
        tip_evidence(saved),
        outcome,
        JevStructuredOutput::empty(),
        provenance(),
        None,
    )
    .unwrap()
}

fn append_request(
    saved: &a2a_lab_dev_kit::SavedFrameRef,
    outcome: DecisionOutcome,
) -> AppendFrameDecisionRequest {
    AppendFrameDecisionRequest::new(
        tip_record(saved, outcome),
        FrameDecisionEvaluationOwnership::new(evaluation_id()),
    )
    .unwrap()
}

fn lab_with(provider: Arc<dyn FrameDecisionLedgerProvider>) -> Arc<dyn A2aLabApi> {
    A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(MemoryVideos::default())
        .with_frame_decisions(provider)
        .share()
}

fn lab_without_decisions() -> Arc<dyn A2aLabApi> {
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
#[allow(clippy::large_enum_variant)]
enum Observed {
    Append(AppendFrameDecisionResult),
    Capabilities(a2a_lab_dev_kit::FrameDecisionCapabilities),
    Error(A2aLabError),
}

async fn observe_a2a(client: &A2aClient, command: A2aLabCommand) -> Observed {
    match command {
        A2aLabCommand::GetFrameDecisionCapabilities(request) => client
            .get_frame_decision_capabilities(request)
            .await
            .map_or_else(Observed::Error, Observed::Capabilities),
        A2aLabCommand::AppendFrameDecision(request) => client
            .append_frame_decision(request)
            .await
            .map_or_else(Observed::Error, Observed::Append),
        A2aLabCommand::ReadFrameDecisionHistory(request) => {
            let _ = client.read_frame_decision_history(request).await;
            Observed::Error(A2aLabError::protocol("unexpected history success"))
        }
        _ => Observed::Error(A2aLabError::protocol("unexpected command")),
    }
}

async fn observe_mcp(mcp: &McpLab, command: A2aLabCommand) -> Observed {
    match mcp.execute(command).await {
        Ok(outcome) => match outcome.task.result {
            A2aLabResult::GetFrameDecisionCapabilities(caps) => Observed::Capabilities(caps),
            A2aLabResult::AppendFrameDecision(result) => Observed::Append(result),
            other => Observed::Error(A2aLabError::protocol(format!("{other:?}"))),
        },
        Err(error) => Observed::Error(error),
    }
}

fn same(left: &Observed, right: &Observed) -> bool {
    match (left, right) {
        (Observed::Append(left), Observed::Append(right)) => {
            left.record().decision_id() == right.record().decision_id()
                && left.record().outcome() == right.record().outcome()
        }
        (Observed::Capabilities(left), Observed::Capabilities(right)) => left == right,
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

fn producer_provider() -> Arc<FakeFrameDecisionLedgerProvider> {
    let config = evaluator_config();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    Arc::new(FakeFrameDecisionLedgerProvider::authorized_producer(&config, limits).unwrap())
}

fn producer_lab() -> Arc<dyn A2aLabApi> {
    lab_with(producer_provider())
}

fn read_only_lab() -> Arc<dyn A2aLabApi> {
    let config = evaluator_config();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let provider =
        Arc::new(FakeFrameDecisionLedgerProvider::read_only_consumer(&config, limits).unwrap());
    lab_with(provider)
}

#[test]
fn capabilities_request_mcp_input_schema_roots_at_object() {
    let schema = schema_for_input::<Parameters<GetFrameDecisionCapabilitiesRequest>>()
        .expect("input schema");
    assert_eq!(
        schema.get("type").and_then(|value| value.as_str()),
        Some("object"),
        "schema root must be object: {schema:?}"
    );
    assert!(
        serde_json::to_value(GetFrameDecisionCapabilitiesRequest::new())
            .unwrap()
            .is_object()
    );
}

#[tokio::test]
async fn capabilities_match_across_transports() {
    let endpoints = open(producer_lab()).await;
    compare(
        &endpoints,
        "capabilities",
        A2aLabCommand::GetFrameDecisionCapabilities(GetFrameDecisionCapabilitiesRequest::new()),
    )
    .await;
}

#[tokio::test]
async fn append_idempotent_replay_matches_across_transports() {
    let saved = frame(b"replay-parity");
    let request = append_request(&saved, DecisionOutcome::Present);
    let provider = producer_provider();
    provider
        .seed(tip_record(&saved, DecisionOutcome::Present))
        .unwrap();
    let endpoints = open(lab_with(provider)).await;
    let a2a = observe_a2a(
        &endpoints.a2a,
        A2aLabCommand::AppendFrameDecision(request.clone()),
    )
    .await;
    let mcp = observe_mcp(&endpoints.mcp, A2aLabCommand::AppendFrameDecision(request)).await;
    assert!(same(&a2a, &mcp));
    assert!(matches!(
        (&a2a, &mcp),
        (Observed::Append(left), Observed::Append(right)) if left.replayed() && right.replayed()
    ));
}

#[tokio::test]
async fn append_idempotency_conflict_matches_across_transports() {
    let saved = frame(b"conflict-parity");
    let provider = producer_provider();
    provider
        .seed(tip_record(&saved, DecisionOutcome::Present))
        .unwrap();
    let endpoints = open(lab_with(provider)).await;
    let conflict = AppendFrameDecisionRequest::new(
        tip_record(&saved, DecisionOutcome::Absent),
        FrameDecisionEvaluationOwnership::new(evaluation_id()),
    )
    .unwrap();
    let a2a = observe_a2a(
        &endpoints.a2a,
        A2aLabCommand::AppendFrameDecision(conflict.clone()),
    )
    .await;
    let mcp = observe_mcp(&endpoints.mcp, A2aLabCommand::AppendFrameDecision(conflict)).await;
    assert!(same(&a2a, &mcp));
    assert!(matches!(
        (&a2a, &mcp),
        (Observed::Error(left), Observed::Error(right))
            if left.code() == "invalid" && right.code() == "invalid"
    ));
}

#[tokio::test]
async fn read_only_append_rejected_equally() {
    let saved = frame(b"readonly-parity");
    let command =
        A2aLabCommand::AppendFrameDecision(append_request(&saved, DecisionOutcome::Present));
    let endpoints = open(read_only_lab()).await;
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
async fn forged_jev_producer_rejected_equally() {
    let saved = frame(b"forged-parity");
    let mut record = tip_record(&saved, DecisionOutcome::Present);
    let mut value = serde_json::to_value(&record).unwrap();
    value["provenance"]["producer"]["producer_id"] =
        serde_json::Value::String("global.openai.gpt-5.6-luna".to_owned());
    record = serde_json::from_value(value).unwrap();
    let bad = AppendFrameDecisionRequest::new(
        record,
        FrameDecisionEvaluationOwnership::new(evaluation_id()),
    );
    let command = match bad {
        Ok(request) => A2aLabCommand::AppendFrameDecision(request),
        Err(error) => {
            assert_eq!(error.code(), "invalid");
            return;
        }
    };
    let endpoints = open(producer_lab()).await;
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
async fn unknown_predicate_subject_mismatch_rejected_equally() {
    let saved = frame(b"unknown-parity");
    let predicate = LabPredicate::new("lab.unknown_operator_rule", 1).unwrap();
    let record = FrameDecisionRecord::new(
        DecisionIdempotencyKey::new("jev-req-parity-001:lab.unknown_operator_rule:x").unwrap(),
        saved.clone(),
        predicate,
        tip_subject(),
        tip_evidence(&saved),
        DecisionOutcome::Present,
        JevStructuredOutput::empty(),
        provenance(),
        None,
    );
    let command = match record {
        Ok(record) => A2aLabCommand::AppendFrameDecision(
            AppendFrameDecisionRequest::new(
                record,
                FrameDecisionEvaluationOwnership::new(evaluation_id()),
            )
            .unwrap(),
        ),
        Err(error) => {
            assert_eq!(error.code(), "invalid");
            return;
        }
    };
    let endpoints = open(producer_lab()).await;
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
async fn malformed_history_request_rejected_equally() {
    let saved = frame(b"malformed-parity");
    let scope = a2a_lab_dev_kit::FrameDecisionReadScope::new(
        saved.source_id().clone(),
        TimeRange::new(
            UtcTimestamp::parse("2024-06-01T12:00:00Z").unwrap(),
            UtcTimestamp::parse("2024-06-01T12:01:00Z").unwrap(),
        )
        .unwrap(),
        Some(saved),
    )
    .unwrap();
    let command = A2aLabCommand::ReadFrameDecisionHistory(
        TckMalformedFrameDecisionRequests::history_zero_page_limit(scope),
    );
    let endpoints = open(producer_lab()).await;
    let a2a = observe_mcp_error_via_execute(&endpoints.a2a, command.clone()).await;
    let mcp = observe_mcp_error_via_execute_mcp(&endpoints.mcp, command).await;
    assert_eq!(a2a, mcp);
    assert_eq!(a2a, "invalid");
}

async fn observe_mcp_error_via_execute(client: &A2aClient, command: A2aLabCommand) -> String {
    match command {
        A2aLabCommand::ReadFrameDecisionHistory(request) => client
            .read_frame_decision_history(request)
            .await
            .expect_err("history")
            .code()
            .to_owned(),
        _ => panic!("unexpected command"),
    }
}

async fn observe_mcp_error_via_execute_mcp(mcp: &McpLab, command: A2aLabCommand) -> String {
    mcp.execute(command)
        .await
        .expect_err("history")
        .code()
        .to_owned()
}

#[tokio::test]
async fn absent_ledger_unavailable_matches_across_transports() {
    let endpoints = open(lab_without_decisions()).await;
    let command = A2aLabCommand::AppendFrameDecision(append_request(
        &frame(b"outage-parity"),
        DecisionOutcome::Present,
    ));
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
async fn jev_evaluator_outage_does_not_invoke_llm_or_description_paths() {
    let config = evaluator_config();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let criteria = OperatorDecisionCriteria::new(
        Sha256Digest::hash_bytes(b"criteria"),
        Sha256Digest::hash_bytes(b"prompt"),
        vec![
            OperatorReferenceImage::new(
                Sha256Digest::hash_bytes(b"ref"),
                "ref-v1",
                FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let saved = frame(b"router-outage");
    let request = FrameDecisionEvaluationRequest::new(
        limits,
        evaluation_id(),
        saved.clone(),
        LabPredicate::pipette_tip_attached_v1(),
        tip_subject(),
        tip_evidence(&saved),
        criteria,
        FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
        1024,
        640,
    )
    .unwrap();
    let evaluator =
        FakeFrameDecisionEvaluator::new(config, limits, FakeJevBehavior::ProviderOutage);
    let mut router = FrameDecisionEvaluationRouter::new(evaluator);
    let err = router
        .evaluate_frames(vec![request])
        .await
        .expect_err("outage");
    assert_eq!(err.as_str(), "provider_unavailable");
    assert!(!err.to_string().to_lowercase().contains("luna"), "{err}");
}
