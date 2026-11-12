//! Continuously drain an injected capture source into a capacity-one latest-frame store.
//!
//! The producer keeps pulling even when consumers lag. Each new frame replaces the
//! previous retained buffer so capture memory stays bounded.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::error::A2aLabError;
use crate::videos::{CaptureFormat, CapturedColorFrame, ColorCaptureSource};

/// Sensor interval for a nominal 30 FPS stream.
pub const SENSOR_30FPS_PERIOD_NS: u64 = 33_333_333;

/// Why a retained frame was replaced before a consumer took it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatestFrameReplacement {
    /// A newer drained frame superseded the previous latest frame.
    Superseded,
    /// [`LatestFrameStore::clear`] or [`CaptureDrainRunner::reconnect`] dropped the slot.
    Cleared,
}

/// Identity of one replaced latest frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplacedLatestFrame {
    sequence: u64,
    reason: LatestFrameReplacement,
}

impl ReplacedLatestFrame {
    /// Source sequence copied from the replaced frame.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    /// Why the frame is no longer retained.
    #[must_use]
    pub const fn reason(self) -> LatestFrameReplacement {
        self.reason
    }
}

/// Counts for one continuously draining runner.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureDrainCounters {
    drained: u64,
    replaced: u64,
    no_frame: u64,
    source_errors: u64,
    reconnects: u64,
}

impl CaptureDrainCounters {
    /// Frames pulled from the source and offered to the store.
    #[must_use]
    pub const fn drained(self) -> u64 {
        self.drained
    }

    /// Frames replaced before a consumer took them.
    #[must_use]
    pub const fn replaced(self) -> u64 {
        self.replaced
    }

    /// Pull attempts that returned no frame yet.
    #[must_use]
    pub const fn no_frame(self) -> u64 {
        self.no_frame
    }

    /// Non-recoverable source faults observed while draining.
    #[must_use]
    pub const fn source_errors(self) -> u64 {
        self.source_errors
    }

    /// Times [`CaptureDrainRunner::reconnect`] cleared stale retention.
    #[must_use]
    pub const fn reconnects(self) -> u64 {
        self.reconnects
    }
}

/// Observable runner phase after the latest drain or control call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureRunnerPhase {
    /// Draining is active and the last pull produced a frame.
    Running,
    /// Draining is active but the source has no frame right now.
    WaitingForFrame,
    /// The last pull failed with a source fault until reconnect.
    SourceFaulted,
    /// [`CaptureDrainRunner::shutdown`] closed the runner.
    Shutdown,
}

/// Capacity-one latest-frame buffer with replacement accounting.
#[derive(Debug, Default)]
pub struct LatestFrameStore {
    frame: Option<CapturedColorFrame>,
    replaced: u64,
}

impl LatestFrameStore {
    /// Maximum frames retained for a slow consumer.
    pub const CAPACITY: usize = 1;

    /// Frames waiting for a consumer. Never more than [`Self::CAPACITY`].
    #[must_use]
    pub fn queue_depth(&self) -> usize {
        usize::from(self.frame.is_some())
    }

    /// Payload bytes retained in the latest slot, if any.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.frame.as_ref().map_or(0, retained_frame_bytes)
    }

    /// Frames replaced before a consumer took them.
    #[must_use]
    pub const fn replaced(&self) -> u64 {
        self.replaced
    }

    /// Newest retained frame without removing it.
    #[must_use]
    pub fn peek(&self) -> Option<&CapturedColorFrame> {
        self.frame.as_ref()
    }

    /// Install `frame` as the latest retained buffer.
    pub fn offer(&mut self, frame: CapturedColorFrame) -> Option<ReplacedLatestFrame> {
        let replaced = self.frame.take().map(|previous| ReplacedLatestFrame {
            sequence: previous.identity().sequence(),
            reason: LatestFrameReplacement::Superseded,
        });
        if replaced.is_some() {
            self.replaced = self.replaced.saturating_add(1);
        }
        self.frame = Some(frame);
        replaced
    }

    /// Remove the latest frame for a consumer.
    pub fn take(&mut self) -> Option<CapturedColorFrame> {
        self.frame.take()
    }

    /// Drop any retained frame without counting it as a superseding replacement.
    pub fn clear(&mut self) -> Option<ReplacedLatestFrame> {
        self.frame.take().map(|previous| ReplacedLatestFrame {
            sequence: previous.identity().sequence(),
            reason: LatestFrameReplacement::Cleared,
        })
    }
}

/// Hardware-free runner that drains a [`ColorCaptureSource`] into [`LatestFrameStore`].
pub struct CaptureDrainRunner<S> {
    source: S,
    store: LatestFrameStore,
    phase: CaptureRunnerPhase,
    counters: CaptureDrainCounters,
    shutdown: bool,
}

