//! H.264 sliding live store bench for browser regression tests.

#![allow(dead_code)]

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use a2a_lab_dev_kit::{
    A2aLabError, A2aLabService, A2aServer, DvrPictureProperties, DvrTrackId, FollowVideoRequest,
    GetVideoClipRequest, GetVideoFrameAtRequest, GetVideoFrameAtResult, GetVideoTimelineRequest,
    H264LiveStore, H264LiveStoreLimits, HLS_PLAYLIST_RELATIVE_PATH,
    HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH, HttpAuthSecurityScheme, ListVideoSourcesRequest,
    MemoryLogs, MemoryMetrics, MemoryTasks, Page, ResolvedMediaAsset, SecurityScheme, UtcTimestamp,
    VideoClipId, VideoCodec, VideoFollow, VideoFollowId, VideoPlayback8xMetadata, VideoProvider,
    VideoSource, VideoSourceId, VideoTimeline, bind_local, live_h264_edge_anchor,
    load_live_hls_asset, package_h264_live_at,
};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

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

const TRACK: &str = "h264";
pub const BENCH_SOURCE: &str = "store-live";
const TOKEN: &str = "h264-live-bearer";
pub const BEARER_TOKEN: &str = TOKEN;
const PLAYBACK_WINDOW_US: u64 = 30_000_000;
const SAMPLE_SPACING_US: u64 = 300_000;
const ORIGIN: &str = "2024-06-01T00:00:00Z";

fn track() -> DvrTrackId {
    DvrTrackId::new(TRACK).expect("track")
}

fn picture() -> DvrPictureProperties {
    DvrPictureProperties::new(640, 480).expect("picture")
}

fn origin() -> UtcTimestamp {
    UtcTimestamp::parse(ORIGIN).expect("origin")
}

pub struct LiveStoreLab {
    pub base: String,
    pub store: Arc<H264LiveStore>,
    pub videos: LiveStoreVideos,
    feeder: JoinHandle<()>,
}

impl Drop for LiveStoreLab {
    fn drop(&mut self) {
        self.feeder.abort();
    }
}

struct PreparedFollow {
    follow: VideoFollow,
    timeline_origin_us: Option<i64>,
    expires_at: Instant,
}

/// Live follow backed by [`H264LiveStore`] epoch HLS packaging.
#[derive(Clone)]
pub struct LiveStoreVideos {
    inner: Arc<LiveStoreVideosInner>,
}

struct LiveStoreVideosInner {
    store: Arc<H264LiveStore>,
    source: VideoSource,
    sessions: Mutex<HashMap<VideoFollowId, PreparedFollow>>,
    next_follow: Mutex<u64>,
}

fn playback_8x_from_session(session: &a2a_lab_dev_kit::EpochHlsSession) -> VideoPlayback8xMetadata {
    if session.trick_playlist().is_some() {
        VideoPlayback8xMetadata::available(
            HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH,
            HLS_PLAYLIST_RELATIVE_PATH,
        )
    } else {
        VideoPlayback8xMetadata::unavailable(
            HLS_PLAYLIST_RELATIVE_PATH,
            "insufficient keyframes for trick play",
        )
    }
}

impl LiveStoreVideos {
    pub fn new(store: Arc<H264LiveStore>) -> Result<Self, A2aLabError> {
        let source = VideoSource::with_capabilities(
            VideoSourceId::new(BENCH_SOURCE)?,
            "Store live",
            "Sliding H.264 live store",
            None,
            None,
            640,
            480,
            30,
            "video/mp4",
            vec![VideoCodec::H264],
        )?;
        Ok(Self {
            inner: Arc::new(LiveStoreVideosInner {
                store,
                source,
                sessions: Mutex::new(HashMap::new()),
                next_follow: Mutex::new(0),
            }),
        })
    }

    async fn purge_expired(&self) {
        let now = Instant::now();
        let mut sessions = self.inner.sessions.lock().await;
        sessions.retain(|_, session| session.expires_at > now);
    }
}

impl VideoProvider for LiveStoreVideos {
    fn list_video_sources(
        &self,
        _request: ListVideoSourcesRequest,
    ) -> impl Future<Output = Result<Page<VideoSource>, A2aLabError>> + Send {
        let source = self.inner.source.clone();
        std::future::ready(Ok(Page::new(vec![source], None)))
    }

