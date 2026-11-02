use std::time::{Duration, Instant};

use futures_util::StreamExt;

use a2a_lab_dev_kit::service::EmptyVideos;
use a2a_lab_dev_kit::{
    A2aLabApi, A2aLabCommand, A2aLabResult, A2aLabService, DEFAULT_MJPEG_BOUNDARY,
    DvrFrameAtRequest, DvrMediaPayload, DvrMediaStore, DvrTrackId, FollowVideoRequest,
    GetVideoClipRequest, GetVideoFrameAtRequest, GetVideoTimelineRequest, ListLogSourcesRequest,
    ListVideoSourcesRequest, MAX_VIDEO_CLIP_DURATION_US, MJPEG_MEDIA_TYPE, MemoryLogs,
    MemoryMetrics, MemoryTasks, MemoryVideos, PageRequest, ReadDvrMediaWindowRequest,
    TckMalformedVideoRequests, TimeRange, UtcTimestamp, VideoClip, VideoCodec, VideoFollowId,
    VideoProvider, VideoSource, VideoSourceId, encode_mjpeg_multipart,
};

const H264_INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67];
const H265_INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x40];

const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
];

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).expect("timestamp")
}

fn source_id(id: &str) -> VideoSourceId {
    VideoSourceId::new(id).unwrap()
}

fn source(id: &str, fps: u32) -> VideoSource {
    VideoSource::new(
        source_id(id),
        id,
        format!("{id} recording"),
        None,
        None,
        1,
        1,
        fps,
        MJPEG_MEDIA_TYPE,
    )
    .unwrap()
}

fn range(start: &str, end: &str) -> TimeRange {
    TimeRange::new(timestamp(start), timestamp(end)).unwrap()
}

fn dvr_track(id: &str) -> DvrTrackId {
    DvrTrackId::new(id).unwrap()
}

fn mjpeg_track(source: &VideoSourceId) -> DvrTrackId {
    MemoryVideos::mjpeg_track_id(source).unwrap()
}

fn page() -> PageRequest {
    PageRequest::new(None, 10).unwrap()
}

async fn fixture() -> MemoryVideos {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam", 1)).await.unwrap();
    for at in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:02Z",
    ] {
        videos
            .insert_frame(source_id("cam"), timestamp(at), JPEG.to_vec())
            .await
            .unwrap();
    }
    videos
}

#[tokio::test]
async fn empty_videos_is_unavailable() {
    let provider = EmptyVideos;
    let source = source_id("cam");
    let listed = provider
        .list_video_sources(ListVideoSourcesRequest::new(page()).unwrap())
        .await
        .unwrap_err();
    let timeline = provider
        .get_video_timeline(GetVideoTimelineRequest::new(source.clone()))
        .await
        .unwrap_err();
    let follow = provider
        .follow_video(FollowVideoRequest::new(source))
        .await
        .unwrap_err();
    let opened = provider
        .open_video_follow(&VideoFollowId::new("follow-1").unwrap())
        .await
        .map(|_| ())
        .expect_err("empty videos");
    for error in [listed, timeline, follow, opened] {
        assert_eq!(error.code(), "unavailable");
        assert_eq!(error.to_string(), "video provider is not configured");
    }
}

#[tokio::test]
async fn constructor_without_with_videos_leaves_video_ops_unavailable() {
    let logs = MemoryLogs::new();
    let service = A2aLabService::new(logs, MemoryMetrics::new(), MemoryTasks::new());
    let listed = service
        .execute(A2aLabCommand::ListLogSources(ListLogSourcesRequest {
            page: page(),
        }))
        .await
        .unwrap();
    assert!(matches!(
        listed.task.result,
        A2aLabResult::ListLogSources(_)
    ));

    for command in [
        A2aLabCommand::ListVideoSources(ListVideoSourcesRequest::new(page()).unwrap()),
        A2aLabCommand::GetVideoFrameAt(GetVideoFrameAtRequest::new(
            source_id("cam"),
            timestamp("2024-01-01T00:00:00Z"),
            None,
        )),
        A2aLabCommand::GetVideoClip(
            GetVideoClipRequest::new(
                source_id("cam"),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
            )
            .unwrap(),
        ),
        A2aLabCommand::GetVideoTimeline(GetVideoTimelineRequest::new(source_id("cam"))),
        A2aLabCommand::FollowVideo(FollowVideoRequest::new(source_id("cam"))),
    ] {
        let error = service.execute(command).await.unwrap_err();
        assert_eq!(error.code(), "unavailable");
    }
    let opened = service
        .open_video_follow(&VideoFollowId::new("follow-1").unwrap())
        .await
        .map(|_| ())
        .expect_err("constructor");
    assert_eq!(opened.code(), "unavailable");
}

