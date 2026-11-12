//! Capacity-one capture drain runner with injected producers.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use a2a_lab_dev_kit::videos::{
    CaptureCapabilities, CaptureClock, CaptureDrainLoop, CaptureDrainRunner, CaptureFormat,
    CaptureIdentity, CaptureMode, CaptureRunnerPhase, CaptureTiming, CapturedColorFrame,
    ColorCaptureSource, ColorMatrix, ColorProperties, ColorRange, LatestFrameReplacement,
    LatestFrameStore, MemoryColorCapture, SENSOR_30FPS_PERIOD_NS,
};
use a2a_lab_dev_kit::{A2aLabError, UtcTimestamp, VideoSourceId};

const JPEG: &[u8] = &[0xFF, 0xD8, 0x01, 0x00, 0xFF, 0xD9];

fn source_id() -> VideoSourceId {
    VideoSourceId::new("cam-runner").expect("source")
}

fn timestamp(second: u32) -> UtcTimestamp {
    UtcTimestamp::parse(&format!("2020-01-01T00:00:{second:02}Z")).expect("timestamp")
}

fn mjpeg_frame(sequence: u64, payload_len: usize) -> CapturedColorFrame {
    let mut jpeg = JPEG.to_vec();
    jpeg.resize(payload_len.max(JPEG.len()), 0xAB);
    CapturedColorFrame::mjpeg(
        CaptureIdentity::new(source_id(), sequence),
        CaptureTiming::new(
            timestamp((sequence % 60) as u32),
            CaptureClock::monotonic(sequence.saturating_mul(SENSOR_30FPS_PERIOD_NS)),
        ),
        64,
        48,
        &jpeg,
        ColorProperties::new(ColorMatrix::Bt709, ColorRange::Limited),
    )
    .expect("frame")
}

fn capabilities() -> CaptureCapabilities {
    CaptureCapabilities::new(vec![
        CaptureMode::new(CaptureFormat::Mjpeg, 64, 48).expect("mode"),
    ])
    .expect("capabilities")
}

#[derive(Debug)]
struct ScriptedSource {
    frames: Vec<Result<CapturedColorFrame, A2aLabError>>,
    index: usize,
}

impl ColorCaptureSource for ScriptedSource {
    fn capabilities(&self) -> &CaptureCapabilities {
        static CAPS: std::sync::OnceLock<CaptureCapabilities> = std::sync::OnceLock::new();
        CAPS.get_or_init(capabilities)
    }

    fn pull_frame(&mut self) -> Result<CapturedColorFrame, A2aLabError> {
        let next = self
            .frames
            .get(self.index)
            .cloned()
            .unwrap_or_else(|| Err(A2aLabError::unavailable("script ended")));
        self.index = self.index.saturating_add(1);
        next
    }
}

#[derive(Debug)]
struct ThirtyFpsSource {
    next: u64,
    payload_len: usize,
    end: u64,
}

impl ThirtyFpsSource {
    const fn new(payload_len: usize, frames: u64) -> Self {
        Self {
            next: 0,
            payload_len,
            end: frames,
        }
    }
}

impl ColorCaptureSource for ThirtyFpsSource {
    fn capabilities(&self) -> &CaptureCapabilities {
        static CAPS: std::sync::OnceLock<CaptureCapabilities> = std::sync::OnceLock::new();
        CAPS.get_or_init(capabilities)
    }

    fn pull_frame(&mut self) -> Result<CapturedColorFrame, A2aLabError> {
        if self.next >= self.end {
            return Err(A2aLabError::unavailable("producer ended"));
        }
        let frame = mjpeg_frame(self.next, self.payload_len);
        self.next = self.next.saturating_add(1);
        Ok(frame)
    }
}

fn retained_bytes(frame: &CapturedColorFrame) -> usize {
    frame.bytes().len()
}

#[test]
fn slow_consumer_sees_only_the_newest_drained_frame() {
    let source = ThirtyFpsSource::new(512, 12);
    let mut runner = CaptureDrainRunner::new(source);
    runner.drain_steps(8);
    assert_eq!(runner.queue_depth(), 1);
    assert_eq!(
        runner.peek_latest().expect("latest").identity().sequence(),
        7
    );

    runner.drain_steps(4);
    let latest = runner.take_latest().expect("take");
    assert_eq!(latest.identity().sequence(), 11);
    assert_eq!(runner.queue_depth(), 0);
    assert!(runner.counters().replaced() >= 10);
}

#[test]
fn replacement_retains_one_buffer_and_accounts_drops() {
    let mut store = LatestFrameStore::default();
    let first = mjpeg_frame(1, 400);
    let second = mjpeg_frame(2, 400);
    let third = mjpeg_frame(3, 400);
    let size = retained_bytes(&first);

    assert!(store.offer(first).is_none());
    assert_eq!(store.queue_depth(), 1);
    assert_eq!(store.retained_bytes(), size);

    let replaced = store.offer(second).expect("replaced");
    assert_eq!(replaced.sequence(), 1);
    assert_eq!(replaced.reason(), LatestFrameReplacement::Superseded);
    assert_eq!(store.queue_depth(), 1);
    assert_eq!(store.retained_bytes(), size);
    assert_eq!(store.replaced(), 1);

    let replaced = store.offer(third).expect("replaced");
    assert_eq!(replaced.sequence(), 2);
    assert_eq!(store.replaced(), 2);

    let taken = store.take().expect("take");
    assert_eq!(taken.identity().sequence(), 3);
    assert_eq!(store.retained_bytes(), 0);
}

