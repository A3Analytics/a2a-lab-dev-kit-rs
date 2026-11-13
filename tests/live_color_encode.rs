//! Live color encode pump, catalog, cadence, and settings watch behavior.

#![cfg(all(feature = "yuv", feature = "openh264"))]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use a2a_lab_dev_kit::config::LoadedLabConfig;
use a2a_lab_dev_kit::config::{VideoCaptureSettings, VideoCaptureSettingsWatch};
use a2a_lab_dev_kit::videos::{
    CadenceDestinations, CadenceDispatch, CaptureClock, CaptureIdentity, CaptureTiming,
    CapturedColorFrame, ColorMatrix, ColorProperties, ColorRange, DvrEnqueueStatus,
    DvrPersistenceQueue, JpegCadenceSelector, LiveColorEncodePump, LiveEncodeTick, LiveH264Catalog,
    OUTPUT_5FPS_PERIOD_NS,
};
use a2a_lab_dev_kit::{UtcTimestamp, VideoSourceId};
use tokio::sync::watch;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{EnvFilter, Layer};

const SENSOR_PERIOD: u64 = 33_333_333;
const TICK_BYTES: &[u8] = &[16, 128, 16, 128, 16, 128, 16, 128];

fn source_id() -> VideoSourceId {
    VideoSourceId::new("mac-built-in").expect("source")
}

fn frame(sequence: u64, monotonic_ns: u64) -> CapturedColorFrame {
    let mut packed = Vec::with_capacity(16 * 16 * 2);
    for _ in 0..(16 * 16 / 2) {
        packed.extend_from_slice(TICK_BYTES);
    }
    CapturedColorFrame::yuy2(
        CaptureIdentity::new(source_id(), sequence),
        CaptureTiming::new(
            UtcTimestamp::parse("2024-06-01T00:00:00Z").expect("utc"),
            CaptureClock::monotonic(monotonic_ns),
        ),
        16,
        16,
        32,
        &packed,
        ColorProperties::new(ColorMatrix::Bt709, ColorRange::Limited),
        None,
    )
    .expect("frame")
}

fn loaded_settings() -> VideoCaptureSettings {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/a2a-lab-v2-semantic-valid.json");
    let loaded = LoadedLabConfig::load(&path).expect("config");
    VideoCaptureSettings::from_video_config(loaded.config.videos.as_ref())
}

fn open_pump(
    fps: u32,
    dvr_save_interval_nanos: Option<u64>,
) -> (LiveColorEncodePump, Arc<a2a_lab_dev_kit::H264LiveStore>) {
    let catalog = LiveH264Catalog::new();
    let live = catalog
        .register(&source_id(), 60, fps, 512 * 1024)
        .expect("register");
    let store = Arc::clone(live.inner());
    let pump =
        LiveColorEncodePump::open(source_id(), live, fps, dvr_save_interval_nanos).expect("pump");
    (pump, store)
}

fn feed_sensor(
    pump: &mut LiveColorEncodePump,
    count: u64,
) -> Vec<a2a_lab_dev_kit::videos::PublishedH264> {
    let mut out = Vec::new();
    for sequence in 0..count {
        let mono = sequence.saturating_mul(SENSOR_PERIOD);
        out.extend(pump.push_frame(frame(sequence, mono)).expect("push"));
    }
    out
}

#[test]
fn live_only_encode_publishes_without_dvr() {
    let live_period = OUTPUT_5FPS_PERIOD_NS;
    let dvr_period = live_period.saturating_mul(10_000);
    let mut selector =
        a2a_lab_dev_kit::videos::LiveDvrCadenceSelector::new(live_period, dvr_period, 8)
            .expect("selector");
    let mut live = 0_u32;
    let mut dvr = 0_u32;
    for sequence in 0..180_u64 {
        let dispatches = selector
            .push(frame(sequence, sequence.saturating_mul(SENSOR_PERIOD)))
            .expect("push");
        for dispatch in dispatches {
            if dispatch.destinations().live() {
                live += 1;
            }
            if dispatch.destinations().dvr() {
                dvr += 1;
            }
        }
    }
    assert!(live > 0);
    assert!(
        dvr <= 1,
        "dvr gate should stay sparse over a short sensor timeline, dvr={dvr}"
    );
    let (mut pump, store) = open_pump(5, Some(dvr_period));
    let published = feed_sensor(&mut pump, 60);
    assert!(published.iter().any(|item| item.destinations.live()));
    assert!(!store.snapshot().expect("snapshot").samples().is_empty());
}

