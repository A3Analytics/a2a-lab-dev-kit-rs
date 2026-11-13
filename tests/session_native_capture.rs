//! Hardware-free session color capture, native drain, and warm still cache.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use a2a_lab_dev_kit::videos::{
    AdvertisedCaptureProfile, CaptureFormat, CaptureFormatPreference, CaptureMode,
    CaptureProfileRequest, CaptureRate, ColorCamera, ColorCaptureSource, NativeCaptureDrain,
    RawCameraSample, SelectedCaptureProfile, SessionColorCapture, WarmStillCache,
};
use a2a_lab_dev_kit::{A2aLabError, VideoSourceId};

const JPEG: &[u8] = &[0xFF, 0xD8, 0x01, 0x00, 0xFF, 0xD9];

fn source_id() -> VideoSourceId {
    VideoSourceId::new("cam-native").expect("source")
}

fn mjpeg_bytes(marker: u8) -> Vec<u8> {
    let mut jpeg = JPEG.to_vec();
    jpeg[3] = marker;
    jpeg
}

fn profile() -> SelectedCaptureProfile {
    SelectedCaptureProfile::negotiated(
        CaptureMode::new(CaptureFormat::Mjpeg, 4, 4).expect("mode"),
        CaptureRate::fps(30).expect("capture"),
        CaptureRate::fps(5).expect("output"),
    )
}

fn negotiated_profile() -> SelectedCaptureProfile {
    profile()
}

fn advertised() -> Vec<AdvertisedCaptureProfile> {
    vec![AdvertisedCaptureProfile::new(
        CaptureMode::new(CaptureFormat::Mjpeg, 4, 4).expect("mode"),
        CaptureRate::fps(30).expect("rate"),
    )]
}

struct SequenceCamera {
    frames: Vec<Vec<u8>>,
    index: AtomicU64,
    pulls: Arc<AtomicU64>,
}

impl SequenceCamera {
    fn new(frames: Vec<Vec<u8>>) -> (Self, Arc<AtomicU64>) {
        let pulls = Arc::new(AtomicU64::new(0));
        (
            Self {
                frames,
                index: AtomicU64::new(0),
                pulls: Arc::clone(&pulls),
            },
            pulls,
        )
    }
}

impl ColorCamera for SequenceCamera {
    fn advertised(&mut self) -> Result<Vec<AdvertisedCaptureProfile>, A2aLabError> {
        Ok(advertised())
    }

    fn open(
        &mut self,
        profile: &SelectedCaptureProfile,
    ) -> Result<SelectedCaptureProfile, A2aLabError> {
        let _ = profile;
        Ok(negotiated_profile())
    }

    fn raw_frame(&mut self) -> Result<RawCameraSample, A2aLabError> {
        self.pulls.fetch_add(1, Ordering::SeqCst);
        let index = usize::try_from(self.index.fetch_add(1, Ordering::SeqCst)).unwrap_or(0);
        let bytes = self.frames.get(index).cloned().unwrap_or_else(|| {
            self.frames
                .last()
                .cloned()
                .unwrap_or_else(|| mjpeg_bytes(0))
        });
        Ok(RawCameraSample::new(CaptureFormat::Mjpeg, 4, 4, 0, bytes))
    }

    fn stop(&mut self) -> Result<(), A2aLabError> {
        Ok(())
    }
}

struct WrongModeCamera;

impl ColorCamera for WrongModeCamera {
    fn advertised(&mut self) -> Result<Vec<AdvertisedCaptureProfile>, A2aLabError> {
        Ok(advertised())
    }

    fn open(
        &mut self,
        profile: &SelectedCaptureProfile,
    ) -> Result<SelectedCaptureProfile, A2aLabError> {
        let _ = profile;
        Ok(negotiated_profile())
    }

    fn raw_frame(&mut self) -> Result<RawCameraSample, A2aLabError> {
        Ok(RawCameraSample::new(
            CaptureFormat::Yuy2,
            4,
            4,
            8,
            vec![1; 32],
        ))
    }

    fn stop(&mut self) -> Result<(), A2aLabError> {
        Ok(())
    }
}

struct FlakyCamera {
    inner: SequenceCamera,
    failures: Mutex<usize>,
}

impl ColorCamera for FlakyCamera {
    fn advertised(&mut self) -> Result<Vec<AdvertisedCaptureProfile>, A2aLabError> {
        self.inner.advertised()
    }

    fn open(
        &mut self,
        profile: &SelectedCaptureProfile,
    ) -> Result<SelectedCaptureProfile, A2aLabError> {
        self.inner.open(profile)
    }

    fn raw_frame(&mut self) -> Result<RawCameraSample, A2aLabError> {
        let mut left = self.failures.lock().expect("failures");
        if *left > 0 {
            *left -= 1;
            return Err(A2aLabError::unavailable("transient"));
        }
        self.inner.raw_frame()
    }

    fn stop(&mut self) -> Result<(), A2aLabError> {
        self.inner.stop()
    }
}

