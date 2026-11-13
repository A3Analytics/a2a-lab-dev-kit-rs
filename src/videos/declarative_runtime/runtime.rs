//! Composed declarative video runtime and public handles.

use std::collections::HashSet;
use std::sync::Arc;
use std::thread::JoinHandle;

use tokio::runtime::Handle;
use tokio::sync::watch;

use crate::a2a::VideoCaptureSettingsHandle;
use crate::config::{
    CameraSelector, ConfigDuration, DEFAULT_LIVE_OUTPUT_FPS, SaveIntervalDuration,
    VideoCaptureSettings, VideoSourceConfig,
};
use crate::error::A2aLabError;
use crate::memory::MemoryVideos;
#[cfg(feature = "nokhwa")]
use crate::videos::resolve_camera_selector;
use crate::videos::{
    CameraDiscoverySnapshot, CameraEnumerator, DvrVideoMaintenance, DvrVideoProvider,
    SqliteChunkDvrStore, VideoCodec, VideoSource, WarmStillCache,
};

#[cfg(feature = "nokhwa")]
use crate::videos::NokhwaEnumerator;

use super::camera_open::WorkerCamera;
use super::config::VideoRuntimeConfig;
use super::disk_job::spawn_disk_writer;
use super::injections::VideoRuntimeInjections;
use super::pump::spawn_source_pump;
use super::shutdown::VideoRuntimeShutdown;

/// Owning runtime started from declarative configuration and optional injections.
pub struct DeclarativeVideoRuntime {
    provider: DvrVideoProvider,
    settings: VideoCaptureSettingsHandle,
    still_cache: WarmStillCache,
    dvr: SqliteChunkDvrStore,
    shutdown: VideoRuntimeShutdown,
    #[allow(dead_code)]
    maintenance: DvrVideoMaintenance,
    workers: Vec<JoinHandle<()>>,
    #[allow(dead_code)]
    stop: watch::Sender<bool>,
}

impl DeclarativeVideoRuntime {
    /// Opens DVR storage, composes the provider, and starts capture workers.
    pub async fn start(
        config: VideoRuntimeConfig,
        mut injections: VideoRuntimeInjections,
    ) -> Result<Self, A2aLabError> {
        let disk_retention = disk_retention_secs(&config);
        let live_retention = live_retention_secs(&config);
        let output_fps = config.video.fps.unwrap_or(DEFAULT_LIVE_OUTPUT_FPS).max(1);
        let dvr_save_interval_nanos = config
            .video
            .dvr
            .as_ref()
            .and_then(|dvr| dvr.save_interval.map(SaveIntervalDuration::as_nanos));

        let dvr = SqliteChunkDvrStore::open_with_hooks(
            &config.storage_root,
            disk_retention,
            config.hooks,
        )
        .await?;
        if let Some(ready) = &config.dvr_ready {
            ready.on_dvr_opened(&dvr).await?;
        }
        let live = MemoryVideos::new();
        let still_cache = WarmStillCache::new();
        let provider = DvrVideoProvider::open(live, dvr.clone(), config.clip_ttl)?
            .with_warm_stills(still_cache.clone());
        let settings = VideoCaptureSettingsHandle::new(VideoCaptureSettings::from_video_config(
            Some(&config.video),
        ));
        let disk_tx = spawn_disk_writer(dvr.clone());

        let explicit_indexes = config
            .video
            .sources
            .iter()
            .all(|source| matches!(source.camera_id, Some(CameraSelector::Index(_))));
        let snapshot = if explicit_indexes {
            CameraDiscoverySnapshot::from_parts(Vec::new(), Vec::new())
        } else if injections.skips_native_discovery() {
            if let Some(enumerator) = injections.enumerator() {
                enumerator.discover()?
            } else {
                CameraDiscoverySnapshot::from_parts(Vec::new(), Vec::new())
            }
        } else {
            discover_cameras()?
        };
        let mut claimed = HashSet::new();
        let stop = watch::channel(false).0;
        let shutdown = VideoRuntimeShutdown::new(stop.clone());
        let runtime = Handle::current();
        let still_arc = Arc::new(still_cache.clone());
        let mut workers = Vec::new();

        for source in &config.video.sources {
            let source_id = source.id.clone();
            insert_catalog_source(&provider, source, output_fps).await?;
            let worker_camera =
                resolve_source_camera(source, &snapshot, &mut claimed, &mut injections);
            let pump = spawn_source_pump(
                provider.clone(),
                source.clone(),
                source_id,
                output_fps,
                live_retention,
                config.max_frame_bytes,
                dvr_save_interval_nanos,
                disk_tx.clone(),
                Arc::clone(&still_arc),
                settings.clone(),
                stop.subscribe(),
                runtime.clone(),
                worker_camera,
            );
            workers.push(pump.handle);
        }

        let maintenance = provider.maintain_disk();
        Ok(Self {
            provider,
            settings,
            still_cache,
            dvr,
            shutdown,
            maintenance,
            workers,
            stop,
        })
    }

