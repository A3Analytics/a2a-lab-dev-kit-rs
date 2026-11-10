//! Contract tests for semantic indexing provider boundaries.

use a2a_lab_dev_kit::{
    DvrStoreNamespace, DvrTrackId, EmbeddingMetric, FakeDescriptionBehavior, FakeEmbeddingBehavior,
    FakeFrameDescriptionProvider, FakeTextEmbeddingProvider, FrameDescriptionBackend,
    FrameDescriptionKey, FrameDescriptionLimits, FrameDescriptionProvider,
    FrameDescriptionProviderConfig, FrameDescriptionRequest, FrameRoiTransform,
    KeyedFrameDescription, ModelSpaceIdentity, ProvenanceGeneration, SavedFrameRef,
    SemanticIndexingFailure, Sha256Digest, TextEmbeddingKey, TextEmbeddingLimits,
    TextEmbeddingProvider, TextEmbeddingProviderConfig, TextEmbeddingRequest, UtcTimestamp,
    VectorNormalization, VideoSourceId, validate_description_batch, validate_embedding_batch,
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

fn description_config() -> FrameDescriptionProviderConfig {
    FrameDescriptionProviderConfig::new(
        FrameDescriptionBackend::Bedrock,
        None::<&str>,
        Sha256Digest::hash_bytes(b"prompt-v1"),
        Sha256Digest::hash_bytes(b"preprocess-v1"),
    )
    .unwrap()
}

fn description_key(payload: &[u8], generation: u64) -> FrameDescriptionKey {
    FrameDescriptionKey::new(
        frame(payload),
        Some(FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap()),
        ProvenanceGeneration::new(generation).unwrap(),
    )
}

fn description_request(payload: &[u8], generation: u64) -> FrameDescriptionRequest {
    FrameDescriptionRequest::new(
        FrameDescriptionLimits::pilot(),
        description_key(payload, generation),
        1024,
        640 * 480,
    )
    .unwrap()
}

fn embedding_config(generation: u64) -> TextEmbeddingProviderConfig {
    let space = ModelSpaceIdentity::new(
        "bedrock",
        "text-embed-v1",
        4,
        VectorNormalization::L2Unit,
        EmbeddingMetric::Cosine,
        ProvenanceGeneration::new(generation).unwrap(),
    )
    .unwrap();
    TextEmbeddingProviderConfig::new("bedrock", "text-embed-v1", space).unwrap()
}

async fn run_describe(
    provider: &FakeFrameDescriptionProvider,
    requests: Vec<FrameDescriptionRequest>,
) -> Result<Vec<KeyedFrameDescription>, SemanticIndexingFailure> {
    provider.describe_frames(requests).await
}

async fn run_embed(
    provider: &FakeTextEmbeddingProvider,
    requests: Vec<TextEmbeddingRequest>,
) -> Result<Vec<a2a_lab_dev_kit::KeyedTextEmbedding>, SemanticIndexingFailure> {
    provider.embed_texts(requests).await
}

#[test]
fn luna_defaults_are_independent_of_chat() {
    let bedrock = FrameDescriptionProviderConfig::new(
        FrameDescriptionBackend::Bedrock,
        None::<&str>,
        Sha256Digest::hash_bytes(b"p"),
        Sha256Digest::hash_bytes(b"pp"),
    )
    .unwrap();
    assert_eq!(
        bedrock.model(),
        a2a_lab_dev_kit::DEFAULT_BEDROCK_DESCRIPTION_MODEL
    );
    let openai = FrameDescriptionProviderConfig::new(
        FrameDescriptionBackend::Openai,
        None::<&str>,
        Sha256Digest::hash_bytes(b"p"),
        Sha256Digest::hash_bytes(b"pp"),
    )
    .unwrap();
    assert_eq!(
        openai.model(),
        a2a_lab_dev_kit::DEFAULT_OPENAI_DESCRIPTION_MODEL
    );
}

#[tokio::test]
async fn description_ok_round_trip_carries_digests_and_generation() {
    let config = description_config();
    let limits = FrameDescriptionLimits::pilot();
    let provider =
        FakeFrameDescriptionProvider::new(config.clone(), limits, FakeDescriptionBehavior::Ok);
    let requests = vec![description_request(b"a", 1), description_request(b"b", 1)];
    let responses = run_describe(&provider, requests.clone()).await.unwrap();
    let validated = validate_description_batch(limits, &requests, &responses).unwrap();
    assert_eq!(validated.len(), 2);
    let provenance = validated[0].to_provenance().unwrap();
    assert_eq!(provenance.prompt_digest(), config.prompt_digest());
    assert_eq!(
        provenance.preprocessing_digest(),
        config.preprocessing_digest()
    );
    assert_eq!(provenance.generation().get(), 1);
}

#[tokio::test]
async fn description_rejects_partial_batch() {
    let limits = FrameDescriptionLimits::pilot();
    let provider = FakeFrameDescriptionProvider::new(
        description_config(),
        limits,
        FakeDescriptionBehavior::Partial,
    );
    let requests = vec![description_request(b"a", 1), description_request(b"b", 1)];
    let responses = run_describe(&provider, requests.clone()).await.unwrap();
    let err = validate_description_batch(limits, &requests, &responses).unwrap_err();
    assert_eq!(err.as_str(), "partial_response");
}

#[tokio::test]
async fn description_rejects_shuffled_duplicate_keys() {
    let limits = FrameDescriptionLimits::pilot();
    let provider = FakeFrameDescriptionProvider::new(
        description_config(),
        limits,
        FakeDescriptionBehavior::ShuffledKeys,
    );
    let requests = vec![description_request(b"a", 1), description_request(b"b", 1)];
    let responses = run_describe(&provider, requests.clone()).await.unwrap();
    let err = validate_description_batch(limits, &requests, &responses).unwrap_err();
    assert_eq!(err.as_str(), "key_mismatch");
}

#[tokio::test]
async fn description_rejects_empty_and_oversized_outputs() {
    let limits = FrameDescriptionLimits::pilot();
    let empty_provider = FakeFrameDescriptionProvider::new(
        description_config(),
        limits,
        FakeDescriptionBehavior::Empty,
    );
    let requests = vec![description_request(b"a", 1)];
    let responses = run_describe(&empty_provider, requests.clone())
        .await
        .unwrap();
    assert_eq!(
        validate_description_batch(limits, &requests, &responses)
            .unwrap_err()
            .as_str(),
        "empty_description"
    );

    let oversized_provider = FakeFrameDescriptionProvider::new(
        description_config(),
        limits,
        FakeDescriptionBehavior::Oversized,
    );
    let responses = run_describe(&oversized_provider, requests.clone())
        .await
        .unwrap();
    assert_eq!(
        validate_description_batch(limits, &requests, &responses)
            .unwrap_err()
            .as_str(),
        "oversized_output"
    );
}

#[tokio::test]
async fn description_fake_timeout_is_deterministic() {
    let provider = FakeFrameDescriptionProvider::new(
        description_config(),
        FrameDescriptionLimits::pilot(),
        FakeDescriptionBehavior::Timeout,
    );
    let err = run_describe(&provider, vec![description_request(b"a", 1)])
        .await
        .unwrap_err();
    assert_eq!(err.as_str(), "deadline_exceeded");
}

#[tokio::test]
async fn embedding_encodes_description_and_query_in_one_space() {
    let config = embedding_config(1);
    let limits = TextEmbeddingLimits::pilot();
    let provider =
        FakeTextEmbeddingProvider::new(config.clone(), limits, FakeEmbeddingBehavior::Ok);
    let description_key = description_key(b"text", 1);
    let requests = vec![
        TextEmbeddingRequest::new(
            limits,
            TextEmbeddingKey::PersistedDescription(description_key),
            "tip visible on mount",
            6,
        )
        .unwrap(),
        TextEmbeddingRequest::new(
            limits,
            TextEmbeddingKey::SearchQuery(Sha256Digest::hash_bytes(b"find tip")),
            "find tip",
            2,
        )
        .unwrap(),
    ];
    let responses = run_embed(&provider, requests.clone()).await.unwrap();
    let validated = validate_embedding_batch(&config, limits, &requests, &responses).unwrap();
    assert_eq!(validated.len(), 2);
    assert_eq!(validated[0].model_space(), config.model_space());
    assert_eq!(validated[1].provenance().dimensions(), 4);
}

#[tokio::test]
async fn embedding_rejects_incompatible_model_space() {
    let config = embedding_config(1);
    let limits = TextEmbeddingLimits::pilot();
    let provider = FakeTextEmbeddingProvider::new(
        config.clone(),
        limits,
        FakeEmbeddingBehavior::WrongModelSpace,
    );
    let description_key = description_key(b"text", 1);
    let requests = vec![
        TextEmbeddingRequest::new(
            limits,
            TextEmbeddingKey::PersistedDescription(description_key),
            "steady scene",
            2,
        )
        .unwrap(),
    ];
    let responses = run_embed(&provider, requests.clone()).await.unwrap();
    let err = validate_embedding_batch(&config, limits, &requests, &responses).unwrap_err();
    assert_eq!(err.as_str(), "incompatible_model_space");
}

#[tokio::test]
async fn embedding_fake_timeout_is_deterministic() {
    let provider = FakeTextEmbeddingProvider::new(
        embedding_config(1),
        TextEmbeddingLimits::pilot(),
        FakeEmbeddingBehavior::Timeout,
    );
    let description_key = description_key(b"t", 1);
    let err = run_embed(
        &provider,
        vec![
            TextEmbeddingRequest::new(
                TextEmbeddingLimits::pilot(),
                TextEmbeddingKey::PersistedDescription(description_key),
                "scene",
                1,
            )
            .unwrap(),
        ],
    )
    .await
    .unwrap_err();
    assert_eq!(err.as_str(), "deadline_exceeded");
}

#[test]
fn provider_traits_exclude_decision_append_surfaces() {
    fn assert_description<T: FrameDescriptionProvider + Send + Sync>() {}
    fn assert_embedding<T: TextEmbeddingProvider + Send + Sync>() {}
    assert_description::<FakeFrameDescriptionProvider>();
    assert_embedding::<FakeTextEmbeddingProvider>();
    let description_source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/videos/semantic_indexing/frame_description.rs"
    ));
    let embedding_source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/videos/semantic_indexing/text_embedding.rs"
    ));
    for token in ["append_decision", "decision_append", "DecisionAppend"] {
        assert!(
            !description_source.contains(token),
            "description contract leaked {token}"
        );
        assert!(
            !embedding_source.contains(token),
            "embedding contract leaked {token}"
        );
    }
}
