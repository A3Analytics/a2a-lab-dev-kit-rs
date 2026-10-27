use std::collections::BTreeSet;

use futures_util::{Stream, StreamExt};
use schemars::JsonSchema;

use a2a_lab_dev_kit::{
    A2aLabApi, A2aLabCommand, A2aLabError, A2aLabResult, A2aLabService, FollowVideoRequest,
    GetVideoClipRequest, GetVideoFrameAtRequest, GetVideoFrameAtResult, GetVideoTimelineRequest,
    Image, ImageDescriptor, ImageId, ImageSourceId, JsonObject, ListVideoSourcesRequest,
    MAX_VIDEO_CLIP_DURATION_US, MJPEG_MEDIA_TYPE, MemoryLogs, MemoryMetrics, MemoryTasks,
    TimeRange, UtcTimestamp, VideoAvailability, VideoClipId, VideoFollow, VideoFollowId,
    VideoFrame, VideoProvider, VideoSource, VideoSourceId, VideoTimeline, video_live_path,
};

const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xD9];

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).expect("timestamp")
}

fn source_id(id: &str) -> VideoSourceId {
    VideoSourceId::new(id).unwrap()
}

fn range(start: &str, end: &str) -> TimeRange {
    TimeRange::new(timestamp(start), timestamp(end)).unwrap()
}

fn timeline(
    intervals: Vec<TimeRange>,
    newest_at: Option<&str>,
    live_available: bool,
) -> Result<VideoTimeline, A2aLabError> {
    VideoTimeline::new(
        source_id("cam"),
        intervals,
        newest_at.map(timestamp),
        live_available,
    )
}

fn schema<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema")
}

fn object_keys(value: &serde_json::Value) -> BTreeSet<String> {
    value.as_object().expect("object").keys().cloned().collect()
}

fn names(value: &serde_json::Value, field: &str) -> BTreeSet<String> {
    match value.get(field).and_then(serde_json::Value::as_array) {
        Some(items) => items
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_owned)
            .collect(),
        None => value
            .get(field)
            .and_then(serde_json::Value::as_object)
            .map(|items| items.keys().cloned().collect())
            .unwrap_or_default(),
    }
}

#[test]
fn timeline_accepts_one_logical_interval_and_genuine_gaps() {
    let continuous = timeline(
        vec![range("2024-01-01T00:00:00Z", "2024-01-01T00:00:05Z")],
        Some("2024-01-01T00:00:04Z"),
        true,
    )
    .unwrap();
    assert_eq!(continuous.intervals().len(), 1);
    assert!(continuous.live_available());
    assert_eq!(
        continuous.newest_at(),
        Some(timestamp("2024-01-01T00:00:04Z"))
    );

    let gapped = timeline(
        vec![
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            range("2024-01-01T00:00:04Z", "2024-01-01T00:00:06Z"),
        ],
        Some("2024-01-01T00:00:05Z"),
        false,
    )
    .unwrap();
    assert_eq!(gapped.intervals().len(), 2);
    assert!(!gapped.live_available());

    let empty = timeline(Vec::new(), None, true).unwrap();
    assert!(empty.intervals().is_empty());
    assert!(empty.newest_at().is_none());
}

