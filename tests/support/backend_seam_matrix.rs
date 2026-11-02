use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aClient, A2aLabApi, A2aLabCommand, A2aLabError, A2aLabResult, A2aLabService, A2aServer,
    DvrFrameAtRequest, DvrMediaPackager, DvrMediaStore, DvrPackagerConfig, DvrTrackId,
    FollowVideoRequest, GetVideoClipRequest, GetVideoTimelineRequest, HLS_PLAYLIST_RELATIVE_PATH,
    HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH, LocalMediaAssetProvider, MAX_VIDEO_CLIP_DURATION_US,
    McpLab, McpServer, MediaAssetProvider, MemoryLogs, MemoryMetrics, MemoryTasks, MemoryVideos,
    PageRequest, ReadDvrMediaWindowRequest, TckMalformedVideoRequests, TimeRange, UtcTimestamp,
    VideoCodec, VideoProvider, VideoSource, VideoSourceId, router, video_clip_path,
};

use crate::dvr_store_contract::FakeDvrMediaStore;
use crate::fake_remote_media::{FakeRemoteAsset, FakeRemoteMediaAssetProvider};

const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xD9];
const H264_INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67];
const H265_INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x40, 0x01];

/// Documented adapter differences the matrix must not treat as regressions.
pub const PERMITTED_ADAPTER_DIFFERENCES: &[&str] = &[
    "A2A and MCP return clip metadata plus an authenticated relative path; SiLA LabVideos uses BinaryDownload for clip bytes",
    "SiLA LabVideos exposes retained timeline metadata and finite BinaryDownload clips but has no follow command and no post-call frame stream",
];

fn seam_fail(case: &str, backend: &str, expected: &str, actual: &str) -> ! {
    panic!(
        "case {case}\nbackend {backend}\nexpected {expected}\nactual {actual}"
    );
}

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

fn source_id(id: &str) -> VideoSourceId {
    VideoSourceId::new(id).unwrap()
}

fn track_id(id: &str) -> DvrTrackId {
    DvrTrackId::new(id).unwrap()
}

fn range(start: &str, end: &str) -> TimeRange {
    TimeRange::new(timestamp(start), timestamp(end)).unwrap()
}

fn packager() -> DvrMediaPackager {
    DvrMediaPackager::new(DvrPackagerConfig::new(120_000_000).unwrap())
}

async fn memory_continuous_mjpeg() -> MemoryVideos {
    let videos = MemoryVideos::new();
    videos
        .insert_source(
            VideoSource::new_mjpeg(source_id("cam"), "Cam", "bench", None, None, 640, 480, 30)
                .unwrap(),
        )
        .await
        .unwrap();
    for second in 0..5 {
        videos
            .insert_frame(
                source_id("cam"),
                timestamp(&format!("2024-01-01T00:00:0{second}Z")),
                JPEG.to_vec(),
            )
            .await
            .unwrap();
    }
    videos
}

