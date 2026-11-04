use a2a_lab_dev_kit::{
    DvrMediaPackager, DvrMediaStore, DvrPackagerConfig, HLS_INIT_RELATIVE_PATH,
    HLS_PLAYLIST_RELATIVE_PATH, MemoryVideos, TimeRange, UtcTimestamp, VideoCodec, VideoSource,
};

use super::dvr_store_contract::FakeDvrMediaStore;

const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xD9];

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).expect("timestamp")
}

fn source_id(id: &str) -> a2a_lab_dev_kit::VideoSourceId {
    a2a_lab_dev_kit::VideoSourceId::new(id).expect("source")
}

fn track_id(id: &str) -> a2a_lab_dev_kit::DvrTrackId {
    a2a_lab_dev_kit::DvrTrackId::new(id).expect("track")
}

fn range(start: &str, end: &str) -> TimeRange {
    TimeRange::new(timestamp(start), timestamp(end)).expect("range")
}

fn packager() -> DvrMediaPackager {
    DvrMediaPackager::new(DvrPackagerConfig::new(120_000_000).unwrap())
}

#[tokio::test]
async fn packager_produces_cmaf_assets_without_jpeg_transcoding() {
    let store = FakeDvrMediaStore::gapped_h264();
    let packager = packager();
    let source = source_id("cam");
    let track = track_id("h264");
    let session = packager
        .package_hls_at(
            &store,
            "/media/video-clips/s1/",
            &source,
            &track,
            timestamp("2024-01-01T00:00:00.500000Z"),
        )
        .await
        .unwrap();
    assert_eq!(session.codec(), VideoCodec::H264);
    let init = packager
        .load_hls_asset(&store, &session, HLS_INIT_RELATIVE_PATH)
        .await
        .unwrap();
    assert_eq!(init.content_type(), "video/mp4");
    assert!(init.bytes().windows(4).any(|window| window == b"ftyp"));
    let segment = packager
        .load_hls_asset(&store, &session, session.segments()[0].relative_path())
        .await
        .unwrap();
    assert_eq!(segment.content_type(), "video/iso.segment");
    assert!(segment.bytes().windows(4).any(|window| window == b"styp"));
    assert!(segment.bytes().windows(4).any(|window| window == b"moof"));
}

#[tokio::test]
async fn packager_h265_init_and_playlist_use_bounded_window() {
    let store = FakeDvrMediaStore::h265_track();
    let packager = packager();
    let session = packager
        .package_hls_at(
            &store,
            "/media/video-clips/h265/",
            &source_id("cam"),
            &track_id("h265"),
            timestamp("2024-01-01T00:00:01Z"),
        )
        .await
        .unwrap();
    assert_eq!(session.segments().len(), 1);
    let playlist = packager
        .load_hls_asset(&store, &session, HLS_PLAYLIST_RELATIVE_PATH)
        .await
        .unwrap();
    assert!(playlist.bytes().starts_with(b"#EXTM3U\n#EXT-X-VERSION:7"));
}

#[tokio::test]
async fn packager_long_retention_manifest_stays_bounded() {
    let store = FakeDvrMediaStore::long_range_h264();
    let packager = packager();
    let session = packager
        .package_hls_at(
            &store,
            "/media/video-clips/long/",
            &source_id("cam"),
            &track_id("h264"),
            timestamp("2024-01-01T01:00:00Z"),
        )
        .await
        .unwrap();
    assert!(
        session.segments().len() <= 121,
        "expected at most ~120 one-second segments, got {}",
        session.segments().len()
    );
    let span = session.window().end().as_micros() - session.window().start().as_micros();
    assert!(span <= 120_000_000);
}

#[tokio::test]
async fn packager_rebase_changes_window_and_session_relative_paths() {
    let store = FakeDvrMediaStore::long_range_h264();
    let packager = packager();
    let root = "/media/video-clips/rebase/";
    let source = source_id("cam");
    let track = track_id("h264");
    let first = packager
        .package_hls_at(
            &store,
            root,
            &source,
            &track,
            timestamp("2024-01-01T00:10:00Z"),
        )
        .await
        .unwrap();
    let second = packager
        .rebase_hls(&store, &first, timestamp("2024-01-01T01:30:00Z"))
        .await
        .unwrap();
    assert_eq!(second.session_root(), root);
    assert_ne!(first.window(), second.window());
    assert!(
        second
            .segments()
            .iter()
            .all(|segment| segment.relative_path().starts_with("seg-"))
    );
    assert!(second.playlist().contains(HLS_INIT_RELATIVE_PATH));
}

