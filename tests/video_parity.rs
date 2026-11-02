use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabCommand, A2aLabError, A2aLabResult, A2aLabService, A2aServer,
    DvrTrackId, FollowVideoRequest, GetVideoClipRequest, GetVideoFrameAtRequest,
    GetVideoFrameAtResult, GetVideoTimelineRequest, HttpAuthSecurityScheme,
    ListVideoSourcesRequest, MAX_VIDEO_CLIP_DURATION_US, MJPEG_MEDIA_TYPE, McpLab, McpServer,
    MemoryLogs, MemoryMetrics, MemoryTasks, MemoryVideos, Page, PageRequest, SecurityScheme,
    TckMalformedVideoRequests, TimeRange, UtcTimestamp, VideoClip, VideoCodec, VideoProvider,
    VideoSource, VideoSourceId, bind_local, video_clip_path,
};

const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
];

const INTENTIONAL_ENVELOPE_DIFFERENCES: &[&str] = &[
    "A2A and MCP return clip metadata plus an authenticated relative path; SiLA LabVideos uses BinaryDownload for clip bytes",
];

struct Endpoints {
    a2a: A2aClient,
    mcp: McpLab,
}

enum Op {
    List(ListVideoSourcesRequest),
    Frame(GetVideoFrameAtRequest),
    Clip(GetVideoClipRequest),
}

enum Outcome {
    Sources(Page<VideoSource>),
    Frame(GetVideoFrameAtResult),
    Clip(VideoClip),
    Error(A2aLabError),
}

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
    videos.insert_source(source("empty")).await.unwrap();
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

async fn open(lab: Arc<dyn A2aLabApi>) -> Endpoints {
    let a2a_lab = lab.clone();
    let (a2a_listener, a2a_address) = bind_local().await.unwrap();
    tokio::spawn(async move {
        A2aServer::new(&a2a_lab).listen(a2a_listener).await.unwrap();
    });
    let (mcp_listener, mcp_address) = bind_local().await.unwrap();
    tokio::spawn(async move {
        McpServer::new(&lab).serve_http(mcp_listener).await.unwrap();
    });
    Endpoints {
        a2a: A2aClient::new(&format!("http://{a2a_address}")).unwrap(),
        mcp: McpLab::connect(&format!("http://{mcp_address}/mcp"))
            .await
            .unwrap(),
    }
}

fn command(op: &Op) -> A2aLabCommand {
    match op {
        Op::List(request) => A2aLabCommand::ListVideoSources(request.clone()),
        Op::Frame(request) => A2aLabCommand::GetVideoFrameAt(request.clone()),
        Op::Clip(request) => A2aLabCommand::GetVideoClip(request.clone()),
    }
}

async fn observe_a2a(client: &A2aClient, op: &Op) -> Outcome {
    let result = match op {
        Op::List(request) => client
            .list_video_sources(request.clone())
            .await
            .map(Outcome::Sources),
        Op::Frame(request) => client
            .get_video_frame_at(request.clone())
            .await
            .map(Outcome::Frame),
        Op::Clip(request) => client
            .get_video_clip(request.clone())
            .await
            .map(Outcome::Clip),
    };
    result.unwrap_or_else(Outcome::Error)
}

async fn observe_mcp(lab: &McpLab, op: &Op) -> Outcome {
    match lab.execute(command(op)).await {
        Ok(outcome) => match (op, outcome.task.result) {
            (Op::List(_), A2aLabResult::ListVideoSources(page)) => Outcome::Sources(page),
            (Op::Frame(_), A2aLabResult::GetVideoFrameAt(frame)) => Outcome::Frame(frame),
            (Op::Clip(_), A2aLabResult::GetVideoClip(clip)) => Outcome::Clip(clip),
            (_, result) => Outcome::Error(A2aLabError::protocol(format!("{result:?}"))),
        },
        Err(error) => Outcome::Error(error),
    }
}

fn same(left: &Outcome, right: &Outcome) -> bool {
    match (left, right) {
        (Outcome::Sources(left), Outcome::Sources(right)) => left == right,
        (Outcome::Frame(left), Outcome::Frame(right)) => {
            left.delta_us() == right.delta_us()
                && left.image().data() == right.image().data()
                && left.actual_captured_at() == right.actual_captured_at()
        }
        (Outcome::Clip(left), Outcome::Clip(right)) => {
            left.source_id() == right.source_id()
                && left.requested_range() == right.requested_range()
                && left.path() == video_clip_path(left.id())
                && right.path() == video_clip_path(right.id())
                && serde_json::to_value(left).unwrap().get("data").is_none()
                && serde_json::to_value(right).unwrap().get("data").is_none()
        }
        (Outcome::Error(left), Outcome::Error(right)) => left.code() == right.code(),
        _ => false,
    }
}

