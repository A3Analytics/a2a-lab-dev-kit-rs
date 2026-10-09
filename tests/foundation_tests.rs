use base64::Engine;

use a2a_lab_dev_kit::{
    A2aLabError, DEFAULT_MAX_IMAGE_BYTES, GetCurrentImageRequest, GetImageRequest, Image,
    ImageDescriptor, ImageId, ImageProvider, ImageSource, ImageSourceId, ImageTransportConfig,
    JsonObject, ListImageSourcesRequest, ListImagesRequest, MAX_PAGE_LIMIT, MetricPoint, Page,
    PageRequest, SearchImagesRequest, SourceId, TimeRange, UtcTimestamp, version,
};

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).expect("timestamp")
}

#[test]
fn reports_package_version() {
    assert_eq!(version(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn rejects_malformed_identifiers() {
    assert!(SourceId::new("").is_err());
    assert!(SourceId::new("has space").is_err());
    assert!(SourceId::new("a/b").is_err());
    assert!(SourceId::new("x".repeat(129)).is_err());
    assert_eq!(SourceId::new("app.log_1").unwrap().as_str(), "app.log_1");
}

#[test]
fn accepts_only_utc_timestamps() {
    assert!(UtcTimestamp::parse("2024-01-01T00:00:00+01:00").is_err());
    assert!(UtcTimestamp::parse("2024-01-01T00:00:00").is_err());
    let parsed = timestamp("2024-01-01T00:00:00Z");
    assert_eq!(timestamp("2024-01-01T00:00:00+00:00"), parsed);
    assert_eq!(parsed.to_rfc3339(), "2024-01-01T00:00:00Z");
}

#[test]
fn rejects_empty_or_reversed_ranges_and_uses_half_open_bounds() {
    let start = timestamp("2024-01-01T00:00:00Z");
    let end = timestamp("2024-01-01T01:00:00Z");
    assert!(TimeRange::new(start, start).is_err());
    assert!(TimeRange::new(end, start).is_err());
    let range = TimeRange::new(start, end).unwrap();
    assert!(range.contains(start));
    assert!(!range.contains(end));
}

#[test]
fn rejects_invalid_pages_and_omits_the_last_cursor() {
    assert!(PageRequest::new(None, 0).is_err());
    assert!(PageRequest::new(None, MAX_PAGE_LIMIT + 1).is_err());
    assert!(PageRequest::new(Some("next".to_owned()), 10).is_err());
    let request = PageRequest::new(Some("2".to_owned()), 2).unwrap();
    assert_eq!(request.cursor(), Some("2"));
    assert_eq!(request.limit(), 2);
}

#[test]
fn rejects_non_object_task_input() {
    assert!(JsonObject::parse("[1]").is_err());
    assert!(JsonObject::parse("\"text\"").is_err());
    assert!(JsonObject::parse("{").is_err());
    let object = JsonObject::parse(r#"{"branch":"main"}"#).unwrap();
    assert_eq!(
        object
            .as_map()
            .get("branch")
            .and_then(serde_json::Value::as_str),
        Some("main")
    );
}

#[test]
fn rejects_non_finite_metric_values() {
    let instant = timestamp("2024-01-01T00:00:00Z");
    assert!(MetricPoint::new(instant, f64::NAN).is_err());
    let error =
        serde_json::from_str::<MetricPoint>(r#"{"timestamp":"2024-01-01T00:00:00Z","value":null}"#);
    assert!(error.is_err());
    let point = MetricPoint::new(instant, 1.5).unwrap();
    let decoded: MetricPoint =
        serde_json::from_str(&serde_json::to_string(&point).unwrap()).unwrap();
    assert_eq!(decoded, point);
}

#[test]
fn image_ids_match_the_public_identifier_rules() {
    assert!(ImageId::new("").is_err());
    assert!(ImageId::new("has space").is_err());
    assert!(ImageSourceId::new("a/b").is_err());
    assert!(ImageSourceId::new("x".repeat(129)).is_err());
    assert_eq!(ImageId::new("cam.frame_1").unwrap().as_str(), "cam.frame_1");
    assert!(serde_json::from_str::<GetImageRequest>(r#"{"id":"bad id"}"#).is_err());
    assert!(serde_json::from_str::<GetCurrentImageRequest>(r#"{"source_id":""}"#).is_err());
}

#[test]
fn image_sources_round_trip_and_descriptors_omit_bytes() {
    let source = ImageSource {
        id: ImageSourceId::new("cam").unwrap(),
        name: "Camera".to_owned(),
        description: "Bench camera".to_owned(),
        asset_id: None,
        semantic_id: None,
    };
    let value = serde_json::to_value(&source).unwrap();
    assert!(value.get("kind").is_none());
    assert_eq!(value["description"], "Bench camera");
    let restored: ImageSource = serde_json::from_value(value).unwrap();
    assert_eq!(restored, source);

    let descriptor = sample_descriptor();
    let value = serde_json::to_value(&descriptor).unwrap();
    assert!(value.get("data").is_none());
    assert_eq!(value["media_type"], "image/png");
    let mut with_payload = value.clone();
    with_payload["data"] = serde_json::Value::String("YQ==".to_owned());
    assert!(serde_json::from_value::<ImageDescriptor>(with_payload).is_err());
}

#[test]
fn image_bytes_round_trip_as_base64() {
    let bytes = vec![0xff, 0x00, 0x10, b'a'];
    let image = Image::new(sample_descriptor(), bytes.clone()).unwrap();
    let value = serde_json::to_value(&image).unwrap();
    let encoded = value["data"].as_str().expect("base64 string");
    assert!(value["data"].is_string());
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    assert_eq!(decoded, bytes);
    let restored: Image = serde_json::from_value(value).unwrap();
    assert_eq!(restored.data(), bytes);
    assert_eq!(restored.descriptor(), image.descriptor());
}

#[test]
fn image_requests_reject_invalid_pages() {
    let page = serde_json::from_str::<PageRequest>(r#"{"limit":0}"#).unwrap();
    assert!(ListImageSourcesRequest::new(page).is_err());
    let page = serde_json::from_str::<PageRequest>(r#"{"cursor":"next","limit":10}"#).unwrap();
    assert!(ListImagesRequest::new(ImageSourceId::new("cam").unwrap(), page).is_err());
}

#[test]
fn image_search_requires_a_half_open_range_or_text() {
    let source = ImageSourceId::new("cam").unwrap();
    let valid_page = PageRequest::new(None, 10).unwrap();
    assert!(SearchImagesRequest::new(None, None, None, valid_page.clone()).is_err());
    assert!(
        SearchImagesRequest::new(Some(source.clone()), None, None, valid_page.clone()).is_err()
    );
    assert!(
        SearchImagesRequest::new(None, None, Some("   ".to_owned()), valid_page.clone()).is_err()
    );
    let reversed = serde_json::from_str::<TimeRange>(
        r#"{"start":"2024-01-01T01:00:00Z","end":"2024-01-01T00:00:00Z"}"#,
    )
    .unwrap();
    assert!(
        SearchImagesRequest::new(
            None,
            Some(reversed),
            Some("cell".to_owned()),
            valid_page.clone()
        )
        .is_err()
    );

    let start = timestamp("2024-01-01T00:00:00Z");
    let end = timestamp("2024-01-01T01:00:00Z");
    let range = TimeRange::new(start, end).unwrap();
    let request = SearchImagesRequest::new(
        Some(source),
        Some(range),
        Some("plate".to_owned()),
        valid_page,
    )
    .unwrap();
    assert!(request.range().unwrap().contains(start));
    assert!(!request.range().unwrap().contains(end));
    assert_eq!(request.text(), Some("plate"));
    assert_eq!(request.source_id().map(ImageSourceId::as_str), Some("cam"));
    let text_only: SearchImagesRequest =
        serde_json::from_str(r#"{"text":"cell","page":{"limit":5}}"#).unwrap();
    assert_eq!(text_only.text(), Some("cell"));
    assert!(text_only.range().is_none());
    assert!(text_only.source_id().is_none());
    assert!(
        serde_json::from_str::<SearchImagesRequest>(r#"{"source_id":"cam","page":{"limit":5}}"#)
            .is_err()
    );
}

#[test]
fn current_image_retrieval_names_a_source() {
    let by_id = serde_json::to_value(GetImageRequest::new(ImageId::new("frame").unwrap())).unwrap();
    let current = serde_json::to_value(GetCurrentImageRequest::new(
        ImageSourceId::new("cam").unwrap(),
    ))
    .unwrap();
    assert!(by_id.get("id").is_some());
    assert!(by_id.get("source_id").is_none());
    assert!(current.get("source_id").is_some());
    assert!(current.get("id").is_none());
}

#[test]
fn images_reject_bad_media_dimensions_base64_and_empty_payloads() {
    assert!(sample_descriptor_with("").is_err());
    assert!(sample_descriptor_with("text/plain").is_err());
    assert!(sample_descriptor_with("image/").is_err());
    assert!(descriptor_sized(0, 1).is_err());
    assert!(descriptor_sized(2, 0).is_err());

    let mut image =
        serde_json::to_value(Image::new(sample_descriptor(), b"png".to_vec()).unwrap()).unwrap();
    image["descriptor"]["media_type"] = serde_json::json!("");
    assert!(serde_json::from_value::<Image>(image.clone()).is_err());
    image["descriptor"]["media_type"] = serde_json::json!("text/plain");
    assert!(serde_json::from_value::<Image>(image.clone()).is_err());
    image["descriptor"]["media_type"] = serde_json::json!("image/png");
    image["descriptor"]["width"] = serde_json::json!(0);
    assert!(serde_json::from_value::<Image>(image.clone()).is_err());
    image["descriptor"]["width"] = serde_json::json!(1);
    image["data"] = serde_json::json!("****");
    assert!(serde_json::from_value::<Image>(image.clone()).is_err());
    image["data"] = serde_json::json!("");
    let inconsistent = serde_json::from_value::<Image>(image).unwrap_err();
    assert!(inconsistent.to_string().contains("inconsistent"));
    let empty = Image::new(sample_descriptor(), Vec::new()).unwrap_err();
    assert!(empty.to_string().contains("inconsistent"));
}

#[test]
fn transport_config_bounds_a_generated_4k_rgba_payload() {
    assert!(ImageTransportConfig::new(0).is_err());
    assert!(ImageTransportConfig::new(u64::MAX).is_err());
    assert!(ImageTransportConfig::new(128 * 1024 * 1024).is_ok());

    let payload = vec![0u8; 3840 * 2160 * 4];
    let config = ImageTransportConfig::default();
    assert_eq!(config.max_image_bytes(), DEFAULT_MAX_IMAGE_BYTES);
    assert_eq!(DEFAULT_MAX_IMAGE_BYTES, 64 * 1024 * 1024);
    assert!(config.check_payload(&payload).is_ok());

    let decoded_len = u64::try_from(payload.len()).unwrap();
    let below = ImageTransportConfig::new(decoded_len - 1).unwrap();
    assert!(below.check_payload(&payload).is_err());
    let raised = ImageTransportConfig::new(below.max_image_bytes() + 1).unwrap();
    assert!(raised.check_payload(&payload).is_ok());

    let descriptor = sample_descriptor();
    assert!(Image::with_transport(descriptor.clone(), payload.clone(), below).is_err());
    assert!(Image::new(descriptor.clone(), payload.clone()).is_ok());
    assert!(Image::with_transport(descriptor, payload, raised).is_ok());
}

#[tokio::test]
async fn image_provider_exposes_the_five_operations() {
    let provider = ContractImages;
    let page = PageRequest::new(None, 10).unwrap();
    let sources = provider
        .list_image_sources(ListImageSourcesRequest::new(page.clone()).unwrap())
        .await
        .unwrap();
    assert!(sources.items().is_empty());
    let source_id = ImageSourceId::new("cam").unwrap();
    let listed = provider
        .list_images(ListImagesRequest::new(source_id.clone(), page.clone()).unwrap())
        .await
        .unwrap();
    assert!(listed.items().is_empty());
    let searched = provider
        .search_images(
            SearchImagesRequest::new(
                Some(source_id.clone()),
                None,
                Some("plate".to_owned()),
                page,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(searched.items().is_empty());
    let missing = provider
        .get_image(GetImageRequest::new(ImageId::new("frame").unwrap()))
        .await
        .unwrap_err();
    assert_eq!(missing, A2aLabError::not_found("image", "frame"));
    let current = provider
        .get_current_image(GetCurrentImageRequest::new(source_id))
        .await
        .unwrap_err();
    assert_eq!(current, A2aLabError::not_found("image source", "cam"));
}

fn sample_descriptor() -> ImageDescriptor {
    sample_descriptor_with("image/png").unwrap()
}

fn sample_descriptor_with(media_type: &str) -> Result<ImageDescriptor, A2aLabError> {
    descriptor(media_type, 2, 2)
}

fn descriptor_sized(width: u32, height: u32) -> Result<ImageDescriptor, A2aLabError> {
    descriptor("image/png", width, height)
}

fn descriptor(media_type: &str, width: u32, height: u32) -> Result<ImageDescriptor, A2aLabError> {
    ImageDescriptor::new(
        ImageId::new("frame").unwrap(),
        ImageSourceId::new("cam").unwrap(),
        timestamp("2024-01-01T00:00:00Z"),
        media_type,
        width,
        height,
        Some("plate".to_owned()),
        JsonObject::empty(),
    )
}

struct ContractImages;

impl ImageProvider for ContractImages {
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
        request: GetImageRequest,
    ) -> impl Future<Output = Result<Image, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::not_found("image", request.id().as_str())))
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

#[test]
fn error_codes_round_trip() {
    let error = A2aLabError::not_found("log source", "app");
    assert_eq!(error.code(), "not_found");
    assert_eq!(
        A2aLabError::from_code(error.code(), error.to_string()).code(),
        "not_found"
    );
}
