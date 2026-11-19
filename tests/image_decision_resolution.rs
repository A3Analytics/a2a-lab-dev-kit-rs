//! One-image resolution and original-source box mapping.

use std::any::TypeId;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use a2a_lab_dev_kit::{
    A2aLabCommand, A2aLabError, A2aLabResult, A2aLabService, AppendFrameDecisionRequest,
    AppendFrameDecisionResult, FakeFrameDecisionEvaluator, FakeFrameDecisionLedgerProvider,
    FakeJevBehavior, FrameDecisionCapabilities, FrameDecisionEvaluator,
    FrameDecisionEvaluatorLimits, FrameDecisionEvaluatorProviderConfig, FrameDecisionHistoryResult,
    FrameDecisionLedgerProvider, GetCurrentImageRequest, GetFrameDecisionCapabilitiesRequest,
    GetImageRequest, IMAGE_DECISION_DOWNSTREAM_DEPENDENCY, IMAGE_DECISION_EXCLUDED_SCOPE,
    IMAGE_DECISION_TRANSPORTS, Image, ImageDecisionImageRef, ImageDecisionModelInput,
    ImageDecisionRequest, ImageDecisionResult, ImageDecisionRoi, ImageDecisionUnmappedAnswer,
    ImageDescriptor, ImageId, ImageProvider, ImageSource, ImageSourceId, JsonObject,
    ListImageSourcesRequest, ListImagesRequest, MemoryImages, MemoryLogs, MemoryMetrics,
    MemoryTasks, Page, ReadFrameDecisionHistoryRequest, SearchImagesRequest, Sha256Digest,
    UtcTimestamp, execute_generic_image_decision, map_model_bounds_onto_source,
};
use async_trait::async_trait;

fn image(id: &str, caption: Option<&str>, bytes: &[u8]) -> Image {
    let descriptor = ImageDescriptor::new(
        ImageId::new(id).unwrap(),
        ImageSourceId::new("cam").unwrap(),
        UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
        "image/png",
        640,
        480,
        caption.map(str::to_owned),
        JsonObject::empty(),
    )
    .unwrap();
    Image::new(descriptor, bytes.to_vec()).unwrap()
}

