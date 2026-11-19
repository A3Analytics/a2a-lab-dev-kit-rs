//! Typed lab-predicate evaluations adapted onto the generic JEV client.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use a2a_lab_dev_kit::{
    DecisionEvaluationCoverage, DecisionEvidenceItem, DecisionOutcome, DecisionSubject,
    DecisionThresholdPolicy, DvrStoreNamespace, DvrTrackId, FakeFrameDecisionEvaluator,
    FakeJevBehavior, ForbiddenDecisionInput, ForeignObjectSubject, FrameDecisionEvaluationFailure,
    FrameDecisionEvaluationRequest, FrameDecisionEvaluationRouter, FrameDecisionEvaluator,
    FrameDecisionEvaluatorLimits, FrameDecisionEvaluatorProviderConfig, FrameDecisionLedger,
    FrameDecisionRecord, FrameRoiTransform, IMAGE_DECISION_DOWNSTREAM_DEPENDENCY,
    IMAGE_DECISION_EXCLUDED_SCOPE, IMAGE_DECISION_TRANSPORTS, Image, ImageDecisionHttpClient,
    ImageDecisionModelAnswer, ImageDecisionModelInput, ImageDecisionRequest, ImageDescriptor,
    ImageId, ImageSourceId, JEV_PRODUCER_ID, JevEvaluationId, JsonObject,
    KeyedFrameDecisionEvaluation, LabPredicate, OperatorDecisionCriteria, OperatorReferenceImage,
    PREDICATE_FOREIGN_OBJECT_PRESENT, PREDICATE_PIPETTE_TIP_ATTACHED, PREDICATE_PLATE_MISALIGNED,
    PipetteChannelSubject, PlateAlignmentReference, PlateSlotSubject,
    RejectedDecisionEvaluatorBackend, SavedFrameRef, Sha256Digest, TypedLabPredicateJevAdapter,
    UtcTimestamp, VideoSourceId, bind_local, execute_generic_image_decision,
    interpret_structured_outcome, reject_decision_evaluator_backend,
    reject_forbidden_decision_input,
};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::Response;
use axum::routing::post;
use serde_json::{Value, json};

#[test]
fn pilot_adapter_keeps_exactly_three_v1_predicates() {
    let config = evaluator_config();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let adapter = TypedLabPredicateJevAdapter::new(
        config.clone(),
        limits,
        idle_client(limits),
        evaluated_at(),
    )
    .unwrap();
    let capabilities = adapter.capabilities();
    let predicates = capabilities.supported_predicates();
    assert_eq!(predicates.len(), 3);
    assert_eq!(predicates[0].id(), PREDICATE_PIPETTE_TIP_ATTACHED);
    assert_eq!(predicates[1].id(), PREDICATE_FOREIGN_OBJECT_PRESENT);
    assert_eq!(predicates[2].id(), PREDICATE_PLATE_MISALIGNED);
    assert!(predicates.iter().all(|predicate| predicate.version() == 1));

    let other = LabPredicate::new("lab.other", 1).unwrap();
    let rejected = FrameDecisionEvaluatorProviderConfig::new(
        "jev-1.14.0",
        Sha256Digest::hash_bytes(b"config-v1"),
        Sha256Digest::hash_bytes(b"threshold-v1"),
        vec![other],
    )
    .unwrap();
    let error =
        TypedLabPredicateJevAdapter::new(rejected, limits, idle_client(limits), evaluated_at())
            .unwrap_err();
    assert_eq!(error.code(), "invalid");

    let fake = FakeFrameDecisionEvaluator::new(config, limits, FakeJevBehavior::OkPresent);
    assert_eq!(fake.capabilities().supported_predicates().len(), 3);
    let http = TypedLabPredicateJevAdapter::http_limits(limits).unwrap();
    assert_eq!(http.deadline(), limits.deadline());
    assert_eq!(http.max_request_bytes(), limits.max_request_bytes());
    assert_eq!(http.max_response_bytes(), limits.max_jev_response_bytes());
    assert_eq!(
        http.image_transport().max_image_bytes(),
        u64::try_from(limits.max_image_bytes()).unwrap()
    );
}

