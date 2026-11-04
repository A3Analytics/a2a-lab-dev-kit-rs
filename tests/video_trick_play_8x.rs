#![allow(dead_code)]

use a2a_lab_dev_kit::{
    DvrMediaPackager, DvrPackagerConfig, DvrTrackId, FollowVideoRequest, HLS_INIT_RELATIVE_PATH,
    HLS_PLAYLIST_RELATIVE_PATH, HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH, MemoryVideos,
    TRICK_8X_UNAVAILABLE_INSUFFICIENT_KEYFRAMES, TRICK_8X_UNAVAILABLE_NO_REDUCTION, TimeRange,
    UtcTimestamp, VideoCodec, VideoProvider, VideoSource, VideoSourceId,
};

#[path = "support/dvr_store_contract.rs"]
mod dvr_store_contract;

use dvr_store_contract::FakeDvrMediaStore;

const H264_INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67];
const H265_INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x40, 0x01];

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

#[tokio::test]
async fn trick_8x_publishes_keyframe_rendition_with_wall_clock_and_gaps() {
    let store = FakeDvrMediaStore::gapped_h264();
    let session = packager()
        .package_hls_at(
            &store,
            "/media/video-clips/gap/",
            &source_id("cam"),
            &track_id("h264"),
            timestamp("2024-01-01T00:00:00Z"),
        )
        .await
        .unwrap();
    let trick = session
        .trick_8x()
        .expect("sparse keyframes should enable trick play");
    assert!(trick.segments().len() <= session.segments().len());
    assert_eq!(
        trick.playlist_relative_path(),
        HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH
    );
    assert!(trick.playlist().contains("#EXT-X-PROGRAM-DATE-TIME:"));
    assert!(trick.playlist().contains("#EXT-X-DISCONTINUITY"));
    assert!(trick.playlist().contains(HLS_INIT_RELATIVE_PATH));
    let init = packager()
        .load_hls_asset(&store, &session, HLS_INIT_RELATIVE_PATH)
        .await
        .unwrap();
    assert_eq!(init.content_type(), "video/mp4");
    let trick_playlist = packager()
        .load_hls_asset(&store, &session, HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH)
        .await
        .unwrap();
    assert_eq!(
        trick_playlist.content_type(),
        "application/vnd.apple.mpegurl"
    );
    let metadata = session.playback_8x_metadata();
    assert_eq!(
        metadata.rendition_path(),
        Some(HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH)
    );
    assert_eq!(metadata.fallback_path(), HLS_PLAYLIST_RELATIVE_PATH);
    assert!(metadata.unavailable_reason().is_none());
}

#[tokio::test]
async fn trick_8x_omits_invalid_renditions_for_sparse_or_single_segment_windows() {
    let store = FakeDvrMediaStore::h265_track();
    let session = packager()
        .package_hls_at(
            &store,
            "/media/video-clips/h265/",
            &source_id("cam"),
            &track_id("h265"),
            timestamp("2024-01-01T00:00:01Z"),
        )
        .await
        .unwrap();
    assert!(session.trick_8x().is_none());
    assert_eq!(
        session.trick_8x_unavailable_reason(),
        Some(TRICK_8X_UNAVAILABLE_NO_REDUCTION)
    );
    let metadata = session.playback_8x_metadata();
    assert!(metadata.rendition_path().is_none());
    assert_eq!(metadata.fallback_path(), HLS_PLAYLIST_RELATIVE_PATH);
    assert_eq!(
        metadata.unavailable_reason(),
        Some(TRICK_8X_UNAVAILABLE_NO_REDUCTION)
    );
}