fn open_session<C: ColorCamera>(
    camera: C,
    preference: CaptureFormatPreference,
) -> SessionColorCapture<C> {
    let request = CaptureProfileRequest::exact(
        preference,
        4,
        4,
        CaptureRate::fps(5).unwrap(),
        CaptureRate::fps(30).unwrap(),
    )
    .unwrap();
    SessionColorCapture::open(camera, request, source_id(), preference).unwrap()
}

#[test]
fn session_capture_converts_raw_samples_into_color_frames() {
    let (camera, pulls) = SequenceCamera::new(vec![mjpeg_bytes(1), mjpeg_bytes(2)]);
    let mut source = open_session(camera, CaptureFormatPreference::Mjpeg);
    let frame = source.pull_frame().expect("frame");
    assert_eq!(frame.format(), CaptureFormat::Mjpeg);
    assert_eq!(frame.identity().sequence(), 0);
    assert_eq!(pulls.load(Ordering::SeqCst), 1);
    let second = source.pull_frame().expect("second");
    assert_eq!(second.identity().sequence(), 1);
}

#[test]
fn session_capture_reports_format_mismatch_without_panic() {
    let source = open_session(WrongModeCamera, CaptureFormatPreference::Mjpeg);
    let mut source = source;
    let error = source.pull_frame().expect_err("mismatch");
    assert!(matches!(
        error,
        A2aLabError::Invalid {
            field: "format",
            ..
        }
    ));
}

#[test]
fn native_drain_updates_warm_still_with_newest_frame() {
    let cache = Arc::new(WarmStillCache::new());
    let (camera, _) = SequenceCamera::new(vec![mjpeg_bytes(1), mjpeg_bytes(2), mjpeg_bytes(3)]);
    let source = open_session(camera, CaptureFormatPreference::Mjpeg);
    let mut drain = NativeCaptureDrain::new(source, "warm", 1024, Arc::clone(&cache));
    drain.drain_handoffs(3);
    let still = cache.latest("warm").expect("remembered");
    assert_eq!(still.jpeg(), mjpeg_bytes(3).as_slice());
    assert_eq!(still.sequence(), 2);
    assert_eq!(drain.queue_depth(), 1);
}

#[test]
fn shutdown_and_invalidation_drop_stale_still() {
    let cache = Arc::new(WarmStillCache::new());
    let (camera, _) = SequenceCamera::new(vec![mjpeg_bytes(9)]);
    let source = open_session(camera, CaptureFormatPreference::Mjpeg);
    let mut drain = NativeCaptureDrain::new(source, "stop", 1024, Arc::clone(&cache));
    drain.drain_pending(1);
    assert!(cache.latest("stop").is_some());
    drain.shutdown();
    assert!(cache.latest("stop").is_none());
    cache.remember("stop", 99, mjpeg_bytes(9));
    assert!(cache.latest("stop").is_none());
}

#[test]
fn reconnect_after_fault_clears_stale_still_and_restores_drain() {
    let cache = Arc::new(WarmStillCache::new());
    let (camera, _) = SequenceCamera::new(vec![mjpeg_bytes(4), mjpeg_bytes(5), mjpeg_bytes(6)]);
    let source = open_session(camera, CaptureFormatPreference::Mjpeg);
    let mut drain = NativeCaptureDrain::new(source, "reconnect", 1024, Arc::clone(&cache));
    drain.drain_handoffs(2);
    assert!(cache.latest("reconnect").is_some());
    drain.reconnect().expect("reconnect");
    assert!(cache.latest("reconnect").is_none());
    drain.drain_handoffs(1);
    assert!(cache.latest("reconnect").is_some());
    assert_eq!(drain.counters().reconnects(), 1);
}

#[test]
fn transient_camera_fault_propagates_without_panic() {
    let (inner, pulls) = SequenceCamera::new(vec![mjpeg_bytes(7)]);
    let mut source = open_session(
        FlakyCamera {
            inner,
            failures: Mutex::new(1),
        },
        CaptureFormatPreference::Mjpeg,
    );
    assert!(matches!(
        source.pull_frame(),
        Err(A2aLabError::Unavailable { .. })
    ));
    assert!(source.pull_frame().is_ok());
    assert_eq!(pulls.load(Ordering::SeqCst), 1);
}

#[test]
fn concurrent_still_reads_see_consistent_newest() {
    let cache = Arc::new(WarmStillCache::new());
    for marker in 0_u8..32 {
        cache.remember("shared", u64::from(marker), mjpeg_bytes(marker));
    }
    let mut handles = Vec::new();
    for _ in 0..8 {
        let cache = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            cache.latest("shared").expect("still")
        }));
    }
    let newest = cache.latest("shared").expect("latest");
    for handle in handles {
        assert_eq!(handle.join().expect("join"), newest);
    }
}

#[test]
fn closed_session_surfaces_source_error_without_panic() {
    let (camera, _) = SequenceCamera::new(vec![mjpeg_bytes(1)]);
    let mut source = open_session(camera, CaptureFormatPreference::Mjpeg);
    source.session_mut().stop().expect("stop");
    let error = source.pull_frame().expect_err("closed");
    assert!(matches!(
        error,
        A2aLabError::Invalid {
            field: "camera",
            ..
        }
    ));
}
