//! Contract tests for Jev-only frame decision evaluator boundaries.

use a2a_lab_dev_kit::{
    DecisionEvaluationCoverage, DecisionEvidenceItem, DecisionOutcome, DecisionSubject,
    DecisionThresholdPolicy, DvrStoreNamespace, DvrTrackId, FakeFrameDecisionEvaluator,
    FakeJevBehavior, ForbiddenDecisionInput, FrameDecisionEvaluationRequest,
    FrameDecisionEvaluationRouter, FrameDecisionEvaluator, FrameDecisionEvaluatorLimits,
    FrameDecisionEvaluatorProviderConfig, FrameDecisionLedger, FrameRoiTransform, JevEvaluationId,
    JevStructuredOutput, LabPredicate, LabProbabilityVector, OperatorDecisionCriteria,
    OperatorReferenceImage, PipetteChannelSubject, RegisteredFrameDecisionEvaluator,
    RejectedDecisionEvaluatorBackend, SavedFrameRef, Sha256Digest, UtcTimestamp, VideoSourceId,
    default_pinned_jev_revision, parse_decision_evaluator_provider,
    reject_decision_evaluator_backend, reject_forbidden_decision_input, validate_evaluation_batch,
};

fn frame(payload: &[u8]) -> SavedFrameRef {
    SavedFrameRef::from_payload_bytes(
        DvrStoreNamespace::new("chupacabra.dvr").unwrap(),
        VideoSourceId::new("deck-left").unwrap(),
        DvrTrackId::new("mjpeg-0").unwrap(),
        UtcTimestamp::parse("2024-06-01T12:00:00Z").unwrap(),
        payload,
        None,
    )
    .unwrap()
}

fn evaluator_config() -> FrameDecisionEvaluatorProviderConfig {
    FrameDecisionEvaluatorProviderConfig::pilot(
        Sha256Digest::hash_bytes(b"config-v1"),
        Sha256Digest::hash_bytes(b"threshold-v1"),
    )
    .unwrap()
}

