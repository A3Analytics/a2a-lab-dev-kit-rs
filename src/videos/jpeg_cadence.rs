//! JPEG pump cadence without a color encode pipeline.

use crate::config::VideoCaptureSettings;
use crate::error::A2aLabError;
use crate::id::VideoSourceId;
use crate::time::UtcTimestamp;
use crate::videos::{
    CadenceDispatch, CaptureClock, CaptureIdentity, CaptureTiming, CapturedColorFrame, ColorMatrix,
    ColorProperties, ColorRange, LiveDvrCadenceSelector,
};

const TICK_W: u32 = 2;
const TICK_H: u32 = 2;
const TICK_BYTES: &[u8] = &[16, 128, 16, 128, 16, 128, 16, 128];

/// Independent live and DVR gates for pre-encoded MJPEG capture pumps.
#[derive(Debug)]
pub struct JpegCadenceSelector {
    inner: LiveDvrCadenceSelector,
    source_id: VideoSourceId,
}

impl JpegCadenceSelector {
    /// Opens a selector for one source id and output spacings.
    pub fn new(
        source_id: VideoSourceId,
        live_period_ns: u64,
        dvr_period_ns: u64,
        capacity: usize,
    ) -> Result<Self, A2aLabError> {
        Ok(Self {
            inner: LiveDvrCadenceSelector::new(live_period_ns, dvr_period_ns, capacity)?,
            source_id,
        })
    }

    /// Aligns cadence gates with the active settings snapshot.
    pub fn apply_settings(&mut self, settings: &VideoCaptureSettings) -> Result<(), A2aLabError> {
        self.inner.set_live_period_ns(settings.live_period_ns())?;
        self.inner.set_dvr_period_ns(settings.dvr_period_ns())?;
        Ok(())
    }

    /// Considers one capture tick at `captured_at` with `monotonic_ns`.
    pub fn push_tick(
        &mut self,
        sequence: u64,
        captured_at: UtcTimestamp,
        monotonic_ns: u64,
    ) -> Result<Vec<CadenceDispatch>, A2aLabError> {
        let frame = cadence_tick_frame(&self.source_id, sequence, captured_at, monotonic_ns)?;
        self.inner.push(frame)
    }

    /// Stops accepting ticks while leaving selected frames pullable downstream.
    pub fn shutdown(&mut self) -> Vec<crate::videos::RejectedFrame> {
        self.inner.shutdown()
    }
}

fn cadence_tick_frame(
    source_id: &VideoSourceId,
    sequence: u64,
    captured_at: UtcTimestamp,
    monotonic_ns: u64,
) -> Result<CapturedColorFrame, A2aLabError> {
    CapturedColorFrame::yuy2(
        CaptureIdentity::new(source_id.clone(), sequence),
        CaptureTiming::new(captured_at, CaptureClock::monotonic(monotonic_ns)),
        TICK_W,
        TICK_H,
        4,
        TICK_BYTES,
        ColorProperties::new(ColorMatrix::Bt709, ColorRange::Limited),
        None,
    )
}
