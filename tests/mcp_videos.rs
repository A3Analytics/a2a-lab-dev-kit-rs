use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use a2a_lab_dev_kit::{
    A2aLabApi, A2aLabCommand, A2aLabError, A2aLabResult, A2aLabService, DEFAULT_MJPEG_BOUNDARY,
    FollowVideoRequest, GetVideoClipRequest, GetVideoFrameAtRequest, GetVideoFrameAtResult,
    GetVideoTimelineRequest, ListVideoSourcesRequest, MJPEG_MEDIA_TYPE, McpLab, McpServer,
    MemoryLogs, MemoryMetrics, MemoryTasks, MemoryVideos, Page, PageRequest, TimeRange,
    UtcTimestamp, VIDEO_CLIP_PATH_PREFIX, VIDEO_LIVE_PATH_PREFIX, VideoAvailability, VideoClip,
    VideoClipId, VideoClipStream, VideoFollow, VideoFollowId, VideoFrame, VideoLiveStream,
    VideoProvider, VideoSource, VideoSourceId, bind_local, decode_mjpeg_multipart, video_clip_path,
    video_live_path,
};
use futures_util::StreamExt;
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
};
use rmcp::service::ServiceError;
use rmcp::transport::StreamableHttpClientTransport;
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

async fn serve_http(lab: Arc<dyn A2aLabApi>) -> String {
    let (listener, address) = bind_local().await.unwrap();
    let origin = format!("http://{address}");
    let server = McpServer::new(&lab);
    tokio::spawn(async move {
        server.serve_http(listener).await.unwrap();
    });
    origin
}

#[tokio::test]
async fn video_operations_round_trip_without_embedding_clip_bytes() {
    let origin = serve_http(service(catalog().await)).await;
    let lab = McpLab::connect(&format!("{origin}/mcp")).await.unwrap();
    let listed = lab
        .execute(A2aLabCommand::ListVideoSources(
            ListVideoSourcesRequest::new(page(10)).unwrap(),
        ))
        .await
        .unwrap();
    let A2aLabResult::ListVideoSources(page) = listed.task.result else {
        panic!("list_video_sources");
    };
    assert_eq!(page.items()[0].id().as_str(), "cam");

    let frame = lab
        .execute(A2aLabCommand::GetVideoFrameAt(GetVideoFrameAtRequest::new(
            VideoSourceId::new("cam").unwrap(),
            timestamp("2024-01-01T00:00:00Z"),
            None,
        )))
        .await
        .unwrap();
    let A2aLabResult::GetVideoFrameAt(frame) = frame.task.result else {
        panic!("get_video_frame_at");
    };
    assert_eq!(frame.image().data(), JPEG);

    let clip = lab
        .execute(A2aLabCommand::GetVideoClip(
            GetVideoClipRequest::new(
                VideoSourceId::new("cam").unwrap(),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            )
            .unwrap(),
        ))
        .await
        .unwrap();
    let A2aLabResult::GetVideoClip(clip) = clip.task.result else {
        panic!("get_video_clip");
    };
    let encoded = serde_json::to_value(&clip).unwrap();
    assert!(encoded.get("data").is_none());
    assert_eq!(metadata_keys(&encoded), clip_metadata_keys());
    assert_eq!(clip.path(), video_clip_path(clip.id()));

    let mut stream = lab.open_video_clip(clip.id()).await.unwrap();
    let mut stamps = Vec::new();
    while let Some(frame) = stream.next().await {
        stamps.push(frame.unwrap().captured_at().to_rfc3339());
    }
    assert_eq!(stamps, ["2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"]);

    let response = reqwest::Client::new()
        .get(format!("{origin}{}", clip.path()))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    let body = response.bytes().await.unwrap();
    let frames = decode_mjpeg_multipart(&body, DEFAULT_MJPEG_BOUNDARY).unwrap();
    assert_eq!(frames.len(), 2);
    assert!(String::from_utf8_lossy(&body).ends_with(&format!("--{DEFAULT_MJPEG_BOUNDARY}--\r\n")));
}