#[test]
fn jpeg_only_cadence_emits_fresh_frames() {
    let dvr_period = OUTPUT_5FPS_PERIOD_NS * 50;
    let mut selector = JpegCadenceSelector::new(source_id(), OUTPUT_5FPS_PERIOD_NS, dvr_period, 8)
        .expect("selector");
    let mut sequences = Vec::new();
    for sequence in 0..200 {
        let dispatches = selector
            .push_tick(
                sequence,
                UtcTimestamp::parse("2024-06-01T00:00:00Z").expect("utc"),
                sequence.saturating_mul(SENSOR_PERIOD),
            )
            .expect("tick");
        for dispatch in dispatches {
            sequences.push(dispatch.frame().identity().sequence());
        }
    }
    assert!(sequences.len() > 5);
    assert_eq!(
        sequences.len(),
        sequences.iter().copied().collect::<HashSet<_>>().len()
    );
}

#[test]
fn shared_encode_fans_out_once() {
    let (mut pump, _) = open_pump(5, Some(OUTPUT_5FPS_PERIOD_NS));
    let published = feed_sensor(&mut pump, 180);
    let shared = published
        .iter()
        .filter(|item| item.destinations.live() && item.destinations.dvr())
        .count();
    assert!(shared > 0, "expected at least one shared encode");
    let live_only = published
        .iter()
        .filter(|item| item.destinations == CadenceDestinations::live_only())
        .count();
    assert_eq!(
        live_only, 0,
        "same cadence should not split live-only encodes"
    );
}

#[test]
fn epoch_changes_when_initialization_changes() {
    let (mut pump, store) = open_pump(5, None);
    let _ = feed_sensor(&mut pump, 40);
    let first_epochs = store.snapshot().expect("snapshot").epochs().len();
    assert_eq!(first_epochs, 1);
    pump.reconnect();
    let _ = feed_sensor(&mut pump, 40);
    let snapshot = store.snapshot().expect("snapshot");
    assert!(!snapshot.epochs().is_empty());
    assert!(!snapshot.samples().is_empty());
}

#[test]
fn settings_watch_adjusts_live_cadence_without_restart() {
    let settings = loaded_settings();
    let watch = VideoCaptureSettingsWatch::new(settings);
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let mut settings_rx = watch.subscribe();
    let catalog = LiveH264Catalog::new();
    let live = catalog
        .register(&source_id(), 60, 5, 256 * 1024)
        .expect("register");
    let mut pump = LiveColorEncodePump::open(source_id(), live, 5, None).expect("pump");

    for sequence in 0..20_u64 {
        let mut control = LiveEncodeTick::new(&mut stop_rx, &mut settings_rx);
        let mono = sequence.saturating_mul(SENSOR_PERIOD);
        let _ = pump
            .tick(frame(sequence, mono), &mut control)
            .expect("tick");
    }

    watch
        .apply_patch(&serde_json::json!({ "videos": { "fps": 10 } }))
        .expect("patch");
    assert!(settings_rx.has_changed().is_ok());
    let updated = watch.snapshot();
    assert_eq!(updated.live_output_fps(), 10);
    assert_eq!(updated.live_period_ns(), 100_000_000);

    let mut control = LiveEncodeTick::new(&mut stop_rx, &mut settings_rx);
    let published = pump
        .tick(frame(40, 40 * SENSOR_PERIOD), &mut control)
        .expect("tick after settings");
    assert!(
        !published.is_empty(),
        "pump should keep encoding after a watch update"
    );

    let _ = stop_tx.send(true);
    let mut control = LiveEncodeTick::new(&mut stop_rx, &mut settings_rx);
    let shutdown = pump
        .tick(frame(999, 9_999_999_999), &mut control)
        .expect("stop");
    assert!(shutdown.is_empty());
    let _ = stop_rx;
}

