use std::collections::BTreeSet;

use a2a_lab_dev_kit::{
    CaptureGroupId, DvrStoreNamespace, DvrTrackId, EmbeddingMetric, FrameDescriptionProvenance,
    FrameEmbeddingVector, FrameRoiTransform, FrameVectorIndexAttachment, ModelSpaceIdentity,
    ProvenanceGeneration, SavedFrameRef, Sha256Digest, TextEmbeddingProvenance, UtcTimestamp,
    VectorNormalization, VideoSourceId, derive_frame_id,
};
use schemars::JsonSchema;

fn store() -> DvrStoreNamespace {
    DvrStoreNamespace::new("chupacabra.dvr").unwrap()
}

fn source(name: &str) -> VideoSourceId {
    VideoSourceId::new(name).unwrap()
}

fn track(name: &str) -> DvrTrackId {
    DvrTrackId::new(name).unwrap()
}

fn at(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

fn frame(source_name: &str, track_name: &str, capture: &str, payload: &[u8]) -> SavedFrameRef {
    SavedFrameRef::from_payload_bytes(
        store(),
        source(source_name),
        track(track_name),
        at(capture),
        payload,
        None,
    )
    .unwrap()
}

fn prompt_digest() -> Sha256Digest {
    Sha256Digest::hash_bytes(b"prompt-v1")
}

fn text_hash(text: &[u8]) -> Sha256Digest {
    Sha256Digest::hash_bytes(text)
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

fn preprocessing_digest() -> Sha256Digest {
    Sha256Digest::hash_bytes(b"preprocess-v1")
}

fn description(text: &[u8], generation: u64) -> FrameDescriptionProvenance {
    FrameDescriptionProvenance::new(
        "bedrock",
        "global.openai.gpt-5.6-luna",
        prompt_digest(),
        preprocessing_digest(),
        Some(FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap()),
        text_hash(text),
        ProvenanceGeneration::new(generation).unwrap(),
    )
    .unwrap()
}

fn embedding(text: &[u8], generation: u64) -> TextEmbeddingProvenance {
    TextEmbeddingProvenance::new(
        "bedrock",
        "text-embed-v1",
        4,
        VectorNormalization::L2Unit,
        EmbeddingMetric::Cosine,
        ProvenanceGeneration::new(generation).unwrap(),
        text_hash(text),
    )
    .unwrap()
}

fn vector(values: &[f32]) -> FrameEmbeddingVector {
    FrameEmbeddingVector::new(values.to_vec(), 4).unwrap()
}

#[test]
fn identity_stable_across_rebuild() {
    let first = frame("deck-left", "mjpeg-0", "2024-06-01T12:00:00Z", b"saved-a");
    let rebuilt = frame("deck-left", "mjpeg-0", "2024-06-01T12:00:00Z", b"saved-a");
    assert_eq!(first.frame_id(), rebuilt.frame_id());
    assert_eq!(
        derive_frame_id(
            first.store(),
            first.source_id(),
            first.track_id(),
            first.captured_at(),
            first.payload_digest(),
        )
        .unwrap(),
        first.frame_id().clone()
    );
}

#[test]
fn payload_replacement_blocks_decision_inheritance() {
    let original = frame(
        "deck-left",
        "h264-0",
        "2024-06-01T12:00:00.200Z",
        b"bytes-v1",
    );
    let replaced = frame(
        "deck-left",
        "h264-0",
        "2024-06-01T12:00:00.200Z",
        b"bytes-v2",
    );
    assert!(original.is_payload_replacement(&replaced));
    assert_ne!(original.frame_id(), replaced.frame_id());
    assert!(!replaced.decisions_may_inherit_from(&original));
}

#[test]
fn two_sources_at_one_instant_do_not_collide() {
    let left = frame(
        "deck-left",
        "mjpeg-0",
        "2024-06-01T12:00:01Z",
        b"same-payload",
    );
    let right = frame(
        "deck-right",
        "mjpeg-0",
        "2024-06-01T12:00:01Z",
        b"same-payload",
    );
    assert_ne!(left.frame_id(), right.frame_id());
}

#[test]
fn duplicate_replay_is_idempotent() {
    let once = frame("overhead", "mjpeg-0", "2024-06-01T12:00:02Z", b"dup");
    let twice = frame("overhead", "mjpeg-0", "2024-06-01T12:00:02Z", b"dup");
    assert_eq!(once, twice);
}

#[test]
fn codec_epochs_on_different_tracks_do_not_collide() {
    let epoch0 = frame("deck-left", "h264-epoch-0", "2024-06-01T12:00:03Z", b"au");
    let epoch1 = frame("deck-left", "h264-epoch-1", "2024-06-01T12:00:03Z", b"au");
    assert_ne!(epoch0.frame_id(), epoch1.frame_id());
}

#[test]
fn reordered_capture_timestamps_stay_distinct() {
    let earlier = frame("deck-left", "mjpeg-0", "2024-06-01T12:00:00.000Z", b"x");
    let later = frame("deck-left", "mjpeg-0", "2024-06-01T12:00:00.400Z", b"x");
    assert_ne!(earlier.frame_id(), later.frame_id());
    let backfill_later = frame("deck-left", "mjpeg-0", "2024-06-01T12:00:00.400Z", b"x");
    let backfill_earlier = frame("deck-left", "mjpeg-0", "2024-06-01T12:00:00.000Z", b"x");
    assert_eq!(later.frame_id(), backfill_later.frame_id());
    assert_eq!(earlier.frame_id(), backfill_earlier.frame_id());
}

#[test]
fn optional_capture_group_round_trips_without_changing_frame_id() {
    let group = CaptureGroupId::new("burst-77").unwrap();
    let with_group = SavedFrameRef::from_payload_bytes(
        store(),
        source("deck-left"),
        track("mjpeg-0"),
        at("2024-06-01T12:00:04Z"),
        b"grouped",
        Some(group.clone()),
    )
    .unwrap();
    let without_group = frame("deck-left", "mjpeg-0", "2024-06-01T12:00:04Z", b"grouped");
    assert_eq!(with_group.frame_id(), without_group.frame_id());
    assert_eq!(with_group.capture_group(), Some(&group));
    let json = serde_json::to_string(&with_group).unwrap();
    let decoded: SavedFrameRef = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, with_group);
}

#[test]
fn index_provenance_fields_are_separate_in_schema() {
    fn schema<T: JsonSchema>() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(T)).unwrap()
    }
    let description = schema::<FrameDescriptionProvenance>();
    let embedding = schema::<TextEmbeddingProvenance>();
    let description_keys = object_keys(&description);
    let embedding_keys = object_keys(&embedding);
    assert!(description_keys.contains("resolved_model"));
    assert!(description_keys.contains("prompt_digest"));
    assert!(embedding_keys.contains("dimensions"));
    assert!(embedding_keys.contains("normalization"));
    assert!(embedding_keys.contains("metric"));
    assert!(!description_keys.contains("dimensions"));
    assert!(!embedding_keys.contains("prompt_digest"));
}

