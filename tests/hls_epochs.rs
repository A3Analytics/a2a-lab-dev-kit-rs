//! Sequential H.264 epochs keep separate HLS initialization.

use a2a_lab_dev_kit::{
    A2aLabError, DvrMediaPackager, DvrPictureProperties, DvrTrackId, EpochPlaylistKind,
    MJPEG_MEDIA_TYPE, MemoryVideos, TimeRange, UtcTimestamp, VideoCodec, VideoSource,
    VideoSourceId, package_h264_epochs,
};

const FIRST_INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67, 0x42];
const SECOND_INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67, 0x64];

fn source() -> VideoSourceId {
    VideoSourceId::new("cam").expect("source")
}

fn track(id: &str) -> DvrTrackId {
    DvrTrackId::new(id).expect("track")
}

fn range(start: &str, end: &str) -> TimeRange {
    TimeRange::new(
        UtcTimestamp::parse(start).expect("start"),
        UtcTimestamp::parse(end).expect("end"),
    )
    .expect("range")
}

async fn store() -> MemoryVideos {
    let videos = MemoryVideos::new();
    videos
        .insert_source(
            VideoSource::new(
                source(),
                "cam",
                "camera",
                None,
                None,
                1,
                1,
                5,
                MJPEG_MEDIA_TYPE,
            )
            .expect("source"),
        )
        .await
        .expect("insert source");
    videos
}

async fn segment(videos: &MemoryVideos, track_id: &str, start: &str, end: &str, keyframe: bool) {
    videos
        .insert_dvr_segment(
            source(),
            track(track_id),
            range(start, end),
            keyframe,
            VideoCodec::H264,
            vec![0x65],
        )
        .await
        .expect("segment");
}

#[tokio::test]
async fn sequential_epochs_map_each_segment_to_its_own_init() {
    let videos = store().await;
    let picture = DvrPictureProperties::new(640, 480).expect("picture");
    videos
        .insert_h264_epoch(source(), track("epoch-a"), FIRST_INIT.to_vec(), picture)
        .await
        .expect("epoch a");
    videos
        .insert_h264_epoch(source(), track("epoch-b"), SECOND_INIT.to_vec(), picture)
        .await
        .expect("epoch b");
    segment(
        &videos,
        "epoch-a",
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:02Z",
        true,
    )
    .await;
    segment(
        &videos,
        "epoch-a",
        "2024-01-01T00:00:02Z",
        "2024-01-01T00:00:04Z",
        false,
    )
    .await;
    segment(
        &videos,
        "epoch-b",
        "2024-01-01T00:00:06Z",
        "2024-01-01T00:00:08Z",
        true,
    )
    .await;

    let window = range("2024-01-01T00:00:00Z", "2024-01-01T00:00:08Z");
    let session = package_h264_epochs(&videos, &source(), window, EpochPlaylistKind::Finite)
        .await
        .expect("playlist");
    assert_eq!(session.segments().len(), 3);
    assert_eq!(session.inits()[0].initialization(), FIRST_INIT);
    assert_eq!(session.inits()[1].initialization(), SECOND_INIT);
    assert_eq!(session.segments()[0].init_relative_path(), "init-0.mp4");
    assert_eq!(session.segments()[1].init_relative_path(), "init-0.mp4");
    assert_eq!(session.segments()[2].init_relative_path(), "init-1.mp4");
    assert!(session.segments()[2].discontinuity_before());
    assert!(session.playlist().contains("#EXT-X-DISCONTINUITY"));
    assert!(session.playlist().contains("init-0.mp4"));
    assert!(session.playlist().contains("init-1.mp4"));
    assert!(session.playlist().contains("#EXT-X-ENDLIST"));
    let trick = session.trick_playlist().expect("8x");
    assert!(trick.contains("seg-0-0.m4s"));
    assert!(!trick.contains("seg-0-1.m4s"));
    assert!(trick.contains("seg-1-0.m4s"));
    assert!(trick.contains("#EXT-X-DISCONTINUITY"));
    assert_eq!(
        session.segments()[2].program_date_time(),
        range("2024-01-01T00:00:06Z", "2024-01-01T00:00:08Z").start()
    );

    let live_window = range("2024-01-01T00:00:01Z", "2024-01-01T00:00:08Z");
    let live = package_h264_epochs(&videos, &source(), live_window, EpochPlaylistKind::Live)
        .await
        .expect("live");
    assert_eq!(live.inits().len(), 2);
    assert!(!live.playlist().contains("#EXT-X-ENDLIST"));
    assert_eq!(live.segments().len(), 3);

    let later = range("2024-01-01T00:00:06Z", "2024-01-01T00:00:08Z");
    let tail = package_h264_epochs(&videos, &source(), later, EpochPlaylistKind::Live)
        .await
        .expect("tail");
    assert_eq!(tail.inits().len(), 1);
    assert_eq!(tail.inits()[0].initialization(), SECOND_INIT);
    assert_eq!(tail.segments().len(), 1);
}

#[tokio::test]
async fn single_epoch_callers_stay_compatible_and_overlaps_are_rejected() {
    let videos = store().await;
    let picture = DvrPictureProperties::new(640, 480).expect("picture");
    videos
        .insert_h264_epoch(source(), track("only"), FIRST_INIT.to_vec(), picture)
        .await
        .expect("epoch");
    segment(
        &videos,
        "only",
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:05Z",
        true,
    )
    .await;
    let packaged = DvrMediaPackager::default()
        .package_hls_range(
            &videos,
            "/media/session/",
            &source(),
            &track("only"),
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:05Z"),
        )
        .await
        .expect("single epoch");
    assert_eq!(packaged.track_id(), &track("only"));
    assert!(packaged.playlist().contains("init.mp4"));

    videos
        .insert_h264_epoch(source(), track("overlap"), SECOND_INIT.to_vec(), picture)
        .await
        .expect("overlap epoch");
    segment(
        &videos,
        "overlap",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:03Z",
        true,
    )
    .await;
    let error = package_h264_epochs(
        &videos,
        &source(),
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:05Z"),
        EpochPlaylistKind::Finite,
    )
    .await
    .expect_err("overlap");
    assert!(matches!(
        error,
        A2aLabError::Invalid {
            field: "tracks",
            ..
        }
    ));
}
