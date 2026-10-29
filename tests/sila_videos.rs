#![cfg(feature = "sila2")]

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use a2a_lab_dev_kit::sila::api::{
    BinaryDownloadClient, BinaryUnion, Boolean, GetChunkRequest, GetFeatureDefinitionParameters,
    GetVideoTimelineParameters, Integer, LabVideosClient, ListVideoSourcesParameters,
    SiLaServiceClient, SilaString, Timestamp, VideoPageRequest, VideoPageStruct, VideoTimeRange,
    VideoTimeStruct,
};
use a2a_lab_dev_kit::sila::{SilaIdentity, SilaServer};
use a2a_lab_dev_kit::{
    A2aLabError, A2aLabService, GetVideoClipRequest, GetVideoFrameAtRequest, GetVideoFrameAtResult,
    GetVideoTimelineRequest, ListVideoSourcesRequest, MJPEG_MEDIA_TYPE, MemoryLogs, MemoryMetrics,
    MemoryTasks, MemoryVideos, Page, UtcTimestamp, VideoClip, VideoClipId, VideoClipStream,
    VideoProvider, VideoSource, VideoSourceId, VideoTimeline,
};
use futures_util::StreamExt;
use tonic::Code;

const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
];
const CHUNK: u32 = 1024 * 1024;

