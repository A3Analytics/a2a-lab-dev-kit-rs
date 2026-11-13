//! One capture worker: drain, encode, live append, and optional DVR enqueue.

use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{thread, thread::JoinHandle};

use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};

use crate::a2a::VideoCaptureSettingsHandle;
use crate::config::VideoCaptureSettings;
use crate::config::VideoSourceConfig;
use crate::error::A2aLabError;
use crate::id::VideoSourceId;
use crate::videos::{
    CaptureFormatPreference, CaptureProfileRequest, CaptureRunnerPhase, DvrVideoProvider,
    LiveColorEncodePump, LiveEncodeTick, NativeCaptureDrain, SessionColorCapture, WarmStillCache,
};

use super::camera_open::WorkerCamera;
use super::disk_job::try_queue_h264;
use super::dyn_camera::DynColorCamera;

#[cfg(feature = "nokhwa")]
use crate::videos::NokhwaCamera;

const HANDOFF_BUDGET: u32 = 32;

enum CaptureWorker {
    Injected(NativeCaptureDrain<DynColorCamera>),
    #[cfg(feature = "nokhwa")]
    Native(NativeCaptureDrain<NokhwaCamera>),
}

impl CaptureWorker {
    fn drain_handoffs(&mut self, max_pulls: u32) -> Vec<crate::videos::CapturedColorFrame> {
        match self {
            Self::Injected(drain) => drain.drain_handoffs(max_pulls),
            #[cfg(feature = "nokhwa")]
            Self::Native(drain) => drain.drain_handoffs(max_pulls),
        }
    }

    fn phase(&self) -> CaptureRunnerPhase {
        match self {
            Self::Injected(drain) => drain.phase(),
            #[cfg(feature = "nokhwa")]
            Self::Native(drain) => drain.phase(),
        }
    }

    fn reconnect(&mut self) -> Result<(), A2aLabError> {
        match self {
            Self::Injected(drain) => drain.reconnect(),
            #[cfg(feature = "nokhwa")]
            Self::Native(drain) => drain.reconnect(),
        }
    }

    fn shutdown(&mut self) {
        match self {
            Self::Injected(drain) => drain.shutdown(),
            #[cfg(feature = "nokhwa")]
            Self::Native(drain) => drain.shutdown(),
        }
    }

    fn wait_polling(&mut self, duration: Duration) {
        match self {
            Self::Injected(drain) => drain.wait_polling(duration),
            #[cfg(feature = "nokhwa")]
            Self::Native(drain) => drain.wait_polling(duration),
        }
    }
}

fn frame_budget(fps: u32) -> Duration {
    Duration::from_nanos(1_000_000_000u64 / u64::from(fps.max(1)))
}