impl<S: ColorCaptureSource> CaptureDrainRunner<S> {
    /// Wraps one injected source. Draining starts only after [`Self::drain_once`].
    #[must_use]
    pub const fn new(source: S) -> Self {
        Self {
            source,
            store: LatestFrameStore {
                frame: None,
                replaced: 0,
            },
            phase: CaptureRunnerPhase::WaitingForFrame,
            counters: CaptureDrainCounters {
                drained: 0,
                replaced: 0,
                no_frame: 0,
                source_errors: 0,
                reconnects: 0,
            },
            shutdown: false,
        }
    }

    /// Current phase after the latest drain or control call.
    #[must_use]
    pub const fn phase(&self) -> CaptureRunnerPhase {
        self.phase
    }

    /// Replacement and fault counts.
    #[must_use]
    pub const fn counters(&self) -> CaptureDrainCounters {
        self.counters
    }

    /// Latest retained frame without removing it.
    #[must_use]
    pub fn peek_latest(&self) -> Option<&CapturedColorFrame> {
        self.store.peek()
    }

    /// Frames retained for a slow consumer.
    #[must_use]
    pub fn queue_depth(&self) -> usize {
        self.store.queue_depth()
    }

    /// Payload bytes retained in the latest slot.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.store.retained_bytes()
    }

    /// Mutable access to the injected source for reconnect hooks in application code.
    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }

    /// Pull one frame when not shut down. Returns whether a frame was offered.
    pub fn drain_once(&mut self) -> bool {
        self.drain_once_handoff().is_some()
    }

    /// Like [`Self::drain_once`], but returns the offered frame for cadence selection.
    pub fn drain_once_handoff(&mut self) -> Option<CapturedColorFrame> {
        if self.shutdown {
            self.phase = CaptureRunnerPhase::Shutdown;
            return None;
        }
        match self.source.pull_frame() {
            Ok(frame) => {
                if let Some(replaced) = self.store.offer(frame.clone()) {
                    let _ = replaced;
                    self.counters.replaced = self.store.replaced();
                }
                self.counters.drained = self.counters.drained.saturating_add(1);
                self.phase = CaptureRunnerPhase::Running;
                Some(frame)
            }
            Err(error) if is_no_frame(&error) => {
                self.counters.no_frame = self.counters.no_frame.saturating_add(1);
                self.phase = CaptureRunnerPhase::WaitingForFrame;
                None
            }
            Err(_) => {
                self.counters.source_errors = self.counters.source_errors.saturating_add(1);
                self.phase = CaptureRunnerPhase::SourceFaulted;
                None
            }
        }
    }

    /// Drain until `steps` calls complete or the runner shuts down.
    pub fn drain_steps(&mut self, steps: usize) -> usize {
        let mut pulled = 0_usize;
        for _ in 0..steps {
            if self.drain_once() {
                pulled = pulled.saturating_add(1);
            }
        }
        pulled
    }

    /// Remove the newest retained frame for a consumer.
    pub fn take_latest(&mut self) -> Option<CapturedColorFrame> {
        self.store.take()
    }

    /// Stop accepting drained frames. Retained latest frames stay readable.
    pub fn shutdown(&mut self) -> usize {
        self.shutdown = true;
        self.phase = CaptureRunnerPhase::Shutdown;
        self.queue_depth()
    }

    /// Clear stale retention after source recovery without reusing old frames.
    pub fn reconnect(&mut self) -> Option<ReplacedLatestFrame> {
        let cleared = self.store.clear();
        self.counters.reconnects = self.counters.reconnects.saturating_add(1);
        self.shutdown = false;
        self.phase = CaptureRunnerPhase::WaitingForFrame;
        cleared
    }
}

/// Handle for a background drain loop over a shared runner.
pub struct CaptureDrainLoop {
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl CaptureDrainLoop {
    /// Starts a thread that calls [`CaptureDrainRunner::drain_once`] until [`Self::stop`].
    pub fn spawn<S>(runner: Arc<Mutex<CaptureDrainRunner<S>>>) -> Self
    where
        S: ColorCaptureSource + Send + 'static,
    {
        let shutdown = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&shutdown);
        let join = thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                if let Ok(mut locked) = runner.lock() {
                    locked.drain_once();
                }
            }
            if let Ok(mut locked) = runner.lock() {
                let _ = locked.shutdown();
            }
        });
        Self {
            shutdown,
            join: Some(join),
        }
    }

    /// Signals the drain thread to exit and waits for it.
    pub fn stop(mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn is_no_frame(error: &A2aLabError) -> bool {
    matches!(error, A2aLabError::Unavailable { .. })
}

fn retained_frame_bytes(frame: &CapturedColorFrame) -> usize {
    let payload = frame.bytes().len();
    if frame.format() == CaptureFormat::Mjpeg {
        return payload;
    }
    let jpeg = frame.original_jpeg().map_or(0, <[u8]>::len);
    payload.saturating_add(jpeg)
}