fn render(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Sources(page) => format!(
            "sources ids={:?}",
            page.items()
                .iter()
                .map(|item| item.id().as_str())
                .collect::<Vec<_>>()
        ),
        Outcome::Frame(frame) => format!(
            "frame at={} delta={} bytes={}",
            frame.actual_captured_at(),
            frame.delta_us(),
            frame.image().data().len()
        ),
        Outcome::Clip(clip) => format!(
            "clip id={} path={} range={:?} data={}",
            clip.id(),
            clip.path(),
            clip.requested_range(),
            serde_json::to_value(clip).unwrap().get("data").is_some()
        ),
        Outcome::Error(error) => format!("error code={} message={error}", error.code()),
    }
}

fn diverge(case: &str, a2a: &Outcome, mcp: &Outcome) -> ! {
    panic!("case {case}\nA2A: {}\nMCP: {}", render(a2a), render(mcp));
}

async fn compare(endpoints: &Endpoints, name: &str, op: Op, expect: fn(&Outcome) -> bool) {
    let a2a = observe_a2a(&endpoints.a2a, &op).await;
    let mcp = observe_mcp(&endpoints.mcp, &op).await;
    if !same(&a2a, &mcp) {
        diverge(name, &a2a, &mcp);
    }
    assert!(
        expect(&a2a),
        "case {name} shared result diverged from the fixture\nA2A: {}\nMCP: {}",
        render(&a2a),
        render(&mcp)
    );
}

fn source_ids(outcome: &Outcome) -> Vec<String> {
    match outcome {
        Outcome::Sources(page) => page
            .items()
            .iter()
            .map(|item| item.id().as_str().to_owned())
            .collect(),
        _ => Vec::new(),
    }
}

fn code_is(outcome: &Outcome, code: &str) -> bool {
    matches!(outcome, Outcome::Error(error) if error.code() == code)
}

fn cam() -> VideoSourceId {
    VideoSourceId::new("cam").unwrap()
}