#[test]
fn timeline_rejects_reversed_overlapping_abutting_and_unordered_intervals() {
    let reversed = serde_json::json!({
        "source_id": "cam",
        "intervals": [{
            "start": "2024-01-01T00:00:02Z",
            "end": "2024-01-01T00:00:00Z"
        }],
        "live_available": false
    });
    assert!(serde_json::from_value::<VideoTimeline>(reversed).is_err());
    assert!(
        timeline(
            vec![
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:03Z"),
                range("2024-01-01T00:00:02Z", "2024-01-01T00:00:04Z"),
            ],
            None,
            false,
        )
        .is_err()
    );
    assert!(
        timeline(
            vec![
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
                range("2024-01-01T00:00:02Z", "2024-01-01T00:00:04Z"),
            ],
            None,
            false,
        )
        .is_err()
    );
    assert!(
        timeline(
            vec![
                range("2024-01-01T00:00:04Z", "2024-01-01T00:00:06Z"),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            ],
            None,
            false,
        )
        .is_err()
    );
    assert!(
        timeline(
            vec![range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z")],
            Some("2024-01-01T00:00:03Z"),
            false,
        )
        .is_err()
    );
    assert!(timeline(Vec::new(), Some("2024-01-01T00:00:00Z"), false).is_err());
}

#[test]
fn timeline_json_hides_storage_boundaries_and_rejects_unknown_fields() {
    let timeline = timeline(
        vec![
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            range("2024-01-01T00:00:04Z", "2024-01-01T00:00:06Z"),
        ],
        Some("2024-01-01T00:00:05Z"),
        true,
    )
    .unwrap();
    let value = serde_json::to_value(&timeline).unwrap();
    assert_eq!(
        object_keys(&value),
        BTreeSet::from([
            "source_id".to_owned(),
            "intervals".to_owned(),
            "newest_at".to_owned(),
            "live_available".to_owned(),
        ])
    );
    let encoded = value.to_string();
    for hidden in ["partition", "file", "chunk", "path", "bytes"] {
        assert!(!encoded.contains(hidden), "{hidden}");
    }
    let mut extra = value.clone();
    extra["partition"] = serde_json::json!("file-1");
    assert!(serde_json::from_value::<VideoTimeline>(extra).is_err());
    let restored: VideoTimeline = serde_json::from_value(value).unwrap();
    assert_eq!(restored, timeline);
}

#[test]
fn live_follow_metadata_is_an_opaque_relative_mjpeg_path() {
    let follow = VideoFollow::new(
        VideoFollowId::new("follow-1").unwrap(),
        source_id("cam"),
        MJPEG_MEDIA_TYPE,
    )
    .unwrap();
    assert_eq!(follow.path(), "/media/video-live/follow-1");
    assert_eq!(follow.path(), video_live_path(follow.id()));
    assert_eq!(follow.media_type(), MJPEG_MEDIA_TYPE);
    assert!(VideoFollow::new(follow.id().clone(), source_id("cam"), "video/mp4").is_err());

    let value = serde_json::to_value(&follow).unwrap();
    assert_eq!(
        object_keys(&value),
        BTreeSet::from([
            "id".to_owned(),
            "source_id".to_owned(),
            "media_type".to_owned(),
            "path".to_owned(),
        ])
    );
    for absent in ["data", "jpeg", "bytes", "bearer", "authorization", "token"] {
        assert!(value.get(absent).is_none(), "{absent}");
    }
    assert!(!follow.path().contains("://"));
    assert!(!follow.path().contains('?'));
    assert!(!follow.path().contains('#'));

    for path in [
        "https://camera.example/media/video-live/follow-1",
        "/media/video-live/follow-1?access_token=secret",
        "/media/video-live/follow-1#fragment",
        "//camera.example/media/video-live/follow-1",
        "media/video-live/follow-1",
    ] {
        let mut rejected = value.clone();
        rejected["path"] = serde_json::json!(path);
        assert!(
            serde_json::from_value::<VideoFollow>(rejected).is_err(),
            "{path}"
        );
    }
    let mut extra = value.clone();
    extra["bearer"] = serde_json::json!("secret");
    assert!(serde_json::from_value::<VideoFollow>(extra).is_err());
    assert_eq!(
        serde_json::from_value::<VideoFollow>(value).unwrap(),
        follow
    );
}

#[test]
fn video_availability_reasons_round_trip_without_parsing_messages() {
    let cases = [
        (
            A2aLabError::no_recording("range is empty"),
            VideoAvailability::NoRecording,
            "no_recording",
        ),
        (
            A2aLabError::recording_gap("range is empty"),
            VideoAvailability::RecordingGap,
            "recording_gap",
        ),
        (
            A2aLabError::clip_expired("range is empty"),
            VideoAvailability::ClipExpired,
            "clip_expired",
        ),
        (
            A2aLabError::live_unavailable("range is empty"),
            VideoAvailability::LiveUnavailable,
            "live_unavailable",
        ),
    ];
    for (error, availability, code) in cases {
        assert_eq!(error.availability(), Some(availability));
        assert_eq!(error.code(), code);
        assert_eq!(availability.as_str(), code);
        let restored = A2aLabError::from_code(error.code(), "range is empty");
        assert_eq!(restored.availability(), Some(availability));
        assert_eq!(restored.code(), code);
        assert_ne!(restored, A2aLabError::not_found("video coverage", code));
        assert_ne!(restored, A2aLabError::unavailable(code));
    }
}

#[test]
fn legacy_video_json_shapes_and_clip_limit_stay_compatible() {
    let source = VideoSource::new(
        source_id("cam"),
        "Camera",
        "Deck recording",
        None,
        None,
        640,
        480,
        30,
        MJPEG_MEDIA_TYPE,
    )
    .unwrap();
    let source_json = serde_json::to_value(&source).unwrap();
    assert_eq!(
        object_keys(&source_json),
        BTreeSet::from([
            "id".to_owned(),
            "name".to_owned(),
            "description".to_owned(),
            "width".to_owned(),
            "height".to_owned(),
            "fps".to_owned(),
            "media_type".to_owned(),
        ])
    );
    let restored = serde_json::from_value::<VideoSource>(source_json).unwrap();
    assert_eq!(restored, source);

    let frame_request =
        GetVideoFrameAtRequest::new(source_id("cam"), timestamp("2024-01-01T00:00:00Z"), None);
    let command =
        serde_json::to_value(A2aLabCommand::GetVideoFrameAt(frame_request.clone())).unwrap();
    assert_eq!(command["operation"], "get_video_frame_at");
    assert_eq!(
        object_keys(&command["params"]),
        BTreeSet::from(["source_id".to_owned(), "at".to_owned()])
    );
    assert!(matches!(
        serde_json::from_value::<A2aLabCommand>(command).unwrap(),
        A2aLabCommand::GetVideoFrameAt(_)
    ));

    let image = Image::new(
        ImageDescriptor::new(
            ImageId::new("frame-1").unwrap(),
            ImageSourceId::new("cam").unwrap(),
            timestamp("2024-01-01T00:00:00Z"),
            "image/jpeg",
            1,
            1,
            None,
            JsonObject::empty(),
        )
        .unwrap(),
        JPEG.to_vec(),
    )
    .unwrap();
    let frame = GetVideoFrameAtResult::new(timestamp("2024-01-01T00:00:00Z"), image);
    let frame_json = serde_json::to_value(A2aLabResult::GetVideoFrameAt(frame)).unwrap();
    assert_eq!(frame_json["operation"], "get_video_frame_at");
    assert_eq!(
        object_keys(&frame_json["result"]),
        BTreeSet::from([
            "requested_at".to_owned(),
            "actual_captured_at".to_owned(),
            "delta_us".to_owned(),
            "image".to_owned(),
        ])
    );

    let clip_command = serde_json::to_value(A2aLabCommand::GetVideoClip(
        GetVideoClipRequest::new(
            source_id("cam"),
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
        )
        .unwrap(),
    ))
    .unwrap();
    assert_eq!(clip_command["operation"], "get_video_clip");
    assert_eq!(
        object_keys(&clip_command["params"]),
        BTreeSet::from(["source_id".to_owned(), "range".to_owned()])
    );

    let start = timestamp("2024-01-01T00:00:00Z");
    let limit = i64::try_from(MAX_VIDEO_CLIP_DURATION_US).unwrap();
    assert!(
        GetVideoClipRequest::new(
            source_id("cam"),
            TimeRange::new(start, start.saturating_add_micros(limit)).unwrap(),
        )
        .is_ok()
    );
    assert!(
        GetVideoClipRequest::new(
            source_id("cam"),
            TimeRange::new(start, start.saturating_add_micros(limit + 1)).unwrap(),
        )
        .is_err()
    );
    let _ = VideoClipId::new("clip-1").unwrap();
}

#[test]
fn video_schema_exposes_timeline_and_preserves_legacy_contracts() {
    let timeline = schema::<VideoTimeline>();
    assert_eq!(
        names(&timeline, "properties"),
        BTreeSet::from([
            "source_id".to_owned(),
            "intervals".to_owned(),
            "newest_at".to_owned(),
            "live_available".to_owned(),
        ])
    );
    assert_eq!(
        names(&timeline, "required"),
        BTreeSet::from([
            "source_id".to_owned(),
            "intervals".to_owned(),
            "live_available".to_owned(),
        ])
    );

    let follow = schema::<VideoFollow>();
    assert_eq!(
        names(&follow, "properties"),
        BTreeSet::from([
            "id".to_owned(),
            "source_id".to_owned(),
            "media_type".to_owned(),
            "path".to_owned(),
        ])
    );
    assert_eq!(names(&follow, "required"), names(&follow, "properties"));

    let frame = schema::<GetVideoFrameAtRequest>();
    assert_eq!(
        names(&frame, "properties"),
        BTreeSet::from([
            "source_id".to_owned(),
            "at".to_owned(),
            "max_delta_us".to_owned(),
        ])
    );
    assert_eq!(
        names(&frame, "required"),
        BTreeSet::from(["source_id".to_owned(), "at".to_owned()])
    );
    let frame_result = schema::<GetVideoFrameAtResult>();
    assert_eq!(
        names(&frame_result, "properties"),
        BTreeSet::from([
            "requested_at".to_owned(),
            "actual_captured_at".to_owned(),
            "delta_us".to_owned(),
            "image".to_owned(),
        ])
    );

    let source = schema::<VideoSource>();
    assert_eq!(
        names(&source, "required"),
        BTreeSet::from([
            "id".to_owned(),
            "name".to_owned(),
            "description".to_owned(),
            "width".to_owned(),
            "height".to_owned(),
            "fps".to_owned(),
            "media_type".to_owned(),
        ])
    );
    assert!(names(&source, "properties").contains("asset_id"));
    assert!(names(&source, "properties").contains("semantic_id"));
    assert!(!names(&source, "required").contains("asset_id"));
    assert!(!names(&source, "required").contains("semantic_id"));
}

struct ScriptedVideos {
    timeline: VideoTimeline,
    follow: VideoFollow,
    frame: VideoFrame,
}

impl VideoProvider for ScriptedVideos {
    fn list_video_sources(
        &self,
        _request: ListVideoSourcesRequest,
    ) -> impl Future<Output = Result<a2a_lab_dev_kit::Page<VideoSource>, A2aLabError>> + Send {
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
        _request: a2a_lab_dev_kit::GetVideoClipRequest,
    ) -> impl Future<Output = Result<a2a_lab_dev_kit::VideoClip, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::unavailable("unused")))
    }

    fn open_video_clip(
        &self,
        _id: &VideoClipId,
    ) -> impl Future<Output = Result<a2a_lab_dev_kit::VideoClipStream, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::unavailable("unused")))
    }

    fn get_video_timeline(
        &self,
        _request: GetVideoTimelineRequest,
    ) -> impl Future<Output = Result<VideoTimeline, A2aLabError>> + Send {
        let timeline = self.timeline.clone();
        std::future::ready(Ok(timeline))
    }

    fn follow_video(
        &self,
        _request: FollowVideoRequest,
    ) -> impl Future<Output = Result<VideoFollow, A2aLabError>> + Send {
        let follow = self.follow.clone();
        std::future::ready(Ok(follow))
    }

    fn open_video_follow(
        &self,
        _id: &VideoFollowId,
    ) -> impl Future<Output = Result<a2a_lab_dev_kit::VideoLiveStream, A2aLabError>> + Send {
        let frame = self.frame.clone();
        let stream: a2a_lab_dev_kit::VideoLiveStream = Box::pin(one_frame(frame));
        std::future::ready(Ok(stream))
    }
}

