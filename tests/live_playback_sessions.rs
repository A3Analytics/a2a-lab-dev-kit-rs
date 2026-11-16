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

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap()
}

fn listen(lab: Arc<dyn A2aLabApi>) -> String {
    let assets = Arc::new(LocalMediaAssetProvider::new(lab));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("media listen runtime")
            .block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                axum::serve(listener, router(assets)).await.unwrap();
            });
    });
    wait_until_serving(address);
    format!("http://{address}")
}

fn wait_until_serving(address: std::net::SocketAddr) {
    use std::io::{Read, Write};
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(mut stream) =
            std::net::TcpStream::connect_timeout(&address, Duration::from_millis(100))
        {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
            let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
            if stream
                .write_all(
                    b"GET /media/video-live/probe HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
                )
                .is_ok()
            {
                let mut buf = [0u8; 12];
                if stream.read(&mut buf).is_ok_and(|n| n > 0) {
                    return;
                }
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "media gateway did not serve on {address}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
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
    let base = listen(share(videos.clone()));
    let follow = videos
        .follow_video(FollowVideoRequest::new(source_id("enc")))
        .await
        .unwrap();
    (base, follow)
}

#[tokio::test(flavor = "multi_thread")]
async fn encoded_live_root_serves_bounded_event_playlist_at_live_edge() {
    let videos = MemoryVideos::new();
    let (base, follow) = prepare_encoded_live(&videos, VideoCodec::H264, H264_INIT, "h264").await;
    assert_eq!(follow.codec(), VideoCodec::H264);
    let playlist = http_client()
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
    let init = http_client()
        .get(format!("{base}{}{HLS_INIT_RELATIVE_PATH}", follow.path()))
        .send()
        .await
        .unwrap();
    assert_eq!(init.status(), 200);
}

#[tokio::test(flavor = "multi_thread")]
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
    let lab = share(videos.clone());
    let base = listen(lab);
    let follow = videos
        .follow_video(FollowVideoRequest::new(source_id("enc")))
        .await
        .unwrap();
    let first = http_client()
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
    let second = http_client()
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

#[tokio::test(flavor = "multi_thread")]
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
    let lab = share(videos.clone());
    let base = listen(Arc::clone(&lab));
    let follow = videos
        .follow_video(FollowVideoRequest::at(
            source_id("enc"),
            timestamp("2024-01-01T00:00:01Z"),
        ))
        .await
        .unwrap();
    assert_eq!(follow.codec(), VideoCodec::H265);
    let rebased = lab
        .rebase_video_follow(follow.id(), timestamp("2024-01-01T01:00:01Z"))
        .await
        .unwrap();
    assert_eq!(rebased.id(), follow.id());
    let playlist = http_client()
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
    let after_retention = http_client()
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

#[tokio::test(flavor = "multi_thread")]
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
    let gateway = http_client()
        .get(format!("{base}{}", follow.path()))
        .send()
        .await
        .unwrap();
    assert_eq!(gateway.status(), 503);
}

#[tokio::test(flavor = "multi_thread")]
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
    let lab = share(videos.clone());
    let base = listen(lab);
    let follow = videos
        .follow_video(FollowVideoRequest::new(source_id("cam")))
        .await
        .unwrap();
    assert_eq!(follow.codec(), VideoCodec::Mjpeg);
    let response = http_client()
        .get(format!("{base}{}", follow.path()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    drop(response);
    let again = http_client()
        .get(format!("{base}{}", follow.path()))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), 400);
}