#[tokio::test]
async fn video_operations_reject_missing_invalid_and_unavailable() {
    let videos = catalog().await;
    let origin = serve_http(service(videos.clone())).await;
    let mcp_url = format!("{origin}/mcp");
    let lab = McpLab::connect(&mcp_url).await.unwrap();
    let missing = lab
        .execute(A2aLabCommand::GetVideoFrameAt(GetVideoFrameAtRequest::new(
            VideoSourceId::new("missing").unwrap(),
            timestamp("2024-01-01T00:00:00Z"),
            None,
        )))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");
    let uncovered = lab
        .execute(A2aLabCommand::GetVideoClip(
            GetVideoClipRequest::new(
                VideoSourceId::new("cam").unwrap(),
                range("2025-01-01T00:00:00Z", "2025-01-01T00:00:01Z"),
            )
            .unwrap(),
        ))
        .await
        .unwrap_err();
    assert_eq!(uncovered.code(), "no_recording");
    let session = connect_raw(&mcp_url).await;
    let args = json!({"page": {"limit": 0}});
    let invalid = session
        .call_tool(
            CallToolRequestParams::new("list_video_sources".to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .expect("tool call");
    assert_eq!(invalid.is_error, Some(true));
    let bad_id = reqwest::Client::new()
        .get(format!("{origin}{VIDEO_CLIP_PATH_PREFIX}bad id"))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_id.status(), reqwest::StatusCode::BAD_REQUEST);

    let Err(unknown) = lab
        .open_video_clip(&a2a_lab_dev_kit::VideoClipId::new("missing-clip").unwrap())
        .await
    else {
        panic!("unknown clip must fail");
    };
    assert_eq!(unknown.code(), "not_found");

    videos.set_unavailable("videos offline").await;
    let unavailable = McpLab::connect(&format!("{origin}/mcp"))
        .await
        .unwrap()
        .execute(A2aLabCommand::ListVideoSources(
            ListVideoSourcesRequest::new(page(10)).unwrap(),
        ))
        .await
        .unwrap_err();
    assert_eq!(unavailable.code(), "unavailable");

    let empty = serve_http(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share(),
    )
    .await;
    let missing_provider = McpLab::connect(&format!("{empty}/mcp"))
        .await
        .unwrap()
        .execute(A2aLabCommand::ListVideoSources(
            ListVideoSourcesRequest::new(page(10)).unwrap(),
        ))
        .await
        .unwrap_err();
    assert_eq!(missing_provider.code(), "unavailable");
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
    let origin = serve_http(service(videos.clone())).await;
    let mcp_url = format!("{origin}/mcp");
    let lab = McpLab::connect(&mcp_url).await.unwrap();
    let names = tool_names(&mcp_url).await;
    for name in [
        "list_video_sources",
        "get_video_frame_at",
        "get_video_clip",
        "get_video_timeline",
        "follow_video",
    ] {
        assert!(names.iter().any(|listed| listed == name), "{name}");
    }

    let request = GetVideoTimelineRequest::new(VideoSourceId::new("cam").unwrap());
    let timeline = lab
        .execute(A2aLabCommand::GetVideoTimeline(request.clone()))
        .await
        .unwrap();
    let A2aLabResult::GetVideoTimeline(timeline) = timeline.task.result else {
        panic!("get_video_timeline");
    };
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
    let wire = tool_value(
        &mcp_url,
        "get_video_timeline",
        serde_json::to_value(&request).unwrap(),
    )
    .await;
    assert_eq!(wire["intervals"].as_array().unwrap().len(), 2);
    assert_eq!(wire["live_available"], false);
    assert!(wire.get("newest_at").is_some());
    assert_metadata_only(&wire);

    videos
        .set_live_available(&VideoSourceId::new("cam").unwrap(), true)
        .await
        .unwrap();
    let live = lab
        .execute(A2aLabCommand::GetVideoTimeline(
            GetVideoTimelineRequest::new(VideoSourceId::new("cam").unwrap()),
        ))
        .await
        .unwrap();
    let A2aLabResult::GetVideoTimeline(live) = live.task.result else {
        panic!("live timeline");
    };
    assert!(live.live_available());
    assert_eq!(live.newest_at(), Some(timestamp("2024-01-01T00:00:10Z")));

    let follow = lab
        .execute(A2aLabCommand::FollowVideo(FollowVideoRequest::new(
            VideoSourceId::new("cam").unwrap(),
        )))
        .await
        .unwrap();
    let A2aLabResult::FollowVideo(follow) = follow.task.result else {
        panic!("follow_video");
    };
    assert_eq!(follow.path(), video_live_path(follow.id()));
    assert_opaque_path(follow.path());
    let follow_wire = tool_value(
        &mcp_url,
        "follow_video",
        serde_json::json!({"source_id": "cam"}),
    )
    .await;
    let wire_follow: VideoFollow = serde_json::from_value(follow_wire).unwrap();
    assert_eq!(wire_follow.path(), video_live_path(wire_follow.id()));
    assert_opaque_path(wire_follow.path());
    assert_metadata_only(&serde_json::to_value(&wire_follow).unwrap());
    let url = lab.video_live_url(follow.id()).unwrap();
    assert!(url.ends_with(follow.path()));
    assert!(!url.contains('?'));
}

#[tokio::test]
async fn mcp_preserves_video_availability_codes() {
    let videos = catalog().await;
    videos
        .insert_frame(
            VideoSourceId::new("cam").unwrap(),
            timestamp("2024-01-01T00:00:10Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let origin = serve_http(service(videos.clone())).await;
    let mcp_url = format!("{origin}/mcp");
    let lab = McpLab::connect(&mcp_url).await.unwrap();
    assert_tool_availability(
        &lab,
        &mcp_url,
        A2aLabCommand::GetVideoClip(
            GetVideoClipRequest::new(
                VideoSourceId::new("cam").unwrap(),
                range("2024-01-01T00:00:04Z", "2024-01-01T00:00:05Z"),
            )
            .unwrap(),
        ),
        "get_video_clip",
        serde_json::json!({
            "source_id": "cam",
            "range": {"start": "2024-01-01T00:00:04Z", "end": "2024-01-01T00:00:05Z"}
        }),
        VideoAvailability::NoRecording,
    )
    .await;
    let requested = range("2024-01-01T00:00:00Z", "2024-01-01T00:00:11Z");
    let spanning = lab
        .execute(A2aLabCommand::GetVideoClip(
            GetVideoClipRequest::new(VideoSourceId::new("cam").unwrap(), requested).unwrap(),
        ))
        .await
        .unwrap();
    let A2aLabResult::GetVideoClip(clip) = spanning.task.result else {
        panic!("get_video_clip");
    };
    assert_eq!(clip.requested_range(), requested);
    assert_eq!(
        clip.effective_range(),
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:11Z")
    );

    let gap_origin = serve_http(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_videos(ClipGapRefused)
            .share(),
    )
    .await;
    let gap_mcp_url = format!("{gap_origin}/mcp");
    let gap_lab = McpLab::connect(&gap_mcp_url).await.unwrap();
    assert_tool_availability(
        &gap_lab,
        &gap_mcp_url,
        A2aLabCommand::GetVideoClip(
            GetVideoClipRequest::new(
                VideoSourceId::new("cam").unwrap(),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
            )
            .unwrap(),
        ),
        "get_video_clip",
        serde_json::json!({
            "source_id": "cam",
            "range": {"start": "2024-01-01T00:00:00Z", "end": "2024-01-01T00:00:01Z"}
        }),
        VideoAvailability::RecordingGap,
    )
    .await;
    assert_expired_clip(&lab, &origin, &videos).await;
    assert_tool_availability(
        &lab,
        &mcp_url,
        A2aLabCommand::FollowVideo(FollowVideoRequest::new(VideoSourceId::new("cam").unwrap())),
        "follow_video",
        serde_json::json!({"source_id": "cam"}),
        VideoAvailability::LiveUnavailable,
    )
    .await;
    assert_closed_follow(&lab, &videos).await;
}

async fn assert_tool_availability(
    lab: &McpLab,
    mcp_url: &str,
    command: A2aLabCommand,
    tool: &str,
    args: serde_json::Value,
    availability: VideoAvailability,
) {
    let error = lab.execute(command).await.unwrap_err();
    let code = availability_error(&error, availability);
    assert_eq!(tool_data_code(mcp_url, tool, args).await, code);
}

async fn assert_expired_clip(lab: &McpLab, origin: &str, videos: &MemoryVideos) {
    let clip = lab
        .execute(A2aLabCommand::GetVideoClip(
            GetVideoClipRequest::new(
                VideoSourceId::new("cam").unwrap(),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            )
            .unwrap(),
        ))
        .await
        .unwrap();
    let A2aLabResult::GetVideoClip(clip) = clip.task.result else {
        panic!("clip handle");
    };
    videos.expire_clips().await;
    let expired = reqwest::Client::new()
        .get(format!("{origin}{}", clip.path()))
        .send()
        .await
        .unwrap();
    assert_eq!(expired.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(error_code_of(expired).await, "clip_expired");
    let Err(opened) = lab.open_video_clip(clip.id()).await else {
        panic!("expired clip opened");
    };
    assert_eq!(
        availability_error(&opened, VideoAvailability::ClipExpired),
        "clip_expired"
    );
}

async fn assert_closed_follow(lab: &McpLab, videos: &MemoryVideos) {
    videos
        .set_live_available(&VideoSourceId::new("cam").unwrap(), true)
        .await
        .unwrap();
    let follow = follow_cam(lab).await;
    videos
        .set_live_available(&VideoSourceId::new("cam").unwrap(), false)
        .await
        .unwrap();
    let closed = reqwest::Client::new()
        .get(lab.video_live_url(follow.id()).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(closed.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code_of(closed).await, "live_unavailable");
}

#[tokio::test]
async fn live_follow_delivers_post_open_frames_without_a_bearer() {
    let videos = catalog().await;
    videos
        .set_live_available(&VideoSourceId::new("cam").unwrap(), true)
        .await
        .unwrap();
    let origin = serve_http(service(videos.clone())).await;
    let lab = McpLab::connect(&format!("{origin}/mcp")).await.unwrap();
    videos
        .insert_frame(
            VideoSourceId::new("cam").unwrap(),
            timestamp("2024-01-01T00:00:04Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let follow = follow_cam(&lab).await;
    let url = lab.video_live_url(follow.id()).unwrap();
    assert!(url.ends_with(follow.path()));
    assert!(!url.contains("access_token"));
    let mut response = reqwest::Client::new()
        .get(&url)
        .header("Range", "bytes=0-1")
        .send()
        .await
        .unwrap();
    assert_live_headers(&response);
    let mut buf = Vec::new();
    assert_live_idle(&mut response, &mut buf).await;
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
    let first = next_stamped_frame(&mut response, &mut buf).await;
    let second = next_stamped_frame(&mut response, &mut buf).await;
    assert_eq!(first.captured_at(), timestamp("2024-01-01T00:00:05Z"));
    assert_eq!(second.captured_at(), timestamp("2024-01-01T00:00:06Z"));
    assert_eq!(first.jpeg(), JPEG);
    UtcTimestamp::parse(&first.captured_at().to_rfc3339()).unwrap();
    assert_live_idle(&mut response, &mut buf).await;

    let later = follow_cam(&lab).await;
    let mut second_stream = reqwest::Client::new()
        .get(lab.video_live_url(later.id()).unwrap())
        .send()
        .await
        .unwrap();
    assert_live_headers(&second_stream);
    let mut second_buf = Vec::new();
    assert_live_idle(&mut second_stream, &mut second_buf).await;
    videos
        .insert_frame(
            VideoSourceId::new("cam").unwrap(),
            timestamp("2024-01-01T00:00:07Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let only = next_stamped_frame(&mut second_stream, &mut second_buf).await;
    assert_eq!(only.captured_at(), timestamp("2024-01-01T00:00:07Z"));

    let missing = reqwest::Client::new()
        .get(format!("{origin}{VIDEO_LIVE_PATH_PREFIX}missing-follow"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(error_code_of(missing).await, "not_found");
    let invalid = reqwest::Client::new()
        .get(format!("{origin}{VIDEO_LIVE_PATH_PREFIX}bad id"))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(error_code_of(invalid).await, "invalid");
}

#[tokio::test]
async fn live_disconnect_cancels_and_keeps_a_bounded_queue() {
    let burst = ProbeVideos::new(ProbeMode::Burst);
    let origin = serve_http(probe_service(burst.clone())).await;
    let lab = McpLab::connect(&format!("{origin}/mcp")).await.unwrap();
    let follow = follow_cam(&lab).await;
    let response = reqwest::Client::new()
        .get(lab.video_live_url(follow.id()).unwrap())
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    wait_until(|| burst.pulled() > 0).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let pulled = burst.pulled();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        burst.pulled(),
        pulled,
        "unread live response kept an unbounded queue"
    );
    drop(response);
    wait_flag(&burst).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(burst.pulled(), pulled);
    assert_eq!(burst.retained(), 0);
    assert!(burst.dropped());

    let delay = ProbeVideos::new(ProbeMode::DelayHold);
    let origin = serve_http(probe_service(delay.clone())).await;
    let lab = McpLab::connect(&format!("{origin}/mcp")).await.unwrap();
    let follow = follow_cam(&lab).await;
    let url = lab.video_live_url(follow.id()).unwrap();
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
}

fn clip_metadata_keys() -> Vec<String> {
    [
        "effective_range",
        "id",
        "media_type",
        "path",
        "requested_range",
        "source_id",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn metadata_keys(value: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

fn assert_metadata_only(value: &serde_json::Value) {
    let text = value.to_string();
    assert!(value.get("data").is_none(), "{value}");
    assert!(value.get("jpeg").is_none(), "{value}");
    assert!(value.get("frames").is_none(), "{value}");
    assert!(!text.contains("data:video"), "{text}");
    assert!(!text.contains("/9j/"), "{text}");
}

fn assert_opaque_path(path: &str) {
    assert!(path.starts_with(VIDEO_LIVE_PATH_PREFIX));
    assert!(!path.contains("://"));
    assert!(!path.contains('?'));
    assert!(!path.contains('#'));
    assert!(!path.contains('@'));
}

fn availability_error(error: &A2aLabError, availability: VideoAvailability) -> String {
    assert_eq!(error.availability(), Some(availability));
    assert_eq!(error.code(), availability.as_str());
    assert_ne!(error.to_string(), availability.as_str());
    error.code().to_owned()
}

async fn follow_cam(lab: &McpLab) -> VideoFollow {
    let follow = lab
        .execute(A2aLabCommand::FollowVideo(FollowVideoRequest::new(
            VideoSourceId::new("cam").unwrap(),
        )))
        .await
        .unwrap();
    let A2aLabResult::FollowVideo(follow) = follow.task.result else {
        panic!("follow_video");
    };
    follow
}

async fn tool_names(url: &str) -> Vec<String> {
    connect_raw(url)
        .await
        .list_tools(None)
        .await
        .unwrap()
        .tools
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect()
}

async fn tool_value(url: &str, name: &str, args: serde_json::Value) -> serde_json::Value {
    let result = connect_raw(url)
        .await
        .call_tool(call_params(name, &args))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert!(
        result
            .content
            .iter()
            .all(|block| { block.as_image().is_none() && block.as_resource().is_none() })
    );
    let text = result
        .content
        .iter()
        .find_map(|block| block.as_text())
        .map(|text| text.text.clone())
        .unwrap_or_default();
    assert!(!text.contains("/9j/"), "{text}");
    let structured = result.structured_content.expect("structured json");
    assert_metadata_only(&structured);
    structured
}

async fn tool_data_code(url: &str, name: &str, args: serde_json::Value) -> String {
    let error = connect_raw(url)
        .await
        .call_tool(call_params(name, &args))
        .await
        .expect_err("structured tool error");
    let ServiceError::McpError(data) = error else {
        panic!("expected mcp error data, got {error}");
    };
    let code = data
        .data
        .as_ref()
        .and_then(|value| value.get("code"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned();
    assert_ne!(data.message.as_ref(), code);
    assert!(!code.is_empty());
    code
}

fn call_params(name: &str, args: &serde_json::Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_owned()).with_arguments(args.as_object().unwrap().clone())
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

fn header(response: &reqwest::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned()
}

async fn assert_live_idle(response: &mut reqwest::Response, buf: &mut Vec<u8>) {
    let idle =
        tokio::time::timeout(Duration::from_millis(150), next_live_frame(response, buf)).await;
    assert!(
        idle.is_err(),
        "retained or cached frames must not arrive and the stream must stay open"
    );
}

async fn next_stamped_frame(response: &mut reqwest::Response, buf: &mut Vec<u8>) -> VideoFrame {
    tokio::time::timeout(Duration::from_secs(2), next_live_frame(response, buf))
        .await
        .expect("live frame")
        .expect("frame bytes")
        .expect("post-open frame")
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

#[derive(Clone, Copy)]
enum ProbeMode {
    Burst,
    DelayHold,
}

#[derive(Clone)]
struct ProbeVideos {
    mode: ProbeMode,
    opening: Arc<AtomicUsize>,
    opened: Arc<AtomicUsize>,
    pulled: Arc<AtomicUsize>,
    retained: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}

impl ProbeVideos {
    fn new(mode: ProbeMode) -> Self {
        Self {
            mode,
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

    async fn open_follow(&self, id: &VideoFollowId) -> Result<VideoLiveStream, A2aLabError> {
        self.opening.fetch_add(1, Ordering::SeqCst);
        if id.as_str() != "follow-probe" {
            return Err(A2aLabError::not_found("video follow", id.to_string()));
        }
        if matches!(self.mode, ProbeMode::DelayHold) {
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        self.opened.fetch_add(1, Ordering::SeqCst);
        self.retained.store(1, Ordering::SeqCst);
        let state = LiveState {
            probe: self.clone(),
            mode: self.mode,
            jpeg: JPEG.to_vec(),
        };
        Ok(Box::pin(futures_util::stream::unfold(state, live_next)))
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
        std::future::ready(VideoFollow::new(
            VideoFollowId::new("follow-probe").unwrap(),
            source_id,
            MJPEG_MEDIA_TYPE,
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

struct LiveState {
    probe: ProbeVideos,
    mode: ProbeMode,
    jpeg: Vec<u8>,
}

impl Drop for LiveState {
    fn drop(&mut self) {
        self.jpeg.clear();
        self.probe.retained.store(0, Ordering::SeqCst);
        self.probe.dropped.store(true, Ordering::SeqCst);
    }
}

async fn live_next(state: LiveState) -> Option<(Result<VideoFrame, A2aLabError>, LiveState)> {
    if matches!(state.mode, ProbeMode::DelayHold) {
        loop {
            std::future::pending::<()>().await;
        }
    }
    let frame = VideoFrame::new(timestamp("2024-01-01T00:00:08Z"), state.jpeg.clone()).unwrap();
    state.probe.pulled.fetch_add(1, Ordering::SeqCst);
    Some((Ok(frame), state))
}

async fn wait_flag(probe: &ProbeVideos) {
    wait_until(|| probe.dropped()).await;
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

async fn connect_raw(url: &str) -> rmcp::service::RunningService<rmcp::RoleClient, ClientConfig> {
    let mut last = String::new();
    for _ in 0..50 {
        let config = ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("a2a-lab-test", "0.1.0"),
        )
        .with_protocol_version(ProtocolVersion::V_2026_07_28);
        match config
            .serve(StreamableHttpClientTransport::from_uri(url))
            .await
        {
            Ok(client) => return client,
            Err(error) => last = error.to_string(),
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("mcp not ready: {last}");
}
