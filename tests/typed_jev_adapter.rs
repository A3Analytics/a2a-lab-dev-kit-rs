//! Typed lab-predicate evaluations adapted onto `TypeSafe` System One questions.

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
    ImageId, ImageSourceId, JEV_MAX_EDGE_PIXELS, JEV_MAX_IMAGE_BYTES, JEV_MIN_EDGE_PIXELS,
    JEV_PRODUCER_ID, JevEvaluationId, JsonObject, KeyedFrameDecisionEvaluation, LabPredicate,
    LoadedLabConfig, OperatorDecisionCriteria, OperatorReferenceImage,
    PREDICATE_FOREIGN_OBJECT_PRESENT, PREDICATE_PIPETTE_TIP_ATTACHED, PREDICATE_PLATE_MISALIGNED,
    PipetteChannelSubject, PlateAlignmentReference, PlateSlotSubject,
    RejectedDecisionEvaluatorBackend, SavedFrameRef, Sha256Digest, TypeSafeSystemOneClient,
    TypedLabPredicateJevAdapter, UtcTimestamp, VideoSourceId, bind_local,
    execute_generic_image_decision, interpret_structured_outcome,
    reject_decision_evaluator_backend, reject_forbidden_decision_input,
};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::response::Response;
use axum::routing::post;
use base64::Engine;
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
    frames.insert(saved.frame_id().clone(), image.clone());
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
    assert_noul_bits(&evaluations[0], 1.0);
    assert_noul_bits(&evaluations[1], 0.0);
    assert_noul_bits(&evaluations[2], 0.5);
    assert!(evaluations.iter().all(|evaluation| {
        evaluation
            .structured_output()
            .unwrap()
            .vendor_confidence()
            .is_none()
    }));
    let sent = seen.0.lock().expect("seen").clone();
    assert_eq!(sent.len(), 3);
    let mut names = Vec::new();
    for captured in sent {
        assert_eq!(
            captured.authorization.as_deref(),
            Some("Bearer typed-adapter-test-key")
        );
        assert_eq!(captured.danger.as_deref(), Some("true"));
        let value: Value = serde_json::from_slice(&captured.body).unwrap();
        assert!(value.get("roi").is_none());
        assert!(value["state"].get("roi").is_none());
        assert_eq!(value["model"], "jev-1.14.0");
        let reference = value["state"]["image"].as_str().unwrap();
        let asset = &value["assets"][reference];
        assert_eq!(asset["format"], "png");
        assert_eq!(
            asset["data_base64"],
            base64::engine::general_purpose::STANDARD.encode(image.data())
        );
        let questions = value["questions"].as_object().unwrap();
        assert_eq!(questions.len(), 1);
        let (name, question) = questions.iter().next().unwrap();
        names.push(name.clone());
        assert_eq!(question["type"], "noul");
        assert_eq!(question["instructions"], question_instructions(name));
    }
    names.sort();
    assert_eq!(
        names,
        vec![
            "foreign_object_present".to_owned(),
            "pipette_tip_attached".to_owned(),
            "plate_misaligned".to_owned(),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_success_preserves_provenance_and_idempotent_append() {
    let answers = noul_body("pipette_tip_attached", 1.0);
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
    let generic_answers = json!({"answers":[{"text":"scored","probability":1.0,"model_box":{"bounds":[0.25,0.25,0.75,0.75]}}]});
    let (generic_url, _) = spawn(Reply::fixed(StatusCode::OK, &generic_answers)).await;
    let generic = ImageDecisionRequest::new(
        image.clone(),
        "Is anything visible?",
        ImageDecisionModelInput::new("jev-1.14.0", None).unwrap(),
    )
    .unwrap();
    let generic_client = ImageDecisionHttpClient::new(
        &generic_url,
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
    let answers = noul_body("pipette_tip_attached", 1.0);
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
    assert_outcome(&full_result, DecisionOutcome::Present);
    assert!(
        full_result
            .structured_output()
            .unwrap()
            .evidence_box()
            .is_none()
    );
    assert_eq!(
        bits(
            full_result
                .provenance()
                .unwrap()
                .model_input_transform()
                .bounds()
        ),
        bits(FrameRoiTransform::full_frame().bounds())
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
    assert_outcome(&cropped_result, DecisionOutcome::Present);
    assert_eq!(
        bits(
            cropped_result
                .provenance()
                .unwrap()
                .model_input_transform()
                .bounds()
        ),
        bits(crop.bounds())
    );

    let sent = seen.0.lock().expect("seen").clone();
    let encoded = base64::engine::general_purpose::STANDARD.encode(image.data());
    for captured in &sent {
        let value: Value = serde_json::from_slice(&captured.body).unwrap();
        assert!(value.get("roi").is_none());
        assert!(value["state"].get("roi").is_none());
        let reference = value["state"]["image"].as_str().unwrap();
        assert_eq!(value["assets"][reference]["data_base64"], encoded);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cropped_edge_maps_a_text_label_onto_the_source_image() {
    let answers = noul_body("foreign_object_present", 1.0);
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
    assert_noul_bits(&result, 1.0);
    assert!(structured.evidence_box().is_none());
    assert_eq!(
        bits(
            result
                .provenance()
                .unwrap()
                .model_input_transform()
                .bounds()
        ),
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
        &noul_body("pipette_tip_attached", 1.0),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loaded_semantic_config_uses_endpoint_model_and_resolved_key() {
    let _env = EnvGuard::clear_typesafe();
    let (url, seen) = spawn(Reply::fixed(
        StatusCode::OK,
        &noul_body("pipette_tip_attached", 1.0),
    ))
    .await;
    let root = temp_config_dir("typed-jev-loaded");
    let config_path = root.join("a2a-lab.json");
    std::fs::write(&config_path, semantic_document(&url, None)).unwrap();
    assert_missing_semantic_key(&config_path);

    std::fs::write(
        root.join("a2a-lab.secret.json"),
        br#"{"typesafe":{"api_key":"loaded-secret-placeholder"}}"#,
    )
    .unwrap();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let image = image_bytes(vec![1, 2, 3, 4], 640, 480);
    let crop = FrameRoiTransform::new([0.25, 0.25, 0.75, 0.75]).unwrap();
    let request = loaded_request(crop, limits);
    let adapter = loaded_adapter(&config_path, limits);
    assert_loaded_result(&adapter, &request, &image, crop).await;
    assert_system_one_call(&seen, 0, "Bearer loaded-secret-placeholder", &image);

    std::fs::remove_file(root.join("a2a-lab.secret.json")).unwrap();
    unsafe {
        std::env::set_var("TYPED_JEV_FALLBACK_KEY", "semantic-fallback-placeholder");
    }
    std::fs::write(
        &config_path,
        semantic_document(&url, Some("TYPED_JEV_FALLBACK_KEY")),
    )
    .unwrap();
    loaded_adapter(&config_path, limits)
        .evaluate_frame(&request, &image)
        .await
        .unwrap();
    assert_system_one_call(&seen, 1, "Bearer semantic-fallback-placeholder", &image);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn semantic_decision_preamble_is_prepended_before_provider_limits() {
    let _env = EnvGuard::clear_typesafe();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let image = image_bytes(vec![1, 2, 3, 4], 640, 480);
    let request = loaded_request(FrameRoiTransform::full_frame(), limits);
    let (url, seen) = spawn(Reply::fixed(
        StatusCode::OK,
        &noul_body("pipette_tip_attached", 1.0),
    ))
    .await;

    let blank_root = write_loaded_config(&url, Some(" \t\n"));
    loaded_adapter(&blank_root.join("a2a-lab.json"), limits)
        .evaluate_frame(&request, &image)
        .await
        .unwrap();
    let blank_body = seen.0.lock().expect("seen").clone();
    let blank: Value = serde_json::from_slice(&blank_body[0].body).unwrap();
    assert_eq!(
        blank["questions"]["pipette_tip_attached"]["instructions"],
        "pipette tip attached"
    );

    let guided_root = write_loaded_config(&url, Some("  Guide"));
    loaded_adapter(&guided_root.join("a2a-lab.json"), limits)
        .evaluate_frame(&request, &image)
        .await
        .unwrap();
    let guided = seen.0.lock().expect("seen").clone();
    let guided: Value = serde_json::from_slice(&guided[1].body).unwrap();
    assert_eq!(
        guided["questions"]["pipette_tip_attached"]["instructions"],
        "  Guide\npipette tip attached"
    );
    assert_eq!(guided["questions"]["pipette_tip_attached"]["type"], "noul");
    let guided_len = guided_body_len(&seen, 1);

    let long_root = write_loaded_config(
        &url,
        Some(&"x".repeat(a2a_lab_dev_kit::MAX_IMAGE_DECISION_QUESTION_CHARS)),
    );
    let error = loaded_adapter(&long_root.join("a2a-lab.json"), limits)
        .evaluate_frame(&request, &image)
        .await
        .unwrap_err();
    assert_eq!(error.as_str(), "limit_exceeded");
    assert!(error.to_string().contains("question"));
    assert_eq!(seen.0.lock().expect("seen").len(), 2);

    let tight = FrameDecisionEvaluatorLimits::new(
        8,
        6,
        4,
        4,
        JEV_MIN_EDGE_PIXELS,
        JEV_MAX_EDGE_PIXELS,
        JEV_MAX_IMAGE_BYTES,
        guided_len - 1,
        256 * 1024,
        Duration::from_secs(45),
    )
    .unwrap();
    let saved = frame(b"loaded-tight");
    let tight_request = FrameDecisionEvaluationRequest::new(
        tight,
        JevEvaluationId::new("jev-req-preamble-tight").unwrap(),
        saved.clone(),
        LabPredicate::pipette_tip_attached_v1(),
        pipette_subject(),
        evidence_for(&saved),
        criteria(),
        FrameRoiTransform::full_frame(),
        1,
        640,
    )
    .unwrap();
    let error = loaded_adapter(&guided_root.join("a2a-lab.json"), tight)
        .evaluate_frame(&tight_request, &image)
        .await
        .unwrap_err();
    assert_eq!(error.as_str(), "limit_exceeded");
    assert!(error.to_string().contains("max_request_bytes"));
    assert_eq!(seen.0.lock().expect("seen").len(), 2);
    let _ = std::fs::remove_dir_all(blank_root);
    let _ = std::fs::remove_dir_all(guided_root);
    let _ = std::fs::remove_dir_all(long_root);
}

fn write_loaded_config(url: &str, preamble: Option<&str>) -> std::path::PathBuf {
    let root = temp_config_dir("typed-jev-preamble");
    std::fs::write(
        root.join("a2a-lab.json"),
        semantic_document_with_preamble(url, None, preamble),
    )
    .unwrap();
    std::fs::write(
        root.join("a2a-lab.secret.json"),
        br#"{"typesafe":{"api_key":"loaded-secret-placeholder"}}"#,
    )
    .unwrap();
    root
}

fn guided_body_len(seen: &Arc<Seen>, index: usize) -> usize {
    seen.0.lock().expect("seen")[index].body.len()
}

fn temp_config_dir(prefix: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn assert_missing_semantic_key(config_path: &std::path::Path) {
    let missing_key = LoadedLabConfig::load(config_path).unwrap();
    let missing = TypedLabPredicateJevAdapter::from_loaded(
        &missing_key,
        evaluator_config(),
        FrameDecisionEvaluatorLimits::pilot(),
        evaluated_at(),
    )
    .unwrap_err();
    assert_eq!(missing.code(), "invalid");
    assert!(!missing.to_string().contains("typed-adapter"));
}

fn loaded_request(
    crop: FrameRoiTransform,
    limits: FrameDecisionEvaluatorLimits,
) -> FrameDecisionEvaluationRequest {
    typed_request(
        frame(b"loaded"),
        LabPredicate::pipette_tip_attached_v1(),
        pipette_subject(),
        crop,
        limits,
        "jev-req-loaded",
    )
}

fn loaded_adapter(
    config_path: &std::path::Path,
    limits: FrameDecisionEvaluatorLimits,
) -> TypedLabPredicateJevAdapter {
    let loaded = LoadedLabConfig::load(config_path).unwrap();
    let adapter = TypedLabPredicateJevAdapter::from_loaded(
        &loaded,
        evaluator_config(),
        limits,
        evaluated_at(),
    )
    .unwrap();
    assert_eq!(adapter.model(), "jev-1.14.0");
    adapter
}

async fn assert_loaded_result(
    adapter: &TypedLabPredicateJevAdapter,
    request: &FrameDecisionEvaluationRequest,
    image: &Image,
    crop: FrameRoiTransform,
) {
    let ledger = FrameDecisionLedger::new();
    let evaluation = adapter.evaluate_frame(request, image).await.unwrap();
    assert!(ledger.history().is_empty());
    assert_outcome(&evaluation, DecisionOutcome::Present);
    assert_eq!(
        bits(
            evaluation
                .provenance()
                .unwrap()
                .model_input_transform()
                .bounds()
        ),
        bits(crop.bounds())
    );
}

fn assert_system_one_call(seen: &Seen, index: usize, authorization: &str, image: &Image) {
    let captured = seen.0.lock().expect("seen").clone();
    assert_eq!(
        captured[index].authorization.as_deref(),
        Some(authorization)
    );
    let body: Value = serde_json::from_slice(&captured[index].body).unwrap();
    assert!(body.get("roi").is_none());
    assert_eq!(body["model"], "jev-1.14.0");
    assert_eq!(
        body["questions"]["pipette_tip_attached"]["instructions"],
        "pipette tip attached"
    );
    assert_eq!(
        body["assets"][body["state"]["image"].as_str().unwrap()]["data_base64"],
        base64::engine::general_purpose::STANDARD.encode(image.data())
    );
}

#[test]
fn from_loaded_requires_semantic_decisions() {
    let root = std::env::temp_dir().join(format!("typed-jev-missing-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let config_path = root.join("a2a-lab.json");
    std::fs::write(&config_path, br#"{"schema_version":1}"#).unwrap();
    let loaded = LoadedLabConfig::load(&config_path).unwrap();
    let error = TypedLabPredicateJevAdapter::from_loaded(
        &loaded,
        evaluator_config(),
        FrameDecisionEvaluatorLimits::pilot(),
        evaluated_at(),
    )
    .unwrap_err();
    assert_eq!(error.code(), "invalid");
    assert!(error.to_string().contains("semantic_indexing.decisions"));
    let _ = std::fs::remove_dir_all(root);
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

fn assert_noul_bits(evaluation: &KeyedFrameDecisionEvaluation, expected: f32) {
    assert_eq!(
        evaluation
            .structured_output()
            .unwrap()
            .noul_probability()
            .map(f32::to_bits),
        Some(expected.to_bits())
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

fn client(url: &str, limits: FrameDecisionEvaluatorLimits) -> TypeSafeSystemOneClient {
    TypeSafeSystemOneClient::new(
        url,
        "typed-adapter-test-key",
        TypedLabPredicateJevAdapter::http_limits(limits).unwrap(),
    )
    .unwrap()
}

fn noul_body(name: &str, noul: f32) -> Value {
    json!({"answers": {name: {"type": "noul", "noul": noul}}})
}

fn question_instructions(name: &str) -> &'static str {
    match name {
        "pipette_tip_attached" => "pipette tip attached",
        "foreign_object_present" => "foreign object present",
        "plate_misaligned" => "plate misaligned",
        _ => panic!("unexpected question {name}"),
    }
}

fn idle_client(limits: FrameDecisionEvaluatorLimits) -> TypeSafeSystemOneClient {
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

#[derive(Clone)]
struct Captured {
    body: Vec<u8>,
    authorization: Option<String>,
    danger: Option<String>,
}

struct Seen(Mutex<Vec<Captured>>);

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

async fn received(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    state.seen.0.lock().expect("seen").push(Captured {
        authorization: header_text(&headers, AUTHORIZATION.as_str()),
        danger: header_text(&headers, "typesafe-danger-zone-images"),
        body: body.to_vec(),
    });
    match &state.reply {
        Reply::ByPredicate => {
            let value: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
            let questions = value["questions"].as_object();
            let answers = if questions.is_some_and(|map| map.contains_key("pipette_tip_attached")) {
                noul_body("pipette_tip_attached", 1.0)
            } else if questions.is_some_and(|map| map.contains_key("foreign_object_present")) {
                noul_body("foreign_object_present", 0.0)
            } else {
                noul_body("plate_misaligned", 0.5)
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

fn semantic_document(endpoint: &str, credentials_env: Option<&str>) -> Vec<u8> {
    semantic_document_with_preamble(endpoint, credentials_env, None)
}

fn semantic_document_with_preamble(
    endpoint: &str,
    credentials_env: Option<&str>,
    preamble: Option<&str>,
) -> Vec<u8> {
    let mut decisions = json!({
        "endpoint": endpoint,
        "model": "jev-1.14.0"
    });
    if let Some(name) = credentials_env {
        decisions["credentials"] = json!({"env": name});
    }
    if let Some(preamble) = preamble {
        decisions["preamble"] = json!(preamble);
    }
    serde_json::to_vec(&json!({
        "schema_version": 2,
        "semantic_indexing": {
            "decisions": decisions,
            "limits": {
                "worker": {
                    "max_batch_items": 1,
                    "max_concurrency": 1,
                    "max_queue_backlog": 1,
                    "max_retry_attempts": 1,
                    "job_deadline": "30s",
                    "retry_backoff": "5s"
                },
                "search": {
                    "top_k": 1,
                    "candidate_limit": 1,
                    "max_neighbor_frames": 1,
                    "query_window": "1h"
                },
                "storage": {
                    "vector_index_path": "./semantic",
                    "max_disk_bytes": 1024
                },
                "decisions": {
                    "max_batch_items": 1,
                    "max_concurrency": 1,
                    "job_deadline": "45s",
                    "coverage_policy": "on_demand"
                }
            }
        }
    }))
    .unwrap()
}

struct EnvGuard {
    previous_key: Option<String>,
    previous_fallback: Option<String>,
}

impl EnvGuard {
    fn clear_typesafe() -> Self {
        let previous_key = std::env::var("TYPESAFE_API_KEY").ok();
        let previous_fallback = std::env::var("TYPED_JEV_FALLBACK_KEY").ok();
        unsafe {
            std::env::remove_var("TYPESAFE_API_KEY");
            std::env::remove_var("TYPED_JEV_FALLBACK_KEY");
        }
        Self {
            previous_key,
            previous_fallback,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        restore_env("TYPESAFE_API_KEY", self.previous_key.as_deref());
        restore_env("TYPED_JEV_FALLBACK_KEY", self.previous_fallback.as_deref());
    }
}

fn restore_env(name: &str, value: Option<&str>) {
    unsafe {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}

fn header_text(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn json_response(status: StatusCode, body: Vec<u8>) -> Response {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("response")
}