async fn catalog(images: Vec<Image>) -> MemoryImages {
    let store = MemoryImages::new();
    store
        .insert_source(ImageSource {
            id: ImageSourceId::new("cam").unwrap(),
            name: "Deck".to_owned(),
            description: "deck camera".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await
        .unwrap();
    for image in images {
        store.insert_image(image).await.unwrap();
    }
    store
}

fn service(
    images: MemoryImages,
) -> A2aLabService<MemoryLogs, MemoryMetrics, MemoryTasks, MemoryImages> {
    A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_images(images)
}

fn model(roi: Option<ImageDecisionRoi>) -> ImageDecisionModelInput {
    ImageDecisionModelInput::new("jev-1.14.0", roi).unwrap()
}

fn answer(bounds: Option<[f32; 4]>) -> ImageDecisionUnmappedAnswer {
    ImageDecisionUnmappedAnswer::new(Some("yes"), Some(0.91), bounds).unwrap()
}

fn bits(bounds: [f32; 4]) -> [u32; 4] {
    bounds.map(f32::to_bits)
}

fn roi(bounds: [f32; 4]) -> ImageDecisionRoi {
    ImageDecisionRoi::new(bounds).unwrap()
}

#[tokio::test]
async fn one_image_resolves_through_the_provider_and_omitted_roi_is_the_full_image() {
    let bytes = vec![9, 8, 7, 6];
    let images = catalog(vec![image("frame", Some("tip"), &bytes)]).await;
    let lab = service(images);
    let full = [0.25, 0.25, 0.75, 0.75];
    for image_ref in [
        ImageDecisionImageRef::by_id(ImageId::new("frame").unwrap()),
        ImageDecisionImageRef::current(ImageSourceId::new("cam").unwrap()),
        ImageDecisionImageRef::query(None, None, Some("tip".to_owned())).unwrap(),
    ] {
        let resolved = lab
            .resolve_generic_image_decision(
                image_ref,
                "Is the tip attached?",
                model(None),
                &[answer(Some(full))],
            )
            .await
            .unwrap();
        assert!(resolved.omits_roi());
        assert_eq!(resolved.source().data(), bytes.as_slice());
        assert_eq!(resolved.model_crop().x(), 0);
        assert_eq!(resolved.model_crop().y(), 0);
        assert_eq!(resolved.model_crop().width(), 640);
        assert_eq!(resolved.model_crop().height(), 480);
        assert_eq!(
            bits(resolved.model_crop().roi().bounds()),
            bits([0.0, 0.0, 1.0, 1.0])
        );
        let source_box = resolved.result().answers()[0].source_box().unwrap();
        assert_eq!(bits(source_box.bounds()), bits(full));
        assert!(
            source_box
                .bounds()
                .into_iter()
                .all(|value| (0.0..=1.0).contains(&value))
        );
    }
}

#[tokio::test]
async fn cropped_model_boxes_map_onto_original_source_coordinates() {
    let bytes = vec![1, 2, 3, 4];
    let images = catalog(vec![image("frame", Some("tip"), &bytes)]).await;
    let lab = service(images);
    let crop = roi([0.25, 0.25, 0.75, 0.75]);
    let resolved = lab
        .resolve_generic_image_decision(
            ImageDecisionImageRef::by_id(ImageId::new("frame").unwrap()),
            "Is the tip attached?",
            model(Some(crop)),
            &[answer(Some([0.0, 0.0, 0.5, 1.0]))],
        )
        .await
        .unwrap();
    assert!(!resolved.omits_roi());
    assert_eq!(resolved.source().data(), bytes.as_slice());
    assert_eq!(
        (
            resolved.model_crop().x(),
            resolved.model_crop().y(),
            resolved.model_crop().width(),
            resolved.model_crop().height()
        ),
        (160, 120, 320, 240)
    );
    assert_eq!(
        bits(
            resolved.result().answers()[0]
                .source_box()
                .unwrap()
                .bounds()
        ),
        bits([0.25, 0.25, 0.5, 0.75])
    );

    let request = ImageDecisionRequest::new(
        image("frame", Some("tip"), &bytes),
        "Is the tip attached?",
        model(Some(crop)),
    )
    .unwrap();
    let typed = a2a_lab_dev_kit::ImageDecisionModelAnswer::new(
        Some("yes"),
        Some(0.91),
        Some(roi([0.0, 0.0, 0.5, 1.0])),
    )
    .unwrap();
    let executed = execute_generic_image_decision(&request, &[typed]).unwrap();
    assert_eq!(
        bits(executed.answers()[0].source_box().unwrap().bounds()),
        bits(
            resolved.result().answers()[0]
                .source_box()
                .unwrap()
                .bounds()
        )
    );
}

#[test]
fn mapping_clamps_crop_edges_and_boxes_that_exceed_the_crop() {
    let crop = roi([0.25, 0.25, 0.75, 0.75]);
    let touching =
        map_model_bounds_onto_source(640, 480, roi([0.5, 0.5, 1.0, 1.0]), [0.0, 0.0, 1.0, 1.0])
            .unwrap();
    assert_eq!(bits(touching.bounds()), bits([0.5, 0.5, 1.0, 1.0]));

    let exceeding = map_model_bounds_onto_source(640, 480, crop, [-1.0, 0.0, 2.0, 1.0]).unwrap();
    assert_eq!(bits(exceeding.bounds()), bits([0.0, 0.25, 1.0, 0.75]));
    assert!(
        exceeding
            .bounds()
            .into_iter()
            .all(|value| (0.0..=1.0).contains(&value))
    );

    let outside = map_model_bounds_onto_source(
        640,
        480,
        ImageDecisionRoi::full_image(),
        [2.0, 2.0, 3.0, 3.0],
    )
    .unwrap_err();
    assert!(matches!(
        outside,
        A2aLabError::Invalid {
            field: "model_box",
            ..
        }
    ));
    assert_eq!(outside.code(), "invalid");
    assert!(outside.to_string().contains("outside the source image"));

    let malformed = map_model_bounds_onto_source(640, 480, crop, [0.8, 0.2, 0.1, 0.9]).unwrap_err();
    assert!(matches!(
        malformed,
        A2aLabError::Invalid {
            field: "model_box",
            ..
        }
    ));
    assert!(ImageDecisionUnmappedAnswer::new(None, None, Some([f32::NAN, 0.0, 1.0, 1.0])).is_err());
    assert!(map_model_bounds_onto_source(u32::MAX, 480, crop, [0.0, 0.0, 1.0, 1.0]).is_err());
}

#[tokio::test]
async fn missing_ambiguous_oversized_and_malformed_inputs_fail_stably() {
    let images = catalog(vec![
        image("frame", Some("plate"), &[1, 2, 3, 4]),
        image("other", Some("plate"), &[5, 6, 7, 8]),
    ])
    .await;
    let lab = service(images);
    let missing = lab
        .resolve_generic_image_decision(
            ImageDecisionImageRef::by_id(ImageId::new("absent").unwrap()),
            "Is the tip attached?",
            model(None),
            &[],
        )
        .await
        .unwrap_err();
    assert!(matches!(
        missing,
        A2aLabError::NotFound { kind: "image", .. }
    ));
    assert_eq!(missing.code(), "not_found");

    let ambiguous = lab
        .resolve_generic_image_decision(
            ImageDecisionImageRef::query(None, None, Some("plate".to_owned())).unwrap(),
            "Is the tip attached?",
            model(None),
            &[],
        )
        .await
        .unwrap_err();
    assert!(matches!(
        ambiguous,
        A2aLabError::Invalid {
            field: "image",
            ref message,
        } if message == "ambiguous image reference"
    ));

    let oversized = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_images(catalog(vec![image("frame", Some("tip"), &[1, 2, 3, 4])]).await)
        .with_image_transport(a2a_lab_dev_kit::ImageTransportConfig::new(2).unwrap());
    let oversized = oversized
        .resolve_generic_image_decision(
            ImageDecisionImageRef::by_id(ImageId::new("frame").unwrap()),
            "Is the tip attached?",
            model(None),
            &[],
        )
        .await
        .unwrap_err();
    assert!(matches!(
        oversized,
        A2aLabError::Invalid { field: "data", .. }
    ));
}

#[tokio::test]
async fn malformed_and_unavailable_image_inputs_fail_stably() {
    let mismatched =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_images(Mismatch);
    let mismatched = mismatched
        .resolve_generic_image_decision(
            ImageDecisionImageRef::by_id(ImageId::new("requested").unwrap()),
            "Is the tip attached?",
            model(None),
            &[],
        )
        .await
        .unwrap_err();
    assert!(matches!(
        mismatched,
        A2aLabError::Invalid {
            field: "image",
            ref message,
        } if message == "resolved image does not match the requested reference"
    ));

    assert!(ImageDecisionImageRef::query(None, None, Some("   ".to_owned())).is_err());
    assert!(
        serde_json::from_value::<ImageDecisionImageRef>(serde_json::json!({
            "select": "query"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<ImageDecisionImageRef>(serde_json::json!({
            "select": "id",
            "id": "frame",
            "extra": true
        }))
        .is_err()
    );

    let unavailable =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .resolve_generic_image_decision(
                ImageDecisionImageRef::by_id(ImageId::new("frame").unwrap()),
                "Is the tip attached?",
                model(None),
                &[],
            )
            .await
            .unwrap_err();
    assert_eq!(unavailable.code(), "unavailable");
}

#[tokio::test]
async fn resolution_does_not_append_typed_ledger_records() {
    let appends = Arc::new(AtomicUsize::new(0));
    let config = FrameDecisionEvaluatorProviderConfig::pilot(
        Sha256Digest::hash_bytes(b"image-resolution"),
        Sha256Digest::hash_bytes(b"threshold"),
    )
    .unwrap();
    let ledger = Arc::new(CountingLedger {
        inner: FakeFrameDecisionLedgerProvider::authorized_producer(
            &config,
            FrameDecisionEvaluatorLimits::pilot(),
        )
        .unwrap(),
        appends: Arc::clone(&appends),
    });
    let images = catalog(vec![image("frame", Some("tip"), &[1, 2, 3, 4])]).await;
    let lab = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_images(images)
        .with_frame_decisions(ledger);
    let resolved = lab
        .resolve_generic_image_decision(
            ImageDecisionImageRef::by_id(ImageId::new("frame").unwrap()),
            "Is the tip attached?",
            model(None),
            &[answer(Some([0.1, 0.1, 0.2, 0.4]))],
        )
        .await
        .unwrap();
    assert_eq!(appends.load(Ordering::SeqCst), 0);
    assert_ne!(
        TypeId::of::<ImageDecisionResult>(),
        TypeId::of::<a2a_lab_dev_kit::FrameDecisionRecord>()
    );
    let encoded = serde_json::to_value(resolved.result()).unwrap();
    assert!(encoded.get("predicate").is_none());
    assert!(encoded.get("decision_id").is_none());
    assert!(encoded.get("idempotency_key").is_none());

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
    let predicates = caps.evaluator().unwrap().supported_predicates();
    assert_eq!(predicates.len(), 3);
    assert!(predicates.iter().all(|predicate| predicate.version() == 1));
    assert_eq!(appends.load(Ordering::SeqCst), 0);

    let evaluator = FakeFrameDecisionEvaluator::new(
        config,
        FrameDecisionEvaluatorLimits::pilot(),
        FakeJevBehavior::OkPresent,
    );
    assert_eq!(evaluator.capabilities().supported_predicates().len(), 3);
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
}

struct CountingLedger {
    inner: FakeFrameDecisionLedgerProvider,
    appends: Arc<AtomicUsize>,
}

#[async_trait]
impl FrameDecisionLedgerProvider for CountingLedger {
    fn client_role(&self) -> a2a_lab_dev_kit::FrameDecisionClientRole {
        self.inner.client_role()
    }

    async fn get_frame_decision_capabilities(
        &self,
        request: GetFrameDecisionCapabilitiesRequest,
    ) -> Result<FrameDecisionCapabilities, A2aLabError> {
        self.inner.get_frame_decision_capabilities(request).await
    }

    async fn append_frame_decision(
        &self,
        request: AppendFrameDecisionRequest,
    ) -> Result<AppendFrameDecisionResult, A2aLabError> {
        self.appends.fetch_add(1, Ordering::SeqCst);
        self.inner.append_frame_decision(request).await
    }

    async fn read_frame_decision_history(
        &self,
        request: ReadFrameDecisionHistoryRequest,
    ) -> Result<FrameDecisionHistoryResult, A2aLabError> {
        self.inner.read_frame_decision_history(request).await
    }
}

struct Mismatch;

impl ImageProvider for Mismatch {
    fn list_image_sources(
        &self,
        _request: ListImageSourcesRequest,
    ) -> impl Future<Output = Result<Page<ImageSource>, A2aLabError>> + Send {
        std::future::ready(Ok(Page::new(Vec::new(), None)))
    }

    fn list_images(
        &self,
        _request: ListImagesRequest,
    ) -> impl Future<Output = Result<Page<ImageDescriptor>, A2aLabError>> + Send {
        std::future::ready(Ok(Page::new(Vec::new(), None)))
    }

    fn search_images(
        &self,
        _request: SearchImagesRequest,
    ) -> impl Future<Output = Result<Page<ImageDescriptor>, A2aLabError>> + Send {
        std::future::ready(Ok(Page::new(Vec::new(), None)))
    }

    fn get_image(
        &self,
        _request: GetImageRequest,
    ) -> impl Future<Output = Result<Image, A2aLabError>> + Send {
        std::future::ready(Ok(image("other", Some("tip"), &[1, 2, 3, 4])))
    }

    fn get_current_image(
        &self,
        request: GetCurrentImageRequest,
    ) -> impl Future<Output = Result<Image, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::not_found(
            "image source",
            request.source_id().as_str(),
        )))
    }
}