#[test]
fn thirty_fps_long_run_keeps_queue_and_bytes_bounded() {
    const FRAMES: u64 = 900;
    const PAYLOAD: usize = 1_024;
    let mut runner = CaptureDrainRunner::new(ThirtyFpsSource::new(PAYLOAD, FRAMES));
    let mut max_depth = 0usize;
    let mut max_bytes = 0usize;
    let mut last_consumer = 0u64;

    let steps = usize::try_from(FRAMES).expect("frames");
    for step in 0..steps {
        runner.drain_once();
        max_depth = max_depth.max(runner.queue_depth());
        max_bytes = max_bytes.max(runner.retained_bytes());
        if step.is_multiple_of(47)
            && let Some(frame) = runner.take_latest()
        {
            last_consumer = frame.identity().sequence();
            assert!(frame.identity().sequence() < FRAMES);
        }
    }

    assert_eq!(max_depth, 1);
    assert!(max_bytes <= PAYLOAD);
    assert_eq!(runner.counters().drained(), FRAMES);
    let latest = runner.peek_latest().expect("latest");
    assert_eq!(latest.identity().sequence(), FRAMES - 1);
    assert!(last_consumer < FRAMES - 1);
}

fn faulted_runner() -> CaptureDrainRunner<ScriptedSource> {
    CaptureDrainRunner::new(ScriptedSource {
        frames: vec![
            Ok(mjpeg_frame(1, 256)),
            Err(A2aLabError::unavailable("no frame yet")),
            Err(A2aLabError::invalid("camera", "disconnected")),
            Ok(mjpeg_frame(2, 256)),
        ],
        index: 0,
    })
}

#[test]
fn no_frame_and_source_fault_states_stay_observable() {
    let mut runner = faulted_runner();
    assert!(runner.drain_once());
    assert_eq!(runner.phase(), CaptureRunnerPhase::Running);
    assert!(!runner.drain_once());
    assert_eq!(runner.phase(), CaptureRunnerPhase::WaitingForFrame);
    assert_eq!(runner.peek_latest().unwrap().identity().sequence(), 1);
    assert!(!runner.drain_once());
    assert_eq!(runner.phase(), CaptureRunnerPhase::SourceFaulted);
    assert_eq!(runner.counters().source_errors(), 1);
}

#[test]
fn reconnect_clears_stale_latest_without_reuse() {
    let mut runner = faulted_runner();
    runner.drain_steps(3);
    let cleared = runner.reconnect().expect("reconnect cleared");
    assert_eq!(cleared.sequence(), 1);
    assert_eq!(cleared.reason(), LatestFrameReplacement::Cleared);
    assert!(runner.peek_latest().is_none());
    assert!(runner.drain_once());
    assert_eq!(runner.peek_latest().unwrap().identity().sequence(), 2);
}

#[test]
fn shutdown_retains_latest_without_further_drains() {
    let mut runner = faulted_runner();
    runner.drain_steps(4);
    assert_eq!(runner.shutdown(), 1);
    assert_eq!(runner.phase(), CaptureRunnerPhase::Shutdown);
    let retained = runner.take_latest().expect("retained through shutdown");
    assert_eq!(retained.identity().sequence(), 2);
    assert!(!runner.drain_once());
}

#[test]
fn background_drain_stops_cleanly_without_deadlock() {
    let frames: Vec<_> = (0..120)
        .map(|sequence| Ok(mjpeg_frame(sequence, 320)))
        .collect();
    let source = ScriptedSource { frames, index: 0 };
    let runner = Arc::new(Mutex::new(CaptureDrainRunner::new(source)));
    let loop_handle = CaptureDrainLoop::spawn(Arc::clone(&runner));

    std::thread::sleep(Duration::from_millis(20));

    let (phase, latest) = {
        let locked = runner.lock().expect("lock");
        (
            locked.phase(),
            locked
                .peek_latest()
                .map(|frame| frame.identity().sequence()),
        )
    };
    assert!(
        matches!(
            phase,
            CaptureRunnerPhase::Running | CaptureRunnerPhase::WaitingForFrame
        ),
        "{phase:?}"
    );
    assert!(latest.is_some());

    loop_handle.stop();

    let mut locked = runner.lock().expect("lock");
    assert_eq!(locked.phase(), CaptureRunnerPhase::Shutdown);
    assert!(!locked.drain_once());
}

#[test]
fn memory_capture_fills_store_without_backlog() {
    let frames: Vec<_> = (0..5).map(|sequence| mjpeg_frame(sequence, 128)).collect();
    let source = MemoryColorCapture::new(capabilities(), frames).expect("memory");
    let mut runner = CaptureDrainRunner::new(source);
    runner.drain_steps(5);
    assert_eq!(runner.queue_depth(), 1);
    assert_eq!(runner.take_latest().unwrap().identity().sequence(), 4);
    assert_eq!(runner.queue_depth(), 0);
    runner.drain_once();
    assert_eq!(runner.phase(), CaptureRunnerPhase::WaitingForFrame);
}