#[tokio::test]
async fn trick_8x_h265_declaration_and_decodable_segments() {
    let videos = MemoryVideos::new();
    videos
        .insert_source(
            VideoSource::with_capabilities(
                source_id("enc"),
                "Enc",
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
    videos
        .insert_dvr_track(
            source_id("enc"),
            track.clone(),
            VideoCodec::H265,
            H265_INIT.to_vec(),
        )
        .await
        .unwrap();
    for second in 0..16 {
        videos
            .insert_dvr_segment(
                source_id("enc"),
                track.clone(),
                range(
                    &format!("2024-01-01T00:00:{second:02}Z"),
                    &format!("2024-01-01T00:00:{:02}Z", second + 1),
                ),
                second % 8 == 0,
                VideoCodec::H265,
                vec![u8::try_from(second).unwrap()],
            )
            .await
            .unwrap();
    }
    videos
        .set_live_available(&source_id("enc"), true)
        .await
        .unwrap();
    let follow = videos
        .follow_video(FollowVideoRequest::new(source_id("enc")))
        .await
        .unwrap();
    assert_eq!(follow.codec(), VideoCodec::H265);
    let metadata = follow.playback_8x();
    assert_eq!(
        metadata.rendition_path(),
        Some(HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH)
    );
    assert_eq!(metadata.fallback_path(), HLS_PLAYLIST_RELATIVE_PATH);
}

async fn prepare_encoded_live_h264(
    videos: &MemoryVideos,
    segment_count: u32,
    keyframe_every: u32,
) -> DvrTrackId {
    videos
        .insert_source(
            VideoSource::with_capabilities(
                source_id("enc"),
                "Enc",
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
        .insert_dvr_track(
            source_id("enc"),
            track.clone(),
            VideoCodec::H264,
            H264_INIT.to_vec(),
        )
        .await
        .unwrap();
    for second in 0..segment_count {
        videos
            .insert_dvr_segment(
                source_id("enc"),
                track.clone(),
                range(
                    &format!("2024-01-01T00:00:{second:02}Z"),
                    &format!("2024-01-01T00:00:{:02}Z", second + 1),
                ),
                second % keyframe_every == 0,
                VideoCodec::H264,
                vec![1, u8::try_from(second).unwrap()],
            )
            .await
            .unwrap();
    }
    videos
        .set_live_available(&source_id("enc"), true)
        .await
        .unwrap();
    track
}

#[tokio::test]
async fn trick_8x_live_refresh_and_rebase_update_metadata_and_manifests() {
    let videos = MemoryVideos::new();
    let track = prepare_encoded_live_h264(&videos, 20, 4).await;
    let follow = videos
        .follow_video(FollowVideoRequest::new(source_id("enc")))
        .await
        .unwrap();
    assert!(follow.playback_8x().rendition_path().is_some());
    let first = videos
        .resolve_live_playback_asset(follow.id(), Some(HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH))
        .await
        .unwrap();
    let first_body = match first {
        a2a_lab_dev_kit::ResolvedMediaAsset::Bytes(asset) => asset.bytes().to_vec(),
        _ => panic!("expected packaged bytes"),
    };
    videos
        .insert_dvr_segment(
            source_id("enc"),
            track,
            range("2024-01-01T00:00:20Z", "2024-01-01T00:00:21Z"),
            true,
            VideoCodec::H264,
            vec![9, 9],
        )
        .await
        .unwrap();
    let second = videos
        .resolve_live_playback_asset(follow.id(), Some(HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH))
        .await
        .unwrap();
    let second_body = match second {
        a2a_lab_dev_kit::ResolvedMediaAsset::Bytes(asset) => asset.bytes().to_vec(),
        _ => panic!("expected packaged bytes"),
    };
    assert_ne!(
        first_body, second_body,
        "live refresh should move the bounded window"
    );
    let rebased = videos
        .rebase_video_follow(follow.id(), timestamp("2024-01-01T00:00:05Z"))
        .await
        .unwrap();
    assert!(rebased.playback_8x().rendition_path().is_some());
    let retained = videos
        .resolve_live_playback_asset(follow.id(), Some(HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH))
        .await
        .unwrap();
    let retained_body = match retained {
        a2a_lab_dev_kit::ResolvedMediaAsset::Bytes(asset) => {
            String::from_utf8_lossy(asset.bytes()).into_owned()
        }
        _ => panic!("expected packaged bytes"),
    };
    assert!(retained_body.contains("#EXT-X-PROGRAM-DATE-TIME:"));
}

#[tokio::test]
async fn trick_8x_sparse_keyframes_omit_rendition_with_recoverable_fallback() {
    let videos = MemoryVideos::new();
    videos.insert_source(source("cam")).await.unwrap();
    let source = source_id("cam");
    let track = track_id("h264");
    videos
        .insert_dvr_track(
            source.clone(),
            track.clone(),
            VideoCodec::H264,
            H264_INIT.to_vec(),
        )
        .await
        .unwrap();
    for second in 0..4 {
        videos
            .insert_dvr_segment(
                source.clone(),
                track.clone(),
                range(
                    &format!("2024-01-01T00:00:0{second}Z"),
                    &format!("2024-01-01T00:00:0{}Z", second + 1),
                ),
                second == 1,
                VideoCodec::H264,
                vec![1, u8::try_from(second).unwrap()],
            )
            .await
            .unwrap();
    }
    videos.set_live_available(&source, true).await.unwrap();
    let follow = videos
        .follow_video(FollowVideoRequest::at(
            source,
            timestamp("2024-01-01T00:00:00.500000Z"),
        ))
        .await
        .unwrap();
    let metadata = follow.playback_8x();
    assert!(metadata.rendition_path().is_none());
    assert_eq!(metadata.fallback_path(), HLS_PLAYLIST_RELATIVE_PATH);
    assert_eq!(
        metadata.unavailable_reason(),
        Some(TRICK_8X_UNAVAILABLE_INSUFFICIENT_KEYFRAMES)
    );
    let normal = videos
        .resolve_live_playback_asset(follow.id(), Some(HLS_PLAYLIST_RELATIVE_PATH))
        .await
        .unwrap();
    assert!(matches!(
        normal,
        a2a_lab_dev_kit::ResolvedMediaAsset::Bytes(_)
    ));
}

fn source(id: &str) -> VideoSource {
    VideoSource::with_capabilities(
        source_id(id),
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
    .unwrap()
}
