//! Lab-file configuration and service resolution for the synchronous image client.

use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aLabCommand, A2aLabConfig, A2aLabError, A2aLabResult, A2aLabService,
    DEFAULT_IMAGE_DECISION_TIMEOUT_SECS, DecisionHistoryPolicy, FakeFrameDecisionLedgerProvider,
    FrameDecisionEvaluatorLimits, FrameDecisionEvaluatorProviderConfig, FrameDecisionReadLimits,
    FrameDecisionReadScope, GetFrameDecisionCapabilitiesRequest,
    IMAGE_DECISION_CLIENT_DOWNSTREAM_DEPENDENCY, IMAGE_DECISION_DOWNSTREAM_DEPENDENCY,
    IMAGE_DECISION_EXCLUDED_SCOPE, IMAGE_DECISION_TRANSPORTS, Image, ImageDecisionClientConfig,
    ImageDecisionModelAnswer, ImageDecisionModelInput, ImageDecisionRequest, ImageDecisionRoi,
    ImageDescriptor, ImageId, ImageSourceId, JEV_MAX_IMAGE_BYTES, JEV_MAX_REQUEST_BYTES,
    JsonObject, LabPredicate, MemoryLogs, MemoryMetrics, MemoryTasks,
    PREDICATE_FOREIGN_OBJECT_PRESENT, PREDICATE_PIPETTE_TIP_ATTACHED, PREDICATE_PLATE_MISALIGNED,
    PageRequest, ReadFrameDecisionHistoryRequest, RegisteredFrameDecisionEvaluator, Sha256Digest,
    TimeRange, UtcTimestamp, VideoSourceId,
};
use serde_json::{Value, json};

fn document(client: &Value) -> String {
    serde_json::to_string(&json!({
        "schema_version": 1,
        "image_decision": client
    }))
    .unwrap()
}

