//! HLS live bench for browser MSE tests.

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, DvrTrackId, HttpAuthSecurityScheme, MemoryLogs, MemoryMetrics,
    MemoryTasks, MemoryVideos, SecurityScheme, TimeRange, UtcTimestamp, VideoCodec, VideoSource,
    VideoSourceId, bind_local,
};

const H264_INIT: &[u8] = &[
    0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x0A, 0xF8, 0x41, 0xA2, 0x00, 0x00, 0x00, 0x01, 0x68,
    0xCE, 0x38, 0x80,
];

fn h264_sample(keyframe: bool) -> Vec<u8> {
    if keyframe {
        vec![0x00, 0x00, 0x00, 0x05, 0x65, 0x88, 0x80, 0x10, 0x00]
    } else {
        vec![0x00, 0x00, 0x00, 0x04, 0x41, 0x9A, 0x24, 0x08]
    }
}
pub fn is_m4s_uri(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("m4s"))
}

pub const BENCH_SOURCE: &str = "enc-live";
const TOKEN: &str = "hls-bearer-token";
pub const BEARER_TOKEN: &str = TOKEN;

pub async fn encoded_live_catalog(segment_count: u32) -> MemoryVideos {
    let videos = MemoryVideos::new();
    videos
        .insert_source(
            VideoSource::with_capabilities(
                VideoSourceId::new(BENCH_SOURCE).unwrap(),
                "Encoded live",
                "HLS bench",
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
            VideoSourceId::new(BENCH_SOURCE).unwrap(),
            track.clone(),
            VideoCodec::H264,
            H264_INIT.to_vec(),
        )
        .await
        .unwrap();
    for second in 0..segment_count {
        let start = format!("2024-01-01T00:00:{second:02}Z");
        let end = format!("2024-01-01T00:00:{:02}Z", second + 1);
        videos
            .insert_dvr_segment(
                VideoSourceId::new(BENCH_SOURCE).unwrap(),
                track.clone(),
                TimeRange::new(
                    UtcTimestamp::parse(&start).unwrap(),
                    UtcTimestamp::parse(&end).unwrap(),
                )
                .unwrap(),
                second % 8 == 0,
                VideoCodec::H264,
                h264_sample(second % 8 == 0),
            )
            .await
            .unwrap();
    }
    videos
        .set_live_available(&VideoSourceId::new(BENCH_SOURCE).unwrap(), true)
        .await
        .unwrap();
    videos
}

pub async fn serve_encoded_live(videos: MemoryVideos) -> String {
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos)
        .share();
    let (listener, address) = bind_local().await.unwrap();
    let server = A2aServer::new(&service).with_web_ui().with_security(
        [(
            "bearerAuth".to_owned(),
            SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                scheme: "bearer".to_owned(),
                description: None,
                bearer_format: None,
            }),
        )]
        .into(),
        vec![[("bearerAuth".to_owned(), Vec::new())].into()],
    );
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

pub async fn bench_lab() -> String {
    serve_encoded_live(encoded_live_catalog(32).await).await
}

#[cfg(test)]
mod tests {
    use a2a_lab_dev_kit::{
        FollowVideoRequest, HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH, ResolvedMediaAsset, VideoProvider,
        VideoSourceId,
    };

    use super::*;

    #[tokio::test]
    async fn bench_exposes_trick_playlist_without_endlist() {
        let videos = encoded_live_catalog(32).await;
        let follow = videos
            .follow_video(FollowVideoRequest::new(
                VideoSourceId::new(BENCH_SOURCE).unwrap(),
            ))
            .await
            .unwrap();
        assert!(follow.playback_8x().rendition_path().is_some());
        let resolved = videos
            .resolve_live_playback_asset(follow.id(), Some(HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH))
            .await
            .unwrap();
        let body = match resolved {
            ResolvedMediaAsset::Bytes(asset) => String::from_utf8_lossy(asset.bytes()).to_string(),
            _ => panic!("expected bytes"),
        };
        assert!(!body.contains("#EXT-X-ENDLIST"));
        assert!(
            body.lines().filter(|line| is_m4s_uri(line)).count() >= 2,
            "expected multiple trick segments:\n{body}"
        );
    }

    #[tokio::test]
    async fn bench_trick_playlist_on_wire_has_no_endlist() {
        let videos = encoded_live_catalog(32).await;
        let base = serve_encoded_live(videos.clone()).await;
        let follow = videos
            .follow_video(FollowVideoRequest::new(
                VideoSourceId::new(BENCH_SOURCE).unwrap(),
            ))
            .await
            .unwrap();
        let url = format!(
            "{base}{}{HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH}",
            follow.path()
        );
        let body = reqwest::Client::new()
            .get(url)
            .header("Authorization", format!("Bearer {BEARER_TOKEN}"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(!body.contains("#EXT-X-ENDLIST"));
        let segment_count = body.lines().filter(|line| is_m4s_uri(line)).count();
        assert!(
            segment_count >= 2,
            "expected multiple trick segments on wire, got {segment_count}:\n{body}"
        );
    }
}
