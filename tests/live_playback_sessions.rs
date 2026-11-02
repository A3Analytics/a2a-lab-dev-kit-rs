#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aLabApi, A2aLabService, DvrTrackId, FollowVideoRequest, HLS_INIT_RELATIVE_PATH,
    HLS_PLAYLIST_RELATIVE_PATH, LocalMediaAssetProvider, MemoryLogs, MemoryMetrics, MemoryTasks,
    MemoryVideos, TimeRange, UtcTimestamp, VideoCodec, VideoProvider, VideoSource, VideoSourceId,
    router,
};

const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xD9];
const H264_INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67];
const H265_INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x40, 0x01];

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

fn source_id(id: &str) -> VideoSourceId {
    VideoSourceId::new(id).unwrap()
}

fn range(start: &str, end: &str) -> TimeRange {
    TimeRange::new(timestamp(start), timestamp(end)).unwrap()
}

fn share(videos: MemoryVideos) -> Arc<dyn A2aLabApi> {
    A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos)
        .share()
}

async fn listen(lab: Arc<dyn A2aLabApi>) -> String {
    let assets = Arc::new(LocalMediaAssetProvider::new(Arc::clone(&lab)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router(assets)).await.unwrap();
    });
    format!("http://{address}")
}

async fn prepare_encoded_live(
    videos: &MemoryVideos,
    codec: VideoCodec,
    init: &[u8],
    track: &str,
) -> (String, a2a_lab_dev_kit::VideoFollow) {
    let media_type = match codec {
        VideoCodec::H264 | VideoCodec::H265 => "video/mp4",
        VideoCodec::Mjpeg => a2a_lab_dev_kit::MJPEG_MEDIA_TYPE,
    };
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
                media_type,
                vec![codec],
            )
            .unwrap(),
        )
        .await
        .unwrap();
    videos
        .insert_dvr_track(
            source_id("enc"),
            DvrTrackId::new(track).unwrap(),
            codec,
            init.to_vec(),
        )
        .await
        .unwrap();
    videos
        .insert_dvr_segment(
            source_id("enc"),
            DvrTrackId::new(track).unwrap(),
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
            true,
            codec,
            vec![1, 2, 3],
        )
        .await
        .unwrap();
    videos
        .set_live_available(&source_id("enc"), true)
        .await
        .unwrap();
    let follow = videos
        .follow_video(FollowVideoRequest::new(source_id("enc")))
        .await
        .unwrap();
    (listen(share(videos.clone())).await, follow)
}

#[tokio::test]
async fn encoded_live_root_serves_bounded_event_playlist_at_live_edge() {
    let videos = MemoryVideos::new();
    let (base, follow) = prepare_encoded_live(&videos, VideoCodec::H264, H264_INIT, "h264").await;
    assert_eq!(follow.codec(), VideoCodec::H264);
    let playlist = reqwest::Client::new()
        .get(format!("{base}{}", follow.path()))
        .send()
        .await
        .unwrap();
    assert!(playlist.status().is_success());
    let body = playlist.text().await.unwrap();
    assert!(body.contains("#EXT-X-PLAYLIST-TYPE:EVENT"));
    assert!(body.contains("#EXT-X-PROGRAM-DATE-TIME:"));
    assert!(!body.contains("#EXT-X-ENDLIST"));
    assert!(!body.contains("h264"));
    assert!(!body.contains("enc"));
    let init = reqwest::Client::new()
        .get(format!("{base}{}{HLS_INIT_RELATIVE_PATH}", follow.path()))
        .send()
        .await
        .unwrap();
    assert_eq!(init.status(), 200);
}