fn criteria() -> OperatorDecisionCriteria {
    OperatorDecisionCriteria::new(
        Sha256Digest::hash_bytes(b"criteria-v1"),
        Sha256Digest::hash_bytes(b"prompt-v1"),
        vec![
            OperatorReferenceImage::new(
                Sha256Digest::hash_bytes(b"operator-ref-still"),
                "expected-tip",
                FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

fn evaluation_request(payload: &[u8]) -> FrameDecisionEvaluationRequest {
    let saved = frame(payload);
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
    FrameDecisionEvaluationRequest::new(
        FrameDecisionEvaluatorLimits::pilot(),
        JevEvaluationId::new("jev-req-chupacabra-001").unwrap(),
        saved,
        LabPredicate::pipette_tip_attached_v1(),
        subject,
        evidence,
        criteria(),
        FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
        1024,
        640,
    )
    .unwrap()
}

#[test]
fn capabilities_expose_jev_provider_limits_and_predicates() {
    let config = evaluator_config();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let registered = RegisteredFrameDecisionEvaluator::register(&config, limits).unwrap();
    let capabilities = registered.capabilities();
    assert_eq!(capabilities.provider_id(), "jev");
    assert_eq!(capabilities.producer_id(), a2a_lab_dev_kit::JEV_PRODUCER_ID);
    assert_eq!(capabilities.model_revision(), default_pinned_jev_revision());
    assert_eq!(
        capabilities.limits().max_image_bytes(),
        a2a_lab_dev_kit::JEV_MAX_IMAGE_BYTES
    );
    assert_eq!(capabilities.supported_predicates().len(), 3);
    assert!(config.supports_predicate(&LabPredicate::pipette_tip_attached_v1()));
}

#[test]
fn luna_and_llm_evaluator_registration_fails() {
    let luna = parse_decision_evaluator_provider("bedrock", "global.openai.gpt-5.6-luna");
    assert!(luna.is_err());
    let openai = parse_decision_evaluator_provider("openai", "gpt-5.6-luna");
    assert!(openai.is_err());
    let description =
        reject_decision_evaluator_backend(RejectedDecisionEvaluatorBackend::LunaDescription);
    assert!(description.is_err());
}

#[test]
fn requests_reject_description_and_similarity_derived_inputs() {
    let err =
        reject_forbidden_decision_input(ForbiddenDecisionInput::EmbeddingSimilarity).unwrap_err();
    assert_eq!(err.as_str(), "forbidden_input");

    let saved = frame(b"forbidden");
    let subject =
        DecisionSubject::PipetteChannel(PipetteChannelSubject::new("p", "c", None).unwrap());
    let evidence = vec![
        DecisionEvidenceItem::reference_image(
            saved.frame_id().clone(),
            FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
            Sha256Digest::hash_bytes(b"description-derived"),
        )
        .unwrap(),
    ];
    let err = FrameDecisionEvaluationRequest::new(
        FrameDecisionEvaluatorLimits::pilot(),
        JevEvaluationId::new("jev-req-forbidden").unwrap(),
        saved,
        LabPredicate::pipette_tip_attached_v1(),
        subject,
        evidence,
        criteria(),
        FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
        512,
        640,
    )
    .unwrap_err();
    assert_eq!(err.as_str(), "forbidden_input");
}

#[test]
fn thresholding_maps_noul_to_present_absent_or_unknown() {
    let policy = DecisionThresholdPolicy::pilot();
    let present = JevStructuredOutput::new(Some(0.91), None, None, None, None, None, None).unwrap();
    assert_eq!(
        a2a_lab_dev_kit::interpret_structured_outcome(&present, policy).unwrap(),
        DecisionOutcome::Present
    );
    let absent = JevStructuredOutput::new(Some(0.05), None, None, None, None, None, None).unwrap();
    assert_eq!(
        a2a_lab_dev_kit::interpret_structured_outcome(&absent, policy).unwrap(),
        DecisionOutcome::Absent
    );
    let unknown = JevStructuredOutput::new(Some(0.5), None, None, None, None, None, None).unwrap();
    assert_eq!(
        a2a_lab_dev_kit::interpret_structured_outcome(&unknown, policy).unwrap(),
        DecisionOutcome::Unknown
    );
}

#[test]
fn failed_coverage_stays_distinct_from_absent_outcome() {
    let unsupported = a2a_lab_dev_kit::unsupported_predicate_result(
        &evaluation_request(b"u"),
        &LabPredicate::foreign_object_present_v1(),
    );
    assert_eq!(
        unsupported.coverage(),
        DecisionEvaluationCoverage::Unsupported
    );
    assert!(unsupported.outcome().is_none());
}

#[tokio::test]
async fn jev_fake_covers_abstention_partial_support_and_invalid_provenance() {
    let config = evaluator_config();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let abstain = FakeFrameDecisionEvaluator::new(config.clone(), limits, FakeJevBehavior::Abstain);
    let requests = vec![evaluation_request(b"abstain")];
    let responses = abstain.evaluate_frames(requests.clone()).await.unwrap();
    let validated = validate_evaluation_batch(&requests, &responses).unwrap();
    assert_eq!(validated[0].outcome(), Some(DecisionOutcome::Unknown));

    let partial =
        FakeFrameDecisionEvaluator::new(config.clone(), limits, FakeJevBehavior::OkPresent)
            .with_partial_predicates(vec![LabPredicate::pipette_tip_attached_v1()]);
    let foreign = {
        let saved = frame(b"foreign");
        let subject = DecisionSubject::ForeignObject(
            a2a_lab_dev_kit::ForeignObjectSubject::new(
                "slot-1",
                FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
                None,
                None,
            )
            .unwrap(),
        );
        let evidence = vec![
            DecisionEvidenceItem::saved_frame_revision(
                saved.frame_id().clone(),
                FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
            )
            .unwrap(),
        ];
        FrameDecisionEvaluationRequest::new(
            limits,
            JevEvaluationId::new("jev-req-foreign").unwrap(),
            saved,
            LabPredicate::foreign_object_present_v1(),
            subject,
            evidence,
            criteria(),
            FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap(),
            512,
            640,
        )
        .unwrap()
    };
    let batch = vec![evaluation_request(b"tip"), foreign];
    let responses = partial.evaluate_frames(batch.clone()).await.unwrap();
    let validated = validate_evaluation_batch(&batch, &responses).unwrap();
    assert_eq!(
        validated[0].coverage(),
        DecisionEvaluationCoverage::Evaluated
    );
    assert_eq!(
        validated[1].coverage(),
        DecisionEvaluationCoverage::Unsupported
    );

    let invalid =
        FakeFrameDecisionEvaluator::new(config, limits, FakeJevBehavior::InvalidProvenance);
    let err = invalid
        .evaluate_frames(vec![evaluation_request(b"bad-prov")])
        .await
        .unwrap_err();
    assert_eq!(err.as_str(), "invalid_provenance");
}

#[tokio::test]
async fn router_uses_jev_only_without_llm_fallback_on_outage() {
    let config = evaluator_config();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let evaluator =
        FakeFrameDecisionEvaluator::new(config, limits, FakeJevBehavior::ProviderOutage);
    let mut router = FrameDecisionEvaluationRouter::new(evaluator);
    let err = router
        .evaluate_frames(vec![evaluation_request(b"outage")])
        .await
        .unwrap_err();
    assert_eq!(err.as_str(), "provider_unavailable");
    assert_eq!(router.audit().jev_calls(), 1);

    let routing_source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/videos/frame_decision/routing.rs"
    ));
    let provider_source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/videos/frame_decision/provider.rs"
    ));
    for token in [
        "arbitration",
        "FrameDescriptionProvider",
        "describe_frames",
        "FakeFrameDescriptionProvider",
        "TextEmbeddingProvider",
    ] {
        assert!(!routing_source.contains(token), "routing leaked {token}");
        assert!(
            !provider_source.contains(token),
            "provider contract leaked {token}"
        );
    }
}

#[tokio::test]
async fn evaluated_response_materializes_appendable_record() {
    let config = evaluator_config();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let evaluator =
        FakeFrameDecisionEvaluator::new(config.clone(), limits, FakeJevBehavior::OkPresent);
    let request = evaluation_request(b"record");
    let response = evaluator
        .evaluate_frames(vec![request.clone()])
        .await
        .unwrap()
        .remove(0);
    let record = response
        .into_record(
            &request,
            &config,
            request.target().captured_at(),
            UtcTimestamp::parse("2024-06-01T12:00:02Z").unwrap(),
            request.model_input_transform(),
        )
        .unwrap();
    let mut ledger = FrameDecisionLedger::new();
    ledger.append(record).unwrap();
    assert_eq!(ledger.history().len(), 1);
}

#[test]
fn choice_distribution_thresholding_is_deterministic() {
    let policy = DecisionThresholdPolicy::pilot();
    let distribution = LabProbabilityVector::new(
        vec!["present".to_owned(), "absent".to_owned()],
        vec![0.92, 0.05],
    )
    .unwrap();
    let structured =
        JevStructuredOutput::new(None, Some(distribution), None, None, None, None, None).unwrap();
    assert_eq!(
        a2a_lab_dev_kit::interpret_structured_outcome(&structured, policy).unwrap(),
        DecisionOutcome::Present
    );
}
