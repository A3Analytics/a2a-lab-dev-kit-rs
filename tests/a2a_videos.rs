use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabCommand, A2aLabError, A2aLabService, A2aServer,
    DEFAULT_MJPEG_BOUNDARY, FollowVideoRequest, GetVideoClipRequest, GetVideoFrameAtRequest,
    GetVideoFrameAtResult, GetVideoTimelineRequest, HttpAuthSecurityScheme, LAB_MEDIA_TYPE,
    ListVideoSourcesRequest, MJPEG_MEDIA_TYPE, MemoryLogs, MemoryMetrics, MemoryTasks,
    MemoryVideos, Page, PageRequest, SecurityScheme, StreamResponse, TimeRange, UtcTimestamp,
    VIDEO_CLIP_PATH_PREFIX, VIDEO_LIVE_PATH_PREFIX, VideoClip, VideoClipId, VideoClipStream,
    VideoFollow, VideoFollowId, VideoFrame, VideoLiveStream, VideoProvider, VideoSource,
    VideoSourceId, bind_local, decode_mjpeg_multipart, video_clip_path, video_live_path,
};
use a2a_types::PartContent;
use futures_util::StreamExt;
use serde_json::json;

const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
];

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

fn page(limit: u32) -> PageRequest {
    PageRequest::new(None, limit).unwrap()
}

fn range(start: &str, end: &str) -> TimeRange {
    TimeRange::new(timestamp(start), timestamp(end)).unwrap()
}

fn source(id: &str) -> VideoSource {
    VideoSource::new(
        VideoSourceId::new(id).unwrap(),
        id,
        format!("{id} recording"),
        None,
        None,
        1,
        1,
        1,
        MJPEG_MEDIA_TYPE,
    )
    .unwrap()
}

async fn catalog() -> MemoryVideos {
    let videos = MemoryVideos::new();
    videos.insert_source(source("zeta")).await.unwrap();
    videos.insert_source(source("cam")).await.unwrap();
    for at in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:02Z",
    ] {
        videos
            .insert_frame(
                VideoSourceId::new("cam").unwrap(),
                timestamp(at),
                JPEG.to_vec(),
            )
            .await
            .unwrap();
    }
    videos
}

fn service(videos: MemoryVideos) -> Arc<dyn A2aLabApi> {
    A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos)
        .share()
}

async fn serve(lab: Arc<dyn A2aLabApi>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&lab);
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

