//! `DvrVideoProvider` routing over live H.264 and SQLite DVR (TASK-146).

#![cfg(all(feature = "yuv", feature = "openh264"))]

use std::time::Duration;

use a2a_lab_dev_kit::videos::WarmStillCache;
use a2a_lab_dev_kit::{
    DvrEnqueueStatus, DvrMediaStore, DvrPersistenceQueue, DvrPictureProperties, DvrTrackId,
    DvrVideoProvider, GetVideoClipRequest, GetVideoFrameAtRequest, HLS_PLAYLIST_RELATIVE_PATH,
    MemoryVideos, ResolvedMediaAsset, SqliteChunkDvrStore, TimeRange, UtcTimestamp, VideoCodec,
    VideoProvider, VideoSource, VideoSourceId,
};

const INIT_A: &[u8] = &[
    0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, 0x9A, 0x74, 0x03, 0xC0, 0x11, 0x00, 0x00, 0x03,
    0x00, 0x01, 0x00, 0x00, 0x03, 0x00, 0x3C, 0x0F, 0x16, 0x2D, 0x96, 0x00, 0x00, 0x00, 0x01, 0x68,
    0xCE, 0x3C, 0x80,
];

const INIT_B: &[u8] = &[
    0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x28, 0x9A, 0x74, 0x03, 0xC0, 0x11, 0x00, 0x00, 0x03,
    0x00, 0x01, 0x00, 0x00, 0x03, 0x00, 0x3C, 0x0F, 0x16, 0x2D, 0x96, 0x00, 0x00, 0x00, 0x01, 0x68,
    0xCE, 0x3C, 0x80,
];

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "a2a-lab-devkit-dvr-provider-{name}-{}",
        std::process::id()
    ))
}

fn ts(text: &str) -> UtcTimestamp {
    UtcTimestamp::parse(text).unwrap()
}

fn sample(tag: u8) -> Vec<u8> {
    vec![0x00, 0x00, 0x00, 0x01, 0x65, tag]
}

fn track(label: &str) -> DvrTrackId {
    DvrTrackId::new(label).unwrap()
}

fn jpeg_32x16() -> Vec<u8> {
    vec![
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00,
        0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x10, 0x00, 0x20, 0x01,
        0x01, 0x11, 0x00, 0xFF, 0xD9,
    ]
}

async fn catalog(name: &str) -> (DvrVideoProvider, VideoSourceId) {
    let dir = temp_root(name);
    let _ = std::fs::remove_dir_all(&dir);
    let store = SqliteChunkDvrStore::open(&dir, 86_400).await.unwrap();
    let live = MemoryVideos::new();
    let source = VideoSourceId::new("cam").unwrap();
    live.insert_source(
        VideoSource::with_capabilities(
            source.clone(),
            "H264",
            "encoded",
            None,
            None,
            640,
            480,
            5,
            "video/mp4",
            vec![VideoCodec::H264],
        )
        .unwrap(),
    )
    .await
    .unwrap();
    live.set_live_available(&source, true).await.unwrap();
    let catalog = DvrVideoProvider::open(live, store, Duration::from_secs(60)).unwrap();
    (catalog, source)
}

fn seed_live(
    catalog: &DvrVideoProvider,
    source: &VideoSourceId,
    track_id: &DvrTrackId,
    init: &[u8],
    at: UtcTimestamp,
    bytes: Vec<u8>,
) {
    let picture = DvrPictureProperties::new(640, 480).unwrap();
    let entry = catalog.live_h264().get(source).unwrap_or_else(|| {
        catalog
            .live_h264()
            .register(source, 60, 5, 1_000_000)
            .unwrap()
    });
    let store = entry.inner();
    if store
        .snapshot()
        .unwrap()
        .epochs()
        .iter()
        .all(|epoch| epoch.track_id() != track_id)
    {
        store
            .open_epoch(track_id.clone(), init.to_vec(), picture)
            .unwrap();
    }
    store.append_sample(track_id, at, bytes, true).unwrap();
}

#[tokio::test]
async fn historical_clip_maps_each_epoch_to_its_init() {
    let (catalog, source) = catalog("clip-boundary").await;
    catalog
        .dvr()
        .append_h264_sample(
            &source,
            ts("2024-06-01T00:00:00Z"),
            200_000,
            true,
            INIT_A,
            &sample(1),
        )
        .await
        .unwrap();
    catalog
        .dvr()
        .append_h264_sample(
            &source,
            ts("2024-06-01T00:00:03Z"),
            200_000,
            true,
            INIT_B,
            &sample(2),
        )
        .await
        .unwrap();
    let range = TimeRange::new(ts("2024-06-01T00:00:00Z"), ts("2024-06-01T00:00:05Z")).unwrap();
    let clip = catalog
        .get_video_clip(GetVideoClipRequest::new(source.clone(), range).unwrap())
        .await
        .unwrap();
    assert_eq!(clip.codec(), VideoCodec::H264);
    let asset = catalog
        .resolve_clip_playback_asset(clip.id(), Some(HLS_PLAYLIST_RELATIVE_PATH))
        .await
        .unwrap();
    let ResolvedMediaAsset::Bytes(body) = asset else {
        panic!("expected packaged playlist bytes");
    };
    let playlist = String::from_utf8(body.bytes().to_vec()).unwrap();
    assert!(playlist.contains("#EXT-X-DISCONTINUITY"));
    assert!(playlist.contains("init-0.mp4"));
    assert!(playlist.contains("init-1.mp4"));
}