#[test]
fn subject_evidence_and_forbidden_inputs_still_fail_before_the_client() {
    let saved = frame(b"forbidden");
    let subject =
        DecisionSubject::PipetteChannel(PipetteChannelSubject::new("p", "c", None).unwrap());
    let evidence = vec![
        DecisionEvidenceItem::reference_image(
            saved.frame_id().clone(),
            FrameRoiTransform::full_frame(),
            Sha256Digest::hash_bytes(b"description-derived"),
        )
        .unwrap(),
    ];
    let forbidden = FrameDecisionEvaluationRequest::new(
        FrameDecisionEvaluatorLimits::pilot(),
        JevEvaluationId::new("jev-req-forbidden").unwrap(),
        saved,
        LabPredicate::pipette_tip_attached_v1(),
        subject,
        evidence,
        criteria(),
        FrameRoiTransform::full_frame(),
        512,
        640,
    )
    .unwrap_err();
    assert_eq!(forbidden.as_str(), "forbidden_input");
    assert_eq!(
        reject_forbidden_decision_input(ForbiddenDecisionInput::ChatOutput)
            .unwrap_err()
            .as_str(),
        "forbidden_input"
    );

    let mismatch = FrameDecisionEvaluationRequest::new(
        FrameDecisionEvaluatorLimits::pilot(),
        JevEvaluationId::new("jev-req-mismatch").unwrap(),
        frame(b"mismatch"),
        LabPredicate::plate_misaligned_v1(),
        pipette_subject(),
        evidence_for(&frame(b"mismatch")),
        criteria(),
        FrameRoiTransform::full_frame(),
        512,
        640,
    )
    .unwrap_err();
    assert_eq!(mismatch.as_str(), "invalid_evidence");

    let mut too_many = evidence_for(&frame(b"limits"));
    for _ in 0..6 {
        too_many.extend(evidence_for(&frame(b"limits")));
    }
    let limited = FrameDecisionEvaluationRequest::new(
        FrameDecisionEvaluatorLimits::pilot(),
        JevEvaluationId::new("jev-req-limits").unwrap(),
        frame(b"limits"),
        LabPredicate::pipette_tip_attached_v1(),
        pipette_subject(),
        too_many,
        criteria(),
        FrameRoiTransform::full_frame(),
        512,
        640,
    )
    .unwrap_err();
    assert_eq!(limited.as_str(), "limit_exceeded");
}