async fn compare_catalog(endpoints: &Endpoints) {
    compare(
        endpoints,
        "list video sources",
        Op::List(ListVideoSourcesRequest::new(page(10)).unwrap()),
        |outcome| source_ids(outcome) == ["cam", "empty", "zeta"],
    )
    .await;
    compare(
        endpoints,
        "frame at exact stamp",
        Op::Frame(GetVideoFrameAtRequest::new(
            cam(),
            timestamp("2024-01-01T00:00:00Z"),
            None,
        )),
        |outcome| matches!(outcome, Outcome::Frame(frame) if frame.delta_us() == 0 && frame.image().data() == JPEG),
    )
    .await;
    compare(
        endpoints,
        "frame nearest within delta",
        Op::Frame(GetVideoFrameAtRequest::new(
            cam(),
            timestamp("2024-01-01T00:00:00.400000Z"),
            Some(1_000_000),
        )),
        |outcome| {
            matches!(outcome, Outcome::Frame(frame) if frame.actual_captured_at().to_rfc3339() == "2024-01-01T00:00:00Z")
        },
    )
    .await;
    compare(
        endpoints,
        "clip half-open range excludes end",
        Op::Clip(
            GetVideoClipRequest::new(cam(), range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"))
                .unwrap(),
        ),
        |outcome| match outcome {
            Outcome::Clip(clip) => {
                let encoded = serde_json::to_value(clip).unwrap();
                encoded.get("data").is_none()
                    && encoded.get("jpeg").is_none()
                    && clip.path() == video_clip_path(clip.id())
                    && clip.requested_range().end() == timestamp("2024-01-01T00:00:02Z")
            }
            _ => false,
        },
    )
    .await;
    compare(
        endpoints,
        "unknown source",
        Op::Frame(GetVideoFrameAtRequest::new(
            VideoSourceId::new("missing").unwrap(),
            timestamp("2024-01-01T00:00:00Z"),
            None,
        )),
        |outcome| code_is(outcome, "not_found"),
    )
    .await;
    compare(
        endpoints,
        "clip outside coverage",
        Op::Clip(
            GetVideoClipRequest::new(cam(), range("2025-01-01T00:00:00Z", "2025-01-01T00:00:01Z"))
                .unwrap(),
        ),
        |outcome| code_is(outcome, "no_recording"),
    )
    .await;
}

async fn compare_malformed(endpoints: &Endpoints) {
    compare(
        endpoints,
        "list_video_sources zero limit",
        Op::List(TckMalformedVideoRequests::list_video_sources_zero_limit()),
        |outcome| code_is(outcome, "invalid"),
    )
    .await;
    compare(
        endpoints,
        "get_video_clip reversed range",
        Op::Clip(TckMalformedVideoRequests::get_video_clip_reversed_range(
            cam(),
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
        )),
        |outcome| code_is(outcome, "invalid"),
    )
    .await;
    compare(
        endpoints,
        "get_video_clip over limit",
        Op::Clip(TckMalformedVideoRequests::get_video_clip_empty_range(
            cam(),
            timestamp("2024-01-01T00:00:00Z"),
        )),
        |outcome| code_is(outcome, "invalid"),
    )
    .await;
}

#[tokio::test]
async fn video_parity_matches_a2a_and_mcp() {
    let endpoints = open(service(catalog().await)).await;
    compare_catalog(&endpoints).await;
    compare_malformed(&endpoints).await;
    const {
        assert!(MAX_VIDEO_CLIP_DURATION_US > 0);
    }
    let offline = catalog().await;
    offline.set_unavailable("videos offline").await;
    let down = open(service(offline)).await;
    compare(
        &down,
        "unavailable provider",
        Op::List(ListVideoSourcesRequest::new(page(10)).unwrap()),
        |outcome| code_is(outcome, "unavailable"),
    )
    .await;
}

fn clip_has_no_inline_bytes(clip: &VideoClip) {
    assert!(
        serde_json::to_value(clip).unwrap().get("data").is_none(),
        "clip JSON must stay metadata-only"
    );
}

#[cfg(feature = "sila2")]
#[tokio::test]
async fn video_clip_sila_binary_download_is_an_intentional_envelope_difference() {
    let lab = service(catalog().await);
    let endpoints = open(lab.clone()).await;
    let request =
        GetVideoClipRequest::new(cam(), range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"))
            .unwrap();
    let a2a_clip = endpoints.a2a.get_video_clip(request.clone()).await.unwrap();
    clip_has_no_inline_bytes(&a2a_clip);
    assert_eq!(a2a_clip.path(), video_clip_path(a2a_clip.id()));
    let mcp = endpoints
        .mcp
        .execute(A2aLabCommand::GetVideoClip(request))
        .await
        .unwrap();
    let A2aLabResult::GetVideoClip(mcp_clip) = mcp.task.result else {
        panic!("mcp clip");
    };
    clip_has_no_inline_bytes(&mcp_clip);
    let payload = sila_clip_payload(lab).await;
    assert!(
        payload.path.starts_with("/media/video-clips/"),
        "SiLA still names the relative path"
    );
    assert!(
        !payload.bytes.is_empty(),
        "SiLA BinaryDownload carries clip bytes"
    );
    assert!(payload.bytes.windows(2).any(|pair| pair == &JPEG[..2]));
    assert_eq!(
        INTENTIONAL_ENVELOPE_DIFFERENCES,
        [
            "A2A and MCP return clip metadata plus an authenticated relative path; SiLA LabVideos uses BinaryDownload for clip bytes"
        ]
    );
}

#[cfg(feature = "sila2")]
struct SilaClipPayload {
    path: String,
    bytes: Vec<u8>,
}

#[cfg(feature = "sila2")]
async fn sila_clip_payload(lab: Arc<dyn A2aLabApi>) -> SilaClipPayload {
    use a2a_lab_dev_kit::sila::api::{
        BinaryDownloadClient, BinaryUnion, Boolean, GetChunkRequest, Integer, LabVideosClient,
        ListVideoSourcesParameters, SilaString, VideoPageRequest, VideoPageStruct, VideoTimeRange,
        VideoTimeStruct,
    };
    use a2a_lab_dev_kit::sila::{SilaIdentity, SilaServer};
    use futures_util::StreamExt;

    let server = SilaServer::new(
        SilaIdentity::lab_dev_kit("11111111-1111-1111-1111-111111111111").unwrap(),
        lab,
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
    let mut client = LabVideosClient::new(channel.clone());
    let listed = client
        .list_video_sources(ListVideoSourcesParameters {
            page: Some(VideoPageRequest {
                page_request: Some(VideoPageStruct {
                    has_cursor: Some(Boolean { value: false }),
                    cursor: Some(SilaString {
                        value: String::new(),
                    }),
                    limit: Some(Integer { value: 10 }),
                }),
            }),
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
    let sila_clip = client
        .get_video_clip(a2a_lab_dev_kit::sila::api::GetVideoClipParameters {
            source_id: Some(SilaString {
                value: "cam".to_owned(),
            }),
            range: Some(VideoTimeRange {
                time_range: Some(VideoTimeStruct {
                    start: Some(sila_utc(2024, 1, 1, 0, 0, 0)),
                    end: Some(sila_utc(2024, 1, 1, 0, 0, 2)),
                }),
            }),
        })
        .await
        .unwrap()
        .into_inner();
    let transfer = match sila_clip.clip.unwrap().union {
        Some(BinaryUnion::BinaryTransferUuid(id)) => id,
        other => panic!("expected BinaryDownload identifier, got {other:?}"),
    };
    let mut downloads = BinaryDownloadClient::new(channel);
    let info = downloads
        .get_binary_info(a2a_lab_dev_kit::sila::api::GetBinaryInfoRequest {
            binary_transfer_uuid: transfer.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    let mut stream = downloads
        .get_chunk(futures_util::stream::iter([GetChunkRequest {
            binary_transfer_uuid: transfer,
            offset: 0,
            length: u32::try_from(info.binary_size).unwrap_or(1024),
        }]))
        .await
        .unwrap()
        .into_inner();
    let chunk = stream.next().await.unwrap().unwrap();
    SilaClipPayload {
        path: sila_clip.path.unwrap().value,
        bytes: chunk.payload,
    }
}

#[cfg(feature = "sila2")]
fn sila_utc(
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> a2a_lab_dev_kit::sila::api::Timestamp {
    a2a_lab_dev_kit::sila::api::Timestamp {
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

include!("support/control_plane_parity.rs");
