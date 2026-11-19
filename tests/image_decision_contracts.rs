//! Contracts for generic JEV-style image decisions.

use std::any::TypeId;
use std::collections::BTreeSet;

use a2a_lab_dev_kit::{
    A2aLabError, FakeFrameDecisionEvaluator, FakeJevBehavior, FrameDecisionEvaluator,
    FrameDecisionEvaluatorLimits, FrameDecisionEvaluatorProviderConfig, FrameDecisionLedger,
    FrameDecisionRecord, IMAGE_DECISION_DOWNSTREAM_DEPENDENCY, IMAGE_DECISION_EXCLUDED_SCOPE,
    IMAGE_DECISION_TRANSPORTS, Image, ImageDecisionAnswer, ImageDecisionClass,
    ImageDecisionModelAnswer, ImageDecisionModelInput, ImageDecisionProviderError,
    ImageDecisionRequest, ImageDecisionResult, ImageDecisionRoi, ImageDescriptor, ImageId,
    ImageSourceId, JsonObject, LabPredicate, MAX_IMAGE_DECISION_ANSWERS,
    PREDICATE_FOREIGN_OBJECT_PRESENT, PREDICATE_PIPETTE_TIP_ATTACHED, PREDICATE_PLATE_MISALIGNED,
    Sha256Digest, UtcTimestamp, execute_generic_image_decision,
};
use schemars::JsonSchema;
use serde_json::{Value, json};

fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema")
}

fn property_names(value: &Value) -> BTreeSet<String> {
    value
        .pointer("/properties")
        .and_then(Value::as_object)
        .map(|properties| properties.keys().cloned().collect())
        .unwrap_or_default()
}

fn required_names(value: &Value) -> BTreeSet<String> {
    value
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn contains_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(map) => {
            map.keys().any(|name| name == key) || map.values().any(|child| contains_key(child, key))
        }
        Value::Array(items) => items.iter().any(|child| contains_key(child, key)),
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => false,
    }
}

fn sample_image() -> Image {
    let descriptor = ImageDescriptor::new(
        ImageId::new("frame").unwrap(),
        ImageSourceId::new("cam").unwrap(),
        UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
        "image/png",
        640,
        480,
        None,
        JsonObject::empty(),
    )
    .unwrap();
    Image::new(descriptor, vec![0xff, 0x00, 0x10, b'a']).unwrap()
}

fn request(roi: Option<ImageDecisionRoi>) -> ImageDecisionRequest {
    ImageDecisionRequest::new(
        sample_image(),
        "Is the tip attached?",
        ImageDecisionModelInput::new("jev-1.14.0", roi).unwrap(),
    )
    .unwrap()
}

fn model_answer(bounds: [f32; 4]) -> ImageDecisionModelAnswer {
    ImageDecisionModelAnswer::new(
        Some("yes"),
        Some(0.91),
        Some(ImageDecisionRoi::new(bounds).unwrap()),
    )
    .unwrap()
}

#[test]
fn request_and_result_schemas_require_image_question_and_model_input() {
    let request_schema = schema::<ImageDecisionRequest>();
    assert_eq!(
        property_names(&request_schema),
        BTreeSet::from([
            "image".to_owned(),
            "question".to_owned(),
            "model_input".to_owned()
        ])
    );
    assert_eq!(
        required_names(&request_schema),
        BTreeSet::from([
            "image".to_owned(),
            "question".to_owned(),
            "model_input".to_owned()
        ])
    );
    assert!(!property_names(&request_schema).contains("predicate"));

    let model_schema = schema::<ImageDecisionModelInput>();
    assert!(required_names(&model_schema).contains("model"));
    assert!(!required_names(&model_schema).contains("roi"));
    assert!(property_names(&model_schema).contains("roi"));

    let answer_schema = schema::<ImageDecisionAnswer>();
    assert!(property_names(&answer_schema).contains("source_box"));
    assert!(!property_names(&answer_schema).contains("model_box"));
    assert!(property_names(&schema::<ImageDecisionResult>()).contains("answers"));

    let error_schema = schema::<ImageDecisionProviderError>();
    for forbidden in ["status_code", "http_status", "jsonrpc", "mcp_error"] {
        assert!(
            !contains_key(&error_schema, forbidden),
            "{forbidden} leaked into the provider error schema"
        );
    }
}

