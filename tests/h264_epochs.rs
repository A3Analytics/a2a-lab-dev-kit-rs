//! Immutable H.264 configuration epochs in the memory DVR.

use a2a_lab_dev_kit::{
    A2aLabError, DvrMediaStore, DvrPictureProperties, DvrTrackId, H264TrackRelation,
    MJPEG_MEDIA_TYPE, MemoryVideos, PageRequest, ReadDvrMediaWindowRequest, TimeRange,
    UtcTimestamp, VideoCodec, VideoSource, VideoSourceId, h264_track_relation,
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

#[tokio::test]
async fn sequential_epochs_keep_original_initialization_and_a_gap() {
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
    let changed = DvrPictureProperties::new(1280, 720).expect("changed");
    let error = videos
        .insert_h264_epoch(source(), track("epoch-a"), SECOND_INIT.to_vec(), changed)
        .await
        .expect_err("immutable");
    assert!(matches!(error, A2aLabError::Invalid { field: "id", .. }));

    videos
        .insert_dvr_segment(
            source(),
            track("epoch-a"),
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            true,
            VideoCodec::H264,
            vec![0x01],
        )
        .await
        .expect("segment a");
    videos
        .insert_dvr_segment(
            source(),
            track("epoch-b"),
            range("2024-01-01T00:00:05Z", "2024-01-01T00:00:07Z"),
            true,
            VideoCodec::H264,
            vec![0x02],
        )
        .await
        .expect("segment b");

    let tracks = videos.list_tracks(&source()).await.expect("tracks");
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].codec_initialization(), FIRST_INIT);
    assert_eq!(tracks[1].codec_initialization(), SECOND_INIT);
    assert_eq!(tracks[0].picture().expect("picture").width(), 640);
    assert_eq!(
        tracks[0].bounds().start(),
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z").start()
    );
    assert_eq!(
        h264_track_relation(&tracks).expect("relation"),
        H264TrackRelation::SequentialEpoch
    );

    let coverage = videos.media_coverage(&source()).await.expect("coverage");
    assert_eq!(coverage.intervals().len(), 2);
    assert_eq!(coverage.discontinuities().len(), 1);

    let page = videos
        .read_media_window(
            ReadDvrMediaWindowRequest::new(
                source(),
                track("epoch-a"),
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:08Z"),
                PageRequest::new(None, 10).expect("page"),
            )
            .expect("request"),
        )
        .await
        .expect("page");
    assert_eq!(page.items().len(), 1);
    assert_eq!(page.items()[0].track_id(), &track("epoch-a"));
}

#[tokio::test]
async fn overlapping_h264_tracks_are_simultaneous_alternates() {
    let videos = store().await;
    let picture = DvrPictureProperties::new(640, 480).expect("picture");
    videos
        .insert_h264_epoch(source(), track("main"), FIRST_INIT.to_vec(), picture)
        .await
        .expect("main");
    videos
        .insert_h264_epoch(source(), track("alt"), SECOND_INIT.to_vec(), picture)
        .await
        .expect("alt");
    let bounds = range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z");
    for id in ["main", "alt"] {
        videos
            .insert_dvr_segment(
                source(),
                track(id),
                bounds,
                true,
                VideoCodec::H264,
                vec![0x11],
            )
            .await
            .expect("segment");
    }
    let tracks = videos.list_tracks(&source()).await.expect("tracks");
    assert_eq!(
        h264_track_relation(&tracks).expect("relation"),
        H264TrackRelation::SimultaneousAlternate
    );
    assert_eq!(tracks[0].id(), &track("main"));
    assert_eq!(tracks[1].id(), &track("alt"));
}

#[test]
fn odd_epoch_dimensions_are_rejected() {
    let error = DvrPictureProperties::new(641, 480).expect_err("width");
    assert!(matches!(error, A2aLabError::Invalid { field: "width", .. }));
}