#[tokio::test]
async fn live_playlist_refresh_advances_window_and_marks_gaps() {
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
                vec![VideoCodec::H264],
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let track = DvrTrackId::new("h264").unwrap();
    videos
        .insert_dvr_track(
            source_id("enc"),
            track.clone(),
            VideoCodec::H264,
            H264_INIT.to_vec(),
        )
        .await
        .unwrap();
    for (start, end) in [
        ("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
        ("2024-01-01T00:00:04Z", "2024-01-01T00:00:06Z"),
    ] {
        videos
            .insert_dvr_segment(
                source_id("enc"),
                track.clone(),
                range(start, end),
                true,
                VideoCodec::H264,
                vec![1, 2],
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
    let lab = share(videos.clone());
    let base = listen(lab).await;
    let first = reqwest::Client::new()
        .get(format!(
            "{base}{}{HLS_PLAYLIST_RELATIVE_PATH}",
            follow.path()
        ))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(first.contains("#EXT-X-DISCONTINUITY"));
    videos
        .insert_dvr_segment(
            source_id("enc"),
            track,
            range("2024-01-01T00:00:08Z", "2024-01-01T00:00:10Z"),
            true,
            VideoCodec::H264,
            vec![9, 9],
        )
        .await
        .unwrap();
    let second = reqwest::Client::new()
        .get(format!(
            "{base}{}{HLS_PLAYLIST_RELATIVE_PATH}",
            follow.path()
        ))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_ne!(first, second);
    assert!(second.contains("seg-"));
}

#[tokio::test]
async fn live_rebase_and_retention_movement_keep_opaque_windows() {
    let videos = MemoryVideos::new();
    let track = DvrTrackId::new("h265").unwrap();
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
    videos
        .insert_dvr_track(
            source_id("enc"),
            track.clone(),
            VideoCodec::H265,
            H265_INIT.to_vec(),
        )
        .await
        .unwrap();
    for start in ["2024-01-01T00:00:00Z", "2024-01-01T01:00:00Z"] {
        let end = UtcTimestamp::parse(start)
            .unwrap()
            .saturating_add_micros(2_000_000);
        videos
            .insert_dvr_segment(
                source_id("enc"),
                track.clone(),
                TimeRange::new(timestamp(start), end).unwrap(),
                true,
                VideoCodec::H265,
                vec![1],
            )
            .await
            .unwrap();
    }
    videos
        .set_live_available(&source_id("enc"), true)
        .await
        .unwrap();
    let follow = videos
        .follow_video(FollowVideoRequest::at(
            source_id("enc"),
            timestamp("2024-01-01T00:00:01Z"),
        ))
        .await
        .unwrap();
    assert_eq!(follow.codec(), VideoCodec::H265);
    let lab = share(videos.clone());
    let rebased = lab
        .rebase_video_follow(follow.id(), timestamp("2024-01-01T01:00:01Z"))
        .await
        .unwrap();
    assert_eq!(rebased.id(), follow.id());
    let base = listen(lab).await;
    let playlist = reqwest::Client::new()
        .get(format!(
            "{base}{}{HLS_PLAYLIST_RELATIVE_PATH}",
            follow.path()
        ))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(playlist.contains("#EXT-X-PROGRAM-DATE-TIME:2024-01-01T01:00:00Z"));
    videos
        .forget_before(&source_id("enc"), timestamp("2024-01-01T00:30:00Z"))
        .await
        .unwrap();
    let after_retention = reqwest::Client::new()
        .get(format!(
            "{base}{}{HLS_PLAYLIST_RELATIVE_PATH}",
            follow.path()
        ))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!after_retention.contains("2024-01-01T00:00:00"));
}

#[tokio::test]
async fn live_sessions_expire_and_release_on_unavailable() {
    let videos = MemoryVideos::with_session_ttl(Duration::from_secs(60), Duration::from_millis(1));
    let (_base, follow) = prepare_encoded_live(&videos, VideoCodec::H264, H264_INIT, "h264").await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    let expired = videos.resolve_live_playback_asset(follow.id(), None).await;
    assert_eq!(
        expired.err().map(|error| error.code()),
        Some("clip_expired")
    );

    let videos = MemoryVideos::new();
    let (base, follow) = prepare_encoded_live(&videos, VideoCodec::H264, H264_INIT, "h264").await;
    videos
        .set_live_available(&source_id("enc"), false)
        .await
        .unwrap();
    let unavailable = videos.resolve_live_playback_asset(follow.id(), None).await;
    assert_eq!(
        unavailable.err().map(|error| error.code()),
        Some("live_unavailable")
    );
    let gateway = reqwest::Client::new()
        .get(format!("{base}{}", follow.path()))
        .send()
        .await
        .unwrap();
    assert_eq!(gateway.status(), 503);
}

#[tokio::test]
async fn mjpeg_live_follow_still_streams_with_gateway_security() {
    let videos = MemoryVideos::new();
    videos
        .insert_source(
            VideoSource::new_mjpeg(source_id("cam"), "Cam", "bench", None, None, 640, 480, 30)
                .unwrap(),
        )
        .await
        .unwrap();
    videos
        .set_live_available(&source_id("cam"), true)
        .await
        .unwrap();
    let follow = videos
        .follow_video(FollowVideoRequest::new(source_id("cam")))
        .await
        .unwrap();
    assert_eq!(follow.codec(), VideoCodec::Mjpeg);
    let lab = share(videos.clone());
    let base = listen(lab).await;
    let response = reqwest::Client::new()
        .get(format!("{base}{}", follow.path()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    drop(response);
    let again = reqwest::Client::new()
        .get(format!("{base}{}", follow.path()))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), 400);
}