#[test]
fn request_result_and_provider_error_round_trip() {
    let decision = request(Some(
        ImageDecisionRoi::new([0.25, 0.25, 0.75, 0.75]).unwrap(),
    ));
    let encoded = serde_json::to_value(&decision).unwrap();
    let restored: ImageDecisionRequest = serde_json::from_value(encoded).unwrap();
    assert_eq!(restored, decision);
    assert_eq!(restored.class(), ImageDecisionClass::Generic);

    let result = ImageDecisionResult::new(vec![
        ImageDecisionAnswer::new(Some("yes"), Some(0.5), Some(ImageDecisionRoi::full_image()))
            .unwrap(),
    ])
    .unwrap();
    let encoded = serde_json::to_value(&result).unwrap();
    assert_eq!(encoded["answers"][0]["source_box"]["bounds"][0], 0.0);
    let restored: ImageDecisionResult = serde_json::from_value(encoded).unwrap();
    assert_eq!(restored, result);

    let error = ImageDecisionProviderError::unavailable("jev down").unwrap();
    let encoded = serde_json::to_value(&error).unwrap();
    assert_eq!(encoded["reason"], "unavailable");
    assert!(encoded.get("status_code").is_none());
    let restored: ImageDecisionProviderError = serde_json::from_value(encoded).unwrap();
    assert_eq!(restored.code(), "unavailable");
    assert!(
        serde_json::from_value::<ImageDecisionProviderError>(json!({
            "reason": "unavailable",
            "message": "jev down",
            "status_code": 503
        }))
        .is_err()
    );
}

#[test]
fn request_rejects_missing_or_unbounded_fields() {
    let missing = json!({
        "question": "Is the tip attached?",
        "model_input": {"model": "jev-1.14.0"}
    });
    assert!(serde_json::from_value::<ImageDecisionRequest>(missing).is_err());

    assert!(ImageDecisionRequest::new(sample_image(), "   ", model_input(None)).is_err());
    assert!(ImageDecisionModelInput::new(" ", None).is_err());
    assert!(ImageDecisionModelInput::new("jev 1", None).is_err());
    assert!(
        ImageDecisionAnswer::new(None, Some(1.5), None).is_err(),
        "probability above 1"
    );
    assert!(ImageDecisionAnswer::new(None, None, None).is_err());
    let too_many = vec![
        ImageDecisionAnswer::new(Some("yes"), None, None).unwrap();
        MAX_IMAGE_DECISION_ANSWERS + 1
    ];
    assert!(matches!(
        ImageDecisionResult::new(too_many),
        Err(A2aLabError::Invalid {
            field: "answers",
            ..
        })
    ));
    let too_many = vec![model_answer([0.1, 0.1, 0.2, 0.2]); MAX_IMAGE_DECISION_ANSWERS + 1];
    let error = execute_generic_image_decision(&request(None), &too_many).unwrap_err();
    assert_eq!(error.code(), "limit_exceeded");
}

#[test]
fn generic_execution_does_not_append_typed_ledger_records() {
    let ledger = FrameDecisionLedger::new();
    let before = ledger.history().to_vec();
    let result =
        execute_generic_image_decision(&request(None), &[model_answer([0.25, 0.25, 0.75, 0.75])])
            .unwrap();
    assert_eq!(ledger.history(), before.as_slice());
    assert_eq!(request(None).class(), ImageDecisionClass::Generic);
    assert_ne!(
        TypeId::of::<ImageDecisionResult>(),
        TypeId::of::<FrameDecisionRecord>()
    );
    let encoded = serde_json::to_value(&result).unwrap();
    assert!(encoded.get("predicate").is_none());
    assert!(encoded.get("idempotency_key").is_none());
    assert!(encoded.get("decision_id").is_none());
}