#[tokio::test]
async fn with_videos_dispatches_list_frame_and_clip() {
    let videos = fixture().await;
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos);
    let listed = service
        .execute(A2aLabCommand::ListVideoSources(
            ListVideoSourcesRequest::new(page()).unwrap(),
        ))
        .await
        .unwrap();
    let A2aLabResult::ListVideoSources(page) = listed.task.result else {
        panic!("with_videos must enable source listing");
    };
    assert_eq!(page.items()[0].id().as_str(), "cam");

    let frame = service
        .execute(A2aLabCommand::GetVideoFrameAt(GetVideoFrameAtRequest::new(
            source_id("cam"),
            timestamp("2024-01-01T00:00:00Z"),
            None,
        )))
        .await
        .unwrap();
    let A2aLabResult::GetVideoFrameAt(result) = frame.task.result else {
        panic!("frame lookup must dispatch");
    };
    assert_eq!(result.delta_us(), 0);
    assert_eq!(result.image().descriptor().media_type(), "image/jpeg");

    let clip = service
        .execute(A2aLabCommand::GetVideoClip(
            GetVideoClipRequest::new(
                source_id("cam"),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            )
            .unwrap(),
        ))
        .await
        .unwrap();
    let A2aLabResult::GetVideoClip(handle) = clip.task.result else {
        panic!("clip must dispatch");
    };
    let encoded = serde_json::to_value(&handle).unwrap();
    assert!(encoded.get("data").is_none());
    assert_eq!(
        handle.path(),
        format!("/media/video-clips/{}/", handle.id().as_str())
    );
    assert!(!handle.path().contains("://"));
}