pub(crate) struct PumpSpawn {
    pub handle: JoinHandle<()>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_source_pump(
    provider: DvrVideoProvider,
    source: VideoSourceConfig,
    source_id: VideoSourceId,
    output_fps: u32,
    live_retention_secs: u64,
    max_bytes: u64,
    dvr_save_interval_nanos: Option<u64>,
    disk_tx: mpsc::Sender<super::disk_job::DiskJob>,
    still_cache: Arc<WarmStillCache>,
    settings: VideoCaptureSettingsHandle,
    stop: watch::Receiver<bool>,
    runtime: Handle,
    worker_camera: WorkerCamera,
) -> PumpSpawn {
    let handle = thread::spawn(move || {
        if let Err(error) = pump_loop(
            &provider,
            &source,
            &source_id,
            output_fps,
            live_retention_secs,
            max_bytes,
            dvr_save_interval_nanos,
            &disk_tx,
            &still_cache,
            &settings,
            stop,
            &runtime,
            worker_camera,
        ) {
            eprintln!("{}: {error}", source_id.as_str());
        }
    });
    PumpSpawn { handle }
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::cognitive_complexity
)]
fn pump_loop(
    provider: &DvrVideoProvider,
    source: &VideoSourceConfig,
    source_id: &VideoSourceId,
    output_fps: u32,
    live_retention_secs: u64,
    max_bytes: u64,
    dvr_save_interval_nanos: Option<u64>,
    disk_tx: &mpsc::Sender<super::disk_job::DiskJob>,
    still_cache: &Arc<WarmStillCache>,
    settings_handle: &VideoCaptureSettingsHandle,
    mut stop: watch::Receiver<bool>,
    runtime: &Handle,
    worker_camera: WorkerCamera,
) -> Result<(), A2aLabError> {
    let request = source.capture_profile_request(output_fps)?;
    let preference = source.format.capture_preference();
    let budget = frame_budget(output_fps);
    let live_period_ns = 1_000_000_000u64 / u64::from(output_fps.max(1));
    let dvr_period_ns = dvr_save_interval_nanos.unwrap_or(live_period_ns);
    let dvr_frame_period_us = dvr_period_ns / 1_000;

    let live_store =
        provider
            .live_h264()
            .register(source_id, live_retention_secs, output_fps, max_bytes)?;
    let mut color_pump = LiveColorEncodePump::open(
        source_id.clone(),
        live_store,
        output_fps,
        dvr_save_interval_nanos,
    )?;

    let mut driver = match open_worker_drain(
        worker_camera,
        request,
        source_id,
        preference,
        max_bytes,
        still_cache,
    ) {
        Ok(driver) => driver,
        Err(error) => {
            eprintln!("{}: open capture drain: {error}", source_id.as_str());
            None
        }
    };
    let mut settings_rx = settings_handle.subscribe();
    let mut cached_settings = settings_handle.snapshot();
    let stop_wait = stop.clone();

    while !*stop.borrow() {
        let mut tick = LiveEncodeTick::new(&mut stop, &mut settings_rx);
        let started = Instant::now();
        if driver.is_none() {
            let _ = runtime.block_on(provider.live().set_live_available(source_id, false));
            if wait_stop(&stop_wait, Duration::from_millis(200)) {
                break;
            }
            continue;
        }
        let pulled = driver.as_mut().unwrap().drain_handoffs(HANDOFF_BUDGET);
        if pulled.is_empty() {
            match driver.as_mut().unwrap().phase() {
                CaptureRunnerPhase::SourceFaulted => {
                    if driver.as_mut().unwrap().reconnect().is_ok() {
                        color_pump.reconnect();
                    } else {
                        driver.as_mut().unwrap().shutdown();
                        driver = None;
                        color_pump.source_lost();
                    }
                }
                CaptureRunnerPhase::Shutdown => driver = None,
                _ => {}
            }
            let _ = runtime.block_on(provider.live().set_live_available(source_id, false));
            if wait_stop(&stop_wait, Duration::from_millis(50)) {
                break;
            }
            continue;
        }
        sync_settings(&mut color_pump, &mut cached_settings, settings_handle)?;
        let mut live_published = false;
        for frame in pulled {
            let published = color_pump.tick(frame, &mut tick)?;
            for unit in published {
                if unit.destinations.live() {
                    live_published = true;
                }
                if unit.fans_to_disk() {
                    try_queue_h264(
                        disk_tx,
                        source_id,
                        unit.captured_at,
                        dvr_frame_period_us,
                        unit.keyframe,
                        unit.initialization,
                        unit.sample,
                    );
                }
            }
        }
        if live_published {
            let _ = runtime.block_on(provider.live().set_live_available(source_id, true));
        }
        if let Some(rest) = budget.checked_sub(started.elapsed()) {
            driver
                .as_mut()
                .unwrap()
                .wait_polling(rest.min(Duration::from_millis(50)));
        }
        if tick.stopped() || *stop.borrow() {
            break;
        }
    }
    if let Some(active) = driver.as_mut() {
        active.shutdown();
    }
    color_pump.cancel();
    let _ = runtime.block_on(provider.live().set_live_available(source_id, false));
    Ok(())
}

fn open_worker_drain(
    worker_camera: WorkerCamera,
    request: CaptureProfileRequest,
    source_id: &VideoSourceId,
    preference: CaptureFormatPreference,
    max_bytes: u64,
    still_cache: &Arc<WarmStillCache>,
) -> Result<Option<CaptureWorker>, A2aLabError> {
    match worker_camera {
        WorkerCamera::Absent => Ok(None),
        WorkerCamera::Injected(camera) => {
            let session = SessionColorCapture::open(
                DynColorCamera(camera),
                request,
                source_id.clone(),
                preference,
            )?;
            Ok(Some(CaptureWorker::Injected(NativeCaptureDrain::new(
                session,
                source_id.as_str(),
                max_bytes,
                Arc::clone(still_cache),
            ))))
        }
        #[cfg(feature = "nokhwa")]
        WorkerCamera::NativeIndex(index) => {
            let session = SessionColorCapture::open(
                NokhwaCamera::open_index(index)?,
                request,
                source_id.clone(),
                preference,
            )?;
            Ok(Some(CaptureWorker::Native(NativeCaptureDrain::new(
                session,
                source_id.as_str(),
                max_bytes,
                Arc::clone(still_cache),
            ))))
        }
    }
}

fn sync_settings(
    pump: &mut LiveColorEncodePump,
    cached: &mut VideoCaptureSettings,
    handle: &VideoCaptureSettingsHandle,
) -> Result<(), A2aLabError> {
    let snapshot = handle.snapshot();
    if *cached == snapshot {
        return Ok(());
    }
    *cached = snapshot;
    pump.apply_settings(cached)
}

fn wait_stop(stop: &watch::Receiver<bool>, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if *stop.borrow() {
            return true;
        }
        thread::sleep(Duration::from_millis(1));
    }
    *stop.borrow()
}