#[tokio::test]
async fn live_follow_uses_memory_store_without_disk_epochs() {
    let (catalog, source) = catalog("live-no-disk").await;
    seed_live(
        &catalog,
        &source,
        &track("epoch-live"),
        INIT_B,
        ts("2024-06-01T00:00:02Z"),
        sample(8),
    );
    seed_live(
        &catalog,
        &source,
        &track("epoch-live"),
        INIT_B,
        ts("2024-06-01T00:00:03Z"),
        sample(9),
    );
    let follow = catalog
        .follow_video(a2a_lab_dev_kit::FollowVideoRequest::new(source.clone()))
        .await
        .unwrap();
    let asset = catalog
        .resolve_live_playback_asset(follow.id(), Some(HLS_PLAYLIST_RELATIVE_PATH))
        .await
        .unwrap();
    let ResolvedMediaAsset::Bytes(body) = asset else {
        panic!("playlist");
    };
    let playlist = String::from_utf8(body.bytes().to_vec()).unwrap();
    assert!(playlist.contains("init-0.mp4"));
    assert!(!playlist.contains("init-1.mp4"));
}

#[tokio::test]
async fn historical_gap_returns_structured_no_recording() {
    let (catalog, source) = catalog("historical-gap").await;
    catalog
        .dvr()
        .append_h264_sample(
            &source,
            ts("2024-06-01T00:00:00Z"),
            200_000,
            true,
            INIT_A,
            &sample(1),
        )
        .await
        .unwrap();
    let range = TimeRange::new(ts("2024-06-01T00:01:00Z"), ts("2024-06-01T00:01:05Z")).unwrap();
    let error = catalog
        .get_video_clip(GetVideoClipRequest::new(source, range).unwrap())
        .await
        .expect_err("empty historical range");
    assert_eq!(error.code(), "no_recording");
}

#[tokio::test]
async fn saturated_dvr_queue_does_not_block_live_follow() {
    let queue = DvrPersistenceQueue::new(1).unwrap();
    assert_eq!(
        queue
            .try_enqueue(ts("2024-06-01T00:00:01Z"), vec![0xAA])
            .unwrap(),
        DvrEnqueueStatus::Accepted
    );
    assert_eq!(
        queue
            .try_enqueue(ts("2024-06-01T00:00:02Z"), vec![0xBB])
            .unwrap(),
        DvrEnqueueStatus::Full
    );
    let (catalog, source) = catalog("queue-pressure").await;
    seed_live(
        &catalog,
        &source,
        &track("epoch-live"),
        INIT_A,
        ts("2024-06-01T00:00:03Z"),
        sample(3),
    );
    seed_live(
        &catalog,
        &source,
        &track("epoch-live"),
        INIT_A,
        ts("2024-06-01T00:00:04Z"),
        sample(4),
    );
    let follow = catalog
        .follow_video(a2a_lab_dev_kit::FollowVideoRequest::new(source))
        .await
        .unwrap();
    catalog
        .resolve_live_playback_asset(follow.id(), Some(HLS_PLAYLIST_RELATIVE_PATH))
        .await
        .unwrap();
}