fn assert_bounds(actual: [f32; 4], expected: [f32; 4]) {
    assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
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

fn model_answer(bounds: [f32; 4]) -> ImageDecisionModelAnswer {
    ImageDecisionModelAnswer::new(
        Some("yes"),
        Some(0.5),
        Some(ImageDecisionRoi::new(bounds).unwrap()),
    )
    .unwrap()
}

fn request(model: &str, roi: Option<ImageDecisionRoi>) -> ImageDecisionRequest {
    ImageDecisionRequest::new(
        sample_image(),
        "is the tip attached?",
        ImageDecisionModelInput::new(model, roi).unwrap(),
    )
    .unwrap()
}

fn invalid(document: &str) -> (String, String) {
    match A2aLabConfig::parse(document).unwrap_err() {
        A2aLabError::Invalid { field, message } => (field.to_owned(), message),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn lab_file_round_trips_the_public_client_shape() {
    let minimal = A2aLabConfig::parse(&document(&json!({
        "endpoint": "https://jev.example/v1/decide",
        "model": "jev-1.14.0"
    })))
    .unwrap();
    let client = minimal.image_decision.clone().unwrap();
    assert!(client.omits_roi());
    assert_bounds(
        client.effective_roi().bounds(),
        ImageDecisionRoi::full_image().bounds(),
    );
    assert_eq!(client.timeout_secs(), DEFAULT_IMAGE_DECISION_TIMEOUT_SECS);
    assert_eq!(
        client.max_image_bytes(),
        u64::try_from(JEV_MAX_IMAGE_BYTES).unwrap()
    );
    assert_eq!(
        client.max_request_bytes(),
        u64::try_from(JEV_MAX_REQUEST_BYTES).unwrap()
    );
    let encoded = serde_json::to_value(&client).unwrap();
    assert!(encoded.get("roi").is_none());
    assert!(encoded.get("timeout").is_none());
    assert!(encoded.get("credentials").is_none());
    let restored = A2aLabConfig::parse(&document(&encoded)).unwrap();
    assert_eq!(restored.image_decision, minimal.image_decision);

    let full = json!({
        "endpoint": "https://jev.example:8443/v1/decide",
        "model": "jev-1.14.0",
        "timeout": "45s",
        "max_image_bytes": 1024,
        "max_request_bytes": 2048,
        "max_response_bytes": 4096,
        "credentials": {"env": "JEV_API_TOKEN"},
        "roi": {"bounds": [0.25, 0.25, 0.75, 0.75]}
    });
    let parsed = A2aLabConfig::parse(&document(&full)).unwrap();
    let encoded = serde_json::to_value(parsed.image_decision.as_ref().unwrap()).unwrap();
    let again: ImageDecisionClientConfig = serde_json::from_value(encoded.clone()).unwrap();
    let round_trip = A2aLabConfig::parse(&document(&encoded)).unwrap();
    assert_eq!(round_trip.image_decision.as_ref(), Some(&again));
    assert_eq!(again.timeout_secs(), 45);
    assert_eq!(again.credentials_env(), Some("JEV_API_TOKEN"));
    assert_bounds(again.roi().unwrap().bounds(), [0.25, 0.25, 0.75, 0.75]);
    assert!(!again.omits_roi());
}

#[test]
#[allow(clippy::too_many_lines)]
fn rejects_missing_malformed_insecure_and_out_of_bounds_settings() {
    let too_large = u64::try_from(JEV_MAX_IMAGE_BYTES).unwrap() + 1;
    let cases = [
        (
            json!({"model": "jev-1.14.0"}),
            "image_decision.endpoint",
            "required field is missing",
        ),
        (
            json!({"endpoint": "https://jev.example/v1/decide"}),
            "image_decision.model",
            "required field is missing",
        ),
        (
            json!({"endpoint": "not-a-url", "model": "jev-1.14.0"}),
            "image_decision.endpoint",
            "must be an https URL",
        ),
        (
            json!({"endpoint": "http://jev.example/v1/decide", "model": "jev-1.14.0"}),
            "image_decision.endpoint",
            "must not embed credentials",
        ),
        (
            json!({"endpoint": "https://user:pass@jev.example/v1", "model": "jev-1.14.0"}),
            "image_decision.endpoint",
            "must not embed credentials",
        ),
        (
            json!({"endpoint": "https://jev.example/v1?token=abc", "model": "jev-1.14.0"}),
            "image_decision.endpoint",
            "must not embed credentials",
        ),
        (
            json!({
                "endpoint": "https://jev.example/v1/decide",
                "model": "jev-1.14.0",
                "api_key": "secret"
            }),
            "image_decision.credentials",
            "inline secrets are not accepted",
        ),
        (
            json!({
                "endpoint": "https://jev.example/v1/decide",
                "model": "jev-1.14.0",
                "timeout": "0s"
            }),
            "image_decision.timeout",
            "must be a positive whole-second duration",
        ),
        (
            json!({
                "endpoint": "https://jev.example/v1/decide",
                "model": "jev-1.14.0",
                "timeout": "121s"
            }),
            "image_decision.timeout",
            "must be 1..=120 seconds",
        ),
        (
            json!({
                "endpoint": "https://jev.example/v1/decide",
                "model": "jev-1.14.0",
                "max_image_bytes": 0
            }),
            "image_decision.max_image_bytes",
            "must be 1..=",
        ),
        (
            json!({
                "endpoint": "https://jev.example/v1/decide",
                "model": "jev-1.14.0",
                "max_image_bytes": too_large
            }),
            "image_decision.max_image_bytes",
            "must be 1..=",
        ),
        (
            json!({
                "endpoint": "https://jev.example/v1/decide",
                "model": "jev-1.14.0",
                "max_request_bytes": 1
            }),
            "image_decision.max_request_bytes",
            "must be greater than or equal to max_image_bytes",
        ),
        (
            json!({"endpoint": "https://jev.example:99999/v1", "model": "jev-1.14.0"}),
            "image_decision.endpoint",
            "port must be 1..=65535",
        ),
        (
            json!({
                "endpoint": "https://jev.example/v1/decide",
                "model": "jev-1.14.0",
                "roi": {"bounds": [0.0, 0.0, 1.2, 1.0]}
            }),
            "image_decision.roi",
            "bounds must be normalized",
        ),
        (
            json!({"endpoint": "https://jev.example/v1/decide", "model": "jev 1"}),
            "image_decision.model",
            "must be a single token",
        ),
        (
            json!({
                "endpoint": "https://jev.example/v1/decide",
                "model": "jev-1.14.0",
                "credentials": {"env": "not_valid"}
            }),
            "image_decision.credentials.env",
            "must be an uppercase environment variable name",
        ),
        (
            json!({
                "endpoint": "https://jev.example/v1/decide",
                "model": "jev-1.14.0",
                "provider": "bedrock"
            }),
            "image_decision.provider",
            "unknown field",
        ),
    ];

    for (client, field, message) in cases {
        let (actual_field, actual_message) = invalid(&document(&client));
        assert_eq!(actual_field, field, "{actual_message}");
        assert!(
            actual_message.contains(message),
            "{actual_field}: {actual_message}"
        );
    }
}

#[test]
fn resolved_client_uses_full_image_by_default_and_maps_cropped_boxes() {
    let config = A2aLabConfig::parse(&document(&json!({
        "endpoint": "https://jev.example/v1/decide",
        "model": "jev-1.14.0"
    })))
    .unwrap();
    let service = A2aLabService::new(
        MemoryLogs::default(),
        MemoryMetrics::default(),
        MemoryTasks::default(),
    )
    .resolve_image_decision_client(&config);
    let client = service.image_decision_client().unwrap();
    assert!(client.omits_roi());
    assert_bounds(
        client.effective_roi().bounds(),
        ImageDecisionRoi::full_image().bounds(),
    );
    let model_input = client.model_input().unwrap();
    assert!(model_input.omits_roi());

    let full = client
        .complete(
            &request("jev-1.14.0", None),
            &[model_answer([0.25, 0.25, 0.75, 0.75])],
        )
        .unwrap();
    assert_bounds(
        full.answers()[0].source_box().unwrap().bounds(),
        [0.25, 0.25, 0.75, 0.75],
    );
    let encoded = serde_json::to_value(&full).unwrap();
    assert!(encoded.get("predicate").is_none());
    assert!(encoded.get("decision_id").is_none());

    let crop = ImageDecisionRoi::new([0.25, 0.25, 0.75, 0.75]).unwrap();
    let cropped = client
        .complete(
            &request("jev-1.14.0", Some(crop)),
            &[model_answer([0.0, 0.0, 0.5, 1.0])],
        )
        .unwrap();
    assert_bounds(
        cropped.answers()[0].source_box().unwrap().bounds(),
        [0.25, 0.25, 0.5, 0.75],
    );

    let edge = ImageDecisionRoi::new([0.5, 0.5, 1.0, 1.0]).unwrap();
    let mapped = client
        .complete(
            &request("jev-1.14.0", Some(edge)),
            &[model_answer([0.0, 0.0, 1.0, 1.0])],
        )
        .unwrap();
    assert_bounds(
        mapped.answers()[0].source_box().unwrap().bounds(),
        [0.5, 0.5, 1.0, 1.0],
    );
}

#[test]
fn configured_roi_is_the_client_crop_and_limits_reject_oversized_payloads() {
    let config = A2aLabConfig::parse(&document(&json!({
        "endpoint": "https://jev.example/v1/decide",
        "model": "jev-1.14.0",
        "max_image_bytes": 4,
        "max_request_bytes": 4,
        "max_response_bytes": 32,
        "roi": {"bounds": [0.5, 0.5, 1.0, 1.0]}
    })))
    .unwrap();
    let client = config.image_decision.clone().unwrap();
    assert_bounds(
        client.model_input().unwrap().effective_roi().bounds(),
        [0.5, 0.5, 1.0, 1.0],
    );
    let resolved = a2a_lab_dev_kit::ResolvedImageDecisionClient::from_config(client);
    let cropped = resolved
        .complete(
            &request(
                "jev-1.14.0",
                Some(ImageDecisionRoi::new([0.5, 0.5, 1.0, 1.0]).unwrap()),
            ),
            &[model_answer([0.0, 0.0, 1.0, 1.0])],
        )
        .unwrap_err();
    assert_eq!(cropped.code(), "limit_exceeded");

    let image_limited = A2aLabConfig::parse(&document(&json!({
        "endpoint": "https://jev.example/v1/decide",
        "model": "jev-1.14.0",
        "max_image_bytes": 3,
        "max_request_bytes": 3,
        "max_response_bytes": 4096
    })))
    .unwrap();
    let resolved = image_limited
        .image_decision
        .map(a2a_lab_dev_kit::ResolvedImageDecisionClient::from_config)
        .unwrap();
    let error = resolved
        .complete(
            &request("jev-1.14.0", None),
            &[model_answer([0.0, 0.0, 1.0, 1.0])],
        )
        .unwrap_err();
    assert_eq!(error.code(), "limit_exceeded");
}

#[tokio::test]
async fn service_resolution_keeps_typed_registration_and_skips_the_ledger() {
    let evaluator = FrameDecisionEvaluatorProviderConfig::pilot(
        Sha256Digest::hash_bytes(b"image-decision-client"),
        Sha256Digest::hash_bytes(b"threshold"),
    )
    .unwrap();
    let limits = FrameDecisionEvaluatorLimits::pilot();
    let registered = RegisteredFrameDecisionEvaluator::register(&evaluator, limits).unwrap();
    let before: Vec<_> = registered
        .capabilities()
        .supported_predicates()
        .iter()
        .map(|predicate| (predicate.id().to_owned(), predicate.version()))
        .collect();
    let provider =
        Arc::new(FakeFrameDecisionLedgerProvider::authorized_producer(&evaluator, limits).unwrap());
    let config = A2aLabConfig::parse(&document(&json!({
        "endpoint": "https://jev.example/v1/decide",
        "model": "jev-1.14.0"
    })))
    .unwrap();
    let service = A2aLabService::new(
        MemoryLogs::default(),
        MemoryMetrics::default(),
        MemoryTasks::default(),
    )
    .with_frame_decisions(provider)
    .resolve_image_decision_client(&config);

    let caps = service
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
    let after: Vec<_> = caps
        .evaluator()
        .unwrap()
        .supported_predicates()
        .iter()
        .map(|predicate| (predicate.id().to_owned(), predicate.version()))
        .collect();
    assert_eq!(before, after);
    assert_eq!(
        after,
        vec![
            (PREDICATE_PIPETTE_TIP_ATTACHED.to_owned(), 1),
            (PREDICATE_FOREIGN_OBJECT_PRESENT.to_owned(), 1),
            (PREDICATE_PLATE_MISALIGNED.to_owned(), 1),
        ]
    );
    assert!(
        LabPredicate::pipette_tip_attached_v1()
            .id()
            .starts_with("lab.")
    );

    let result = service
        .image_decision_client()
        .unwrap()
        .complete(
            &request("jev-1.14.0", None),
            &[model_answer([0.25, 0.25, 0.75, 0.75])],
        )
        .unwrap();
    assert_eq!(result.answers().len(), 1);

    let history = service
        .execute(A2aLabCommand::ReadFrameDecisionHistory(
            ReadFrameDecisionHistoryRequest::new(
                FrameDecisionReadScope::new(
                    VideoSourceId::new("deck").unwrap(),
                    TimeRange::new(
                        UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
                        UtcTimestamp::parse("2024-01-01T00:01:00Z").unwrap(),
                    )
                    .unwrap(),
                    None,
                )
                .unwrap(),
                None,
                None,
                None,
                DecisionHistoryPolicy::FullHistory,
                FrameDecisionReadLimits::pilot(),
                PageRequest::new(None, 10).unwrap(),
                Vec::new(),
            )
            .unwrap(),
        ))
        .await
        .unwrap()
        .task
        .result;
    let A2aLabResult::ReadFrameDecisionHistory(history) = history else {
        panic!("expected history");
    };
    assert!(history.entries().items().is_empty());
}

#[test]
fn downstream_dependency_stays_a_published_git_revision() {
    assert_eq!(
        IMAGE_DECISION_CLIENT_DOWNSTREAM_DEPENDENCY,
        IMAGE_DECISION_DOWNSTREAM_DEPENDENCY
    );
    assert_eq!(
        IMAGE_DECISION_CLIENT_DOWNSTREAM_DEPENDENCY,
        "published-git-revision"
    );
    assert_ne!(IMAGE_DECISION_CLIENT_DOWNSTREAM_DEPENDENCY, "path");
    assert_eq!(IMAGE_DECISION_TRANSPORTS, ["http", "mcp", "a2a"]);
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
}