fn one_frame(frame: VideoFrame) -> impl Stream<Item = Result<VideoFrame, A2aLabError>> + Send {
    futures_util::stream::iter(std::iter::once(Ok(frame)))
}

#[tokio::test]
async fn service_dispatches_timeline_and_live_follow() {
    let follow = VideoFollow::new(
        VideoFollowId::new("follow-1").unwrap(),
        source_id("cam"),
        MJPEG_MEDIA_TYPE,
    )
    .unwrap();
    let videos = ScriptedVideos {
        timeline: timeline(
            vec![range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z")],
            Some("2024-01-01T00:00:00Z"),
            true,
        )
        .unwrap(),
        follow: follow.clone(),
        frame: VideoFrame::new(timestamp("2024-01-01T00:00:01Z"), JPEG.to_vec()).unwrap(),
    };
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos);
    let timeline = service
        .execute(A2aLabCommand::GetVideoTimeline(
            GetVideoTimelineRequest::new(source_id("cam")),
        ))
        .await
        .unwrap();
    let A2aLabResult::GetVideoTimeline(timeline) = timeline.task.result else {
        panic!("timeline dispatch");
    };
    assert_eq!(timeline.intervals().len(), 1);
    assert!(timeline.live_available());

    let followed = service
        .execute(A2aLabCommand::FollowVideo(FollowVideoRequest::new(
            source_id("cam"),
        )))
        .await
        .unwrap();
    let A2aLabResult::FollowVideo(handle) = followed.task.result else {
        panic!("live follow dispatch");
    };
    let encoded = serde_json::to_value(&handle).unwrap();
    assert!(encoded.get("data").is_none());
    assert_eq!(handle, follow);

    let api: std::sync::Arc<dyn A2aLabApi> = service.share();
    let mut stream = api.open_video_follow(handle.id()).await.unwrap();
    let frame = stream.next().await.unwrap().unwrap();
    assert_eq!(frame.captured_at(), timestamp("2024-01-01T00:00:01Z"));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn empty_videos_constructor_leaves_new_operations_unavailable() {
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new());
    let timeline = service
        .execute(A2aLabCommand::GetVideoTimeline(
            GetVideoTimelineRequest::new(source_id("cam")),
        ))
        .await
        .unwrap_err();
    let follow = service
        .execute(A2aLabCommand::FollowVideo(FollowVideoRequest::new(
            source_id("cam"),
        )))
        .await
        .unwrap_err();
    let opened = service
        .open_video_follow(&VideoFollowId::new("follow-1").unwrap())
        .await
        .map(|_| ())
        .expect_err("empty service");
    for error in [timeline, follow, opened] {
        assert_eq!(error.code(), "unavailable");
        assert!(error.availability().is_none());
    }
}