#[tokio::test]
async fn sila_videos_list_frame_and_clip_through_binary_download() {
    let server = serve_videos(catalog().await).await;
    let channel = channel_of(&server).await;
    let mut client = LabVideosClient::new(channel.clone());
    let listed = client
        .list_video_sources(ListVideoSourcesParameters {
            page: Some(page(10)),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        listed.sources[0]
            .video_source
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .value,
        "cam"
    );

    let frame = client
        .get_video_frame_at(a2a_lab_dev_kit::sila::api::GetVideoFrameAtParameters {
            source_id: Some(text("cam")),
            at: Some(utc(2024, 1, 1, 0, 0, 0)),
            has_max_delta_us: Some(Boolean { value: false }),
            max_delta_us: Some(Integer { value: 0 }),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(frame.delta_us.unwrap().value, 0);
    let mut downloads = BinaryDownloadClient::new(channel);
    assert_eq!(
        download(&mut downloads, &transfer_id(frame.image.unwrap())).await,
        JPEG
    );

    let clip = client
        .get_video_clip(a2a_lab_dev_kit::sila::api::GetVideoClipParameters {
            source_id: Some(text("cam")),
            range: Some(range(utc(2024, 1, 1, 0, 0, 0), utc(2024, 1, 1, 0, 0, 2))),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(clip.path.unwrap().value.starts_with("/media/video-clips/"));
    let body = download(&mut downloads, &transfer_id(clip.clip.unwrap())).await;
    assert!(String::from_utf8_lossy(&body).contains("X-Captured-At:"));
    assert!(body.windows(2).any(|pair| pair == &JPEG[..2]));
}

#[tokio::test]
async fn sila_videos_reject_missing_invalid_and_unavailable() {
    let server = serve_videos(catalog().await).await;
    let mut client = LabVideosClient::new(channel_of(&server).await);
    let missing = client
        .get_video_frame_at(a2a_lab_dev_kit::sila::api::GetVideoFrameAtParameters {
            source_id: Some(text("missing")),
            at: Some(utc(2024, 1, 1, 0, 0, 0)),
            has_max_delta_us: Some(Boolean { value: false }),
            max_delta_us: Some(Integer { value: 0 }),
        })
        .await
        .unwrap_err();
    assert_eq!(missing.code(), Code::Aborted);
    assert!(decoded(&missing).contains("VideoSourceNotFound"));
    let invalid = client
        .list_video_sources(ListVideoSourcesParameters {
            page: Some(page(0)),
        })
        .await
        .unwrap_err();
    assert_eq!(invalid.code(), Code::Aborted);
    let missing_clip = client
        .get_video_clip(a2a_lab_dev_kit::sila::api::GetVideoClipParameters {
            source_id: Some(text("cam")),
            range: Some(range(utc(2025, 1, 1, 0, 0, 0), utc(2025, 1, 1, 0, 0, 1))),
        })
        .await
        .unwrap_err();
    assert_eq!(missing_clip.code(), Code::Aborted);
    assert!(defined(&missing_clip, "NoRecording"));
    assert!(decoded(&missing_clip).contains("no retained recording"));
    assert!(!defined(&missing_clip, "VideoClipNotFound"));

    let empty = serve_videos_lab(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share(),
    )
    .await;
    let mut empty_client = LabVideosClient::new(channel_of(&empty).await);
    let unavailable = empty_client
        .list_video_sources(ListVideoSourcesParameters {
            page: Some(page(10)),
        })
        .await
        .unwrap_err();
    assert_eq!(unavailable.code(), Code::Aborted);
    assert!(decoded(&unavailable).contains("ProviderUnavailable"));
}

async fn catalog() -> MemoryVideos {
    let videos = MemoryVideos::new();
    videos
        .insert_source(
            VideoSource::new(
                VideoSourceId::new("cam").unwrap(),
                "cam",
                "Deck camera",
                None,
                None,
                1,
                1,
                1,
                MJPEG_MEDIA_TYPE,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    for at in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:02Z",
    ] {
        videos
            .insert_frame(
                VideoSourceId::new("cam").unwrap(),
                UtcTimestamp::parse(at).unwrap(),
                JPEG.to_vec(),
            )
            .await
            .unwrap();
    }
    videos
}

async fn serve_videos(videos: MemoryVideos) -> a2a_lab_dev_kit::sila::SilaServerHandle {
    serve_videos_lab(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_videos(videos)
            .share(),
    )
    .await
}

async fn serve_videos_lab(
    lab: Arc<dyn a2a_lab_dev_kit::A2aLabApi>,
) -> a2a_lab_dev_kit::sila::SilaServerHandle {
    SilaServer::new(
        SilaIdentity::lab_dev_kit("11111111-1111-1111-1111-111111111111").unwrap(),
        lab,
    )
    .plaintext()
    .serve("127.0.0.1:0".parse().unwrap())
    .await
    .unwrap()
}

async fn channel_of(server: &a2a_lab_dev_kit::sila::SilaServerHandle) -> tonic::transport::Channel {
    tonic::transport::Channel::from_shared(format!("http://{}", server.local_addr()))
        .unwrap()
        .connect()
        .await
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
        .get_binary_info(a2a_lab_dev_kit::sila::api::GetBinaryInfoRequest {
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
        bytes.extend(chunk.payload);
        offset += u64::from(length);
    }
    bytes
}

fn decoded(status: &tonic::Status) -> String {
    String::from_utf8_lossy(
        &base64::Engine::decode(&base64::engine::general_purpose::STANDARD, status.message())
            .unwrap(),
    )
    .into_owned()
}

fn page(limit: i64) -> VideoPageRequest {
    VideoPageRequest {
        page_request: Some(VideoPageStruct {
            has_cursor: Some(Boolean { value: false }),
            cursor: Some(SilaString {
                value: String::new(),
            }),
            limit: Some(Integer { value: limit }),
        }),
    }
}

fn range(start: Timestamp, end: Timestamp) -> VideoTimeRange {
    VideoTimeRange {
        time_range: Some(VideoTimeStruct {
            start: Some(start),
            end: Some(end),
        }),
    }
}

fn utc(year: u32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> Timestamp {
    Timestamp {
        year,
        month,
        day,
        hour,
        minute,
        second,
        millisecond: 0,
        timezone: Some(a2a_lab_dev_kit::sila::api::Timezone {
            hours: 0,
            minutes: 0,
        }),
    }
}

fn text(value: &str) -> SilaString {
    SilaString {
        value: value.to_owned(),
    }
}

fn defined(status: &tonic::Status, identifier: &str) -> bool {
    decoded(status).contains(&format!("DefinedExecutionError/{identifier}"))
}

fn timeline_request(source_id: &str) -> GetVideoTimelineParameters {
    GetVideoTimelineParameters {
        source_id: Some(text(source_id)),
    }
}

fn expect_utc(stamp: &Timestamp, second: u32) {
    assert_eq!(
        (
            stamp.year,
            stamp.month,
            stamp.day,
            stamp.hour,
            stamp.minute,
            stamp.second
        ),
        (2024, 1, 1, 0, 0, second)
    );
}

fn bounds(range: &VideoTimeRange) -> (&Timestamp, &Timestamp) {
    let inner = range.time_range.as_ref().unwrap();
    (inner.start.as_ref().unwrap(), inner.end.as_ref().unwrap())
}

async fn add_source(videos: &MemoryVideos, id: &str) {
    videos
        .insert_source(
            VideoSource::new(
                VideoSourceId::new(id).unwrap(),
                id,
                "Deck camera",
                None,
                None,
                1,
                1,
                1,
                MJPEG_MEDIA_TYPE,
            )
            .unwrap(),
        )
        .await
        .unwrap();
}

async fn add_frame(videos: &MemoryVideos, id: &str, at: &str) {
    videos
        .insert_frame(
            VideoSourceId::new(id).unwrap(),
            UtcTimestamp::parse(at).unwrap(),
            JPEG.to_vec(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn sila_video_timeline_preserves_gaps_and_empty_retention() {
    let videos = MemoryVideos::new();
    add_source(&videos, "cam").await;
    add_source(&videos, "empty").await;
    for at in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:10Z",
    ] {
        add_frame(&videos, "cam", at).await;
    }
    videos
        .set_live_available(&VideoSourceId::new("cam").unwrap(), true)
        .await
        .unwrap();
    let server = serve_videos(videos).await;
    let mut client = LabVideosClient::new(channel_of(&server).await);
    let timeline = client
        .get_video_timeline(timeline_request("cam"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(timeline.source_id.unwrap().value, "cam");
    assert_eq!(timeline.intervals.len(), 2);
    let (start, end) = bounds(&timeline.intervals[0]);
    expect_utc(start, 0);
    expect_utc(end, 2);
    let (gap_start, gap_end) = bounds(&timeline.intervals[1]);
    expect_utc(gap_start, 10);
    expect_utc(gap_end, 11);
    assert!(timeline.has_newest_at.unwrap().value);
    expect_utc(timeline.newest_at.as_ref().unwrap(), 10);
    assert!(timeline.live_available.unwrap().value);

    let empty = client
        .get_video_timeline(timeline_request("empty"))
        .await
        .unwrap()
        .into_inner();
    assert!(empty.intervals.is_empty());
    assert!(!empty.has_newest_at.unwrap().value);
    assert!(empty.newest_at.is_none());
    assert!(!empty.live_available.unwrap().value);
}

#[tokio::test]
async fn sila_video_timeline_rejects_malformed_requests() {
    let server = serve_videos(catalog().await).await;
    let mut client = LabVideosClient::new(channel_of(&server).await);
    let missing = client
        .get_video_timeline(GetVideoTimelineParameters { source_id: None })
        .await
        .unwrap_err();
    assert_eq!(missing.code(), Code::Aborted);
    assert!(decoded(&missing).contains("GetVideoTimeline/Parameter/SourceId"));
    assert!(!defined(&missing, "NoRecording"));
    let blank = client
        .get_video_timeline(timeline_request(""))
        .await
        .unwrap_err();
    assert!(decoded(&blank).contains("GetVideoTimeline/Parameter/SourceId"));
    let spaced = client
        .get_video_timeline(timeline_request("bad id"))
        .await
        .unwrap_err();
    assert_eq!(spaced.code(), Code::Aborted);
    assert!(decoded(&spaced).contains("GetVideoTimeline/Parameter/SourceId"));
    assert!(!decoded(&spaced).contains("DefinedExecutionError/"));
}

#[tokio::test]
async fn sila_videos_map_availability_without_collapsing_lookup_errors() {
    let videos = MemoryVideos::new();
    add_source(&videos, "cam").await;
    for at in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:10Z",
    ] {
        add_frame(&videos, "cam", at).await;
    }
    let server = serve_videos(videos).await;
    let mut client = LabVideosClient::new(channel_of(&server).await);
    let missing_source = client
        .get_video_timeline(timeline_request("missing"))
        .await
        .unwrap_err();
    assert!(defined(&missing_source, "VideoSourceNotFound"));
    assert!(!defined(&missing_source, "NoRecording"));
    assert!(!defined(&missing_source, "LiveUnavailable"));

    let uncovered = client
        .get_video_clip(a2a_lab_dev_kit::sila::api::GetVideoClipParameters {
            source_id: Some(text("cam")),
            range: Some(range(utc(2025, 1, 1, 0, 0, 0), utc(2025, 1, 1, 0, 0, 1))),
        })
        .await
        .unwrap_err();
    assert!(defined(&uncovered, "NoRecording"));
    assert!(!defined(&uncovered, "VideoClipNotFound"));
    assert!(decoded(&uncovered).contains("no retained recording"));

    assert_sila_gap_spanning_clip_succeeds(&mut client).await;
    assert_sila_maps_recording_gap_from_provider().await;

    let frame = client
        .get_video_frame_at(a2a_lab_dev_kit::sila::api::GetVideoFrameAtParameters {
            source_id: Some(text("cam")),
            at: Some(utc(2025, 1, 1, 0, 0, 0)),
            has_max_delta_us: Some(Boolean { value: false }),
            max_delta_us: Some(Integer { value: 0 }),
        })
        .await
        .unwrap_err();
    assert!(defined(&frame, "VideoFrameNotFound"));
    assert!(!defined(&frame, "NoRecording"));

    let clip = client
        .get_video_clip(a2a_lab_dev_kit::sila::api::GetVideoClipParameters {
            source_id: Some(text("cam")),
            range: Some(range(utc(2024, 1, 1, 0, 0, 0), utc(2024, 1, 1, 0, 0, 2))),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(clip.path.unwrap().value.starts_with("/media/video-clips/"));
    assert!(clip.clip.is_some());

    let expired_videos = MemoryVideos::with_clip_ttl(Duration::ZERO);
    add_source(&expired_videos, "cam").await;
    add_frame(&expired_videos, "cam", "2024-01-01T00:00:00Z").await;
    let expired_server = serve_videos(expired_videos).await;
    let mut expired_client = LabVideosClient::new(channel_of(&expired_server).await);
    let expired = expired_client
        .get_video_clip(a2a_lab_dev_kit::sila::api::GetVideoClipParameters {
            source_id: Some(text("cam")),
            range: Some(range(utc(2024, 1, 1, 0, 0, 0), utc(2024, 1, 1, 0, 0, 1))),
        })
        .await
        .unwrap_err();
    assert!(defined(&expired, "ClipExpired"));
    assert!(!defined(&expired, "VideoClipNotFound"));
    assert!(!defined(&expired, "NoRecording"));

    let empty = serve_videos_lab(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new()).share(),
    )
    .await;
    let unavailable = LabVideosClient::new(channel_of(&empty).await)
        .get_video_timeline(timeline_request("cam"))
        .await
        .unwrap_err();
    assert!(defined(&unavailable, "ProviderUnavailable"));
    assert!(!defined(&unavailable, "LiveUnavailable"));

    let refused = serve_videos_lab(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_videos(LiveRefused)
            .share(),
    )
    .await;
    let live = LabVideosClient::new(channel_of(&refused).await)
        .get_video_timeline(timeline_request("cam"))
        .await
        .unwrap_err();
    assert!(defined(&live, "LiveUnavailable"));
    assert!(!defined(&live, "ProviderUnavailable"));
    assert!(!defined(&live, "NoRecording"));
    assert!(decoded(&live).contains("live follow is not available"));
}

#[tokio::test]
async fn sila_lab_videos_minor_version_matches_proto() {
    let xml = include_str!("../src/sila/standard/LabVideos.sila.xml");
    let proto = include_str!("../proto/sila/LabVideos.proto");
    assert!(xml.contains("FeatureVersion=\"1.1\""));
    assert!(proto.contains("package sila2.com.a3analytics.lab.labvideos.v1;"));
    assert!(xml.contains("<Identifier>GetVideoTimeline</Identifier>"));
    assert!(proto.contains("rpc GetVideoTimeline "));
    assert!(xml.contains("No command delivers frames added after the call."));
    assert!(proto.contains("No command delivers frames added after the call."));
    assert!(!xml.contains("<Observable>Yes</Observable>"));
    assert!(!xml.contains("FollowVideo"));
    assert!(!proto.contains("FollowVideo"));
    assert!(proto.contains("Binary Clip = 7;"));
    for field in ["Intervals", "HasNewestAt", "NewestAt", "LiveAvailable"] {
        assert!(
            xml.contains(&format!("<Identifier>{field}</Identifier>")),
            "{field}"
        );
        assert!(proto.contains(&format!(" {field} =")), "{field}");
    }
    for error in [
        "NoRecording",
        "RecordingGap",
        "ClipExpired",
        "LiveUnavailable",
        "VideoSourceNotFound",
        "VideoFrameNotFound",
        "VideoClipNotFound",
        "ProviderUnavailable",
    ] {
        assert!(
            xml.contains(&format!("<Identifier>{error}</Identifier>")),
            "{error}"
        );
    }

    let server = serve_videos(catalog().await).await;
    let definition = SiLaServiceClient::new(channel_of(&server).await)
        .get_feature_definition(GetFeatureDefinitionParameters {
            feature_identifier: Some(text("com.a3analytics/lab/LabVideos/v1")),
        })
        .await
        .unwrap()
        .into_inner()
        .feature_definition
        .unwrap()
        .value;
    assert_eq!(definition, xml);
}

async fn assert_sila_gap_spanning_clip_succeeds(
    client: &mut LabVideosClient<tonic::transport::Channel>,
) {
    let spanning = client
        .get_video_clip(a2a_lab_dev_kit::sila::api::GetVideoClipParameters {
            source_id: Some(text("cam")),
            range: Some(range(utc(2024, 1, 1, 0, 0, 0), utc(2024, 1, 1, 0, 0, 11))),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        spanning
            .path
            .unwrap()
            .value
            .starts_with("/media/video-clips/")
    );
    assert!(spanning.clip.is_some());
}

async fn assert_sila_maps_recording_gap_from_provider() {
    let gap_server = serve_videos_lab(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_videos(ClipGapRefused)
            .share(),
    )
    .await;
    let gap = LabVideosClient::new(channel_of(&gap_server).await)
        .get_video_clip(a2a_lab_dev_kit::sila::api::GetVideoClipParameters {
            source_id: Some(text("cam")),
            range: Some(range(utc(2024, 1, 1, 0, 0, 0), utc(2024, 1, 1, 0, 0, 1))),
        })
        .await
        .unwrap_err();
    assert!(defined(&gap, "RecordingGap"));
    assert!(!defined(&gap, "NoRecording"));
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

struct LiveRefused;

impl VideoProvider for LiveRefused {
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
        std::future::ready(Err(A2aLabError::unavailable("unused")))
    }

    fn open_video_clip(
        &self,
        _id: &VideoClipId,
    ) -> impl Future<Output = Result<VideoClipStream, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::unavailable("unused")))
    }

    fn get_video_timeline(
        &self,
        _request: GetVideoTimelineRequest,
    ) -> impl Future<Output = Result<VideoTimeline, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::live_unavailable(
            "live follow is not available",
        )))
    }
}
