use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabCommand, A2aLabService, A2aServer, GetCurrentImageRequest,
    GetImageRequest, Image, ImageDescriptor, ImageId, ImageSource, ImageSourceId,
    ImageTransportConfig, JsonObject, LAB_MEDIA_TYPE, ListImageSourcesRequest, ListImagesRequest,
    MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks, PageRequest, SearchImagesRequest,
    StreamResponse, UtcTimestamp, bind_local,
};
use serde_json::{Value, json};

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

fn page(limit: u32) -> PageRequest {
    PageRequest::new(None, limit).unwrap()
}

fn source(id: &str) -> ImageSource {
    ImageSource {
        id: ImageSourceId::new(id).unwrap(),
        name: id.to_owned(),
        description: format!("{id} frames"),
        asset_id: None,
        semantic_id: None,
    }
}

fn frame(id: &str, source_id: &str, stamp: &str, caption: Option<&str>, bytes: Vec<u8>) -> Image {
    Image::new(
        ImageDescriptor::new(
            ImageId::new(id).unwrap(),
            ImageSourceId::new(source_id).unwrap(),
            timestamp(stamp),
            "image/png",
            1,
            1,
            caption.map(str::to_owned),
            JsonObject::empty(),
        )
        .unwrap(),
        bytes,
    )
    .unwrap()
}

fn service(images: MemoryImages, transport: Option<ImageTransportConfig>) -> Arc<dyn A2aLabApi> {
    let mut built = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_images(images);
    if let Some(transport) = transport {
        built = built.with_image_transport(transport);
    }
    built.share()
}

async fn serve(service: Arc<dyn A2aLabApi>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service);
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

async fn catalog() -> (MemoryImages, Vec<u8>) {
    let images = MemoryImages::new();
    images.insert_source(source("zeta")).await.unwrap();
    images.insert_source(source("mid")).await.unwrap();
    images.insert_source(source("alpha")).await.unwrap();
    let bytes = vec![9, 8, 7, 6];
    for (id, source_id, stamp, caption, payload) in [
        (
            "c",
            "alpha",
            "2024-01-01T01:00:00Z",
            Some("Well"),
            vec![1, 2, 3, 4],
        ),
        (
            "a",
            "alpha",
            "2024-01-01T00:00:00Z",
            Some("Plate"),
            bytes.clone(),
        ),
        (
            "b",
            "alpha",
            "2024-01-01T00:30:00Z",
            Some("plate well"),
            vec![1, 2, 3, 4],
        ),
        (
            "m",
            "mid",
            "2024-01-01T00:15:00Z",
            Some("other"),
            vec![1, 2, 3, 4],
        ),
        (
            "z",
            "zeta",
            "2024-01-01T03:00:00Z",
            Some("Other"),
            vec![1, 2, 3, 4],
        ),
    ] {
        images
            .insert_image(frame(id, source_id, stamp, caption, payload))
            .await
            .unwrap();
    }
    images
        .set_current(
            ImageSourceId::new("alpha").unwrap(),
            ImageId::new("a").unwrap(),
        )
        .await
        .unwrap();
    (images, bytes)
}

fn ids(descriptors: &[ImageDescriptor]) -> Vec<&str> {
    descriptors
        .iter()
        .map(|descriptor| descriptor.id().as_str())
        .collect()
}

fn artifact_updates(events: &[StreamResponse]) -> Vec<(Value, Option<bool>)> {
    events
        .iter()
        .filter_map(|event| {
            let StreamResponse::ArtifactUpdate(update) = event else {
                return None;
            };
            Some((
                serde_json::to_value(&update.artifact).unwrap(),
                update.last_chunk,
            ))
        })
        .collect()
}