#[tokio::test]
async fn nearest_frame_breaks_ties_toward_earlier_and_honors_tolerance() {
    let videos = fixture().await;
    let at = timestamp("2024-01-01T00:00:01Z");
    let tied = videos
        .get_video_frame_at(GetVideoFrameAtRequest::new(source_id("cam"), at, None))
        .await
        .unwrap();
    assert_eq!(tied.actual_captured_at(), timestamp("2024-01-01T00:00:01Z"));

    let midway = timestamp("2024-01-01T00:00:00.500000Z");
    let earlier = videos
        .get_video_frame_at(GetVideoFrameAtRequest::new(source_id("cam"), midway, None))
        .await
        .unwrap();
    assert_eq!(
        earlier.actual_captured_at(),
        timestamp("2024-01-01T00:00:00Z")
    );
    assert_eq!(earlier.delta_us(), -500_000);
    assert_eq!(earlier.requested_at(), midway);

    let tight = videos
        .get_video_frame_at(GetVideoFrameAtRequest::new(
            source_id("cam"),
            midway,
            Some(400_000),
        ))
        .await
        .unwrap_err();
    assert_eq!(tight.code(), "not_found");

    let outside = videos
        .get_video_frame_at(GetVideoFrameAtRequest::new(
            source_id("cam"),
            timestamp("2024-01-01T00:00:03Z"),
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(outside.code(), "not_found");
}

#[tokio::test]
async fn frame_lookup_rejects_coverage_gaps() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("gappy", 1)).await.unwrap();
    videos
        .insert_frame(
            source_id("gappy"),
            timestamp("2024-01-01T00:00:00Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    videos
        .insert_frame(
            source_id("gappy"),
            timestamp("2024-01-01T00:00:01Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    videos
        .insert_frame(
            source_id("gappy"),
            timestamp("2024-01-01T00:00:10Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let gap = videos
        .get_video_frame_at(GetVideoFrameAtRequest::new(
            source_id("gappy"),
            timestamp("2024-01-01T00:00:05Z"),
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(gap.code(), "not_found");
    let before = videos
        .get_video_frame_at(GetVideoFrameAtRequest::new(
            source_id("gappy"),
            timestamp("2024-01-01T00:00:01Z"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        before.actual_captured_at().to_rfc3339(),
        "2024-01-01T00:00:01Z"
    );
}

#[tokio::test]
async fn empty_coverage_is_not_found_not_clamped() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("empty", 1)).await.unwrap();
    let error = videos
        .get_video_frame_at(GetVideoFrameAtRequest::new(
            source_id("empty"),
            timestamp("2024-01-01T00:00:00Z"),
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "not_found");
}

#[tokio::test]
async fn clip_uses_half_open_bounds_and_tolerates_gaps() {
    let videos = fixture().await;
    let first_two = videos
        .get_video_clip(
            GetVideoClipRequest::new(
                source_id("cam"),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let mut stream = videos.open_video_clip(first_two.id()).await.unwrap();
    let mut stamps = Vec::new();
    while let Some(frame) = stream.next().await {
        stamps.push(frame.unwrap().captured_at().to_rfc3339());
    }
    assert_eq!(
        stamps,
        vec![
            "2024-01-01T00:00:00Z".to_owned(),
            "2024-01-01T00:00:01Z".to_owned()
        ]
    );

    videos.insert_source(source("gappy", 1)).await.unwrap();
    videos
        .insert_frame(
            source_id("gappy"),
            timestamp("2024-01-01T00:00:00Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    videos
        .insert_frame(
            source_id("gappy"),
            timestamp("2024-01-01T00:00:01Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    videos
        .insert_frame(
            source_id("gappy"),
            timestamp("2024-01-01T00:00:10Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let requested = range("2024-01-01T00:00:00Z", "2024-01-01T00:00:11Z");
    let spanning = videos
        .get_video_clip(GetVideoClipRequest::new(source_id("gappy"), requested).unwrap())
        .await
        .unwrap();
    assert_eq!(spanning.requested_range(), requested);
    assert_eq!(
        spanning.effective_range(),
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:11Z")
    );
    let mut stream = videos.open_video_clip(spanning.id()).await.unwrap();
    let mut stamps = Vec::new();
    while let Some(frame) = stream.next().await {
        stamps.push(frame.unwrap().captured_at().to_rfc3339());
    }
    assert_eq!(
        stamps,
        vec![
            "2024-01-01T00:00:00Z".to_owned(),
            "2024-01-01T00:00:01Z".to_owned(),
            "2024-01-01T00:00:10Z".to_owned(),
        ]
    );
}

#[tokio::test]
async fn clips_expire_and_json_omits_video_bytes() {
    let videos = MemoryVideos::with_clip_ttl(Duration::from_secs(60));
    videos.insert_source(source("cam", 1)).await.unwrap();
    videos
        .insert_frame(
            source_id("cam"),
            timestamp("2024-01-01T00:00:00Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let clip = videos
        .get_video_clip(
            GetVideoClipRequest::new(
                source_id("cam"),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(serde_json::to_string(&clip).unwrap().find("data").is_none());
    videos.expire_clips().await;
    match videos.open_video_clip(clip.id()).await {
        Err(error) => assert_eq!(error.code(), "clip_expired"),
        Ok(_) => panic!("expired clip must not open"),
    }
    let Err(unknown) = videos
        .open_video_clip(&a2a_lab_dev_kit::VideoClipId::new("missing-clip").unwrap())
        .await
    else {
        panic!("unknown clip must not open");
    };
    assert_eq!(unknown.code(), "not_found");

    let immediate = MemoryVideos::with_clip_ttl(Duration::ZERO);
    immediate.insert_source(source("cam", 1)).await.unwrap();
    immediate
        .insert_frame(
            source_id("cam"),
            timestamp("2024-01-01T00:00:00Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let handle = immediate
        .get_video_clip(
            GetVideoClipRequest::new(
                source_id("cam"),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    match immediate.open_video_clip(handle.id()).await {
        Err(error) => assert_eq!(error.code(), "clip_expired"),
        Ok(_) => panic!("zero-TTL clip must not open"),
    }
}

#[tokio::test]
async fn multipart_is_ordered_and_closed() {
    let videos = fixture().await;
    let clip = videos
        .get_video_clip(
            GetVideoClipRequest::new(
                source_id("cam"),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:03Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let mut stream = videos.open_video_clip(clip.id()).await.unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = stream.next().await {
        frames.push(frame.unwrap());
    }
    let body = encode_mjpeg_multipart(&frames, DEFAULT_MJPEG_BOUNDARY);
    let text = String::from_utf8_lossy(&body);
    assert!(text.ends_with(&format!("--{DEFAULT_MJPEG_BOUNDARY}--\r\n")));
    let captured: Vec<_> = text
        .lines()
        .filter_map(|line| line.strip_prefix("X-Captured-At: "))
        .collect();
    assert_eq!(
        captured,
        [
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:01Z",
            "2024-01-01T00:00:02Z"
        ]
    );
    assert_eq!(clip.media_type(), MJPEG_MEDIA_TYPE);
}

#[tokio::test]
async fn shared_api_opens_clip_stream() {
    let videos = fixture().await;
    let api: std::sync::Arc<dyn A2aLabApi> =
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_videos(videos)
            .share();
    let outcome = api
        .execute(A2aLabCommand::GetVideoClip(
            GetVideoClipRequest::new(
                source_id("cam"),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
            )
            .unwrap(),
        ))
        .await
        .unwrap();
    let A2aLabResult::GetVideoClip(clip) = outcome.task.result else {
        panic!("clip handle");
    };
    let mut stream = api.open_video_clip(clip.id()).await.unwrap();
    let frame = stream.next().await.unwrap().unwrap();
    assert_eq!(frame.captured_at(), timestamp("2024-01-01T00:00:00Z"));
    assert_eq!(frame.jpeg(), JPEG);
}

#[tokio::test]
async fn memory_timeline_orders_continuous_coverage_and_gaps() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam", 1)).await.unwrap();
    let empty = videos
        .get_video_timeline(GetVideoTimelineRequest::new(source_id("cam")))
        .await
        .unwrap();
    assert!(empty.intervals().is_empty());
    assert!(empty.newest_at().is_none());
    assert!(!empty.live_available());

    for at in ["2024-01-01T00:00:01Z", "2024-01-01T00:00:00Z"] {
        videos
            .insert_frame(source_id("cam"), timestamp(at), JPEG.to_vec())
            .await
            .unwrap();
    }
    let partial = videos
        .get_video_timeline(GetVideoTimelineRequest::new(source_id("cam")))
        .await
        .unwrap();
    assert_eq!(
        partial.intervals(),
        [range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z")]
    );
    assert_eq!(partial.newest_at(), Some(timestamp("2024-01-01T00:00:01Z")));

    for at in ["2024-01-01T00:00:02Z", "2024-01-01T00:00:03Z"] {
        videos
            .insert_frame(source_id("cam"), timestamp(at), JPEG.to_vec())
            .await
            .unwrap();
    }
    let continuous = videos
        .get_video_timeline(GetVideoTimelineRequest::new(source_id("cam")))
        .await
        .unwrap();
    assert_eq!(
        continuous.intervals(),
        [range("2024-01-01T00:00:00Z", "2024-01-01T00:00:04Z")]
    );
    assert_eq!(
        continuous.newest_at(),
        Some(timestamp("2024-01-01T00:00:03Z"))
    );

    videos
        .insert_frame(
            source_id("cam"),
            timestamp("2024-01-01T00:00:00.500000Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let unchanged = videos
        .get_video_timeline(GetVideoTimelineRequest::new(source_id("cam")))
        .await
        .unwrap();
    assert_eq!(unchanged.intervals().len(), 1);
    assert_eq!(
        unchanged.newest_at(),
        Some(timestamp("2024-01-01T00:00:03Z"))
    );

    videos
        .insert_frame(
            source_id("cam"),
            timestamp("2024-01-01T00:00:10Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let gapped = videos
        .get_video_timeline(GetVideoTimelineRequest::new(source_id("cam")))
        .await
        .unwrap();
    assert_eq!(
        gapped.intervals(),
        [
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:04Z"),
            range("2024-01-01T00:00:10Z", "2024-01-01T00:00:11Z"),
        ]
    );
    assert_eq!(gapped.newest_at(), Some(timestamp("2024-01-01T00:00:10Z")));
    let missing = videos
        .get_video_timeline(GetVideoTimelineRequest::new(source_id("missing")))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");
}

#[tokio::test]
async fn memory_live_follow_delivers_later_frames_in_timestamp_order() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam", 1)).await.unwrap();
    videos
        .insert_frame(
            source_id("cam"),
            timestamp("2024-01-01T00:00:00Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let unavailable = videos
        .follow_video(FollowVideoRequest::new(source_id("cam")))
        .await
        .unwrap_err();
    assert_eq!(unavailable.code(), "live_unavailable");

    videos
        .set_live_available(&source_id("cam"), true)
        .await
        .unwrap();
    assert!(
        videos
            .get_video_timeline(GetVideoTimelineRequest::new(source_id("cam")))
            .await
            .unwrap()
            .live_available()
    );
    let follow = videos
        .follow_video(FollowVideoRequest::new(source_id("cam")))
        .await
        .unwrap();
    let encoded = serde_json::to_value(&follow).unwrap();
    assert!(encoded.get("data").is_none());
    assert_eq!(
        follow.path(),
        format!("/media/video-live/{}/", follow.id().as_str())
    );
    assert!(!follow.path().contains("://"));
    let Err(unknown) = videos
        .open_video_follow(&VideoFollowId::new("missing-follow").unwrap())
        .await
    else {
        panic!("unknown follow must not open");
    };
    assert_eq!(unknown.code(), "not_found");

    let mut stream = videos.open_video_follow(follow.id()).await.unwrap();
    let waiting = tokio::time::timeout(Duration::from_millis(50), stream.next()).await;
    assert!(waiting.is_err(), "live stream stays open");
    videos
        .insert_frame(
            source_id("cam"),
            timestamp("2024-01-01T00:00:02Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    videos
        .insert_frame(
            source_id("cam"),
            timestamp("2024-01-01T00:00:01Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let second = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first.captured_at(), timestamp("2024-01-01T00:00:01Z"));
    assert_eq!(second.captured_at(), timestamp("2024-01-01T00:00:02Z"));
    assert_eq!(first.jpeg(), JPEG);
}

#[tokio::test]
async fn memory_live_follow_stops_on_unavailable_and_cancel() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam", 1)).await.unwrap();
    videos
        .set_live_available(&source_id("cam"), true)
        .await
        .unwrap();
    let follow = videos
        .follow_video(FollowVideoRequest::new(source_id("cam")))
        .await
        .unwrap();
    let mut stream = videos.open_video_follow(follow.id()).await.unwrap();
    videos
        .set_live_available(&source_id("cam"), false)
        .await
        .unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .unwrap();
    assert!(ended.is_none());
    assert_eq!(
        videos
            .follow_video(FollowVideoRequest::new(source_id("cam")))
            .await
            .unwrap_err()
            .code(),
        "live_unavailable"
    );
    assert!(
        !videos
            .get_video_timeline(GetVideoTimelineRequest::new(source_id("cam")))
            .await
            .unwrap()
            .live_available()
    );

    videos
        .set_live_available(&source_id("cam"), true)
        .await
        .unwrap();
    let again = videos
        .follow_video(FollowVideoRequest::new(source_id("cam")))
        .await
        .unwrap();
    let mut live = videos.open_video_follow(again.id()).await.unwrap();
    let reader = tokio::spawn(async move { live.next().await });
    let started = Instant::now();
    reader.abort();
    let joined = tokio::time::timeout(Duration::from_secs(1), reader)
        .await
        .expect("cancel promptly");
    match joined {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("aborted reader must stop"),
    }
    assert!(started.elapsed() < Duration::from_secs(1));
    tokio::time::timeout(
        Duration::from_secs(1),
        videos.insert_frame(
            source_id("cam"),
            timestamp("2024-01-01T00:00:04Z"),
            JPEG.to_vec(),
        ),
    )
    .await
    .expect("insert after cancel")
    .unwrap();

    let later = videos
        .follow_video(FollowVideoRequest::new(source_id("cam")))
        .await
        .unwrap();
    let mut later_stream = videos.open_video_follow(later.id()).await.unwrap();
    videos
        .insert_frame(
            source_id("cam"),
            timestamp("2024-01-01T00:00:05Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(1), later_stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(frame.captured_at(), timestamp("2024-01-01T00:00:05Z"));
}

#[tokio::test]
async fn memory_clips_report_coverage_reasons_and_keep_the_sixty_second_limit() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam", 1)).await.unwrap();
    let empty = videos
        .get_video_clip(
            GetVideoClipRequest::new(
                source_id("cam"),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(empty.code(), "no_recording");

    for at in ["2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"] {
        videos
            .insert_frame(source_id("cam"), timestamp(at), JPEG.to_vec())
            .await
            .unwrap();
    }
    videos
        .insert_frame(
            source_id("cam"),
            timestamp("2024-01-01T00:00:02Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let across = videos
        .get_video_clip(
            GetVideoClipRequest::new(
                source_id("cam"),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:03Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let mut stream = videos.open_video_clip(across.id()).await.unwrap();
    let mut stamps = Vec::new();
    while let Some(frame) = stream.next().await {
        stamps.push(frame.unwrap().captured_at().to_rfc3339());
    }
    assert_eq!(
        stamps,
        vec![
            "2024-01-01T00:00:00Z".to_owned(),
            "2024-01-01T00:00:01Z".to_owned(),
            "2024-01-01T00:00:02Z".to_owned(),
        ]
    );

    videos
        .insert_frame(
            source_id("cam"),
            timestamp("2024-01-01T00:00:10Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let requested = range("2024-01-01T00:00:00Z", "2024-01-01T00:00:11Z");
    let spanning = videos
        .get_video_clip(GetVideoClipRequest::new(source_id("cam"), requested).unwrap())
        .await
        .unwrap();
    assert_eq!(spanning.requested_range(), requested);
    assert_eq!(
        spanning.effective_range(),
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:11Z")
    );
    let mut gap_stream = videos.open_video_clip(spanning.id()).await.unwrap();
    let mut gap_stamps = Vec::new();
    while let Some(frame) = gap_stream.next().await {
        gap_stamps.push(frame.unwrap().captured_at().to_rfc3339());
    }
    assert_eq!(
        gap_stamps,
        vec![
            "2024-01-01T00:00:00Z".to_owned(),
            "2024-01-01T00:00:01Z".to_owned(),
            "2024-01-01T00:00:02Z".to_owned(),
            "2024-01-01T00:00:10Z".to_owned(),
        ]
    );
    let hole = videos
        .get_video_clip(
            GetVideoClipRequest::new(
                source_id("cam"),
                range("2024-01-01T00:00:03Z", "2024-01-01T00:00:10Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(hole.code(), "no_recording");
}

#[tokio::test]
async fn memory_clips_keep_the_sixty_second_half_open_boundary() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("hour", 1)).await.unwrap();
    let start = timestamp("2024-01-01T00:00:00Z");
    for second in 0..=60 {
        videos
            .insert_frame(
                source_id("hour"),
                start.saturating_add_micros(i64::from(second) * 1_000_000),
                JPEG.to_vec(),
            )
            .await
            .unwrap();
    }
    let limit = i64::try_from(MAX_VIDEO_CLIP_DURATION_US).unwrap();
    let bounded = videos
        .get_video_clip(
            GetVideoClipRequest::new(
                source_id("hour"),
                TimeRange::new(start, start.saturating_add_micros(limit)).unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let mut hour = videos.open_video_clip(bounded.id()).await.unwrap();
    let mut count = 0_usize;
    let mut last = None;
    while let Some(frame) = hour.next().await {
        last = Some(frame.unwrap().captured_at());
        count += 1;
    }
    assert_eq!(count, 60);
    assert_eq!(last, Some(start.saturating_add_micros(59 * 1_000_000)));
    let long = videos
        .get_video_clip(
            GetVideoClipRequest::new(
                source_id("hour"),
                TimeRange::new(start, start.saturating_add_micros(61 * 1_000_000)).unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let mut long_stream = videos.open_video_clip(long.id()).await.unwrap();
    let mut long_count = 0_usize;
    while long_stream.next().await.is_some() {
        long_count += 1;
    }
    assert_eq!(long_count, 61);
    let empty = videos
        .get_video_clip(TckMalformedVideoRequests::get_video_clip_empty_range(
            source_id("hour"),
            start,
        ))
        .await
        .unwrap_err();
    assert_eq!(empty.code(), "invalid");
}

#[tokio::test]
async fn forget_before_drops_old_frames_without_touching_later_ones() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("bench", 4)).await.unwrap();
    for stamp in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:02:00Z",
    ] {
        videos
            .insert_frame(source_id("bench"), timestamp(stamp), JPEG.to_vec())
            .await
            .unwrap();
    }
    videos
        .forget_before(&source_id("bench"), timestamp("2024-01-01T00:01:00Z"))
        .await
        .unwrap();
    let timeline = videos
        .get_video_timeline(GetVideoTimelineRequest::new(source_id("bench")))
        .await
        .unwrap();
    assert_eq!(timeline.intervals().len(), 1);
    assert_eq!(
        timeline.intervals()[0].start(),
        timestamp("2024-01-01T00:02:00Z")
    );

    let requested = range("2024-01-01T00:01:30Z", "2024-01-01T00:02:30Z");
    let trimmed = videos
        .get_video_clip(GetVideoClipRequest::new(source_id("bench"), requested).unwrap())
        .await
        .unwrap();
    assert_eq!(trimmed.requested_range(), requested);
    assert_eq!(
        trimmed.effective_range(),
        TimeRange::new(
            timestamp("2024-01-01T00:02:00Z"),
            timestamp("2024-01-01T00:02:00Z").saturating_add_micros(250_000),
        )
        .unwrap()
    );
    let mut stream = videos.open_video_clip(trimmed.id()).await.unwrap();
    let frame = stream.next().await.unwrap().unwrap();
    assert_eq!(frame.captured_at(), timestamp("2024-01-01T00:02:00Z"));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn memory_videos_dvr_indexes_mjpeg_frames_with_codec_metadata() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam", 1)).await.unwrap();
    let source = source_id("cam");
    for second in 0..5 {
        videos
            .insert_frame(
                source.clone(),
                timestamp(&format!("2024-01-01T00:00:0{second}Z")),
                JPEG.to_vec(),
            )
            .await
            .unwrap();
    }
    let track = mjpeg_track(&source);
    let tracks = videos.list_tracks(&source).await.unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].codec(), VideoCodec::Mjpeg);
    assert!(tracks[0].codec_initialization().is_empty());

    let frame = videos
        .frame_at(DvrFrameAtRequest::new(
            source.clone(),
            track.clone(),
            timestamp("2024-01-01T00:00:02.500000Z"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(frame.actual_at(), timestamp("2024-01-01T00:00:02Z"));
    assert!(matches!(frame.payload(), DvrMediaPayload::MjpegFrame(_)));
}

#[tokio::test]
async fn memory_videos_dvr_h264_gaps_sparse_keyframes_and_window_reads() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam", 1)).await.unwrap();
    let source = source_id("cam");
    let track = dvr_track("h264");
    videos
        .insert_dvr_track(
            source.clone(),
            track.clone(),
            VideoCodec::H264,
            H264_INIT.to_vec(),
        )
        .await
        .unwrap();
    for (index, start_text) in ["2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"]
        .iter()
        .enumerate()
    {
        let start = timestamp(start_text);
        videos
            .insert_dvr_segment(
                source.clone(),
                track.clone(),
                TimeRange::new(start, start.saturating_add_micros(1_000_000)).unwrap(),
                index == 0,
                VideoCodec::H264,
                vec![0x00, u8::try_from(index).unwrap()],
            )
            .await
            .unwrap();
    }
    for (index, start_text) in ["2024-01-01T00:00:04Z", "2024-01-01T00:00:05Z"]
        .iter()
        .enumerate()
    {
        let start = timestamp(start_text);
        videos
            .insert_dvr_segment(
                source.clone(),
                track.clone(),
                TimeRange::new(start, start.saturating_add_micros(1_000_000)).unwrap(),
                index == 0,
                VideoCodec::H264,
                vec![0x10, u8::try_from(index).unwrap()],
            )
            .await
            .unwrap();
    }
    let coverage = videos.media_coverage(&source).await.unwrap();
    assert_eq!(coverage.intervals().len(), 2);
    assert_eq!(coverage.discontinuities().len(), 1);
    let gap = videos
        .frame_at(DvrFrameAtRequest::new(
            source.clone(),
            track.clone(),
            timestamp("2024-01-01T00:00:03Z"),
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(gap.code(), "recording_gap");
    let page = videos
        .read_media_window(
            ReadDvrMediaWindowRequest::new(
                source,
                track,
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
                PageRequest::new(None, 10).unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.items().len(), 2);
    assert!(page.items()[0].keyframe());
    assert!(!page.items()[1].keyframe());
}

#[tokio::test]
async fn memory_videos_dvr_h265_track_exposes_initialization() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam", 1)).await.unwrap();
    let source = source_id("cam");
    let track = dvr_track("h265");
    videos
        .insert_dvr_track(
            source.clone(),
            track.clone(),
            VideoCodec::H265,
            H265_INIT.to_vec(),
        )
        .await
        .unwrap();
    let bounds = range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z");
    videos
        .insert_dvr_segment(
            source.clone(),
            track.clone(),
            bounds,
            true,
            VideoCodec::H265,
            vec![0xAA, 0xBB],
        )
        .await
        .unwrap();
    let listed = videos.list_tracks(&source).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].codec(), VideoCodec::H265);
    assert_eq!(listed[0].codec_initialization(), H265_INIT);
}

#[tokio::test]
async fn memory_videos_dvr_hour_plus_retention_uses_paged_window_reads() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam", 1)).await.unwrap();
    let source = source_id("cam");
    let track = dvr_track("h264-hour");
    videos
        .insert_dvr_track(
            source.clone(),
            track.clone(),
            VideoCodec::H264,
            H264_INIT.to_vec(),
        )
        .await
        .unwrap();
    let start = timestamp("2024-01-01T00:00:00Z");
    for second in 0..3_700 {
        let at = start.saturating_add_micros(i64::from(second) * 1_000_000);
        videos
            .insert_dvr_segment(
                source.clone(),
                track.clone(),
                TimeRange::new(at, at.saturating_add_micros(1_000_000)).unwrap(),
                second % 30 == 0,
                VideoCodec::H264,
                vec![0x01, u8::try_from(second % 255).unwrap()],
            )
            .await
            .unwrap();
    }
    let retention = videos.retention_bounds(&source).await.unwrap();
    assert!(retention.newest().as_micros() - retention.oldest().as_micros() >= 3_600_000_000);
    let window = range("2024-01-01T01:00:00Z", "2024-01-01T01:00:05Z");
    let page = videos
        .read_media_window(
            ReadDvrMediaWindowRequest::new(
                source,
                track,
                window,
                PageRequest::new(None, 3).unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.items().len(), 3);
    assert!(page.next_cursor().is_some());
}

#[tokio::test]
async fn memory_videos_dvr_forget_before_trims_retention_bounds() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam", 1)).await.unwrap();
    let source = source_id("cam");
    for stamp in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:02:00Z",
    ] {
        videos
            .insert_frame(source.clone(), timestamp(stamp), JPEG.to_vec())
            .await
            .unwrap();
    }
    videos
        .forget_before(&source, timestamp("2024-01-01T00:01:00Z"))
        .await
        .unwrap();
    let retention = videos.retention_bounds(&source).await.unwrap();
    assert_eq!(retention.oldest(), timestamp("2024-01-01T00:02:00Z"));
    let coverage = videos.media_coverage(&source).await.unwrap();
    assert_eq!(coverage.intervals().len(), 1);
}

#[tokio::test]
async fn memory_videos_dvr_clip_handles_still_expire() {
    let videos = MemoryVideos::with_clip_ttl(Duration::ZERO);
    videos.insert_source(source("cam", 1)).await.unwrap();
    let source = source_id("cam");
    videos
        .insert_frame(
            source.clone(),
            timestamp("2024-01-01T00:00:00Z"),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
    let clip = videos
        .get_video_clip(
            GetVideoClipRequest::new(
                source,
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    match videos.open_video_clip(clip.id()).await {
        Err(error) => assert_eq!(error.code(), "clip_expired"),
        Ok(_) => panic!("zero-TTL clip must not open"),
    }
}

#[test]
fn video_clip_path_is_relative() {
    let id = a2a_lab_dev_kit::VideoClipId::new("clip-1").unwrap();
    let clip = VideoClip::new_mjpeg(
        id,
        source_id("cam"),
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
    )
    .unwrap();
    assert_eq!(clip.path(), "/media/video-clips/clip-1/");
}
