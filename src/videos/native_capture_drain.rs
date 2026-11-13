//! Capacity-one runner polled from live capture threads.

use std::sync::Arc;

use crate::error::A2aLabError;
use crate::videos::capture_still::{captured_frame_jpeg, is_bounded_jpeg};
use crate::videos::{
    CaptureDrainRunner, CaptureRunnerPhase, CapturedColorFrame, ColorCamera, WarmStillCache,
};

use super::session_color_capture::SessionColorCapture;

/// One opened session drained through the latest-frame store and warm still cache.
pub struct NativeCaptureDrain<C: ColorCamera> {
    runner: CaptureDrainRunner<SessionColorCapture<C>>,
    source_id: String,
    max_bytes: u64,
    still_cache: Arc<WarmStillCache>,
}

impl<C: ColorCamera> NativeCaptureDrain<C> {
    /// Wraps an already-open [`SessionColorCapture`].
    #[must_use]
    pub fn new(
        source: SessionColorCapture<C>,
        source_id: &str,
        max_bytes: u64,
        still_cache: Arc<WarmStillCache>,
    ) -> Self {
        Self {
            runner: CaptureDrainRunner::new(source),
            source_id: source_id.to_owned(),
            max_bytes,
            still_cache,
        }
    }

    /// Drains up to `max_pulls`, discards intermediate frames, and returns only the newest.
    pub fn drain_handoffs(&mut self, max_pulls: u32) -> Vec<CapturedColorFrame> {
        let mut newest = None;
        for _ in 0..max_pulls {
            match self.runner.drain_once_handoff() {
                Some(frame) => newest = Some(frame),
                None => break,
            }
        }
        let Some(frame) = newest else {
            return Vec::new();
        };
        self.offer_still(&frame);
        vec![frame]
    }

    /// Pulls frames until the source has no frame ready or `max_pulls` is reached.
    pub fn drain_pending(&mut self, max_pulls: u32) {
        for _ in 0..max_pulls {
            if !self.runner.drain_once() {
                break;
            }
            self.refresh_still();
        }
    }

    /// Newest retained color frame after one drain pass.
    pub fn latest_captured(&mut self) -> Result<CapturedColorFrame, A2aLabError> {
        self.drain_pending(8);
        self.runner
            .peek_latest()
            .cloned()
            .ok_or_else(|| A2aLabError::unavailable("capture has no frame yet"))
    }

    /// Runner phase after the latest drain.
    #[must_use]
    pub fn phase(&self) -> CaptureRunnerPhase {
        self.runner.phase()
    }

    /// Frames waiting for a slow consumer. Never more than one.
    #[must_use]
    pub fn queue_depth(&self) -> usize {
        self.runner.queue_depth()
    }

    /// Fault and replacement counts from the underlying runner.
    #[must_use]
    pub fn counters(&self) -> crate::videos::CaptureDrainCounters {
        self.runner.counters()
    }

    /// Clears stale retention and reopens the underlying session after a source fault.
    pub fn reconnect(&mut self) -> Result<(), A2aLabError> {
        self.still_cache.reset_source(&self.source_id);
        self.runner.reconnect();
        self.runner.source_mut().session_mut().reconnect()?;
        Ok(())
    }

    /// Stops draining, closes the device stream, and invalidates warm still retention.
    pub fn shutdown(&mut self) {
        let _ = self.runner.source_mut().session_mut().stop();
        let _ = self.runner.shutdown();
        self.still_cache.invalidate(&self.source_id);
    }

    fn offer_still(&self, frame: &CapturedColorFrame) {
        if let Ok(jpeg) = captured_frame_jpeg(frame, self.max_bytes)
            && is_bounded_jpeg(&jpeg)
        {
            self.still_cache
                .remember(&self.source_id, frame.identity().sequence(), jpeg);
        }
    }

    fn refresh_still(&mut self) {
        let Some(frame) = self.runner.peek_latest() else {
            return;
        };
        self.offer_still(frame);
    }
}