async fn memory_gapped_h264() -> MemoryVideos {
    let videos = MemoryVideos::new();
    videos
        .insert_source(
            VideoSource::with_capabilities(
                source_id("cam"),
                "Cam",
                "bench",
                None,
                None,
                640,
                480,
                30,
                "video/mp4",
                vec![VideoCodec::H264],
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let track = track_id("h264");
    videos
        .insert_dvr_track(source_id("cam"), track.clone(), VideoCodec::H264, H264_INIT.to_vec())
        .await
        .unwrap();
    for (index, start_text) in ["2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"]
        .iter()
        .enumerate()
    {
        let start = timestamp(start_text);
        let end = start.saturating_add_micros(1_000_000);
        videos
            .insert_dvr_segment(
                source_id("cam"),
                track.clone(),
                TimeRange::new(start, end).unwrap(),
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
        let end = start.saturating_add_micros(1_000_000);
        videos
            .insert_dvr_segment(
                source_id("cam"),
                track.clone(),
                TimeRange::new(start, end).unwrap(),
                index == 0,
                VideoCodec::H264,
                vec![0x10, u8::try_from(index).unwrap()],
            )
            .await
            .unwrap();
    }
    videos
}

async fn memory_h265_track() -> MemoryVideos {
    let videos = MemoryVideos::new();
    videos
        .insert_source(
            VideoSource::with_capabilities(
                source_id("cam"),
                "Cam",
                "bench",
                None,
                None,
                640,
                480,
                30,
                "video/mp4",
                vec![VideoCodec::H265],
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let track = track_id("h265");
    let bounds = range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z");
    videos
        .insert_dvr_track(
            source_id("cam"),
            track.clone(),
            VideoCodec::H265,
            H265_INIT.to_vec(),
        )
        .await
        .unwrap();
    videos
        .insert_dvr_segment(
            source_id("cam"),
            track,
            bounds,
            true,
            VideoCodec::H265,
            vec![0xAA, 0xBB],
        )
        .await
        .unwrap();
    videos
}

fn assert_same_error_code(
    case: &str,
    backend: &str,
    left: Result<(), A2aLabError>,
    right: Result<(), A2aLabError>,
) {
    match (left, right) {
        (Ok(()), Ok(())) => {}
        (Err(left), Err(right)) if left.code() == right.code() => {}
        (Ok(()), Err(error)) => {
            seam_fail(case, backend, "ok", error.code());
        }
        (Err(error), Ok(())) => {
            seam_fail(case, backend, error.code(), "ok");
        }
        (Err(left), Err(right)) => {
            seam_fail(
                case,
                backend,
                left.code(),
                right.code(),
            );
        }
    }
}

async fn compare_retention(case: &str, memory: &MemoryVideos, fake: &FakeDvrMediaStore) {
    let source = source_id("cam");
    let mem = memory.retention_bounds(&source).await;
    let fake = fake.retention_bounds(&source).await;
    match (mem, fake) {
        (Ok(left), Ok(right)) => {
            if left.oldest() != right.oldest() {
                seam_fail(
                    case,
                    "DvrMediaStore retention oldest",
                    &right.oldest().to_string(),
                    &left.oldest().to_string(),
                );
            }
            let slack = left.newest().as_micros().abs_diff(right.newest().as_micros());
            if slack > 1_000_000 {
                seam_fail(
                    case,
                    "DvrMediaStore retention newest",
                    &right.newest().to_string(),
                    &left.newest().to_string(),
                );
            }
        }
        (Err(left), Err(right)) => {
            assert_same_error_code(
                case,
                "DvrMediaStore retention",
                Err(left),
                Err(right),
            );
        }
        (left, right) => {
            seam_fail(
                case,
                "DvrMediaStore retention",
                &format!("{left:?}"),
                &format!("{right:?}"),
            );
        }
    }
}

async fn compare_window_segments(
    case: &str,
    memory: &MemoryVideos,
    fake: &FakeDvrMediaStore,
    mem_track: &DvrTrackId,
    fake_track: &DvrTrackId,
    window: TimeRange,
    expected: usize,
) {
    let source = source_id("cam");
    let page = PageRequest::new(None, 10).unwrap();
    let mem = memory
        .read_media_window(
            ReadDvrMediaWindowRequest::new(source.clone(), mem_track.clone(), window, page.clone())
                .unwrap(),
        )
        .await
        .unwrap()
        .items()
        .len();
    let fake = fake
        .read_media_window(
            ReadDvrMediaWindowRequest::new(source, fake_track.clone(), window, page).unwrap(),
        )
        .await
        .unwrap()
        .items()
        .len();
    if mem != fake || mem != expected {
        seam_fail(
            case,
            "DvrMediaStore window segments",
            &expected.to_string(),
            &format!("memory={mem} fake={fake}"),
        );
    }
}

async fn compare_mjpeg_store(case: &str, memory: &MemoryVideos, fake: &FakeDvrMediaStore) {
    let source = source_id("cam");
    let mem_track = MemoryVideos::mjpeg_track_id(&source).unwrap();
    let fake_track = track_id("mjpeg");
    compare_window_segments(
        case,
        memory,
        fake,
        &mem_track,
        &fake_track,
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:05Z"),
        5,
    )
    .await;
    compare_frame_at_code(
        case,
        memory,
        fake,
        &mem_track,
        "2024-01-01T00:00:02Z",
        "ok",
    )
    .await;
}

async fn compare_mjpeg_packager(
    case: &str,
    memory: &MemoryVideos,
    fake: &FakeDvrMediaStore,
) {
    let source = source_id("cam");
    let mem_track = MemoryVideos::mjpeg_track_id(&source).unwrap();
    let fake_track = track_id("mjpeg");
    let window = range("2024-01-01T00:00:00Z", "2024-01-01T00:00:03Z");
    let packager = packager();
    let mem = packager
        .export_finite_mjpeg(memory, &source, &mem_track, window)
        .await;
    let fake_export = packager
        .export_finite_mjpeg(fake, &source, &fake_track, window)
        .await;
    match (mem, fake_export) {
        (Ok(left), Ok(right)) => {
            if left.bytes().is_empty() || right.bytes().is_empty() {
                seam_fail(case, "MJPEG export", "non-empty bodies", "empty");
            }
        }
        (Err(left), Err(right)) if left.code() == right.code() => {}
        (left, right) => {
            seam_fail(
                case,
                "MJPEG export",
                &format!("{left:?}"),
                &format!("{right:?}"),
            );
        }
    }
}

async fn compare_frame_at_code(
    case: &str,
    memory: &MemoryVideos,
    fake: &FakeDvrMediaStore,
    track: &DvrTrackId,
    at: &str,
    expected_code: &str,
) {
    let source = source_id("cam");
    let fake_track = if track.as_str().ends_with("-mjpeg") {
        track_id("mjpeg")
    } else {
        track.clone()
    };
    let mem = memory
        .frame_at(DvrFrameAtRequest::new(
            source.clone(),
            track.clone(),
            timestamp(at),
            None,
        ))
        .await;
    let fake_result = fake
        .frame_at(DvrFrameAtRequest::new(
            source,
            fake_track,
            timestamp(at),
            None,
        ))
        .await;
    let mem_code = mem.as_ref().err().map_or("ok", A2aLabError::code);
    let fake_code = fake_result
        .as_ref()
        .err()
        .map_or("ok", A2aLabError::code);
    if mem_code != fake_code {
        seam_fail(
            case,
            "DvrMediaStore frame_at",
            fake_code,
            mem_code,
        );
    }
    if mem_code != expected_code {
        seam_fail(
            case,
            "DvrMediaStore frame_at",
            expected_code,
            mem_code,
        );
    }
}

async fn compare_store_fixture(
    case: &str,
    memory: &MemoryVideos,
    fake: &FakeDvrMediaStore,
    track: &DvrTrackId,
) {
    compare_retention(case, memory, fake).await;
    let fake_track = track.clone();
    compare_window_segments(
        case,
        memory,
        fake,
        track,
        &fake_track,
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
        2,
    )
    .await;
    compare_window_segments(
        case,
        memory,
        fake,
        track,
        &fake_track,
        range("2024-01-01T00:00:04Z", "2024-01-01T00:00:06Z"),
        2,
    )
    .await;
}

async fn assert_catalog_timeline_from_frames(case: &str, videos: &MemoryVideos) {
    let source = source_id("cam");
    let timeline = videos
        .get_video_timeline(GetVideoTimelineRequest::new(source.clone()))
        .await
        .unwrap();
    if timeline.intervals().is_empty() {
        seam_fail(case, "catalog timeline", "retained intervals", "empty");
    }
}

async fn compare_packager_on_stores(
    case: &str,
    memory: &MemoryVideos,
    fake: &FakeDvrMediaStore,
    track: &DvrTrackId,
    anchor: &str,
) {
    let source = source_id("cam");
    let root = "/media/video-clips/matrix/";
    let packager = packager();
    let mem_session = packager
        .package_hls_at(memory, root, &source, track, timestamp(anchor))
        .await;
    let fake_session = packager
        .package_hls_at(fake, root, &source, track, timestamp(anchor))
        .await;
    match (mem_session, fake_session) {
        (Ok(left), Ok(right)) => {
            if left.segments().len() != right.segments().len() {
                seam_fail(
                    case,
                    "packager segment count",
                    &right.segments().len().to_string(),
                    &left.segments().len().to_string(),
                );
            }
            if left.trick_8x().is_some() != right.trick_8x().is_some() {
                seam_fail(
                    case,
                    "optional trick 8x",
                    &format!("present={}", right.trick_8x().is_some()),
                    &format!("present={}", left.trick_8x().is_some()),
                );
            }
        }
        (Err(left), Err(right)) if left.code() == right.code() => {}
        (left, right) => {
            seam_fail(
                case,
                "packager",
                &format!("{left:?}"),
                &format!("{right:?}"),
            );
        }
    }
}

async fn listen(assets: Arc<dyn MediaAssetProvider>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router(assets)).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
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

async fn gateway_observation(base: &str, path: &str) -> (u16, String, String) {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{base}{path}"))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let cache = header(&response, "cache-control");
    let nosniff = header(&response, "x-content-type-options");
    drop(response);
    let again = client
        .get(format!("{base}{path}"))
        .send()
        .await
        .unwrap();
    let stable = again.status().as_u16();
    drop(again);
    if stable != status {
        seam_fail(
            "gateway cancellation stable status",
            base,
            &status.to_string(),
            &stable.to_string(),
        );
    }
    (status, cache, nosniff)
}

async fn compare_gateway_providers(case: &str, path: &str, expected_status: u16) {
    let local = Arc::new(LocalMediaAssetProvider::new(
        A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
            .with_videos(MemoryVideos::new())
            .share(),
    )) as Arc<dyn MediaAssetProvider>;
    let remote = Arc::new(FakeRemoteMediaAssetProvider::new()) as Arc<dyn MediaAssetProvider>;
    for (label, assets) in [("local", local), ("remote", remote)] {
        let base = listen(assets).await;
        let (status, cache, nosniff) = gateway_observation(&base, path).await;
        if status != expected_status {
            seam_fail(
                case,
                &format!("gateway {label}"),
                &expected_status.to_string(),
                &status.to_string(),
            );
        }
        if cache != "no-store" {
            seam_fail(case, &format!("gateway {label} cache"), "no-store", &cache);
        }
        if nosniff != "nosniff" {
            seam_fail(case, &format!("gateway {label} nosniff"), "nosniff", &nosniff);
        }
        let credential = reqwest::Client::new()
            .get(format!("{base}{path}?token=secret"))
            .send()
            .await
            .unwrap();
        let cred_status = credential.status().as_u16();
        let body = credential.text().await.unwrap_or_default();
        if cred_status != 400 {
            seam_fail(
                case,
                &format!("gateway {label} credential query"),
                "400",
                &cred_status.to_string(),
            );
        }
        if body.contains("secret") || body.contains("vault.internal") {
            seam_fail(
                case,
                &format!("gateway {label} upstream secrecy"),
                "no secrets in body",
                &body,
            );
        }
    }
}

async fn compare_a2a_mcp_timeline(case: &str, videos: MemoryVideos) {
    let lab = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos)
        .share();
    let a2a_lab = Arc::clone(&lab);
    let (a2a_listener, a2a_address) = a2a_lab_dev_kit::bind_local().await.unwrap();
    tokio::spawn(async move {
        A2aServer::new(&a2a_lab)
            .listen(a2a_listener)
            .await
            .unwrap();
    });
    let mcp_lab = Arc::clone(&lab);
    let (mcp_listener, mcp_address) = a2a_lab_dev_kit::bind_local().await.unwrap();
    tokio::spawn(async move {
        McpServer::new(&mcp_lab)
            .serve_http(mcp_listener)
            .await
            .unwrap();
    });
    let a2a = A2aClient::new(&format!("http://{a2a_address}")).unwrap();
    let mcp = McpLab::connect(&format!("http://{mcp_address}/mcp"))
        .await
        .unwrap();
    let request = GetVideoTimelineRequest::new(source_id("cam"));
    let a2a_value = serde_json::to_value(a2a.get_video_timeline(request.clone()).await.unwrap())
        .unwrap();
    let outcome = mcp
        .execute(A2aLabCommand::GetVideoTimeline(request))
        .await
        .unwrap();
    let A2aLabResult::GetVideoTimeline(timeline) = outcome.task.result else {
        seam_fail(case, "adapter MCP", "GetVideoTimeline", "unexpected result");
    };
    let mcp_value = serde_json::to_value(timeline).unwrap();
    if a2a_value != mcp_value {
        seam_fail(
            case,
            "adapter A2A vs MCP",
            &mcp_value.to_string(),
            &a2a_value.to_string(),
        );
    }
}

#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
async fn run_dvr_and_catalog_matrix() {
    assert_ne!(
        TypeId::of::<FakeDvrMediaStore>(),
        TypeId::of::<MemoryVideos>()
    );
    let _: &dyn Any = &FakeDvrMediaStore::continuous_mjpeg();

    let memory = memory_continuous_mjpeg().await;
    let fake = FakeDvrMediaStore::continuous_mjpeg();
    compare_mjpeg_store("MJPEG continuous store", &memory, &fake).await;
    assert_catalog_timeline_from_frames("MJPEG catalog", &memory).await;
    compare_mjpeg_packager("MJPEG finite export window", &memory, &fake).await;

    let memory = memory_gapped_h264().await;
    let fake = FakeDvrMediaStore::gapped_h264();
    compare_store_fixture("H.264 gapped store", &memory, &fake, &track_id("h264")).await;
    compare_frame_at_code(
        "H.264 gap instant",
        &memory,
        &fake,
        &track_id("h264"),
        "2024-01-01T00:00:03Z",
        "recording_gap",
    )
    .await;
    compare_packager_on_stores(
        "H.264 bounded window with gaps",
        &memory,
        &fake,
        &track_id("h264"),
        "2024-01-01T00:00:00Z",
    )
    .await;
    compare_packager_on_stores(
        "optional 8x trick play",
        &memory,
        &fake,
        &track_id("h264"),
        "2024-01-01T00:00:00Z",
    )
    .await;

    let memory = memory_h265_track().await;
    let fake = FakeDvrMediaStore::h265_track();
    compare_retention("H.265 store", &memory, &fake).await;
    compare_window_segments(
        "H.265 store",
        &memory,
        &fake,
        &track_id("h265"),
        &track_id("h265"),
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
        1,
    )
    .await;
    compare_packager_on_stores(
        "H.265 success",
        &memory,
        &fake,
        &track_id("h265"),
        "2024-01-01T00:00:01Z",
    )
    .await;

    let long_memory = memory_gapped_h264().await;
    let long_fake = FakeDvrMediaStore::long_range_h264();
    let packager = packager();
    let source = source_id("cam");
    let track = track_id("h264");
    let first = packager
        .package_hls_at(
            &long_fake,
            "/media/video-clips/long/",
            &source,
            &track,
            timestamp("2024-01-01T01:00:00Z"),
        )
        .await
        .unwrap();
    assert!(first.segments().len() <= 121);
    let rebased = packager
        .rebase_hls(&long_fake, &first, timestamp("2024-01-01T01:30:00Z"))
        .await
        .unwrap();
    assert_ne!(first.window(), rebased.window());

    let clip = long_memory
        .get_video_clip(
            GetVideoClipRequest::new(
                source.clone(),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        clip.requested_range().end(),
        timestamp("2024-01-01T00:00:02Z")
    );
    assert!(serde_json::to_value(&clip).unwrap().get("data").is_none());
    assert_eq!(clip.path(), video_clip_path(clip.id()));

    let start = timestamp("2024-01-01T00:00:00Z");
    let end = start.saturating_add_micros(i64::try_from(MAX_VIDEO_CLIP_DURATION_US).unwrap());
    assert!(
        GetVideoClipRequest::new(source, TimeRange::new(start, end).unwrap()).is_ok(),
        "one-hour-plus selection must remain valid"
    );

    let invalid = TckMalformedVideoRequests::get_video_clip_reversed_range(
        source_id("cam"),
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
    );
    assert_eq!(
        long_memory.get_video_clip(invalid).await.unwrap_err().code(),
        "invalid"
    );

    let expired = MemoryVideos::with_clip_ttl(Duration::from_millis(1));
    expired
        .insert_source(
            VideoSource::new_mjpeg(source_id("cam"), "Cam", "bench", None, None, 640, 480, 30)
                .unwrap(),
        )
        .await
        .unwrap();
    expired
        .insert_frame(source_id("cam"), timestamp("2024-01-01T00:00:00Z"), JPEG.to_vec())
        .await
        .unwrap();
    let clip = expired
        .get_video_clip(
            GetVideoClipRequest::new(
                source_id("cam"),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    match expired.open_video_clip(clip.id()).await {
        Err(error) if error.code() == "clip_expired" => {}
        Err(error) => seam_fail(
            "expired clip asset",
            "catalog open_video_clip",
            "clip_expired",
            error.code(),
        ),
        Ok(_) => seam_fail(
            "expired clip asset",
            "catalog open_video_clip",
            "clip_expired",
            "ok",
        ),
    }

    let quiet = memory_continuous_mjpeg().await;
    assert_eq!(
        quiet
            .follow_video(FollowVideoRequest::new(source_id("cam")))
            .await
            .unwrap_err()
            .code(),
        "live_unavailable"
    );

    let encoded = memory_gapped_h264().await;
    encoded
        .set_live_available(&source_id("cam"), true)
        .await
        .unwrap();
    let follow = encoded
        .follow_video(FollowVideoRequest::new(source_id("cam")))
        .await
        .unwrap();
    let playback_8x = follow.playback_8x();
    assert_eq!(
        playback_8x.rendition_path(),
        Some(HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH)
    );
    assert_eq!(playback_8x.fallback_path(), HLS_PLAYLIST_RELATIVE_PATH);
}

async fn run_gateway_matrix() {
    compare_gateway_providers(
        "missing packaged asset",
        "/media/video-clips/missing/",
        404,
    )
    .await;

    let remote = Arc::new(FakeRemoteMediaAssetProvider::new());
    let clip_id = a2a_lab_dev_kit::VideoClipId::new("matrix-remote").unwrap();
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
    remote.bind_clip(clip_id, assets).unwrap();
    let base = listen(remote).await;
    let playlist = reqwest::Client::new()
        .get(format!("{base}{root}{HLS_PLAYLIST_RELATIVE_PATH}"))
        .send()
        .await
        .unwrap();
    assert_eq!(header(&playlist, "cache-control"), "no-store");
    let text = playlist.text().await.unwrap();
    if text.contains("vault.internal") {
        seam_fail(
            "fake remote proxy",
            "gateway upstream secrecy",
            "no upstream host",
            &text,
        );
    }
}

async fn run_adapter_matrix() {
    assert_eq!(PERMITTED_ADAPTER_DIFFERENCES.len(), 2);
    compare_a2a_mcp_timeline("gapped timeline parity", memory_gapped_h264().await).await;
}

pub async fn run_matrix() {
    run_dvr_and_catalog_matrix().await;
    run_gateway_matrix().await;
    run_adapter_matrix().await;
}

#[tokio::test]
async fn video_backend_and_adapter_seam_matrix() {
    run_matrix().await;
}