#[test]
fn jpeg_cadence_honors_save_interval_update() {
    let settings = loaded_settings().with_negotiated_sensor_period_ns(SENSOR_PERIOD);
    let watch = VideoCaptureSettingsWatch::new(settings);
    let mut settings_rx = watch.subscribe();
    let mut selector = JpegCadenceSelector::new(
        source_id(),
        watch.snapshot().live_period_ns(),
        watch.snapshot().dvr_period_ns(),
        8,
    )
    .expect("selector");

    watch
        .apply_patch(&serde_json::json!({
            "videos": {
                "dvr": { "save_interval": "2s" }
            }
        }))
        .expect("patch");
    assert!(settings_rx.has_changed().is_ok());
    let snapshot = settings_rx.borrow_and_update().clone();
    selector.apply_settings(&snapshot).expect("apply");

    let mut dvr = 0_u32;
    for sequence in 0..240_u64 {
        let dispatches = selector
            .push_tick(
                sequence,
                UtcTimestamp::parse("2024-06-01T00:00:00Z").expect("utc"),
                sequence.saturating_mul(SENSOR_PERIOD),
            )
            .expect("tick");
        for dispatch in dispatches {
            if matches!(dispatch, CadenceDispatch::Dvr(_)) {
                dvr += 1;
            }
        }
    }
    assert!(dvr <= 3, "2s DVR cadence should stay sparse, dvr={dvr}");
}

#[test]
fn slow_encode_emits_structured_warning() {
    let (capture, dispatch) = install_warn_capture();
    tracing::dispatcher::with_default(&dispatch, || {
        let (mut pump, _) = open_pump(5, None);
        let _ = pump.push_frame(frame(0, 0)).expect("origin");
        pump.inject_encode_elapsed_ns(OUTPUT_5FPS_PERIOD_NS);
        let _ = pump
            .push_frame(frame(1, OUTPUT_5FPS_PERIOD_NS))
            .expect("slow");
    });
    let text = warn_text(&capture);
    assert!(text.contains("slow color encode may miss live output cadence"));
    assert!(text.contains(&format!("\"elapsed_ns\":{OUTPUT_5FPS_PERIOD_NS}")));
}

#[test]
fn live_append_survives_full_dvr_queue() {
    let (mut pump, store) = open_pump(5, Some(OUTPUT_5FPS_PERIOD_NS));
    let queue = DvrPersistenceQueue::new(1).expect("queue");
    let published = feed_sensor(&mut pump, 60);
    assert!(!published.is_empty());
    let live_before = store.snapshot().expect("snapshot").samples().len();
    assert!(live_before > 0);
    let dvr_sample = published
        .iter()
        .find(|item| item.fans_to_disk())
        .expect("shared cadence sample");
    assert_eq!(
        queue
            .try_enqueue(dvr_sample.captured_at, dvr_sample.sample.clone())
            .expect("enqueue"),
        DvrEnqueueStatus::Accepted
    );
    assert_eq!(
        queue
            .try_enqueue(dvr_sample.captured_at, vec![0xFF])
            .expect("enqueue"),
        DvrEnqueueStatus::Full
    );
    let _ = feed_sensor(&mut pump, 20);
    assert!(
        store.snapshot().expect("snapshot").samples().len() > live_before,
        "live append must not wait on a saturated DVR queue"
    );
}

#[test]
fn clean_shutdown_rejects_late_frames() {
    let (mut pump, store) = open_pump(5, None);
    let _ = feed_sensor(&mut pump, 10);
    let before = store.snapshot().expect("snapshot").samples().len();
    pump.cancel();
    let published = pump
        .push_frame(frame(99, 99 * SENSOR_PERIOD))
        .expect("late");
    assert!(published.is_empty());
    assert_eq!(store.snapshot().expect("snapshot").samples().len(), before);
}

fn install_warn_capture() -> (Arc<Mutex<Vec<u8>>>, tracing::Dispatch) {
    let capture = Arc::new(Mutex::new(Vec::new()));
    let writer = Arc::clone(&capture);
    let filter = EnvFilter::new("warn");
    let layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(move || WarnWriter::new(Arc::clone(&writer)))
        .with_filter(filter);
    let dispatch = tracing_subscriber::registry().with(layer).into();
    (capture, dispatch)
}

fn warn_text(capture: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8(capture.lock().expect("lock").clone()).expect("utf8")
}

struct WarnWriter {
    sink: Arc<Mutex<Vec<u8>>>,
}

impl WarnWriter {
    fn new(sink: Arc<Mutex<Vec<u8>>>) -> Self {
        Self { sink }
    }
}

impl std::io::Write for WarnWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sink.lock().expect("lock").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
