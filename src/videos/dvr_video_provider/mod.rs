//! [`VideoProvider`] over [`H264LiveStore`] and [`SqliteChunkDvrStore`].

mod epoch_playback;
mod frame_at;
mod provider;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::error::A2aLabError;
use crate::id::{DvrTrackId, VideoSourceId};
use crate::memory::MemoryVideos;
use crate::time::{TimeRange, UtcTimestamp};
use crate::videos::{
    EpochHlsSession, H264LiveStore, LiveH264Catalog, SqliteChunkDvrStore, VideoClip, VideoFollow,
    WarmStillCache,
};

pub use provider::DvrVideoMaintenance;

/// Periodic SQLite retention sweep interval.
pub const VIDEO_DVR_CLEANUP_INTERVAL: Duration = Duration::from_hours(1);

#[derive(Clone)]
pub(super) struct StoredClip {
    clip: VideoClip,
    mjpeg_track: Option<DvrTrackId>,
    expires_at: std::time::Instant,
    hls: Option<EpochHlsSession>,
}

#[derive(Clone)]
pub(super) struct StoredFollow {
    follow: VideoFollow,
    source_id: VideoSourceId,
    anchor: Option<UtcTimestamp>,
    expires_at: std::time::Instant,
    hls: Option<EpochHlsSession>,
}

/// Combined live window and SQLite-backed DVR catalog.
#[derive(Clone)]
pub struct DvrVideoProvider {
    pub(super) live: MemoryVideos,
    pub(super) live_h264: LiveH264Catalog,
    pub(super) dvr: SqliteChunkDvrStore,
    pub(super) warm_stills: Option<WarmStillCache>,
    pub(super) clips: Arc<Mutex<Vec<StoredClip>>>,
    pub(super) next_clip: Arc<Mutex<u64>>,
    pub(super) follows: Arc<Mutex<Vec<StoredFollow>>>,
    pub(super) next_follow: Arc<Mutex<u64>>,
    pub(super) clip_ttl: Duration,
}

impl DvrVideoProvider {
    /// Opens a provider over live memory and SQLite DVR.
    pub fn open(
        live: MemoryVideos,
        dvr: SqliteChunkDvrStore,
        clip_ttl: Duration,
    ) -> Result<Self, A2aLabError> {
        Ok(Self {
            live,
            live_h264: LiveH264Catalog::new(),
            dvr,
            warm_stills: None,
            clips: Arc::new(Mutex::new(Vec::new())),
            next_clip: Arc::new(Mutex::new(0)),
            follows: Arc::new(Mutex::new(Vec::new())),
            next_follow: Arc::new(Mutex::new(0)),
            clip_ttl,
        })
    }

    /// Attaches warm JPEG stills consulted before SQLite `frame_at`.
    #[must_use]
    pub fn with_warm_stills(mut self, cache: WarmStillCache) -> Self {
        self.warm_stills = Some(cache);
        self
    }

    #[must_use]
    pub fn live(&self) -> &MemoryVideos {
        &self.live
    }

    #[must_use]
    pub fn live_h264(&self) -> &LiveH264Catalog {
        &self.live_h264
    }

    #[must_use]
    pub fn dvr(&self) -> &SqliteChunkDvrStore {
        &self.dvr
    }

    pub async fn cleanup_disk(&self, now: UtcTimestamp) -> Result<(), A2aLabError> {
        self.dvr.cleanup_at(now).await?;
        self.purge_expired_clips().await;
        Ok(())
    }

    /// Periodic disk cleanup until the returned guard is dropped.
    #[must_use]
    pub fn maintain_disk(&self) -> DvrVideoMaintenance {
        let catalog = self.clone();
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(VIDEO_DVR_CLEANUP_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Err(error) = catalog.cleanup_disk(UtcTimestamp::now()).await {
                    eprintln!("video DVR: {error}");
                }
            }
        });
        DvrVideoMaintenance { task }
    }

    pub(super) async fn purge_expired_clips(&self) {
        let now = std::time::Instant::now();
        self.clips.lock().await.retain(|clip| clip.expires_at > now);
    }

    pub(super) fn live_h264_store(
        &self,
        source_id: &VideoSourceId,
    ) -> Result<Arc<H264LiveStore>, A2aLabError> {
        self.live_h264
            .get(source_id)
            .map(|store| Arc::clone(store.inner()))
            .ok_or_else(|| {
                A2aLabError::live_unavailable("in-memory H.264 live store is not registered")
            })
    }

    pub(super) async fn h264_live_ready(&self, source_id: &VideoSourceId) -> bool {
        let Ok(true) = self.live.live_available(source_id).await else {
            return false;
        };
        let Some(store) = self.live_h264.get(source_id) else {
            return false;
        };
        store
            .inner()
            .snapshot()
            .is_ok_and(|snapshot| !snapshot.samples().is_empty())
    }
}

pub(super) fn newest_inside(
    interval: TimeRange,
    live_newest: Option<UtcTimestamp>,
) -> UtcTimestamp {
    live_newest
        .filter(|at| interval.contains(*at))
        .unwrap_or_else(|| interval.end().saturating_add_micros(-1))
}

pub(super) fn refreshes_live_playlist(path: &str) -> bool {
    path == crate::videos::HLS_PLAYLIST_RELATIVE_PATH
        || path == crate::videos::HLS_TRICK_8X_PLAYLIST_RELATIVE_PATH
}