    fn get_video_frame_at(
        &self,
        _request: GetVideoFrameAtRequest,
    ) -> impl Future<Output = Result<GetVideoFrameAtResult, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::no_recording(
            "live store retains H.264 only",
        )))
    }

    fn get_video_clip(
        &self,
        _request: GetVideoClipRequest,
    ) -> impl Future<Output = Result<a2a_lab_dev_kit::VideoClip, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::no_recording(
            "live store has no finite clips",
        )))
    }

    fn open_video_clip(
        &self,
        id: &VideoClipId,
    ) -> impl Future<Output = Result<a2a_lab_dev_kit::VideoClipStream, A2aLabError>> + Send {
        let id = id.to_string();
        std::future::ready(Err(A2aLabError::not_found("video clip", id)))
    }

    fn get_video_timeline(
        &self,
        request: GetVideoTimelineRequest,
    ) -> impl Future<Output = Result<VideoTimeline, A2aLabError>> + Send {
        if request.source_id().as_str() != BENCH_SOURCE {
            return std::future::ready(Err(A2aLabError::not_found(
                "video source",
                request.source_id().to_string(),
            )));
        }
        let source_id = request.source_id().clone();
        std::future::ready(VideoTimeline::new(source_id, vec![], None, true))
    }

    fn follow_video(
        &self,
        request: FollowVideoRequest,
    ) -> impl Future<Output = Result<VideoFollow, A2aLabError>> + Send {
        let videos = self.clone();
        async move {
            if request.source_id().as_str() != BENCH_SOURCE {
                return Err(A2aLabError::not_found(
                    "video source",
                    request.source_id().to_string(),
                ));
            }
            videos
                .inner
                .store
                .snapshot()?
                .samples()
                .last()
                .ok_or_else(|| {
                    A2aLabError::live_unavailable("live follow has no retained samples")
                })?;
            videos.purge_expired().await;
            let mut next = videos.inner.next_follow.lock().await;
            *next += 1;
            let id = VideoFollowId::new(format!("follow-{next}"))?;
            let snapshot = videos.inner.store.snapshot()?;
            let anchor = live_h264_edge_anchor(&snapshot)?;
            let packaged = package_h264_live_at(&snapshot, anchor, PLAYBACK_WINDOW_US)?;
            let follow = VideoFollow::new_encoded(
                id.clone(),
                request.source_id().clone(),
                VideoCodec::H264,
            )?
            .with_playback_8x(playback_8x_from_session(&packaged));
            let session = PreparedFollow {
                follow: follow.clone(),
                timeline_origin_us: None,
                expires_at: Instant::now() + Duration::from_secs(300),
            };
            videos.inner.sessions.lock().await.insert(id, session);
            Ok(follow)
        }
    }

    fn open_video_follow(
        &self,
        _id: &VideoFollowId,
    ) -> impl Future<Output = Result<a2a_lab_dev_kit::VideoLiveStream, A2aLabError>> + Send {
        std::future::ready(Err(A2aLabError::invalid(
            "id",
            "encoded live sessions use packaged HLS assets",
        )))
    }

    fn resolve_live_playback_asset(
        &self,
        id: &VideoFollowId,
        relative_path: Option<&str>,
    ) -> impl Future<Output = Result<ResolvedMediaAsset, A2aLabError>> + Send {
        let videos = self.clone();
        let id = id.clone();
        let relative_path = relative_path.map(str::to_owned);
        async move {
            videos.purge_expired().await;
            let mut sessions = videos.inner.sessions.lock().await;
            let session = sessions
                .get_mut(&id)
                .ok_or_else(|| A2aLabError::not_found("video follow", id.to_string()))?;
            if session.expires_at <= Instant::now() {
                sessions.remove(&id);
                return Err(A2aLabError::clip_expired(format!(
                    "live session `{id}` has expired"
                )));
            }
            session.expires_at = Instant::now() + Duration::from_secs(300);
            let timeline_origin_us = session.timeline_origin_us;
            drop(sessions);

            let path = match relative_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                None => HLS_PLAYLIST_RELATIVE_PATH,
                Some(candidate) if candidate.contains("..") || candidate.starts_with('/') => {
                    return Err(A2aLabError::invalid(
                        "path",
                        "must be a session-relative asset path",
                    ));
                }
                Some(candidate) => candidate,
            };

            let snapshot = videos.inner.store.snapshot()?;
            let anchor = live_h264_edge_anchor(&snapshot)?;
            let mut packaged = package_h264_live_at(&snapshot, anchor, PLAYBACK_WINDOW_US)?;
            if let Some(origin_us) = timeline_origin_us {
                packaged.pin_timeline_origin(origin_us);
            }
            let asset = load_live_hls_asset(&snapshot, &packaged, path)?;

            let playback_8x = playback_8x_from_session(&packaged);
            let mut sessions = videos.inner.sessions.lock().await;
            if let Some(session) = sessions.get_mut(&id) {
                session.timeline_origin_us = Some(packaged.timeline_origin_us());
                session.follow = session.follow.clone().with_playback_8x(playback_8x);
            }
            Ok(ResolvedMediaAsset::Bytes(asset))
        }
    }
}