#[test]
fn downstream_boundary_excludes_fallback_and_path_dependencies() {
    assert_eq!(
        IMAGE_DECISION_DOWNSTREAM_DEPENDENCY,
        "published-git-revision"
    );
    assert_ne!(IMAGE_DECISION_DOWNSTREAM_DEPENDENCY, "path");
    assert_eq!(IMAGE_DECISION_TRANSPORTS, ["http", "mcp", "a2a"].as_slice());
    for excluded in [
        "settings-pages",
        "whole-video-indexing",
        "whole-video-backfill",
        "agentcore",
        "strands",
        "aws-sdk",
        "profile-1.3.0",
    ] {
        assert!(IMAGE_DECISION_EXCLUDED_SCOPE.contains(&excluded));
    }
    assert!(
        reject_decision_evaluator_backend(RejectedDecisionEvaluatorBackend::GenericLlm).is_err()
    );
    assert!(
        reject_decision_evaluator_backend(RejectedDecisionEvaluatorBackend::OpenaiLlm).is_err()
    );
    assert_eq!(a2a_lab_dev_kit::version(), "0.1.0");
    assert_ne!(
        std::any::TypeId::of::<KeyedFrameDecisionEvaluation>(),
        std::any::TypeId::of::<a2a_lab_dev_kit::ImageDecisionResult>()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn three_v1_predicates_map_generic_answers_without_a_ledger_append() {
    let (url, seen) = spawn(Reply::ByPredicate).await;
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let saved = frame(b"typed-batch");
    let image = image_bytes(vec![0xff, 0x00, 0x10, b'a'], 640, 480);
    let requests = vec![
        typed_request(
            saved.clone(),
            LabPredicate::pipette_tip_attached_v1(),
            pipette_subject(),
            FrameRoiTransform::full_frame(),
            limits,
            "jev-req-batch",
        ),
        typed_request(
            saved.clone(),
            LabPredicate::foreign_object_present_v1(),
            foreign_subject(),
            FrameRoiTransform::full_frame(),
            limits,
            "jev-req-batch",
        ),
        typed_request(
            saved.clone(),
            LabPredicate::plate_misaligned_v1(),
            plate_subject(),
            FrameRoiTransform::full_frame(),
            limits,
            "jev-req-batch",
        ),
    ];
    let mut frames = HashMap::new();
    frames.insert(saved.frame_id().clone(), image);
    let adapter = TypedLabPredicateJevAdapter::new(
        evaluator_config(),
        limits,
        client(&url, limits),
        evaluated_at(),
    )
    .unwrap()
    .with_frames(frames);
    let mut router = FrameDecisionEvaluationRouter::new(adapter);
    let ledger = FrameDecisionLedger::new();
    let evaluations = router.evaluate_frames(requests.clone()).await.unwrap();
    assert_eq!(router.audit().jev_calls(), 1);
    assert!(ledger.history().is_empty());
    assert_outcome(&evaluations[0], DecisionOutcome::Present);
    assert_outcome(&evaluations[1], DecisionOutcome::Absent);
    assert_outcome(&evaluations[2], DecisionOutcome::Unknown);
    assert_eq!(
        evaluations[0]
            .structured_output()
            .unwrap()
            .noul_probability(),
        Some(1.0)
    );
    assert_eq!(
        evaluations[1]
            .structured_output()
            .unwrap()
            .noul_probability(),
        Some(0.0)
    );
    assert_eq!(
        evaluations[2]
            .structured_output()
            .unwrap()
            .noul_probability(),
        Some(0.5)
    );
    assert!(evaluations.iter().all(|evaluation| {
        evaluation
            .structured_output()
            .unwrap()
            .vendor_confidence()
            .is_none()
    }));
    let sent = seen.0.lock().expect("seen").clone();
    assert_eq!(sent.len(), 3);
    for body in sent {
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert!(value["model_input"].get("roi").is_none());
        assert_eq!(value["model_input"]["model"], "jev-1.14.0");
        assert!(value["question"].as_str().unwrap().contains(" v1 "));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_success_preserves_provenance_and_idempotent_append() {
    let answers = json!({"answers":[{"text":"scored","probability":1.0,"model_box":{"bounds":[0.25,0.25,0.75,0.75]}}]});
    let (url, _) = spawn(Reply::fixed(StatusCode::OK, &answers)).await;
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let config = evaluator_config();
    let saved = frame(b"ledger");
    let image = image_bytes(vec![9, 8, 7, 6], 640, 480);
    let request = typed_request(
        saved,
        LabPredicate::pipette_tip_attached_v1(),
        pipette_subject(),
        FrameRoiTransform::full_frame(),
        limits,
        "jev-req-ledger",
    );
    let adapter = TypedLabPredicateJevAdapter::new(
        config.clone(),
        limits,
        client(&url, limits),
        evaluated_at(),
    )
    .unwrap();
    let mut ledger = FrameDecisionLedger::new();
    let first = adapter.evaluate_frame(&request, &image).await.unwrap();
    assert!(ledger.history().is_empty());
    let generic = ImageDecisionRequest::new(
        image.clone(),
        "Is anything visible?",
        ImageDecisionModelInput::new("jev-1.14.0", None).unwrap(),
    )
    .unwrap();
    let generic_client = ImageDecisionHttpClient::new(
        &url,
        TypedLabPredicateJevAdapter::http_limits(limits).unwrap(),
    )
    .unwrap();
    let generic_result = generic_client.decide(&generic).await.unwrap();
    assert!(generic_result.answers()[0].source_box().is_some());
    assert_eq!(
        generic.class(),
        a2a_lab_dev_kit::ImageDecisionClass::Generic
    );
    execute_generic_image_decision(
        &generic,
        &[ImageDecisionModelAnswer::new(Some("yes"), Some(1.0), None).unwrap()],
    )
    .unwrap();
    assert!(ledger.history().is_empty());
    assert_ne!(
        std::any::TypeId::of::<a2a_lab_dev_kit::ImageDecisionResult>(),
        std::any::TypeId::of::<FrameDecisionRecord>()
    );

    let record = materialize(&first, &request, &config, &adapter);
    let first_id = ledger.append(record).unwrap().decision_id().clone();
    assert_eq!(ledger.history().len(), 1);
    assert_appended_record(
        ledger.get(&first_id).unwrap(),
        &request,
        &config,
        &adapter,
        &first,
    );

    let second = adapter.evaluate_frame(&request, &image).await.unwrap();
    let replay = materialize(&second, &request, &config, &adapter);
    let replay_id = ledger.append(replay).unwrap().decision_id().clone();
    assert_eq!(replay_id, first_id);
    assert_eq!(ledger.history().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_and_cropped_images_keep_source_normalized_boxes() {
    let answers = json!({"answers":[{"text":"present","probability":1.0,"model_box":{"bounds":[0.0,0.0,0.5,1.0]}}]});
    let (url, seen) = spawn(Reply::fixed(StatusCode::OK, &answers)).await;
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let adapter = TypedLabPredicateJevAdapter::new(
        evaluator_config(),
        limits,
        client(&url, limits),
        evaluated_at(),
    )
    .unwrap();
    let image = image_bytes(vec![1, 2, 3, 4], 640, 480);
    let full = typed_request(
        frame(b"full"),
        LabPredicate::pipette_tip_attached_v1(),
        pipette_subject(),
        FrameRoiTransform::full_frame(),
        limits,
        "jev-req-full",
    );
    let full_result = adapter.evaluate_frame(&full, &image).await.unwrap();
    assert_eq!(
        bits(
            full_result
                .structured_output()
                .unwrap()
                .evidence_box()
                .unwrap()
                .bounds()
        ),
        bits([0.0, 0.0, 0.5, 1.0])
    );

    let crop = FrameRoiTransform::new([0.25, 0.25, 0.75, 0.75]).unwrap();
    let cropped = typed_request(
        frame(b"crop"),
        LabPredicate::pipette_tip_attached_v1(),
        pipette_subject(),
        crop,
        limits,
        "jev-req-crop",
    );
    let cropped_result = adapter.evaluate_frame(&cropped, &image).await.unwrap();
    assert_eq!(
        bits(
            cropped_result
                .structured_output()
                .unwrap()
                .evidence_box()
                .unwrap()
                .bounds()
        ),
        bits([0.25, 0.25, 0.5, 0.75])
    );

    let sent = seen.0.lock().expect("seen").clone();
    let full_json: Value = serde_json::from_slice(&sent[0]).unwrap();
    let crop_json: Value = serde_json::from_slice(&sent[1]).unwrap();
    assert!(full_json["model_input"].get("roi").is_none());
    assert_eq!(
        crop_json["model_input"]["roi"]["bounds"],
        json!([0.25, 0.25, 0.75, 0.75])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cropped_edge_maps_a_text_label_onto_the_source_image() {
    let answers = json!({"answers":[{"text":"present","model_box":{"bounds":[0.0,0.0,1.0,1.0]}}]});
    let (url, _) = spawn(Reply::fixed(StatusCode::OK, &answers)).await;
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let image = image_bytes(vec![1, 2, 3, 4], 640, 480);
    let request = typed_request(
        frame(b"edge"),
        LabPredicate::foreign_object_present_v1(),
        foreign_subject(),
        FrameRoiTransform::new([0.5, 0.5, 1.0, 1.0]).unwrap(),
        limits,
        "jev-req-edge",
    );
    let adapter = TypedLabPredicateJevAdapter::new(
        evaluator_config(),
        limits,
        client(&url, limits),
        evaluated_at(),
    )
    .unwrap();
    let result = adapter.evaluate_frame(&request, &image).await.unwrap();
    assert_eq!(result.outcome(), Some(DecisionOutcome::Present));
    let structured = result.structured_output().unwrap();
    assert!(structured.noul_probability().is_none());
    assert_eq!(
        bits(structured.evidence_box().unwrap().bounds()),
        bits([0.5, 0.5, 1.0, 1.0])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_failures_and_typed_limits_stay_on_the_evaluator() {
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let request = typed_request(
        frame(b"failure"),
        LabPredicate::pipette_tip_attached_v1(),
        pipette_subject(),
        FrameRoiTransform::full_frame(),
        limits,
        "jev-req-failure",
    );
    let image = image_bytes(vec![1, 2, 3, 4], 640, 480);

    let (timeout_url, _) = spawn(Reply::delayed(
        StatusCode::OK,
        &json!({"answers":[{"text":"present"}]}),
        Duration::from_secs(2),
    ))
    .await;
    let timeout_limits = custom_limits(8, 280, 1568, 1024, 1024 * 1024, Duration::from_millis(200));
    let timeout_request = typed_request(
        frame(b"timeout"),
        LabPredicate::pipette_tip_attached_v1(),
        pipette_subject(),
        FrameRoiTransform::full_frame(),
        timeout_limits,
        "jev-req-timeout",
    );
    let timeout_adapter = TypedLabPredicateJevAdapter::new(
        evaluator_config(),
        timeout_limits,
        client(&timeout_url, timeout_limits),
        evaluated_at(),
    )
    .unwrap();
    let timed_out = timeout_adapter
        .evaluate_frame(&timeout_request, &image)
        .await
        .unwrap_err();
    assert_eq!(timed_out.as_str(), "deadline_exceeded");

    let (down_url, _) = spawn(Reply::status(StatusCode::SERVICE_UNAVAILABLE, "down")).await;
    let down = TypedLabPredicateJevAdapter::new(
        evaluator_config(),
        limits,
        client(&down_url, limits),
        evaluated_at(),
    )
    .unwrap();
    let unavailable = down.evaluate_frame(&request, &image).await.unwrap_err();
    assert!(matches!(
        unavailable,
        FrameDecisionEvaluationFailure::ProviderUnavailable { ref message }
            if message == "http status 503"
    ));

    let (bad_url, _) = spawn(Reply::raw(StatusCode::OK, b"{\"answers\":")).await;
    let bad = TypedLabPredicateJevAdapter::new(
        evaluator_config(),
        limits,
        client(&bad_url, limits),
        evaluated_at(),
    )
    .unwrap();
    let invalid = bad.evaluate_frame(&request, &image).await.unwrap_err();
    assert_eq!(invalid.as_str(), "invalid_provenance");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_limits_stop_before_the_generic_client() {
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let image = image_bytes(vec![1, 2, 3, 4], 640, 480);
    let byte_limits = custom_limits(8, 1, 4000, 2, 1024 * 1024, Duration::from_secs(2));
    let byte_request = typed_request(
        frame(b"bytes"),
        LabPredicate::pipette_tip_attached_v1(),
        pipette_subject(),
        FrameRoiTransform::full_frame(),
        byte_limits,
        "jev-req-bytes",
    );
    let byte_adapter = TypedLabPredicateJevAdapter::new(
        evaluator_config(),
        byte_limits,
        idle_client(byte_limits),
        evaluated_at(),
    )
    .unwrap();
    let bytes = byte_adapter
        .evaluate_frame(&byte_request, &image)
        .await
        .unwrap_err();
    assert!(matches!(
        bytes,
        FrameDecisionEvaluationFailure::LimitExceeded { ref limit, .. } if limit == "max_image_bytes"
    ));

    let edge_image = image_bytes(vec![1, 2, 3, 4], 2000, 480);
    let request = typed_request(
        frame(b"edge-limit"),
        LabPredicate::pipette_tip_attached_v1(),
        pipette_subject(),
        FrameRoiTransform::full_frame(),
        limits,
        "jev-req-edge-limit",
    );
    let edge = TypedLabPredicateJevAdapter::new(
        evaluator_config(),
        limits,
        idle_client(limits),
        evaluated_at(),
    )
    .unwrap()
    .evaluate_frame(&request, &edge_image)
    .await
    .unwrap_err();
    assert!(matches!(
        edge,
        FrameDecisionEvaluationFailure::LimitExceeded { ref limit, .. } if limit == "edge_pixels"
    ));

    let request_limits = custom_limits(8, 1, 4000, 1024 * 1024, 64, Duration::from_secs(2));
    let saved = frame(b"request-bytes");
    let small_request = FrameDecisionEvaluationRequest::new(
        request_limits,
        JevEvaluationId::new("jev-req-small").unwrap(),
        saved.clone(),
        LabPredicate::pipette_tip_attached_v1(),
        pipette_subject(),
        evidence_for(&saved),
        criteria(),
        FrameRoiTransform::full_frame(),
        32,
        640,
    )
    .unwrap();
    let request_error = TypedLabPredicateJevAdapter::new(
        evaluator_config(),
        request_limits,
        idle_client(request_limits),
        evaluated_at(),
    )
    .unwrap()
    .evaluate_frame(&small_request, &image)
    .await
    .unwrap_err();
    assert!(matches!(
        request_error,
        FrameDecisionEvaluationFailure::LimitExceeded { ref limit, .. } if limit == "max_request_bytes"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_predicate_does_not_call_the_generic_client() {
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let only_tip = FrameDecisionEvaluatorProviderConfig::new(
        "jev-1.14.0",
        Sha256Digest::hash_bytes(b"config-v1"),
        Sha256Digest::hash_bytes(b"threshold-v1"),
        vec![LabPredicate::pipette_tip_attached_v1()],
    )
    .unwrap();
    let foreign = typed_request(
        frame(b"foreign-unsupported"),
        LabPredicate::foreign_object_present_v1(),
        foreign_subject(),
        FrameRoiTransform::full_frame(),
        limits,
        "jev-req-unsupported",
    );
    let partial =
        TypedLabPredicateJevAdapter::new(only_tip, limits, idle_client(limits), evaluated_at())
            .unwrap();
    let mut router = FrameDecisionEvaluationRouter::new(partial);
    let unsupported = router.evaluate_frames(vec![foreign]).await.unwrap_err();
    assert_eq!(unsupported.as_str(), "unsupported_predicate");
    assert_eq!(router.audit().jev_calls(), 0);

    let empty = TypedLabPredicateJevAdapter::new(
        evaluator_config(),
        limits,
        idle_client(limits),
        evaluated_at(),
    )
    .unwrap();
    assert!(empty.evaluate_frames(Vec::new()).await.unwrap().is_empty());
    let request = typed_request(
        frame(b"unbound"),
        LabPredicate::pipette_tip_attached_v1(),
        pipette_subject(),
        FrameRoiTransform::full_frame(),
        limits,
        "jev-req-unbound",
    );
    let missing = empty.evaluate_frames(vec![request]).await.unwrap_err();
    assert_eq!(missing.as_str(), "invalid_evidence");
}

fn assert_appended_record(
    stored: &FrameDecisionRecord,
    request: &FrameDecisionEvaluationRequest,
    config: &FrameDecisionEvaluatorProviderConfig,
    adapter: &TypedLabPredicateJevAdapter,
    evaluation: &KeyedFrameDecisionEvaluation,
) {
    assert_eq!(stored.predicate(), request.predicate());
    assert_eq!(stored.subject(), request.subject());
    assert_eq!(stored.evidence(), request.evidence());
    assert_eq!(stored.outcome(), DecisionOutcome::Present);
    assert_eq!(stored.idempotency_key(), request.idempotency_key());
    assert!(
        stored
            .idempotency_key()
            .as_str()
            .starts_with(request.evaluation_id().as_str())
    );
    assert_provenance(stored, request, config, adapter, evaluation);
}

fn assert_provenance(
    stored: &FrameDecisionRecord,
    request: &FrameDecisionEvaluationRequest,
    config: &FrameDecisionEvaluatorProviderConfig,
    adapter: &TypedLabPredicateJevAdapter,
    evaluation: &KeyedFrameDecisionEvaluation,
) {
    let provenance = stored.provenance();
    assert_eq!(provenance.evaluation_id(), request.evaluation_id());
    assert_eq!(provenance.captured_at(), request.target().captured_at());
    assert_eq!(provenance.evaluated_at(), adapter.evaluated_at());
    assert_eq!(provenance.producer().producer_id(), JEV_PRODUCER_ID);
    assert_eq!(provenance.config_digest(), config.config_digest());
    assert_eq!(
        provenance.prompt_digest(),
        request.criteria().prompt_digest()
    );
    assert_eq!(
        provenance.criteria_digest(),
        request.criteria().criteria_digest()
    );
    assert_eq!(
        provenance.threshold_policy_digest(),
        config.threshold_policy_digest()
    );
    assert_eq!(provenance.coverage(), DecisionEvaluationCoverage::Evaluated);
    assert_eq!(
        bits(provenance.model_input_transform().bounds()),
        bits(request.model_input_transform().bounds())
    );
    assert_eq!(
        provenance.jev_response_digest(),
        evaluation.jev_response_digest().unwrap()
    );
}

fn assert_outcome(evaluation: &KeyedFrameDecisionEvaluation, outcome: DecisionOutcome) {
    assert_eq!(evaluation.coverage(), DecisionEvaluationCoverage::Evaluated);
    assert_eq!(evaluation.outcome(), Some(outcome));
    let structured = evaluation.structured_output().unwrap();
    assert_eq!(
        interpret_structured_outcome(structured, DecisionThresholdPolicy::pilot()).unwrap(),
        outcome
    );
    assert!(evaluation.provenance().is_some());
    assert!(evaluation.jev_response_digest().is_some());
}

fn materialize(
    evaluation: &KeyedFrameDecisionEvaluation,
    request: &FrameDecisionEvaluationRequest,
    config: &FrameDecisionEvaluatorProviderConfig,
    adapter: &TypedLabPredicateJevAdapter,
) -> FrameDecisionRecord {
    evaluation
        .clone()
        .into_record(
            request,
            config,
            request.target().captured_at(),
            adapter.evaluated_at(),
            request.model_input_transform(),
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

fn evaluated_at() -> UtcTimestamp {
    UtcTimestamp::parse("2024-06-01T12:00:02Z").unwrap()
}

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

fn criteria() -> OperatorDecisionCriteria {
    OperatorDecisionCriteria::new(
        Sha256Digest::hash_bytes(b"criteria-v1"),
        Sha256Digest::hash_bytes(b"prompt-v1"),
        vec![
            OperatorReferenceImage::new(
                Sha256Digest::hash_bytes(b"operator-ref-still"),
                "expected-tip",
                FrameRoiTransform::full_frame(),
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

fn pipette_subject() -> DecisionSubject {
    DecisionSubject::PipetteChannel(PipetteChannelSubject::new("left", "channel-1", None).unwrap())
}

fn foreign_subject() -> DecisionSubject {
    DecisionSubject::ForeignObject(
        ForeignObjectSubject::new("deck", FrameRoiTransform::full_frame(), None, None).unwrap(),
    )
}

fn plate_subject() -> DecisionSubject {
    DecisionSubject::PlateSlot(
        PlateSlotSubject::new(
            "plate-1",
            "slot-2",
            PlateAlignmentReference::ExpectedPoseDigest(Sha256Digest::hash_bytes(b"pose")),
            None,
        )
        .unwrap(),
    )
}

fn evidence_for(saved: &SavedFrameRef) -> Vec<DecisionEvidenceItem> {
    vec![
        DecisionEvidenceItem::saved_frame_revision(
            saved.frame_id().clone(),
            FrameRoiTransform::full_frame(),
        )
        .unwrap(),
    ]
}

fn typed_request(
    saved: SavedFrameRef,
    predicate: LabPredicate,
    subject: DecisionSubject,
    transform: FrameRoiTransform,
    limits: FrameDecisionEvaluatorLimits,
    evaluation_id: &str,
) -> FrameDecisionEvaluationRequest {
    let evidence = evidence_for(&saved);
    FrameDecisionEvaluationRequest::new(
        limits,
        JevEvaluationId::new(evaluation_id).unwrap(),
        saved,
        predicate,
        subject,
        evidence,
        criteria(),
        transform,
        512,
        640,
    )
    .unwrap()
}

fn image_bytes(bytes: Vec<u8>, width: u32, height: u32) -> Image {
    Image::new(
        ImageDescriptor::new(
            ImageId::new("frame").unwrap(),
            ImageSourceId::new("cam").unwrap(),
            UtcTimestamp::parse("2024-06-01T12:00:00Z").unwrap(),
            "image/png",
            width,
            height,
            None,
            JsonObject::empty(),
        )
        .unwrap(),
        bytes,
    )
    .unwrap()
}

fn custom_limits(
    max_batch: u32,
    min_edge: u32,
    max_edge: u32,
    max_image_bytes: usize,
    max_request_bytes: usize,
    deadline: Duration,
) -> FrameDecisionEvaluatorLimits {
    FrameDecisionEvaluatorLimits::new(
        max_batch,
        6,
        4,
        4,
        min_edge,
        max_edge,
        max_image_bytes,
        max_request_bytes,
        256 * 1024,
        deadline,
    )
    .unwrap()
}

fn client(url: &str, limits: FrameDecisionEvaluatorLimits) -> ImageDecisionHttpClient {
    ImageDecisionHttpClient::new(
        url,
        TypedLabPredicateJevAdapter::http_limits(limits).unwrap(),
    )
    .unwrap()
}

fn idle_client(limits: FrameDecisionEvaluatorLimits) -> ImageDecisionHttpClient {
    client("http://127.0.0.1:9/decide", limits)
}

fn bits(bounds: [f32; 4]) -> [u32; 4] {
    bounds.map(f32::to_bits)
}

#[derive(Clone)]
enum Reply {
    Fixed {
        status: StatusCode,
        body: Vec<u8>,
        delay: Duration,
    },
    ByPredicate,
}

impl Reply {
    fn fixed(status: StatusCode, body: &Value) -> Self {
        Self::Fixed {
            status,
            body: serde_json::to_vec(body).unwrap(),
            delay: Duration::ZERO,
        }
    }

    fn status(status: StatusCode, body: &str) -> Self {
        Self::Fixed {
            status,
            body: body.as_bytes().to_vec(),
            delay: Duration::ZERO,
        }
    }

    fn raw(status: StatusCode, body: &[u8]) -> Self {
        Self::Fixed {
            status,
            body: body.to_vec(),
            delay: Duration::ZERO,
        }
    }

    fn delayed(status: StatusCode, body: &Value, delay: Duration) -> Self {
        Self::Fixed {
            status,
            body: serde_json::to_vec(body).unwrap(),
            delay,
        }
    }
}

struct Seen(Mutex<Vec<Vec<u8>>>);

#[derive(Clone)]
struct AppState {
    reply: Reply,
    seen: Arc<Seen>,
}

async fn spawn(reply: Reply) -> (String, Arc<Seen>) {
    let seen = Arc::new(Seen(Mutex::new(Vec::new())));
    let (listener, address) = bind_local().await.unwrap();
    let state = AppState {
        reply,
        seen: Arc::clone(&seen),
    };
    tokio::spawn(async move {
        let app = Router::new().route("/", post(received)).with_state(state);
        let _ = axum::serve(listener, app).await;
    });
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(address).await.is_ok() {
            return (format!("http://{address}/"), seen);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("decision server did not accept connections");
}

async fn received(State(state): State<AppState>, body: Bytes) -> Response {
    state.seen.0.lock().expect("seen").push(body.to_vec());
    match &state.reply {
        Reply::ByPredicate => {
            let value: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
            let question = value["question"].as_str().unwrap_or("");
            let answers = if question.contains(PREDICATE_PIPETTE_TIP_ATTACHED) {
                json!({"answers":[{"text":"scored","probability":1.0,"model_box":{"bounds":[0.25,0.25,0.75,0.75]}}]})
            } else if question.contains(PREDICATE_FOREIGN_OBJECT_PRESENT) {
                json!({"answers":[{"text":"scored","probability":0.0}]})
            } else {
                json!({"answers":[{"text":"scored","probability":0.5}]})
            };
            json_response(StatusCode::OK, serde_json::to_vec(&answers).unwrap())
        }
        Reply::Fixed {
            status,
            body,
            delay,
        } => {
            if !delay.is_zero() {
                tokio::time::sleep(*delay).await;
            }
            if *status == StatusCode::OK {
                json_response(*status, body.clone())
            } else {
                Response::builder()
                    .status(*status)
                    .header(CONTENT_TYPE, "text/plain")
                    .body(Body::from(body.clone()))
                    .expect("response")
            }
        }
    }
}

fn json_response(status: StatusCode, body: Vec<u8>) -> Response {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("response")
}