#[tokio::test]
async fn live_follow_pins_timeline_origin_on_refresh() {
    let (catalog, source) = catalog("origin-pin").await;
    seed_live(
        &catalog,
        &source,
        &track("epoch-live"),
        INIT_A,
        ts("2024-06-01T00:00:00Z"),
        sample(0),
    );
    seed_live(
        &catalog,
        &source,
        &track("epoch-live"),
        INIT_A,
        ts("2024-06-01T00:00:01Z"),
        sample(1),
    );
    let follow = catalog
        .follow_video(a2a_lab_dev_kit::FollowVideoRequest::new(source.clone()))
        .await
        .unwrap();
    let first = catalog
        .resolve_live_playback_asset(follow.id(), Some(HLS_PLAYLIST_RELATIVE_PATH))
        .await
        .unwrap();
    seed_live(
        &catalog,
        &source,
        &track("epoch-live"),
        INIT_A,
        ts("2024-06-01T00:00:02Z"),
        sample(2),
    );
    let second = catalog
        .resolve_live_playback_asset(follow.id(), Some(HLS_PLAYLIST_RELATIVE_PATH))
        .await
        .unwrap();
    let ResolvedMediaAsset::Bytes(first) = first else {
        panic!("first playlist");
    };
    let ResolvedMediaAsset::Bytes(second) = second else {
        panic!("second playlist");
    };
    let first_text = String::from_utf8(first.bytes().to_vec()).unwrap();
    let second_text = String::from_utf8(second.bytes().to_vec()).unwrap();
    let first_origin = first_text
        .lines()
        .find_map(|line| line.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:"))
        .expect("program date");
    let second_origin = second_text
        .lines()
        .find_map(|line| line.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:"))
        .expect("program date second");
    assert_eq!(first_origin, second_origin);
}

#[tokio::test]
async fn frame_at_reads_dvr_after_warm_still_miss() {
    let (catalog, source) = catalog("frame-at-dvr").await;
    let at = ts("2024-06-01T00:00:00Z");
    catalog
        .dvr()
        .append_mjpeg_frame(&source, at, 1_000_000, &jpeg_32x16())
        .await
        .unwrap();
    let frame = catalog
        .get_video_frame_at(GetVideoFrameAtRequest::new(source.clone(), at, None))
        .await
        .unwrap();
    assert_eq!(frame.actual_captured_at(), at);
}

#[tokio::test]
async fn frame_at_prefers_warm_still_near_now() {
    let dir = temp_root("warm-still");
    let _ = std::fs::remove_dir_all(&dir);
    let store = SqliteChunkDvrStore::open(&dir, 86_400).await.unwrap();
    let live = MemoryVideos::new();
    let source = VideoSourceId::new("warm").unwrap();
    live.insert_source(
        VideoSource::new_mjpeg(source.clone(), "warm", "still", None, None, 32, 16, 5).unwrap(),
    )
    .await
    .unwrap();
    let cache = WarmStillCache::new();
    cache.remember(source.as_str(), 1, jpeg_32x16());
    let provider = DvrVideoProvider::open(live, store, Duration::from_secs(60))
        .unwrap()
        .with_warm_stills(cache);
    let now = UtcTimestamp::now();
    let frame = provider
        .get_video_frame_at(GetVideoFrameAtRequest::new(source, now, None))
        .await
        .unwrap();
    assert!(!frame.image().data().is_empty());
}

#[tokio::test]
async fn frame_at_reports_gap_without_unrelated_frame() {
    let (catalog, source) = catalog("frame-gap").await;
    let first = ts("2024-06-01T00:00:00Z");
    let gap = ts("2024-06-01T00:01:00Z");
    catalog
        .dvr()
        .append_mjpeg_frame(&source, first, 1_000_000, &jpeg_32x16())
        .await
        .unwrap();
    let error = catalog
        .get_video_frame_at(GetVideoFrameAtRequest::new(source, gap, None))
        .await
        .expect_err("gap instant");
    assert!(
        matches!(error.code(), "recording_gap" | "no_recording"),
        "{error}"
    );
}

#[tokio::test]
async fn finite_export_single_epoch_mp4() {
    let (catalog, source) = catalog("finite-export").await;
    catalog
        .dvr()
        .append_h264_sample(
            &source,
            ts("2024-06-01T00:00:00Z"),
            200_000,
            true,
            INIT_A,
            &sample(1),
        )
        .await
        .unwrap();
    let range = TimeRange::new(ts("2024-06-01T00:00:00Z"), ts("2024-06-01T00:00:02Z")).unwrap();
    let clip = catalog
        .get_video_clip(GetVideoClipRequest::new(source, range).unwrap())
        .await
        .unwrap();
    let asset = catalog.export_finite_video_clip(clip.id()).await.unwrap();
    assert_eq!(asset.content_type(), "video/mp4");
    assert!(!asset.bytes().is_empty());
}

#[tokio::test]
async fn retention_cleanup_drops_expired_chunks() {
    let dir = temp_root("retention");
    let _ = std::fs::remove_dir_all(&dir);
    let store = SqliteChunkDvrStore::open(&dir, 1).await.unwrap();
    let live = MemoryVideos::new();
    let source = VideoSourceId::new("retain").unwrap();
    let provider = DvrVideoProvider::open(live, store, Duration::from_secs(60)).unwrap();
    let old = UtcTimestamp::now().saturating_add_micros(-5_000_000);
    provider
        .dvr()
        .append_mjpeg_frame(&source, old, 1_000_000, &jpeg_32x16())
        .await
        .unwrap();
    provider.cleanup_disk(UtcTimestamp::now()).await.unwrap();
    let track = MemoryVideos::mjpeg_track_id(&source).unwrap();
    let error = provider
        .dvr()
        .frame_at(a2a_lab_dev_kit::DvrFrameAtRequest::new(
            source, track, old, None,
        ))
        .await
        .expect_err("expired media");
    assert!(
        matches!(error.code(), "no_recording" | "not_found"),
        "{error}"
    );
}