fn seed_store(store: &H264LiveStore, count: u32) -> Result<(), A2aLabError> {
    store.open_epoch(track(), H264_INIT.to_vec(), picture())?;
    let base = origin();
    for step in 0..count {
        store.append_sample(
            &track(),
            base.saturating_add_micros(i64::from(step) * i64::try_from(SAMPLE_SPACING_US).unwrap()),
            h264_sample(step.is_multiple_of(8)),
            true,
        )?;
    }
    Ok(())
}

pub fn spawn_feeder(store: Arc<H264LiveStore>, start: UtcTimestamp) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut at = start;
        let mut tick = 0u32;
        let mut interval = tokio::time::interval(Duration::from_millis(SAMPLE_SPACING_US / 1000));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            tick += 1;
            at = at.saturating_add_micros(i64::try_from(SAMPLE_SPACING_US).unwrap());
            let _ = store.append_sample(&track(), at, h264_sample(tick.is_multiple_of(8)), true);
        }
    })
}

pub async fn live_store_lab(initial_samples: u32) -> Result<LiveStoreLab, A2aLabError> {
    let store = Arc::new(H264LiveStore::new(H264LiveStoreLimits::new(
        64,
        4_000_000,
        120_000_000,
    )?));
    seed_store(&store, initial_samples)?;
    let last = origin().saturating_add_micros(
        i64::from(initial_samples.saturating_sub(1)) * i64::try_from(SAMPLE_SPACING_US).unwrap(),
    );
    let feeder = spawn_feeder(store.clone(), last);
    let videos = LiveStoreVideos::new(store.clone())?;
    let service = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_videos(videos.clone())
        .share();
    let (listener, address) = bind_local().await?;
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
        server.listen(listener).await.expect("listen");
    });
    Ok(LiveStoreLab {
        base: format!("http://{address}"),
        store,
        videos,
        feeder,
    })
}

#[cfg(test)]
mod tests {
    use a2a_lab_dev_kit::{
        FollowVideoRequest, HLS_PLAYLIST_RELATIVE_PATH, ResolvedMediaAsset, VideoProvider,
    };

    use super::*;

    #[tokio::test]
    async fn provider_refresh_slides_live_manifest() {
        let lab = live_store_lab(12).await.expect("lab");
        let videos = lab.videos.clone();
        let follow = videos
            .follow_video(FollowVideoRequest::new(
                VideoSourceId::new(BENCH_SOURCE).unwrap(),
            ))
            .await
            .expect("follow");
        let first = match videos
            .resolve_live_playback_asset(follow.id(), Some(HLS_PLAYLIST_RELATIVE_PATH))
            .await
            .expect("playlist")
        {
            ResolvedMediaAsset::Bytes(asset) => String::from_utf8_lossy(asset.bytes()).to_string(),
            _ => panic!("expected bytes"),
        };
        tokio::time::sleep(Duration::from_millis(500)).await;
        let second = match videos
            .resolve_live_playback_asset(follow.id(), Some(HLS_PLAYLIST_RELATIVE_PATH))
            .await
            .expect("refresh")
        {
            ResolvedMediaAsset::Bytes(asset) => String::from_utf8_lossy(asset.bytes()).to_string(),
            _ => panic!("expected bytes"),
        };
        assert!(first.contains("#EXTM3U"));
        assert!(!first.contains("#EXT-X-ENDLIST"));
        assert_ne!(first, second, "sliding store should advance the playlist");
    }

    #[tokio::test]
    async fn gateway_serves_authorized_live_manifest() {
        let lab = live_store_lab(12).await.expect("lab");
        let follow = lab
            .videos
            .follow_video(FollowVideoRequest::new(
                VideoSourceId::new(BENCH_SOURCE).unwrap(),
            ))
            .await
            .expect("follow");
        let url = format!("{}{}{HLS_PLAYLIST_RELATIVE_PATH}", lab.base, follow.path());
        let body = reqwest::Client::new()
            .get(url)
            .header("Authorization", format!("Bearer {BEARER_TOKEN}"))
            .send()
            .await
            .expect("manifest")
            .text()
            .await
            .expect("playlist");
        assert!(body.contains("#EXTM3U"));
        assert!(!body.contains("#EXT-X-ENDLIST"));
    }
}