#[test]
fn typed_three_predicate_evaluator_remains_available() {
    let config = FrameDecisionEvaluatorProviderConfig::pilot(
        Sha256Digest::hash_bytes(b"image-decision-contract"),
        Sha256Digest::hash_bytes(b"threshold"),
    )
    .unwrap();
    let evaluator = FakeFrameDecisionEvaluator::new(
        config,
        FrameDecisionEvaluatorLimits::pilot(),
        FakeJevBehavior::OkPresent,
    );
    let capabilities = evaluator.capabilities();
    let predicates = capabilities.supported_predicates();
    assert_eq!(predicates.len(), 3);
    assert_eq!(predicates[0].id(), PREDICATE_PIPETTE_TIP_ATTACHED);
    assert_eq!(predicates[1].id(), PREDICATE_FOREIGN_OBJECT_PRESENT);
    assert_eq!(predicates[2].id(), PREDICATE_PLATE_MISALIGNED);
    assert!(predicates.iter().all(|predicate| predicate.version() == 1));
    assert!(
        LabPredicate::pipette_tip_attached_v1()
            .id()
            .starts_with("lab.")
    );
}

#[test]
fn omitted_roi_is_the_full_image_and_boxes_return_to_source_coordinates() {
    let omitted = ImageDecisionModelInput::new("jev-1.14.0", None).unwrap();
    let encoded = serde_json::to_value(&omitted).unwrap();
    assert!(encoded.get("roi").is_none());
    let restored: ImageDecisionModelInput = serde_json::from_value(encoded).unwrap();
    assert!(restored.omits_roi());
    assert_eq!(
        roi_bits(restored.effective_roi()),
        roi_bits(ImageDecisionRoi::full_image())
    );

    let full =
        execute_generic_image_decision(&request(None), &[model_answer([0.25, 0.25, 0.75, 0.75])])
            .unwrap();
    assert_eq!(
        roi_bits(full.answers()[0].source_box().unwrap()),
        bits([0.25, 0.25, 0.75, 0.75])
    );

    let crop = ImageDecisionRoi::new([0.25, 0.25, 0.75, 0.75]).unwrap();
    let cropped =
        execute_generic_image_decision(&request(Some(crop)), &[model_answer([0.0, 0.0, 0.5, 1.0])])
            .unwrap();
    assert_eq!(
        roi_bits(cropped.answers()[0].source_box().unwrap()),
        bits([0.25, 0.25, 0.5, 0.75])
    );

    let edge = execute_generic_image_decision(
        &request(Some(ImageDecisionRoi::new([0.5, 0.5, 1.0, 1.0]).unwrap())),
        &[model_answer([0.0, 0.0, 1.0, 1.0])],
    )
    .unwrap();
    assert_eq!(
        roi_bits(edge.answers()[0].source_box().unwrap()),
        bits([0.5, 0.5, 1.0, 1.0])
    );
    assert!(ImageDecisionRoi::new([0.0, 0.0, 1.2, 1.0]).is_err());
}

#[test]
fn public_contract_documents_the_published_revision_boundary() {
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
        assert!(
            IMAGE_DECISION_EXCLUDED_SCOPE.contains(&excluded),
            "{excluded} must stay out of this contract"
        );
    }
}

fn roi_bits(roi: ImageDecisionRoi) -> [u32; 4] {
    bits(roi.bounds())
}

fn bits(bounds: [f32; 4]) -> [u32; 4] {
    bounds.map(f32::to_bits)
}

fn model_input(roi: Option<ImageDecisionRoi>) -> ImageDecisionModelInput {
    ImageDecisionModelInput::new("jev-1.14.0", roi).unwrap()
}
