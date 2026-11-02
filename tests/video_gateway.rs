#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aLabApi, A2aLabResult, A2aLabService, A2aServer, DEFAULT_MJPEG_BOUNDARY, DvrMediaPackager,
    DvrPackagerConfig, DvrTrackId, FollowVideoRequest, GetVideoClipRequest, HLS_INIT_RELATIVE_PATH,
    HLS_PLAYLIST_RELATIVE_PATH, HttpAuthSecurityScheme, LocalMediaAssetProvider,
    MediaAssetProvider, MemoryLogs, MemoryMetrics, MemoryTasks, MemoryVideos, SecurityScheme,
    TimeRange, UtcTimestamp, VideoClipId, VideoProvider, VideoSource, VideoSourceId, bind_local,
    decode_mjpeg_multipart, router,
};

#[path = "support/dvr_store_contract.rs"]
mod dvr_store_contract;

#[path = "support/fake_remote_media.rs"]
mod fake_remote_media;

use dvr_store_contract::FakeDvrMediaStore;
use fake_remote_media::{FakeRemoteAsset, FakeRemoteMediaAssetProvider};

const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xD9];

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

fn source_id(id: &str) -> VideoSourceId {
    VideoSourceId::new(id).unwrap()
}

fn range(start: &str, end: &str) -> TimeRange {
    TimeRange::new(timestamp(start), timestamp(end)).unwrap()
}

fn share(videos: MemoryVideos) -> Arc<dyn A2aLabApi> {
    A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos)
        .share()
}

async fn listen(assets: Arc<dyn MediaAssetProvider>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router(assets)).await.unwrap();
    });
    format!("http://{address}")
}

async fn listen_auth(lab: Arc<dyn A2aLabApi>, assets: Arc<dyn MediaAssetProvider>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&lab)
        .with_security(
            [(
                "bearerAuth".to_owned(),
                SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                    scheme: "bearer".to_owned(),
                    description: None,
                    bearer_format: None,
                }),
            )]
            .into(),
            vec![[("bearerAuth".to_owned(), Vec::new())].into()],
        )
        .with_media_assets(assets);
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

fn header(response: &reqwest::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned()
}

fn assert_media_headers(response: &reqwest::Response) {
    assert_eq!(header(response, "cache-control"), "no-store");
    assert_eq!(header(response, "x-content-type-options"), "nosniff");
}