fn object_keys(value: &serde_json::Value) -> BTreeSet<String> {
    value
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .map(|props| props.keys().cloned().collect())
        .unwrap_or_default()
}

#[test]
fn valid_vector_attachment_round_trips() {
    let text = b"tip visible on left mount";
    let attachment = FrameVectorIndexAttachment::new(
        model_space(1),
        description(text, 1),
        embedding(text, 1),
        vector(&[0.1, 0.2, -0.3, 0.4]),
    )
    .unwrap();
    let json = serde_json::to_string(&attachment).unwrap();
    let decoded: FrameVectorIndexAttachment = serde_json::from_str(&json).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded, attachment);
}

#[test]
fn rejects_invalid_vectors() {
    let nan_err = FrameEmbeddingVector::new(vec![f32::NAN, 0.1, 0.2, 0.3], 4).expect_err("nan");
    assert!(nan_err.to_string().contains("vector"));

    let short_err = FrameEmbeddingVector::new(vec![0.1, 0.2], 4).expect_err("short");
    assert!(short_err.to_string().contains("vector"));
}

#[test]
fn rejects_cross_generation_model_space_mismatch() {
    let text = b"steady";
    let err = FrameVectorIndexAttachment::new(
        model_space(1),
        description(text, 1),
        embedding(text, 2),
        vector(&[0.1, 0.0, 0.0, 0.0]),
    )
    .expect_err("generation");
    assert!(err.to_string().contains("generation"));
}

#[test]
fn rejects_cross_generation_description_text_mismatch() {
    let err = FrameVectorIndexAttachment::new(
        model_space(2),
        description(b"luna v2", 2),
        embedding(b"luna v1", 2),
        vector(&[0.2, 0.1, 0.0, 0.0]),
    )
    .expect_err("text");
    assert!(err.to_string().contains("source_text_hash"));
}

#[test]
fn existing_video_contract_exports_unchanged() {
    use a2a_lab_dev_kit::{DvrFrameAtRequest, GetVideoFrameAtResult, VideoFrame};
    fn _uses_video_types(_: DvrFrameAtRequest, _: GetVideoFrameAtResult, _: VideoFrame) {}
    let names = [
        "DvrFrameAtRequest",
        "GetVideoFrameAtResult",
        "VideoFrame",
        "VideoSourceId",
    ];
    let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"));
    for name in names {
        assert!(source.contains(name), "missing export {name}");
    }
}
