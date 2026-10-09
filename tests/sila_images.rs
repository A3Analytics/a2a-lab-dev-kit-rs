#![cfg(feature = "sila2")]

use std::sync::Arc;

use a2a_lab_dev_kit::sila::api::{
    BinaryDownloadClient, BinaryUnion, Boolean, DeleteBinaryRequest, GetBinaryInfoRequest,
    GetChunkRequest, GetCurrentImageParameters, GetImageParameters, ImagePageRequest,
    ImagePageStruct, Integer, LabImagesClient, ListImageSourcesParameters, SilaString,
};
use a2a_lab_dev_kit::sila::{SilaIdentity, SilaServer};
use a2a_lab_dev_kit::{
    A2aLabService, Image, ImageDescriptor, ImageId, ImageSource, ImageSourceId, JsonObject,
    MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks, UtcTimestamp,
};
use futures_util::StreamExt;
use tonic::Code;

const CHUNK: u32 = 1024 * 1024;

#[tokio::test]
async fn sila_images_always_use_binary_download() {
    let images = MemoryImages::new();
    images
        .insert_source(ImageSource {
            id: ImageSourceId::new("deck").unwrap(),
            name: "Deck camera".to_owned(),
            description: "Overhead view of the deck".to_owned(),
            asset_id: Some("opentrons-ot2".to_owned()),
            semantic_id: None,
        })
        .await
        .unwrap();
    let small = b"small-frame".to_vec();
    images
        .insert_image(stored("frame-1", "deck", &small))
        .await
        .unwrap();
    let large = vec![7_u8; (2 * 1024 * 1024) + 8];
    images
        .insert_image(stored("frame-2", "deck", &large))
        .await
        .unwrap();
    images
        .set_current(
            ImageSourceId::new("deck").unwrap(),
            ImageId::new("frame-1").unwrap(),
        )
        .await
        .unwrap();

    let lab = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_images(images)
        .share();
    let server = SilaServer::new(
        SilaIdentity::lab_dev_kit("11111111-1111-1111-1111-111111111111").unwrap(),
        Arc::clone(&lab),
    )
    .plaintext()
    .serve("127.0.0.1:0".parse().unwrap())
    .await
    .unwrap();
    let channel = tonic::transport::Channel::from_shared(format!("http://{}", server.local_addr()))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut images = LabImagesClient::new(channel.clone());
    let listed = images
        .list_image_sources(ListImageSourcesParameters {
            page: Some(page(10)),
        })
        .await
        .unwrap()
        .into_inner();
    let source = listed.sources[0].image_source.as_ref().unwrap();
    assert_eq!(source.id.as_ref().unwrap().value, "deck");
    assert_eq!(
        source.description.as_ref().unwrap().value,
        "Overhead view of the deck"
    );

    let current = images
        .get_current_image(GetCurrentImageParameters {
            source_id: Some(text("deck")),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(current.image_id.unwrap().value, "frame-1");
    let small_id = transfer_id(current.image.unwrap());
    let mut downloads = BinaryDownloadClient::new(channel);
    assert_eq!(download(&mut downloads, &small_id).await, small);
    downloads
        .delete_binary(DeleteBinaryRequest {
            binary_transfer_uuid: small_id,
        })
        .await
        .unwrap();

    let fetched = images
        .get_image(GetImageParameters {
            image_id: Some(text("frame-2")),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(fetched.media_type.unwrap().value, "image/png");
    assert!(!fetched.has_caption.unwrap().value);
    let large_id = transfer_id(fetched.image.unwrap());
    assert_eq!(download(&mut downloads, &large_id).await, large);

    let missing = images
        .get_current_image(GetCurrentImageParameters {
            source_id: Some(text("missing")),
        })
        .await
        .unwrap_err();
    assert_eq!(missing.code(), Code::Aborted);
    let decoded = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        missing.message(),
    )
    .unwrap();
    let text = String::from_utf8_lossy(&decoded);
    assert!(text.contains("ImageSourceNotFound"));
}

fn stored(id: &str, source: &str, bytes: &[u8]) -> Image {
    Image::new(
        ImageDescriptor::new(
            ImageId::new(id).unwrap(),
            ImageSourceId::new(source).unwrap(),
            UtcTimestamp::parse("2024-01-01T00:30:00Z").unwrap(),
            "image/png",
            1,
            1,
            None,
            JsonObject::empty(),
        )
        .unwrap(),
        bytes.to_vec(),
    )
    .unwrap()
}

fn transfer_id(binary: a2a_lab_dev_kit::sila::api::Binary) -> String {
    match binary.union {
        Some(BinaryUnion::BinaryTransferUuid(id)) => id,
        other => panic!("expected a binary download identifier, got {other:?}"),
    }
}

async fn download(
    client: &mut BinaryDownloadClient<tonic::transport::Channel>,
    id: &str,
) -> Vec<u8> {
    let info = client
        .get_binary_info(GetBinaryInfoRequest {
            binary_transfer_uuid: id.to_owned(),
        })
        .await
        .unwrap()
        .into_inner();
    let mut offset = 0_u64;
    let mut bytes = Vec::new();
    while offset < info.binary_size {
        let length = u32::try_from(info.binary_size - offset)
            .unwrap_or(CHUNK)
            .min(CHUNK);
        let mut stream = client
            .get_chunk(futures_util::stream::iter([GetChunkRequest {
                binary_transfer_uuid: id.to_owned(),
                offset,
                length,
            }]))
            .await
            .unwrap()
            .into_inner();
        let chunk = stream.next().await.unwrap().unwrap();
        assert!(chunk.payload.len() <= CHUNK as usize);
        bytes.extend(chunk.payload);
        offset += u64::from(length);
    }
    bytes
}

fn page(limit: i64) -> ImagePageRequest {
    ImagePageRequest {
        page_request: Some(ImagePageStruct {
            has_cursor: Some(Boolean { value: false }),
            cursor: Some(SilaString {
                value: String::new(),
            }),
            limit: Some(Integer { value: limit }),
        }),
    }
}

fn text(value: &str) -> SilaString {
    SilaString {
        value: value.to_owned(),
    }
}