#[tokio::test]
async fn video_operations_list_frame_clip_and_download() {
    let videos = catalog().await;
    let base = serve(service(videos)).await;
    let client = A2aClient::new(&base).unwrap();
    let sources = client
        .list_video_sources(ListVideoSourcesRequest::new(page(10)).unwrap())
        .await
        .unwrap();
    assert_eq!(
        sources
            .items()
            .iter()
            .map(|source| source.id().as_str())
            .collect::<Vec<_>>(),
        ["cam", "zeta"]
    );
    let events = client
        .send_stream(A2aLabCommand::ListVideoSources(
            ListVideoSourcesRequest::new(page(10)).unwrap(),
        ))
        .await
        .unwrap();
    assert!(events.len() > 1);

    let frame = client
        .get_video_frame_at(GetVideoFrameAtRequest::new(
            VideoSourceId::new("cam").unwrap(),
            timestamp("2024-01-01T00:00:00Z"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(frame.delta_us(), 0);
    assert_eq!(frame.image().data(), JPEG);
    let frame_json = serde_json::to_value(&frame).unwrap();
    assert!(frame_json.get("path").is_none());

    let clip = client
        .get_video_clip(
            GetVideoClipRequest::new(
                VideoSourceId::new("cam").unwrap(),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(clip.path(), video_clip_path(clip.id()));
    let encoded = serde_json::to_value(&clip).unwrap();
    assert!(encoded.get("data").is_none());
    assert!(encoded.get("jpeg").is_none());
    assert!(!encoded.to_string().contains("data:video"));

    let mut stream = client.open_video_clip(clip.id()).await.unwrap();
    let mut stamps = Vec::new();
    while let Some(frame) = stream.next().await {
        stamps.push(frame.unwrap().captured_at().to_rfc3339());
    }
    assert_eq!(stamps, ["2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"]);
}

#[tokio::test]
async fn video_operations_reject_malformed_missing_and_unavailable_input() {
    let videos = catalog().await;
    let base = serve(service(videos.clone())).await;
    let client = A2aClient::new(&base).unwrap();
    let response = reqwest::Client::new()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", "1.0")
        .json(&json!({
            "message": {
                "messageId": "bad-limit",
                "role": "ROLE_USER",
                "parts": [{
                    "mediaType": LAB_MEDIA_TYPE,
                    "data": {"operation": "list_video_sources", "params": {"page": {"limit": 0}}}
                }]
            }
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_client_error());
    assert!(
        response
            .text()
            .await
            .unwrap()
            .to_ascii_lowercase()
            .contains("invalid")
    );

    let missing_source = client
        .get_video_frame_at(GetVideoFrameAtRequest::new(
            VideoSourceId::new("missing").unwrap(),
            timestamp("2024-01-01T00:00:00Z"),
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(missing_source.code(), "not_found");
    let missing_frame = client
        .get_video_frame_at(GetVideoFrameAtRequest::new(
            VideoSourceId::new("cam").unwrap(),
            timestamp("2025-01-01T00:00:00Z"),
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(missing_frame.code(), "not_found");
    let missing_clip = client
        .get_video_clip(
            GetVideoClipRequest::new(
                VideoSourceId::new("cam").unwrap(),
                range("2025-01-01T00:00:00Z", "2025-01-01T00:00:01Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(missing_clip.code(), "no_recording");

    videos.set_unavailable("videos offline").await;
    let unavailable = client
        .list_video_sources(ListVideoSourcesRequest::new(page(10)).unwrap())
        .await
        .unwrap_err();
    assert_eq!(unavailable.code(), "unavailable");

    let empty = serve(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share(),
    )
    .await;
    let missing_provider = A2aClient::new(&empty)
        .unwrap()
        .list_video_sources(ListVideoSourcesRequest::new(page(10)).unwrap())
        .await
        .unwrap_err();
    assert_eq!(missing_provider.code(), "unavailable");
}

#[tokio::test]
async fn video_operations_require_bearer_when_secured() {
    let base = serve_auth(service(catalog().await)).await;
    let anonymous = A2aClient::new(&base).unwrap();
    let list = anonymous
        .list_video_sources(ListVideoSourcesRequest::new(page(10)).unwrap())
        .await
        .unwrap_err();
    assert_auth_rejected(&list, "list_video_sources");
    let frame = anonymous
        .get_video_frame_at(GetVideoFrameAtRequest::new(
            VideoSourceId::new("cam").unwrap(),
            timestamp("2024-01-01T00:00:00Z"),
            None,
        ))
        .await
        .unwrap_err();
    assert_auth_rejected(&frame, "get_video_frame_at");
    let clip = anonymous
        .get_video_clip(
            GetVideoClipRequest::new(
                VideoSourceId::new("cam").unwrap(),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert_auth_rejected(&clip, "get_video_clip");

    let granted = A2aClient::new(&base)
        .unwrap()
        .with_bearer_token("lab-token");
    let sources = granted
        .list_video_sources(ListVideoSourcesRequest::new(page(10)).unwrap())
        .await
        .unwrap();
    assert_eq!(sources.items()[0].id().as_str(), "cam");
}

#[tokio::test]
async fn clip_get_is_auth_gated_finite_multipart() {
    let base = serve_auth(service(catalog().await)).await;
    let client = A2aClient::new(&base)
        .unwrap()
        .with_bearer_token("lab-token");
    let clip = client
        .get_video_clip(
            GetVideoClipRequest::new(
                VideoSourceId::new("cam").unwrap(),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let url = format!("{base}{}", clip.path());
    let anonymous = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(anonymous.status(), reqwest::StatusCode::UNAUTHORIZED);
    let card = reqwest::Client::new()
        .get(format!("{base}/.well-known/agent-card.json"))
        .send()
        .await
        .unwrap();
    assert!(card.status().is_success());

    let granted = reqwest::Client::new()
        .get(&url)
        .bearer_auth("lab-token")
        .send()
        .await
        .unwrap();
    assert_clip_body(granted).await;

    let invalid = get_auth(&base, &format!("{VIDEO_CLIP_PATH_PREFIX}bad id")).await;
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);
    let missing = get_auth(&base, &format!("{VIDEO_CLIP_PATH_PREFIX}missing-clip")).await;
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
}

async fn serve_auth(lab: Arc<dyn A2aLabApi>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&lab).with_security(
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
    );
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

async fn get_auth(base: &str, path: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(format!("{base}{path}"))
        .bearer_auth("lab-token")
        .send()
        .await
        .unwrap()
}

async fn assert_clip_body(granted: reqwest::Response) {
    assert!(granted.status().is_success());
    let content_type = header(&granted, "content-type");
    assert!(content_type.contains("multipart/x-mixed-replace"));
    assert!(content_type.contains(DEFAULT_MJPEG_BOUNDARY));
    let cache = header(&granted, "cache-control");
    assert_eq!(cache, "no-store");
    assert!(!cache.contains("public"));
    assert_eq!(header(&granted, "x-content-type-options"), "nosniff");
    assert!(granted.headers().get("accept-ranges").is_none());
    let body = granted.bytes().await.unwrap();
    assert!(String::from_utf8_lossy(&body).ends_with(&format!("--{DEFAULT_MJPEG_BOUNDARY}--\r\n")));
    let frames = decode_mjpeg_multipart(&body, DEFAULT_MJPEG_BOUNDARY).unwrap();
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.captured_at().to_rfc3339())
            .collect::<Vec<_>>(),
        ["2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"]
    );
    for frame in &frames {
        UtcTimestamp::parse(&frame.captured_at().to_rfc3339()).unwrap();
        assert_eq!(frame.jpeg(), JPEG);
    }
}

fn assert_auth_rejected(error: &a2a_lab_dev_kit::A2aLabError, operation: &str) {
    let text = error.to_string();
    assert!(
        text.contains("401") || text.to_ascii_lowercase().contains("auth"),
        "{operation}: {text}"
    );
}

fn header(response: &reqwest::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned()
}

#[tokio::test]
async fn timeline_and_live_handle_round_trip_without_media_bytes() {
    let videos = catalog().await;
    videos
        .insert_frame(
            VideoSourceId::new("cam").unwrap(),
            timestamp("2024-01-01T00:00:10Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let base = serve(service(videos.clone())).await;
    let client = A2aClient::new(&base).unwrap();
    let card = client.agent_card().await.unwrap();
    let skills: Vec<_> = card
        .skills
        .iter()
        .map(|skill| skill.id.replace('-', "_"))
        .collect::<Vec<_>>();
    assert!(skills.iter().any(|skill| skill == "get_video_timeline"));
    assert!(skills.iter().any(|skill| skill == "follow_video"));

    let request = GetVideoTimelineRequest::new(VideoSourceId::new("cam").unwrap());
    let timeline = client.get_video_timeline(request.clone()).await.unwrap();
    assert_eq!(
        timeline.intervals(),
        [
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:03Z"),
            range("2024-01-01T00:00:10Z", "2024-01-01T00:00:11Z"),
        ]
    );
    assert_eq!(
        timeline.newest_at(),
        Some(timestamp("2024-01-01T00:00:10Z"))
    );
    assert!(!timeline.live_available());
    let events = client
        .send_stream(A2aLabCommand::GetVideoTimeline(request))
        .await
        .unwrap();
    let payload = only_payload(&events);
    assert_eq!(payload["operation"], "get_video_timeline");
    assert_eq!(payload["result"]["intervals"].as_array().unwrap().len(), 2);
    assert_eq!(payload["result"]["live_available"], false);
    assert!(payload["result"].get("newest_at").is_some());
    assert_metadata_only(&payload);

    videos
        .set_live_available(&VideoSourceId::new("cam").unwrap(), true)
        .await
        .unwrap();
    let live = client
        .get_video_timeline(GetVideoTimelineRequest::new(
            VideoSourceId::new("cam").unwrap(),
        ))
        .await
        .unwrap();
    assert!(live.live_available());
    assert_eq!(live.newest_at(), Some(timestamp("2024-01-01T00:00:10Z")));

    let follow = client
        .follow_video(FollowVideoRequest::new(VideoSourceId::new("cam").unwrap()))
        .await
        .unwrap();
    assert_eq!(follow.path(), video_live_path(follow.id()));
    assert_opaque_path(follow.path());
    let follow_events = client
        .send_stream(A2aLabCommand::FollowVideo(FollowVideoRequest::new(
            VideoSourceId::new("cam").unwrap(),
        )))
        .await
        .unwrap();
    let follow_payload = only_payload(&follow_events);
    assert_eq!(follow_payload["operation"], "follow_video");
    assert_opaque_path(follow_payload["result"]["path"].as_str().unwrap());
    assert_metadata_only(&follow_payload);
}

#[tokio::test]
async fn client_preserves_video_availability_reasons() {
    let videos = catalog().await;
    videos
        .insert_frame(
            VideoSourceId::new("cam").unwrap(),
            timestamp("2024-01-01T00:00:10Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let base = serve(service(videos.clone())).await;
    let client = A2aClient::new(&base).unwrap();
    let missing = client
        .get_video_clip(
            GetVideoClipRequest::new(
                VideoSourceId::new("cam").unwrap(),
                range("2024-01-01T00:00:04Z", "2024-01-01T00:00:05Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "no_recording");
    let requested = range("2024-01-01T00:00:00Z", "2024-01-01T00:00:11Z");
    let spanning = client
        .get_video_clip(
            GetVideoClipRequest::new(VideoSourceId::new("cam").unwrap(), requested).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(spanning.requested_range(), requested);
    assert_eq!(
        spanning.effective_range(),
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:11Z")
    );

    assert_a2a_maps_recording_gap_from_provider().await;

    let clip = client
        .get_video_clip(
            GetVideoClipRequest::new(
                VideoSourceId::new("cam").unwrap(),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    videos.expire_clips().await;
    let Err(expired) = client.open_video_clip(clip.id()).await else {
        panic!("expired clip opened");
    };
    assert_eq!(expired.code(), "clip_expired");

    let live = client
        .follow_video(FollowVideoRequest::new(VideoSourceId::new("cam").unwrap()))
        .await
        .unwrap_err();
    assert_eq!(live.code(), "live_unavailable");
    let body = send_operation(
        &base,
        &json!({
            "operation": "follow_video",
            "params": {"source_id": "cam"}
        }),
    )
    .await;
    assert_eq!(video_metadata_code(&body), Some("live_unavailable"));
    assert_ne!(video_metadata_code(&body), Some("INTERNAL_ERROR"));

    videos
        .set_live_available(&VideoSourceId::new("cam").unwrap(), true)
        .await
        .unwrap();
    let follow = client
        .follow_video(FollowVideoRequest::new(VideoSourceId::new("cam").unwrap()))
        .await
        .unwrap();
    videos
        .set_live_available(&VideoSourceId::new("cam").unwrap(), false)
        .await
        .unwrap();
    let closed = reqwest::Client::new()
        .get(client.video_live_url(follow.id()))
        .send()
        .await
        .unwrap();
    assert_eq!(closed.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code_of(closed).await, "live_unavailable");
}

#[tokio::test]
async fn live_follow_delivers_post_open_frames_and_stays_open() {
    let videos = catalog().await;
    videos
        .set_live_available(&VideoSourceId::new("cam").unwrap(), true)
        .await
        .unwrap();
    let base = serve_auth(service(videos.clone())).await;
    let client = A2aClient::new(&base)
        .unwrap()
        .with_bearer_token("lab-token");
    let follow = client
        .follow_video(FollowVideoRequest::new(VideoSourceId::new("cam").unwrap()))
        .await
        .unwrap();
    videos
        .insert_frame(
            VideoSourceId::new("cam").unwrap(),
            timestamp("2024-01-01T00:00:04Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let url = client.video_live_url(follow.id());
    assert!(url.ends_with(follow.path()));
    assert!(!url.contains("lab-token"));
    let anonymous = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(anonymous.status(), reqwest::StatusCode::UNAUTHORIZED);
    let queried = reqwest::Client::new()
        .get(format!("{url}?access_token=lab-token"))
        .send()
        .await
        .unwrap();
    assert_eq!(queried.status(), reqwest::StatusCode::UNAUTHORIZED);

    let mut response = reqwest::Client::new()
        .get(&url)
        .bearer_auth("lab-token")
        .header("Range", "bytes=0-1")
        .send()
        .await
        .unwrap();
    assert_live_headers(&response);
    read_post_open_frames(&videos, &mut response).await;
}

fn assert_live_headers(response: &reqwest::Response) {
    assert!(response.status().is_success());
    assert_ne!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert!(response.headers().get("accept-ranges").is_none());
    assert!(response.headers().get("content-range").is_none());
    let content_type = header(response, "content-type");
    assert!(content_type.contains("multipart/x-mixed-replace"));
    assert!(content_type.contains(DEFAULT_MJPEG_BOUNDARY));
    let cache = header(response, "cache-control");
    assert_eq!(cache, "no-store");
    assert!(!cache.contains("public"));
    assert_eq!(header(response, "x-content-type-options"), "nosniff");
}

async fn read_post_open_frames(videos: &MemoryVideos, response: &mut reqwest::Response) {
    let mut buf = Vec::new();
    let idle = tokio::time::timeout(
        Duration::from_millis(150),
        next_live_frame(response, &mut buf),
    )
    .await;
    assert!(
        idle.is_err(),
        "pre-open frame must not arrive and the stream must stay open"
    );
    videos
        .insert_frame(
            VideoSourceId::new("cam").unwrap(),
            timestamp("2024-01-01T00:00:06Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    videos
        .insert_frame(
            VideoSourceId::new("cam").unwrap(),
            timestamp("2024-01-01T00:00:05Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let first = next_stamped_frame(response, &mut buf).await;
    let second = next_stamped_frame(response, &mut buf).await;
    assert_eq!(first.captured_at(), timestamp("2024-01-01T00:00:05Z"));
    assert_eq!(second.captured_at(), timestamp("2024-01-01T00:00:06Z"));
    assert_eq!(first.jpeg(), JPEG);
    UtcTimestamp::parse(&first.captured_at().to_rfc3339()).unwrap();
    let still_open = tokio::time::timeout(
        Duration::from_millis(150),
        next_live_frame(response, &mut buf),
    )
    .await;
    assert!(
        still_open.is_err(),
        "advancing the live edge must not close the stream"
    );
}

async fn next_stamped_frame(response: &mut reqwest::Response, buf: &mut Vec<u8>) -> VideoFrame {
    tokio::time::timeout(Duration::from_secs(2), next_live_frame(response, buf))
        .await
        .expect("live frame")
        .expect("frame bytes")
        .expect("post-open frame")
}

#[tokio::test]
async fn live_media_paths_stay_relative_and_clip_boundary_stays_closed() {
    let base = serve_auth(service(catalog().await)).await;
    let client = A2aClient::new(&base)
        .unwrap()
        .with_bearer_token("lab-token");
    let card = reqwest::Client::new()
        .get(format!("{base}/.well-known/agent-card.json"))
        .send()
        .await
        .unwrap();
    assert!(card.status().is_success());
    let missing = get_auth(&base, &format!("{VIDEO_LIVE_PATH_PREFIX}missing-follow")).await;
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(error_code_of(missing).await, "not_found");
    let invalid = get_auth(&base, &format!("{VIDEO_LIVE_PATH_PREFIX}bad id")).await;
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(error_code_of(invalid).await, "invalid");

    let clip = client
        .get_video_clip(
            GetVideoClipRequest::new(
                VideoSourceId::new("cam").unwrap(),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let granted = reqwest::Client::new()
        .get(format!("{base}{}", clip.path()))
        .bearer_auth("lab-token")
        .send()
        .await
        .unwrap();
    assert_clip_body(granted).await;

    let events = client
        .send_stream(A2aLabCommand::GetVideoFrameAt(GetVideoFrameAtRequest::new(
            VideoSourceId::new("cam").unwrap(),
            timestamp("2024-01-01T00:00:00Z"),
            None,
        )))
        .await
        .unwrap();
    assert_eq!(only_payload(&events)["operation"], "get_video_frame_at");
    let listed = client
        .send_stream(A2aLabCommand::ListVideoSources(
            ListVideoSourcesRequest::new(page(10)).unwrap(),
        ))
        .await
        .unwrap();
    let listed = payloads(&listed);
    assert!(
        listed.len() > 1,
        "source pages still stream one item at a time"
    );
    assert!(
        listed
            .iter()
            .all(|payload| payload["operation"] == "list_video_sources")
    );
}

#[tokio::test]
async fn live_producer_stops_without_retaining_frames() {
    let burst = ProbeVideos::new(ProbeMode::Burst);
    let base = serve(probe_service(burst.clone())).await;
    let client = A2aClient::new(&base).unwrap();
    let follow = client
        .follow_video(FollowVideoRequest::new(VideoSourceId::new("cam").unwrap()))
        .await
        .unwrap();
    let response = reqwest::Client::new()
        .get(client.video_live_url(follow.id()))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    drop(response);
    wait_flag(&burst).await;
    let pulled = burst.pulled();
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(burst.pulled(), pulled);
    assert_eq!(burst.retained(), 0);
    assert!(burst.dropped());

    let delay = ProbeVideos::new(ProbeMode::DelayHold);
    let base = serve(probe_service(delay.clone())).await;
    let client = A2aClient::new(&base).unwrap();
    let follow = client
        .follow_video(FollowVideoRequest::new(VideoSourceId::new("cam").unwrap()))
        .await
        .unwrap();
    let url = client.video_live_url(follow.id());
    let request = tokio::spawn(async move { reqwest::Client::new().get(url).send().await });
    wait_until(|| delay.opening() > 0).await;
    request.abort();
    let _ = tokio::time::timeout(Duration::from_secs(1), request).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(delay.pulled(), 0);
    if delay.opened() > 0 {
        assert!(
            delay.dropped(),
            "cancelled live route retained its producer"
        );
        assert_eq!(delay.retained(), 0);
    }

    let fail = ProbeVideos::new(ProbeMode::Fail);
    let base = serve(probe_service(fail.clone())).await;
    let client = A2aClient::new(&base).unwrap();
    let follow = client
        .follow_video(FollowVideoRequest::new(VideoSourceId::new("cam").unwrap()))
        .await
        .unwrap();
    let mut failed = reqwest::Client::new()
        .get(client.video_live_url(follow.id()))
        .send()
        .await
        .unwrap();
    let read = tokio::time::timeout(Duration::from_secs(2), async {
        while let Ok(Some(_)) = failed.chunk().await {}
    })
    .await;
    assert!(read.is_ok(), "provider failure must end the live body");
    wait_flag(&fail).await;
    assert_eq!(fail.retained(), 0);

    let finish = ProbeVideos::new(ProbeMode::Finish);
    let base = serve(probe_service(finish.clone())).await;
    let client = A2aClient::new(&base).unwrap();
    let follow = client
        .follow_video(FollowVideoRequest::new(VideoSourceId::new("cam").unwrap()))
        .await
        .unwrap();
    let completed = reqwest::Client::new()
        .get(client.video_live_url(follow.id()))
        .send()
        .await
        .unwrap();
    assert!(completed.status().is_success());
    let body = completed.bytes().await.unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("2024-01-01T00:00:08Z"));
    assert!(text.ends_with(&format!("--{DEFAULT_MJPEG_BOUNDARY}--\r\n")));
    wait_flag(&finish).await;
    assert_eq!(finish.retained(), 0);
}

fn only_payload(events: &[StreamResponse]) -> serde_json::Value {
    let mut payloads = payloads(events);
    assert_eq!(payloads.len(), 1);
    payloads.remove(0)
}

fn payloads(events: &[StreamResponse]) -> Vec<serde_json::Value> {
    let mut payloads = Vec::new();
    for event in events {
        let StreamResponse::ArtifactUpdate(update) = event else {
            continue;
        };
        let Some(part) = update.artifact.parts.first() else {
            continue;
        };
        let PartContent::Data(data) = &part.content else {
            continue;
        };
        payloads.push(data.clone());
    }
    payloads
}

fn assert_metadata_only(payload: &serde_json::Value) {
    let text = payload.to_string();
    assert!(payload.pointer("/result/data").is_none());
    assert!(payload.pointer("/result/jpeg").is_none());
    assert!(!text.contains("data:video"));
    assert!(!text.contains("/9j/"));
}

fn assert_opaque_path(path: &str) {
    assert!(path.starts_with(VIDEO_LIVE_PATH_PREFIX));
    assert!(!path.contains("://"));
    assert!(!path.contains('?'));
    assert!(!path.contains('#'));
    assert!(!path.contains('@'));
    assert!(!path.contains("lab-token"));
}

async fn send_operation(base: &str, operation: &serde_json::Value) -> serde_json::Value {
    let response = reqwest::Client::new()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", "1.0")
        .json(&json!({
            "message": {
                "messageId": "video-reason",
                "role": "ROLE_USER",
                "parts": [{
                    "mediaType": LAB_MEDIA_TYPE,
                    "data": operation
                }]
            }
        }))
        .send()
        .await
        .unwrap();
    let body = response.text().await.unwrap();
    serde_json::from_str(&body).unwrap_or_else(|_| panic!("error json: {body}"))
}

fn video_metadata_code(body: &serde_json::Value) -> Option<&str> {
    body.pointer("/error/details")?
        .as_array()?
        .iter()
        .find_map(|detail| detail.pointer("/metadata/code")?.as_str())
}

async fn next_live_frame(
    response: &mut reqwest::Response,
    buf: &mut Vec<u8>,
) -> Result<Option<VideoFrame>, String> {
    loop {
        if let Some(frame) = take_live_frame(buf) {
            return Ok(Some(frame));
        }
        if live_closed(buf) {
            return Ok(None);
        }
        match response.chunk().await {
            Ok(Some(chunk)) => buf.extend_from_slice(&chunk),
            Ok(None) => return Ok(None),
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn take_live_frame(buf: &mut Vec<u8>) -> Option<VideoFrame> {
    let marker = format!("--{DEFAULT_MJPEG_BOUNDARY}\r\n");
    let start = find_bytes(buf, marker.as_bytes())?;
    let header_at = start + marker.len();
    let header_rel = find_bytes(&buf[header_at..], b"\r\n\r\n")?;
    let headers = std::str::from_utf8(&buf[header_at..header_at + header_rel]).ok()?;
    let captured = part_header(headers, "X-Captured-At")?.to_owned();
    let length: usize = part_header(headers, "Content-Length")?.parse().ok()?;
    let body_at = header_at + header_rel + 4;
    let end = body_at.checked_add(length)?;
    if buf.len() < end {
        return None;
    }
    let jpeg = buf[body_at..end].to_vec();
    let mut consumed = end;
    if buf.get(consumed..consumed + 2) == Some(b"\r\n") {
        consumed += 2;
    }
    buf.drain(..consumed);
    VideoFrame::new(UtcTimestamp::parse(&captured).ok()?, jpeg).ok()
}

fn live_closed(buf: &[u8]) -> bool {
    let end = format!("--{DEFAULT_MJPEG_BOUNDARY}--");
    find_bytes(buf, end.as_bytes()).is_some() && take_live_frame(&mut buf.to_vec()).is_none()
}

fn part_header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name)
            .then(|| value.trim())
            .filter(|value| !value.is_empty())
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn error_code_of(response: reqwest::Response) -> String {
    let body = response.text().await.unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/code")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_default()
}

const BURST_FRAMES: usize = 30;

#[derive(Clone, Copy)]
enum ProbeMode {
    Burst,
    DelayHold,
    Fail,
    Finish,
}

#[derive(Clone)]
struct ProbeVideos {
    mode: Arc<Mutex<ProbeMode>>,
    opening: Arc<AtomicUsize>,
    opened: Arc<AtomicUsize>,
    pulled: Arc<AtomicUsize>,
    retained: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}

impl ProbeVideos {
    fn new(mode: ProbeMode) -> Self {
        Self {
            mode: Arc::new(Mutex::new(mode)),
            opening: Arc::new(AtomicUsize::new(0)),
            opened: Arc::new(AtomicUsize::new(0)),
            pulled: Arc::new(AtomicUsize::new(0)),
            retained: Arc::new(AtomicUsize::new(0)),
            dropped: Arc::new(AtomicBool::new(false)),
        }
    }

    fn opening(&self) -> usize {
        self.opening.load(Ordering::SeqCst)
    }

    fn opened(&self) -> usize {
        self.opened.load(Ordering::SeqCst)
    }

    fn pulled(&self) -> usize {
        self.pulled.load(Ordering::SeqCst)
    }

    fn retained(&self) -> usize {
        self.retained.load(Ordering::SeqCst)
    }

    fn dropped(&self) -> bool {
        self.dropped.load(Ordering::SeqCst)
    }
}

fn probe_service(videos: ProbeVideos) -> Arc<dyn A2aLabApi> {
    A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos)
        .share()
}

fn probe_down<T: Send>() -> impl Future<Output = Result<T, A2aLabError>> + Send {
    std::future::ready(Err(A2aLabError::unavailable("probe")))
}

impl VideoProvider for ProbeVideos {
    fn list_video_sources(
        &self,
        _request: ListVideoSourcesRequest,
    ) -> impl Future<Output = Result<Page<VideoSource>, A2aLabError>> + Send {
        probe_down()
    }

    fn get_video_frame_at(
        &self,
        _request: GetVideoFrameAtRequest,
    ) -> impl Future<Output = Result<GetVideoFrameAtResult, A2aLabError>> + Send {
        probe_down()
    }

    fn get_video_clip(
        &self,
        _request: GetVideoClipRequest,
    ) -> impl Future<Output = Result<VideoClip, A2aLabError>> + Send {
        probe_down()
    }

    fn open_video_clip(
        &self,
        _id: &VideoClipId,
    ) -> impl Future<Output = Result<VideoClipStream, A2aLabError>> + Send {
        probe_down()
    }

    fn follow_video(
        &self,
        request: FollowVideoRequest,
    ) -> impl Future<Output = Result<VideoFollow, A2aLabError>> + Send {
        let source_id = request.source_id().clone();
        std::future::ready(VideoFollow::new_mjpeg(
            VideoFollowId::new("follow-probe").unwrap(),
            source_id,
        ))
    }

    fn open_video_follow(
        &self,
        id: &VideoFollowId,
    ) -> impl Future<Output = Result<VideoLiveStream, A2aLabError>> + Send {
        let probe = self.clone();
        let id = id.clone();
        async move { probe.open_follow(&id).await }
    }
}

impl ProbeVideos {
    async fn open_follow(&self, id: &VideoFollowId) -> Result<VideoLiveStream, A2aLabError> {
        self.opening.fetch_add(1, Ordering::SeqCst);
        if id.as_str() != "follow-probe" {
            return Err(A2aLabError::not_found("video follow", id.to_string()));
        }
        let mode = *self.mode.lock().expect("probe mode");
        if matches!(mode, ProbeMode::DelayHold) {
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        self.opened.fetch_add(1, Ordering::SeqCst);
        let frames = match mode {
            ProbeMode::Burst => burst_frames(),
            ProbeMode::Finish => {
                vec![VideoFrame::new(timestamp("2024-01-01T00:00:08Z"), JPEG.to_vec()).unwrap()]
            }
            ProbeMode::DelayHold | ProbeMode::Fail => Vec::new(),
        };
        self.retained.store(frames.len(), Ordering::SeqCst);
        let state = LiveState {
            probe: self.clone(),
            frames,
            mode,
            index: 0,
        };
        Ok(Box::pin(futures_util::stream::unfold(state, live_next)))
    }
}

struct LiveState {
    probe: ProbeVideos,
    frames: Vec<VideoFrame>,
    mode: ProbeMode,
    index: usize,
}

impl Drop for LiveState {
    fn drop(&mut self) {
        self.frames.clear();
        self.probe.retained.store(0, Ordering::SeqCst);
        self.probe.dropped.store(true, Ordering::SeqCst);
    }
}

async fn live_next(mut state: LiveState) -> Option<(Result<VideoFrame, A2aLabError>, LiveState)> {
    match state.mode {
        ProbeMode::Fail => {
            if state.index > 0 {
                return None;
            }
            state.index += 1;
            Some((Err(A2aLabError::unavailable("camera failed")), state))
        }
        ProbeMode::Finish | ProbeMode::Burst => {
            if state.index >= state.frames.len() {
                if matches!(state.mode, ProbeMode::Finish) {
                    return None;
                }
                loop {
                    std::future::pending::<()>().await;
                }
            } else {
                let frame = state.frames[state.index].clone();
                state.index += 1;
                state.probe.pulled.fetch_add(1, Ordering::SeqCst);
                Some((Ok(frame), state))
            }
        }
        ProbeMode::DelayHold => loop {
            std::future::pending::<()>().await;
        },
    }
}

async fn assert_a2a_maps_recording_gap_from_provider() {
    let gap_base = serve(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_videos(ClipGapRefused)
            .share(),
    )
    .await;
    let gap_client = A2aClient::new(&gap_base).unwrap();
    let gap = gap_client
        .get_video_clip(
            GetVideoClipRequest::new(
                VideoSourceId::new("cam").unwrap(),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(gap.code(), "recording_gap");
}

struct ClipGapRefused;

impl VideoProvider for ClipGapRefused {
    fn list_video_sources(
        &self,
        _request: ListVideoSourcesRequest,
    ) -> impl Future<Output = Result<Page<VideoSource>, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::unavailable("unused")))
    }

    fn get_video_frame_at(
        &self,
        _request: GetVideoFrameAtRequest,
    ) -> impl Future<Output = Result<GetVideoFrameAtResult, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::unavailable("unused")))
    }

    fn get_video_clip(
        &self,
        _request: GetVideoClipRequest,
    ) -> impl Future<Output = Result<VideoClip, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::recording_gap(
            "requested range crosses a recording gap",
        )))
    }

    fn open_video_clip(
        &self,
        _id: &VideoClipId,
    ) -> impl Future<Output = Result<VideoClipStream, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::unavailable("unused")))
    }
}

fn burst_frames() -> Vec<VideoFrame> {
    let start = timestamp("2024-01-01T00:00:00Z");
    (0..BURST_FRAMES)
        .map(|index| {
            let at = start.saturating_add_micros(i64::try_from(index).unwrap_or(0) * 1_000_000);
            VideoFrame::new(at, JPEG.to_vec()).unwrap()
        })
        .collect()
}

async fn wait_flag(probe: &ProbeVideos) {
    wait_until(|| probe.dropped()).await;
}

async fn wait_until(mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "condition timed out"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
