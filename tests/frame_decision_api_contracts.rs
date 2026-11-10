//! Contract tests for authorized Jev decision append and history APIs.

use std::sync::Arc;

use a2a_lab_dev_kit::PREDICATE_PIPETTE_TIP_ATTACHED;
use a2a_lab_dev_kit::{
    A2aLabCommand, A2aLabResult, A2aLabService, AppendFrameDecisionRequest,
    DecisionEvaluationCoverage, DecisionEvidenceItem, DecisionHistoryPolicy,
    DecisionIdempotencyKey, DecisionOutcome, DecisionSubject, DvrStoreNamespace, DvrTrackId,
    EmptyImages, FakeFrameDecisionLedgerProvider, FrameDecisionClientRole,
    FrameDecisionEvaluationOwnership, FrameDecisionEvaluatorLimits,
    FrameDecisionEvaluatorProviderConfig, FrameDecisionLedgerProvider, FrameDecisionOperation,
    FrameDecisionReadLimits, FrameDecisionReadScope, FrameDecisionRecord, FrameRoiTransform,
    GetFrameDecisionCapabilitiesRequest, JevEvaluationId, JevEvaluationProvenance,
    JevProducerIdentity, JevStructuredOutput, LabPredicate, MemoryLogs, MemoryMetrics, MemoryTasks,
    MemoryVideos, PipetteChannelSubject, ReadFrameDecisionHistoryRequest,
    RegisteredFrameDecisionEvaluator, RejectedDecisionProvenance, Sha256Digest,
    TckMalformedFrameDecisionRequests, TimeRange, UtcTimestamp, VideoSourceId,
    reject_decision_provenance,
};

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
    JevEvaluationId::new("jev-req-chupacabra-001").unwrap()
}