#[tokio::test]
async fn packager_playlist_maps_wall_clock_and_marks_gaps() {
    let store = FakeDvrMediaStore::gapped_h264();
    let packager = packager();
    let session = packager
        .package_hls_at(
            &store,
            "/media/video-clips/gap/",
            &source_id("cam"),
            &track_id("h264"),
            timestamp("2024-01-01T00:00:00Z"),
        )
        .await
        .unwrap();
    let playlist = session.playlist();
    assert!(playlist.contains("#EXT-X-PROGRAM-DATE-TIME:"));
    assert!(playlist.contains("#EXT-X-DISCONTINUITY"));
}

#[tokio::test]
async fn packager_exports_finite_mp4_and_mjpeg_multipart() {
    let h264 = FakeDvrMediaStore::gapped_h264();
    let packager = packager();
    let mp4 = packager
        .export_finite_mp4(
            &h264,
            &source_id("cam"),
            &track_id("h264"),
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
        )
        .await
        .unwrap();
    assert_eq!(mp4.content_type(), "video/mp4");
    assert!(mp4.bytes().windows(4).any(|window| window == b"ftyp"));

    let mjpeg = FakeDvrMediaStore::continuous_mjpeg();
    let multipart = packager
        .export_finite_mjpeg(
            &mjpeg,
            &source_id("cam"),
            &track_id("mjpeg"),
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:03Z"),
        )
        .await
        .unwrap();
    assert!(
        multipart
            .content_type()
            .starts_with("multipart/x-mixed-replace")
    );
    assert!(multipart.bytes().starts_with(b"--"));
    assert!(multipart.bytes().ends_with(b"--\r\n"));
}

#[tokio::test]
async fn packager_fake_store_covers_malformed_init_and_missing_assets() {
    let store = FakeDvrMediaStore::malformed_h264_init();
    let packager = packager();
    let error = packager
        .package_hls_at(
            &store,
            "/media/video-clips/bad/",
            &source_id("cam"),
            &track_id("h264"),
            timestamp("2024-01-01T00:00:00Z"),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "invalid");

    let store = FakeDvrMediaStore::gapped_h264();
    let session = packager
        .package_hls_at(
            &store,
            "/media/video-clips/missing/",
            &source_id("cam"),
            &track_id("h264"),
            timestamp("2024-01-01T00:00:00Z"),
        )
        .await
        .unwrap();
    let missing = packager
        .load_hls_asset(&store, &session, "seg-9999.m4s")
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");

    let sparse = FakeDvrMediaStore::segment_read_hole();
    let mp4 = packager
        .export_finite_mp4(
            &sparse,
            &source_id("cam"),
            &track_id("h264"),
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
        )
        .await
        .unwrap();
    assert!(mp4.bytes().windows(4).any(|window| window == b"moof"));
}

#[tokio::test]
async fn memory_videos_reuses_shared_packager_over_dvr_store() {
    let videos = MemoryVideos::new();
    videos
        .insert_source(
            VideoSource::new_mjpeg(source_id("cam"), "Cam", "bench", None, None, 640, 480, 30)
                .unwrap(),
        )
        .await
        .unwrap();
    let source = source_id("cam");
    for second in 0..3 {
        videos
            .insert_frame(
                source.clone(),
                timestamp(&format!("2024-01-01T00:00:0{second}Z")),
                JPEG.to_vec(),
            )
            .await
            .unwrap();
    }
    let tracks = videos.list_tracks(&source).await.unwrap();
    let track = tracks[0].id().clone();
    let packager = packager();
    let export = packager
        .export_finite_mjpeg(
            &videos,
            &source,
            &track,
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
        )
        .await
        .unwrap();
    assert!(export.bytes().len() > JPEG.len());
}