async fn prepare_mjpeg_clip(videos: &MemoryVideos) -> (VideoClipId, String) {
    videos
        .insert_source(
            VideoSource::new_mjpeg(source_id("cam"), "Cam", "bench", None, None, 640, 480, 30)
                .unwrap(),
        )
        .await
        .unwrap();
    for second in 0..2 {
        videos
            .insert_frame(
                source_id("cam"),
                timestamp(&format!("2024-01-01T00:00:0{second}Z")),
                JPEG.to_vec(),
            )
            .await
            .unwrap();
    }
    let clip = videos
        .get_video_clip(
            GetVideoClipRequest::new(
                source_id("cam"),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    (clip.id().clone(), clip.path().to_owned())
}

#[tokio::test]
async fn local_gateway_serves_packaged_hls_assets_with_content_types() {
    let videos = MemoryVideos::new();
    let (clip_id, root) = prepare_mjpeg_clip(&videos).await;
    let lab = share(videos);
    let assets = Arc::new(LocalMediaAssetProvider::new(Arc::clone(&lab)));
    let dvr = FakeDvrMediaStore::gapped_h264();
    let packager = DvrMediaPackager::new(DvrPackagerConfig::new(120_000_000).unwrap());
    let session = packager
        .package_hls_at(
            &dvr,
            &root,
            &source_id("cam"),
            &DvrTrackId::new("h264").unwrap(),
            timestamp("2024-01-01T00:00:00Z"),
        )
        .await
        .unwrap();
    assets
        .register_hls_clip(clip_id, session, &dvr)
        .await
        .unwrap();

    let base = listen(assets).await;
    let playlist = reqwest::Client::new()
        .get(format!("{base}{root}{HLS_PLAYLIST_RELATIVE_PATH}"))
        .send()
        .await
        .unwrap();
    assert_media_headers(&playlist);
    assert_eq!(
        header(&playlist, "content-type"),
        "application/vnd.apple.mpegurl"
    );
    let init = reqwest::Client::new()
        .get(format!("{base}{root}{HLS_INIT_RELATIVE_PATH}"))
        .send()
        .await
        .unwrap();
    assert_media_headers(&init);
    assert_eq!(header(&init, "content-type"), "video/mp4");
}

#[tokio::test]
async fn mjpeg_root_has_closed_boundary_through_gateway() {
    let videos = MemoryVideos::new();
    let (_id, root) = prepare_mjpeg_clip(&videos).await;
    let assets = Arc::new(LocalMediaAssetProvider::new(share(videos)));
    let base = listen(assets).await;
    let response = reqwest::Client::new()
        .get(format!("{base}{root}"))
        .send()
        .await
        .unwrap();
    assert_media_headers(&response);
    let body = response.bytes().await.unwrap();
    assert!(String::from_utf8_lossy(&body).ends_with(&format!("--{DEFAULT_MJPEG_BOUNDARY}--\r\n")));
    let frames = decode_mjpeg_multipart(&body, DEFAULT_MJPEG_BOUNDARY).unwrap();
    assert_eq!(frames.len(), 2);
}

#[tokio::test]
async fn gateway_rejects_traversal_credential_queries_and_unknown_assets() {
    let videos = MemoryVideos::new();
    let (clip_id, root) = prepare_mjpeg_clip(&videos).await;
    let assets = Arc::new(LocalMediaAssetProvider::new(share(videos)));
    let base = listen(assets).await;
    let traversal = reqwest::Client::new()
        .get(format!("{base}{root}../other/init.mp4"))
        .send()
        .await
        .unwrap();
    assert_eq!(traversal.status(), 404);
    assert_media_headers(&traversal);

    let credential = reqwest::Client::new()
        .get(format!("{base}{root}?access_token=secret"))
        .send()
        .await
        .unwrap();
    assert_eq!(credential.status(), 400);
    assert_media_headers(&credential);
    let body = credential.text().await.unwrap();
    assert!(!body.contains("secret"));
    assert!(!body.contains("vault.internal"));

    let unknown = reqwest::Client::new()
        .get(format!("{base}{root}missing.m4s"))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), 404);
    assert_media_headers(&unknown);

    let absolute = reqwest::Client::new()
        .get(format!(
            "{base}/media/video-clips/{clip_id}/%2Fetc%2Fpasswd"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(absolute.status(), 400);
    assert_media_headers(&absolute);
}

#[tokio::test]
async fn auth_gated_responses_include_media_security_headers() {
    let videos = MemoryVideos::new();
    let (_id, root) = prepare_mjpeg_clip(&videos).await;
    let lab = share(videos);
    let assets =
        Arc::new(LocalMediaAssetProvider::new(Arc::clone(&lab))) as Arc<dyn MediaAssetProvider>;
    let base = listen_auth(lab, assets).await;
    let denied = reqwest::Client::new()
        .get(format!("{base}{root}"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
    assert_media_headers(&denied);
    let ok = reqwest::Client::new()
        .get(format!("{base}{root}"))
        .bearer_auth("lab-token")
        .send()
        .await
        .unwrap();
    assert_media_headers(&ok);
}

#[tokio::test]
async fn fake_remote_provider_proxies_without_upstream_leaks() {
    let remote = Arc::new(FakeRemoteMediaAssetProvider::new());
    let clip_id = VideoClipId::new("remote-clip").unwrap();
    let root = format!("/media/video-clips/{clip_id}/");
    let mut assets = HashMap::new();
    assets.insert(
        HLS_PLAYLIST_RELATIVE_PATH.to_owned(),
        FakeRemoteAsset::new(
            "application/vnd.apple.mpegurl",
            b"#EXTM3U\n#EXT-X-VERSION:7\n".to_vec(),
        )
        .unwrap(),
    );
    assets.insert(
        HLS_INIT_RELATIVE_PATH.to_owned(),
        FakeRemoteAsset::new("video/mp4", vec![0, 0, 0, 0x18, b'f', b't', b'y', b'p', 1]).unwrap(),
    );
    remote.bind_clip(clip_id, assets).unwrap();

    let base = listen(remote).await;
    let playlist = reqwest::Client::new()
        .get(format!("{base}{root}{HLS_PLAYLIST_RELATIVE_PATH}"))
        .send()
        .await
        .unwrap();
    assert_media_headers(&playlist);
    let text = playlist.text().await.unwrap();
    assert!(text.starts_with("#EXTM3U"));
    assert!(!text.contains("vault.internal"));

    let missing = reqwest::Client::new()
        .get(format!("{base}{root}seg-9999.m4s"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
    let message = missing.text().await.unwrap();
    assert!(!message.contains("vault.internal"));
    assert!(!message.contains("9999/private"));
}

#[tokio::test]
async fn local_and_remote_share_error_and_header_behavior() {
    let local = Arc::new(LocalMediaAssetProvider::new(share(MemoryVideos::new())))
        as Arc<dyn MediaAssetProvider>;
    let remote = Arc::new(FakeRemoteMediaAssetProvider::new()) as Arc<dyn MediaAssetProvider>;
    for (label, assets) in [("local", local), ("remote", remote)] {
        let base = listen(assets).await;
        let response = reqwest::Client::new()
            .get(format!("{base}/media/video-clips/missing/"))
            .send()
            .await
            .unwrap();
        assert_media_headers(&response);
        assert_eq!(response.status(), 404, "{label}");
        let credential = reqwest::Client::new()
            .get(format!("{base}/media/video-clips/missing/?token=secret"))
            .send()
            .await
            .unwrap();
        assert_media_headers(&credential);
        assert_eq!(credential.status(), 400, "{label}");
    }
}

#[tokio::test]
async fn expired_clip_is_rejected_before_bytes() {
    let videos = MemoryVideos::with_clip_ttl(Duration::from_millis(1));
    let (_id, root) = prepare_mjpeg_clip(&videos).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    let assets = Arc::new(LocalMediaAssetProvider::new(share(videos)));
    let base = listen(assets).await;
    let response = reqwest::Client::new()
        .get(format!("{base}{root}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    assert_media_headers(&response);
}

#[tokio::test]
async fn live_follow_session_mismatch_and_double_open_are_rejected() {
    let videos = MemoryVideos::new();
    videos
        .insert_source(
            VideoSource::new_mjpeg(source_id("cam"), "Cam", "bench", None, None, 640, 480, 30)
                .unwrap(),
        )
        .await
        .unwrap();
    videos
        .set_live_available(&source_id("cam"), true)
        .await
        .unwrap();
    let lab = share(videos);
    let followed = lab
        .execute(a2a_lab_dev_kit::A2aLabCommand::FollowVideo(
            FollowVideoRequest::new(source_id("cam")),
        ))
        .await
        .unwrap();
    let A2aLabResult::FollowVideo(handle) = followed.task.result else {
        panic!("follow");
    };
    let assets = Arc::new(LocalMediaAssetProvider::new(lab));
    let base = listen(assets).await;
    let first = reqwest::Client::new()
        .get(format!("{base}{}", handle.path()))
        .send()
        .await
        .unwrap();
    assert!(first.status().is_success());
    drop(first);
    let second = reqwest::Client::new()
        .get(format!("{base}{}", handle.path()))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 400);
    assert_media_headers(&second);

    let mismatch = reqwest::Client::new()
        .get(format!(
            "{base}/media/video-clips/{}/index.m3u8",
            handle.id()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(mismatch.status(), 404);
    assert_media_headers(&mismatch);
}