fn provenance(captured: &str, evaluated: &str) -> JevEvaluationProvenance {
    JevEvaluationProvenance::new(
        UtcTimestamp::parse(captured).unwrap(),
        UtcTimestamp::parse(evaluated).unwrap(),
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
    key: DecisionIdempotencyKey,
) -> FrameDecisionRecord {
    FrameDecisionRecord::new(
        key,
        saved.clone(),
        LabPredicate::pipette_tip_attached_v1(),
        tip_subject(),
        tip_evidence(saved),
        outcome,
        JevStructuredOutput::empty(),
        provenance("2024-06-01T12:00:00Z", "2024-06-01T12:00:02Z"),
        None,
    )
    .unwrap()
}

fn scope(saved: &a2a_lab_dev_kit::SavedFrameRef) -> FrameDecisionReadScope {
    FrameDecisionReadScope::new(
        saved.source_id().clone(),
        TimeRange::new(
            UtcTimestamp::parse("2024-06-01T12:00:00Z").unwrap(),
            UtcTimestamp::parse("2024-06-01T12:01:00Z").unwrap(),
        )
        .unwrap(),
        Some(saved.clone()),
    )
    .unwrap()
}

fn lab_with_decisions(
    provider: Arc<dyn FrameDecisionLedgerProvider>,
) -> A2aLabService<MemoryLogs, MemoryMetrics, MemoryTasks, EmptyImages, MemoryVideos> {
    A2aLabService::new(
        MemoryLogs::default(),
        MemoryMetrics::default(),
        MemoryTasks::default(),
    )
    .with_videos(MemoryVideos::default())
    .with_frame_decisions(provider)
}

#[tokio::test]
async fn capability_discovery_distinguishes_roles_and_schemas() {
    let config = evaluator_config();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let producer =
        Arc::new(FakeFrameDecisionLedgerProvider::authorized_producer(&config, limits).unwrap());
    let reader =
        Arc::new(FakeFrameDecisionLedgerProvider::read_only_consumer(&config, limits).unwrap());
    let producer_lab = lab_with_decisions(producer);
    let reader_lab = lab_with_decisions(reader);

    let producer_caps = producer_lab
        .execute(A2aLabCommand::GetFrameDecisionCapabilities(
            GetFrameDecisionCapabilitiesRequest::new(),
        ))
        .await
        .unwrap()
        .task
        .result;
    let reader_caps = reader_lab
        .execute(A2aLabCommand::GetFrameDecisionCapabilities(
            GetFrameDecisionCapabilitiesRequest::new(),
        ))
        .await
        .unwrap()
        .task
        .result;

    let A2aLabResult::GetFrameDecisionCapabilities(producer_caps) = producer_caps else {
        panic!("expected capabilities");
    };
    let A2aLabResult::GetFrameDecisionCapabilities(reader_caps) = reader_caps else {
        panic!("expected capabilities");
    };

    assert!(producer_caps.is_registered());
    assert_eq!(
        producer_caps.client_role(),
        FrameDecisionClientRole::AuthorizedProducer
    );
    assert!(
        producer_caps
            .operations()
            .contains(&FrameDecisionOperation::AppendFrameDecision)
    );
    assert!(producer_caps.schemas().is_some());

    assert_eq!(
        reader_caps.client_role(),
        FrameDecisionClientRole::ReadOnlyConsumer
    );
    assert!(
        !reader_caps
            .operations()
            .contains(&FrameDecisionOperation::AppendFrameDecision)
    );
}

#[tokio::test]
async fn unregistered_provider_reports_no_operations() {
    let provider = Arc::new(FakeFrameDecisionLedgerProvider::unregistered(
        FrameDecisionClientRole::ReadOnlyConsumer,
    ));
    let lab = lab_with_decisions(provider);
    let caps = lab
        .execute(A2aLabCommand::GetFrameDecisionCapabilities(
            GetFrameDecisionCapabilitiesRequest::new(),
        ))
        .await
        .unwrap()
        .task
        .result;
    let A2aLabResult::GetFrameDecisionCapabilities(caps) = caps else {
        panic!("expected capabilities");
    };
    assert!(!caps.is_registered());
    assert!(caps.operations().is_empty());
    let err = lab
        .execute(A2aLabCommand::ReadFrameDecisionHistory(
            ReadFrameDecisionHistoryRequest::new(
                scope(&frame(b"missing")),
                None,
                None,
                None,
                DecisionHistoryPolicy::FullHistory,
                FrameDecisionReadLimits::pilot(),
                a2a_lab_dev_kit::PageRequest::default(),
                Vec::new(),
            )
            .unwrap(),
        ))
        .await
        .expect_err("unregistered read");
    assert_eq!(err.code(), "unavailable");
}

#[tokio::test]
async fn append_rejects_read_only_consumers_and_replays_idempotently() {
    let config = evaluator_config();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let producer =
        Arc::new(FakeFrameDecisionLedgerProvider::authorized_producer(&config, limits).unwrap());
    let reader =
        Arc::new(FakeFrameDecisionLedgerProvider::read_only_consumer(&config, limits).unwrap());
    let saved = frame(b"append-api");
    let key = DecisionIdempotencyKey::new("jev-req-chupacabra-001:lab.pipette_tip_attached:left-1")
        .unwrap();
    let record = tip_record(&saved, DecisionOutcome::Present, key.clone());
    let request = AppendFrameDecisionRequest::new(
        record,
        FrameDecisionEvaluationOwnership::new(evaluation_id()),
    )
    .unwrap();

    let producer_lab =
        lab_with_decisions(Arc::clone(&producer) as Arc<dyn FrameDecisionLedgerProvider>);
    let first = producer_lab
        .execute(A2aLabCommand::AppendFrameDecision(request.clone()))
        .await
        .unwrap()
        .task
        .result;
    let second = producer_lab
        .execute(A2aLabCommand::AppendFrameDecision(request))
        .await
        .unwrap()
        .task
        .result;

    let A2aLabResult::AppendFrameDecision(first) = first else {
        panic!("append result");
    };
    let A2aLabResult::AppendFrameDecision(second) = second else {
        panic!("append replay");
    };
    assert!(!first.replayed());
    assert!(second.replayed());

    let reader_lab = lab_with_decisions(reader);
    let unauthorized = reader_lab
        .execute(A2aLabCommand::AppendFrameDecision(
            AppendFrameDecisionRequest::new(
                tip_record(&saved, DecisionOutcome::Absent, key),
                FrameDecisionEvaluationOwnership::new(evaluation_id()),
            )
            .unwrap(),
        ))
        .await
        .expect_err("read-only append");
    assert_eq!(unauthorized.code(), "invalid");
}

#[tokio::test]
async fn append_conflict_and_forged_provenance_rejected() {
    let config = evaluator_config();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let provider =
        Arc::new(FakeFrameDecisionLedgerProvider::authorized_producer(&config, limits).unwrap());
    let lab = lab_with_decisions(provider);
    let saved = frame(b"conflict-api");
    let key = DecisionIdempotencyKey::new("jev-req-chupacabra-001:lab.pipette_tip_attached:left-1")
        .unwrap();
    lab.execute(A2aLabCommand::AppendFrameDecision(
        AppendFrameDecisionRequest::new(
            tip_record(&saved, DecisionOutcome::Present, key.clone()),
            FrameDecisionEvaluationOwnership::new(evaluation_id()),
        )
        .unwrap(),
    ))
    .await
    .unwrap();
    let conflict = lab
        .execute(A2aLabCommand::AppendFrameDecision(
            AppendFrameDecisionRequest::new(
                tip_record(&saved, DecisionOutcome::Absent, key),
                FrameDecisionEvaluationOwnership::new(evaluation_id()),
            )
            .unwrap(),
        ))
        .await
        .expect_err("conflict");
    assert_eq!(conflict.code(), "invalid");

    let llm_err =
        reject_decision_provenance(RejectedDecisionProvenance::LlmAssessment).expect_err("llm");
    assert!(llm_err.to_string().contains("LLM"));
}

#[tokio::test]
async fn history_reports_unavailable_evidence_without_changing_outcome() {
    let config = evaluator_config();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let provider =
        Arc::new(FakeFrameDecisionLedgerProvider::authorized_producer(&config, limits).unwrap());
    let saved = frame(b"history-api");
    let key = DecisionIdempotencyKey::new("jev-req-chupacabra-001:lab.pipette_tip_attached:left-1")
        .unwrap();
    provider
        .seed(tip_record(&saved, DecisionOutcome::Unknown, key))
        .unwrap();
    let lab = lab_with_decisions(provider);
    let report_key = format!("{}:saved_frame_revision", saved.frame_id().as_str());
    let history = lab
        .execute(A2aLabCommand::ReadFrameDecisionHistory(
            ReadFrameDecisionHistoryRequest::new(
                scope(&saved),
                Some(LabPredicate::pipette_tip_attached_v1()),
                None,
                None,
                DecisionHistoryPolicy::FullHistory,
                FrameDecisionReadLimits::pilot(),
                a2a_lab_dev_kit::PageRequest::default(),
                vec![report_key],
            )
            .unwrap(),
        ))
        .await
        .unwrap()
        .task
        .result;
    let A2aLabResult::ReadFrameDecisionHistory(history) = history else {
        panic!("history");
    };
    let entry = history.entries().items().first().expect("one row");
    assert_eq!(entry.record().outcome(), DecisionOutcome::Unknown);
    assert!(matches!(
        entry.evidence_availability().first(),
        Some(a2a_lab_dev_kit::DecisionEvidenceAvailability::Unavailable { .. })
    ));
}

#[tokio::test]
async fn command_roundtrip_preserves_append_result_envelope() {
    let config = evaluator_config();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let provider =
        Arc::new(FakeFrameDecisionLedgerProvider::authorized_producer(&config, limits).unwrap());
    let lab = lab_with_decisions(provider);
    let saved = frame(b"serde-roundtrip");
    let key = DecisionIdempotencyKey::new("jev-req-chupacabra-001:lab.pipette_tip_attached:left-1")
        .unwrap();
    let request = AppendFrameDecisionRequest::new(
        tip_record(&saved, DecisionOutcome::Present, key),
        FrameDecisionEvaluationOwnership::new(evaluation_id()),
    )
    .unwrap();
    let encoded =
        serde_json::to_value(A2aLabCommand::AppendFrameDecision(request.clone())).unwrap();
    let decoded: A2aLabCommand = serde_json::from_value(encoded).unwrap();
    let A2aLabCommand::AppendFrameDecision(decoded) = decoded else {
        panic!("command tag");
    };
    assert_eq!(
        decoded.record().decision_id(),
        request.record().decision_id()
    );
    let outcome = lab
        .execute(A2aLabCommand::AppendFrameDecision(decoded))
        .await
        .unwrap()
        .task
        .result;
    let wire = serde_json::to_value(&outcome).unwrap();
    assert!(wire.to_string().contains("append_frame_decision"));
}

#[test]
fn malformed_history_fixtures_fail_validation() {
    let saved = frame(b"malformed");
    let scope = scope(&saved);
    assert!(
        TckMalformedFrameDecisionRequests::history_zero_page_limit(scope.clone())
            .check()
            .is_err()
    );
    assert!(
        TckMalformedFrameDecisionRequests::history_page_exceeds_max_rows(scope)
            .check()
            .is_err()
    );
}

#[test]
fn registered_evaluator_lists_initial_predicates_only() {
    let config = evaluator_config();
    let registered =
        RegisteredFrameDecisionEvaluator::register(&config, FrameDecisionEvaluatorLimits::pilot())
            .unwrap();
    let caps = registered.capabilities();
    let ids: Vec<_> = caps
        .supported_predicates()
        .iter()
        .map(LabPredicate::id)
        .collect();
    assert!(ids.contains(&PREDICATE_PIPETTE_TIP_ATTACHED));
}