    /// [`VideoProvider`] over live H.264 and SQLite DVR.
    #[must_use]
    pub fn provider(&self) -> DvrVideoProvider {
        self.provider.clone()
    }

    /// Settings handle for [`crate::A2aServer::with_capture_settings`].
    #[must_use]
    pub fn settings_handle(&self) -> VideoCaptureSettingsHandle {
        self.settings.clone()
    }

    /// Warm JPEG still cache consulted before SQLite `frame_at`.
    #[must_use]
    pub fn still_cache(&self) -> &WarmStillCache {
        &self.still_cache
    }

    /// Opened SQLite chunk store and pool for lab-specific tables.
    #[must_use]
    pub fn dvr(&self) -> &SqliteChunkDvrStore {
        &self.dvr
    }

    /// Cooperative stop without dropping the runtime.
    #[must_use]
    pub fn shutdown_handle(&self) -> VideoRuntimeShutdown {
        self.shutdown.clone()
    }
}

impl Drop for DeclarativeVideoRuntime {
    fn drop(&mut self) {
        self.shutdown.stop();
        while let Some(worker) = self.workers.pop() {
            let _ = worker.join();
        }
    }
}

fn disk_retention_secs(config: &VideoRuntimeConfig) -> u64 {
    config
        .video
        .dvr
        .as_ref()
        .and_then(|dvr| dvr.retention.map(ConfigDuration::as_secs))
        .unwrap_or(crate::videos::DEFAULT_DISK_VIDEO_RETENTION_SECS)
}

fn live_retention_secs(config: &VideoRuntimeConfig) -> u64 {
    config
        .video
        .live_retention
        .map_or(120, ConfigDuration::as_secs)
}

fn discover_cameras() -> Result<CameraDiscoverySnapshot, A2aLabError> {
    #[cfg(feature = "nokhwa")]
    {
        NokhwaEnumerator.discover()
    }
    #[cfg(not(feature = "nokhwa"))]
    {
        Ok(CameraDiscoverySnapshot::from_parts(Vec::new(), Vec::new()))
    }
}

fn resolve_source_camera(
    source: &VideoSourceConfig,
    snapshot: &CameraDiscoverySnapshot,
    claimed: &mut HashSet<u32>,
    injections: &mut VideoRuntimeInjections,
) -> WorkerCamera {
    if let Some(camera) = injections.take_camera(source.id.as_str()) {
        return WorkerCamera::Injected(camera);
    }
    #[cfg(feature = "nokhwa")]
    {
        if let Some(CameraSelector::Index(index)) = source.camera_id {
            if claimed.insert(index) {
                return WorkerCamera::NativeIndex(index);
            }
            return WorkerCamera::Absent;
        }
        let Ok(discovered) = resolve_camera_selector(source.camera_id.as_ref(), snapshot, claimed)
        else {
            return WorkerCamera::Absent;
        };
        claimed.insert(discovered.index());
        WorkerCamera::NativeIndex(discovered.index())
    }
    #[cfg(not(feature = "nokhwa"))]
    {
        let _ = (source, snapshot, claimed);
        WorkerCamera::Absent
    }
}

async fn insert_catalog_source(
    provider: &DvrVideoProvider,
    source: &VideoSourceConfig,
    fps: u32,
) -> Result<(), A2aLabError> {
    let width = source.width.unwrap_or(640);
    let height = source.height.unwrap_or(480);
    provider
        .live()
        .insert_source(VideoSource::with_capabilities(
            source.id.clone(),
            source.display_name(),
            source.display_description(),
            None,
            None,
            width,
            height,
            fps,
            "video/mp4",
            vec![VideoCodec::H264],
        )?)
        .await?;
    provider.live().set_live_available(&source.id, false).await
}