fn chunk_ids(events: &[StreamResponse]) -> Vec<String> {
    artifact_updates(events)
        .into_iter()
        .map(|(artifact, _)| {
            artifact["parts"][0]["data"]["result"]["items"][0]["id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

fn assert_final_chunk_only(events: &[StreamResponse]) {
    let updates = artifact_updates(events);
    assert!(updates.len() > 1);
    assert!(
        updates[..updates.len() - 1]
            .iter()
            .all(|(_, last)| *last != Some(true))
    );
    assert_eq!(updates.last().unwrap().1, Some(true));
}

fn assert_single_image_part(events: &[StreamResponse], image: &Image, operation: &str) {
    let updates = artifact_updates(events);
    assert_eq!(updates.len(), 1);
    let (artifact, last_chunk) = &updates[0];
    assert_eq!(*last_chunk, Some(true));
    let parts = artifact["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 1);
    let part = &parts[0];
    for key in ["text", "raw", "url", "file"] {
        assert!(part.get(key).is_none(), "{key} part on {operation}");
    }
    assert_eq!(part["mediaType"], LAB_MEDIA_TYPE);
    assert_eq!(part["data"]["operation"], operation);
    let encoded = part["data"]["result"]["data"].as_str().unwrap();
    let expected = serde_json::to_value(image).unwrap();
    assert_eq!(encoded, expected["data"].as_str().unwrap());
    assert!(part["data"]["result"].get("file").is_none());
}

#[tokio::test]
async fn image_operations_list_search_retrieve_and_page() {
    let (images, bytes) = catalog().await;
    let base = serve(service(images, None)).await;
    let client = A2aClient::new(&base).unwrap();
    page_sources_and_images(&client).await;
    search_images(&client).await;
    retrieve_images(&client, &bytes).await;
}

async fn page_sources_and_images(client: &A2aClient) {
    let sources = client
        .list_image_sources(ListImageSourcesRequest::new(page(2)).unwrap())
        .await
        .unwrap();
    let source_ids: Vec<_> = sources
        .items()
        .iter()
        .map(|source| source.id.as_str())
        .collect();
    assert_eq!(source_ids, ["alpha", "mid"]);
    let cursor = sources.next_cursor().unwrap().to_owned();
    let source_events = client
        .send_stream(A2aLabCommand::ListImageSources(
            ListImageSourcesRequest::new(page(2)).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(chunk_ids(&source_events), ["alpha", "mid"]);
    assert_final_chunk_only(&source_events);
    let rest = client
        .list_image_sources(
            ListImageSourcesRequest::new(PageRequest::new(Some(cursor), 2).unwrap()).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rest.items()[0].id.as_str(), "zeta");
    assert!(rest.next_cursor().is_none());

    let listed = client
        .list_images(ListImagesRequest::new(ImageSourceId::new("alpha").unwrap(), page(2)).unwrap())
        .await
        .unwrap();
    assert_eq!(ids(listed.items()), ["a", "b"]);
    let image_events = client
        .send_stream(A2aLabCommand::ListImages(
            ListImagesRequest::new(ImageSourceId::new("alpha").unwrap(), page(2)).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(chunk_ids(&image_events), ["a", "b"]);
    assert_final_chunk_only(&image_events);
    let image_cursor = listed.next_cursor().unwrap().to_owned();
    let listed_rest = client
        .list_images(
            ListImagesRequest::new(
                ImageSourceId::new("alpha").unwrap(),
                PageRequest::new(Some(image_cursor), 2).unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ids(listed_rest.items()), ["c"]);
}

async fn search_images(client: &A2aClient) {
    let by_range = client
        .search_images(
            SearchImagesRequest::new(
                Some(ImageSourceId::new("alpha").unwrap()),
                Some(
                    a2a_lab_dev_kit::TimeRange::new(
                        timestamp("2024-01-01T00:00:00Z"),
                        timestamp("2024-01-01T01:00:00Z"),
                    )
                    .unwrap(),
                ),
                None,
                page(10),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ids(by_range.items()), ["a", "b"]);
    let by_text = client
        .search_images(
            SearchImagesRequest::new(None, None, Some("WELL".to_owned()), page(10)).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ids(by_text.items()), ["b", "c"]);
    let search_events = client
        .send_stream(A2aLabCommand::SearchImages(
            SearchImagesRequest::new(None, None, Some("WELL".to_owned()), page(10)).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(chunk_ids(&search_events), ["b", "c"]);
    assert_final_chunk_only(&search_events);
}

async fn retrieve_images(client: &A2aClient, bytes: &[u8]) {
    let image = client
        .get_image(GetImageRequest::new(ImageId::new("a").unwrap()))
        .await
        .unwrap();
    assert_eq!(image.data(), bytes);
    assert_eq!(image.descriptor().source_id().as_str(), "alpha");
    let image_events = client
        .send_stream(A2aLabCommand::GetImage(GetImageRequest::new(
            ImageId::new("a").unwrap(),
        )))
        .await
        .unwrap();
    assert_single_image_part(&image_events, &image, "get_image");

    let current = client
        .get_current_image(GetCurrentImageRequest::new(
            ImageSourceId::new("alpha").unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(current.descriptor().id().as_str(), "a");
    assert_eq!(current.data(), bytes);
    let current_events = client
        .send_stream(A2aLabCommand::GetCurrentImage(GetCurrentImageRequest::new(
            ImageSourceId::new("alpha").unwrap(),
        )))
        .await
        .unwrap();
    assert_single_image_part(&current_events, &current, "get_current_image");
    let latest = client
        .get_current_image(GetCurrentImageRequest::new(
            ImageSourceId::new("zeta").unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(latest.descriptor().id().as_str(), "z");
}

#[tokio::test]
async fn image_operations_reject_malformed_missing_and_unavailable_input() {
    let (images, _) = catalog().await;
    let base = serve(service(images.clone(), None)).await;
    let client = A2aClient::new(&base).unwrap();

    for (id, data) in [
        (
            "bad-limit",
            json!({"operation": "list_image_sources", "params": {"page": {"limit": 0}}}),
        ),
        (
            "empty-search",
            json!({"operation": "search_images", "params": {"page": {"limit": 10}}}),
        ),
        (
            "blank-text",
            json!({"operation": "search_images", "params": {"text": " ", "page": {"limit": 10}}}),
        ),
    ] {
        let response = reqwest::Client::new()
            .post(format!("{base}/message:send"))
            .header("Content-Type", "application/a2a+json")
            .header("A2A-Version", "1.0")
            .json(&json!({
                "message": {
                    "messageId": id,
                    "role": "ROLE_USER",
                    "parts": [{
                        "mediaType": LAB_MEDIA_TYPE,
                        "data": data
                    }]
                }
            }))
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_client_error(),
            "{id} {}",
            response.status()
        );
        let body = response.text().await.unwrap();
        assert!(body.to_ascii_lowercase().contains("invalid"), "{id} {body}");
    }

    let missing_image = client
        .get_image(GetImageRequest::new(ImageId::new("missing").unwrap()))
        .await
        .unwrap_err();
    assert_eq!(missing_image.code(), "not_found");
    let missing_source = client
        .list_images(
            ListImagesRequest::new(ImageSourceId::new("missing").unwrap(), page(10)).unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(missing_source.code(), "not_found");
    let missing_current = client
        .get_current_image(GetCurrentImageRequest::new(
            ImageSourceId::new("missing").unwrap(),
        ))
        .await
        .unwrap_err();
    assert_eq!(missing_current.code(), "not_found");

    images.set_unavailable("images offline").await;
    let unavailable = client
        .list_image_sources(ListImageSourcesRequest::new(page(10)).unwrap())
        .await
        .unwrap_err();
    assert_eq!(unavailable.code(), "unavailable");
    assert!(unavailable.to_string().contains("images offline"));

    let empty = serve(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share(),
    )
    .await;
    let missing_provider = A2aClient::new(&empty)
        .unwrap()
        .search_images(
            SearchImagesRequest::new(None, None, Some("plate".to_owned()), page(10)).unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(missing_provider.code(), "unavailable");
}

#[tokio::test]
async fn four_k_rgba_round_trip_honors_client_and_server_limits() {
    let mut payload = vec![0_u8; 3840 * 2160 * 4];
    payload[0] = 0x11;
    payload[1] = 0x22;
    let last = payload.len() - 1;
    payload[last] = 0x33;
    let decoded_len = u64::try_from(payload.len()).unwrap();
    let images = MemoryImages::new();
    images.insert_source(source("cam")).await.unwrap();
    images
        .insert_image(
            Image::new(
                ImageDescriptor::new(
                    ImageId::new("frame").unwrap(),
                    ImageSourceId::new("cam").unwrap(),
                    timestamp("2024-01-01T00:00:00Z"),
                    "image/png",
                    3840,
                    2160,
                    None,
                    JsonObject::empty(),
                )
                .unwrap(),
                payload.clone(),
            )
            .unwrap(),
        )
        .await
        .unwrap();

    let base = serve(service(images.clone(), None)).await;
    let image = A2aClient::new(&base)
        .unwrap()
        .get_image(GetImageRequest::new(ImageId::new("frame").unwrap()))
        .await
        .unwrap();
    assert!(
        image.data() == payload.as_slice(),
        "default limit dropped image bytes"
    );

    let below = ImageTransportConfig::new(decoded_len - 1).unwrap();
    let client_limited = A2aClient::new(&base)
        .unwrap()
        .with_image_transport(below)
        .get_image(GetImageRequest::new(ImageId::new("frame").unwrap()))
        .await
        .unwrap_err();
    assert_eq!(client_limited.code(), "invalid");
    assert!(client_limited.to_string().contains("maximum"));

    let tight = serve(service(images.clone(), Some(below))).await;
    let server_limited = A2aClient::new(&tight)
        .unwrap()
        .get_image(GetImageRequest::new(ImageId::new("frame").unwrap()))
        .await
        .unwrap_err();
    assert_eq!(server_limited.code(), "invalid");
    assert!(server_limited.to_string().contains("maximum"));

    let raised = ImageTransportConfig::new(decoded_len).unwrap();
    let raised_base = serve(service(images, Some(raised))).await;
    let raised_image = A2aClient::new(&raised_base)
        .unwrap()
        .with_image_transport(raised)
        .get_image(GetImageRequest::new(ImageId::new("frame").unwrap()))
        .await
        .unwrap();
    assert!(
        raised_image.data() == payload.as_slice(),
        "raised limit dropped image bytes"
    );
}
