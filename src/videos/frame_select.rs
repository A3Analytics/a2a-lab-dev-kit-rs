//! Choose real captured frames at an output rate.
//!
//! Sensor time and the output period are separate. Selection uses the capture
//! monotonic clock and integer multiples of the output period from the epoch
//! origin, so a rate that does not divide the sensor interval does not drift.
//! A long gap stays a gap: no timestamp is invented and no frame is repeated.
//! Rejected frames are identified here, before a caller decodes JPEG or encodes
//! H.264. This module does not convert pixels and does not write a recording.

use std::collections::VecDeque;

use crate::error::A2aLabError;
use crate::videos::CapturedColorFrame;

/// One output frame every 200 ms.
pub const OUTPUT_5FPS_PERIOD_NS: u64 = 200_000_000;

/// Why a captured frame was not left available for decode or encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameRejection {
    /// A newer frame arrived before the output deadline.
    Superseded,
    /// The ready queue was full, so the oldest selected frame was dropped.
    DroppedOldest,
    /// The capture clock identity changed or its monotonic sample moved backward.
    ClockReset,
    /// The gate is shut and does not accept another frame.
    Shutdown,
}

/// Identity of a frame the gate will not hand to a decoder or encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RejectedFrame {
    sequence: u64,
    reason: FrameRejection,
}

impl RejectedFrame {
    /// Source sequence copied from the captured frame.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    /// Why the frame is not selected.
    #[must_use]
    pub const fn reason(self) -> FrameRejection {
        self.reason
    }
}

/// Bounded selector from capture time to the output rate.
#[derive(Debug)]
pub struct FreshFrameGate {
    period_ns: u64,
    capacity: usize,
    epoch: Option<Epoch>,
    held: Option<CapturedColorFrame>,
    ready: VecDeque<CapturedColorFrame>,
    stale: u64,
    dropped: u64,
    closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Epoch {
    clock: Option<String>,
    origin_ns: u64,
    due_ns: u64,
    last_ns: u64,
}

impl FreshFrameGate {
    /// `period_ns` is the output spacing. `capacity` is the maximum selected
    /// frames kept for a slow consumer. The newest selected frames are kept.
    pub fn new(period_ns: u64, capacity: usize) -> Result<Self, A2aLabError> {
        if period_ns == 0 {
            return Err(A2aLabError::invalid(
                "period",
                "output period must be positive",
            ));
        }
        if capacity == 0 {
            return Err(A2aLabError::invalid(
                "capacity",
                "ready queue capacity must be positive",
            ));
        }
        Ok(Self {
            period_ns,
            capacity,
            epoch: None,
            held: None,
            ready: VecDeque::new(),
            stale: 0,
            dropped: 0,
            closed: false,
        })
    }

    /// Gate whose output period is [`OUTPUT_5FPS_PERIOD_NS`].
    pub fn at_5fps(capacity: usize) -> Result<Self, A2aLabError> {
        Self::new(OUTPUT_5FPS_PERIOD_NS, capacity)
    }

    /// Output period in nanoseconds.
    #[must_use]
    pub const fn period_ns(&self) -> u64 {
        self.period_ns
    }

    /// Frames replaced before their output deadline.
    #[must_use]
    pub const fn stale(&self) -> u64 {
        self.stale
    }

    /// Selected frames dropped because the consumer left the queue full.
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Selected frames waiting for the consumer, newest at the back.
    #[must_use]
    pub fn ready_len(&self) -> usize {
        self.ready.len()
    }

    /// Consider one captured frame. Rejections name the source sequence.
    pub fn push(&mut self, frame: CapturedColorFrame) -> Result<Vec<RejectedFrame>, A2aLabError> {
        if self.closed {
            return Ok(vec![reject(&frame, FrameRejection::Shutdown)]);
        }
        let sample = monotonic_ns(&frame)?;
        let clock = frame.timing().clock().identity().map(str::to_owned);
        let mut rejected = Vec::new();
        if self.epoch.is_none()
            || self
                .epoch
                .as_ref()
                .is_some_and(|epoch| epoch.clock != clock || sample < epoch.last_ns)
        {
            let reset = self.epoch.is_some();
            if reset {
                self.discard_held(&mut rejected, FrameRejection::ClockReset);
            }
            self.epoch = Some(Epoch {
                clock,
                origin_ns: sample,
                due_ns: sample.saturating_add(self.period_ns),
                last_ns: sample,
            });
            self.enqueue(frame, &mut rejected);
            return Ok(rejected);
        }
        let due = self
            .epoch
            .as_ref()
            .map(|epoch| epoch.due_ns)
            .ok_or_else(|| A2aLabError::invalid("clock", "frame selection has no active epoch"))?;
        if sample < due {
            self.discard_held(&mut rejected, FrameRejection::Superseded);
            self.note_sample(sample);
            self.held = Some(frame);
            return Ok(rejected);
        }
        if sample == due {
            self.discard_held(&mut rejected, FrameRejection::Superseded);
            self.enqueue(frame, &mut rejected);
            self.advance_due(sample);
            return Ok(rejected);
        }
        if let Some(held) = self.held.take() {
            self.enqueue(held, &mut rejected);
        } else {
            self.enqueue(frame, &mut rejected);
            self.advance_due(sample);
            return Ok(rejected);
        }
        self.advance_due(sample);
        let due = self
            .epoch
            .as_ref()
            .map(|epoch| epoch.due_ns)
            .ok_or_else(|| A2aLabError::invalid("clock", "frame selection has no active epoch"))?;
        if sample < due {
            self.held = Some(frame);
        } else {
            self.enqueue(frame, &mut rejected);
            self.advance_due(sample);
        }
        Ok(rejected)
    }

    /// Remove the oldest selected frame.
    pub fn pull(&mut self) -> Option<CapturedColorFrame> {
        self.ready.pop_front()
    }

    /// Stop accepting frames. The unselected candidate is rejected. Already
    /// selected frames stay pullable. Nothing is synthesized to fill the period.
    pub fn shutdown(&mut self) -> Vec<RejectedFrame> {
        self.closed = true;
        let mut rejected = Vec::new();
        self.discard_held(&mut rejected, FrameRejection::Shutdown);
        rejected
    }

    fn note_sample(&mut self, sample: u64) {
        if let Some(epoch) = self.epoch.as_mut() {
            epoch.last_ns = sample;
        }
    }

    fn advance_due(&mut self, sample: u64) {
        let Some(epoch) = self.epoch.as_mut() else {
            return;
        };
        epoch.last_ns = sample;
        let elapsed = sample.saturating_sub(epoch.origin_ns);
        let steps = elapsed / self.period_ns;
        epoch.due_ns = epoch
            .origin_ns
            .saturating_add(steps.saturating_add(1).saturating_mul(self.period_ns));
    }

    fn discard_held(&mut self, rejected: &mut Vec<RejectedFrame>, reason: FrameRejection) {
        if let Some(held) = self.held.take() {
            if reason == FrameRejection::Superseded {
                self.stale = self.stale.saturating_add(1);
            }
            rejected.push(reject(&held, reason));
        }
    }

    fn enqueue(&mut self, frame: CapturedColorFrame, rejected: &mut Vec<RejectedFrame>) {
        if self.ready.len() == self.capacity
            && let Some(oldest) = self.ready.pop_front()
        {
            self.dropped = self.dropped.saturating_add(1);
            rejected.push(reject(&oldest, FrameRejection::DroppedOldest));
        }
        if let Some(epoch) = self.epoch.as_mut()
            && let Some(sample) = frame.timing().clock().monotonic_ns()
        {
            epoch.last_ns = sample;
        }
        self.ready.push_back(frame);
    }
}

fn monotonic_ns(frame: &CapturedColorFrame) -> Result<u64, A2aLabError> {
    frame.timing().clock().monotonic_ns().ok_or_else(|| {
        A2aLabError::invalid("clock", "frame selection requires a monotonic timestamp")
    })
}

fn reject(frame: &CapturedColorFrame, reason: FrameRejection) -> RejectedFrame {
    RejectedFrame {
        sequence: frame.identity().sequence(),
        reason,
    }
}

/// Whether a dispatch asks for live output, DVR persistence, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CadenceDestinations {
    live: bool,
    dvr: bool,
}

impl CadenceDestinations {
    /// Live viewers only.
    #[must_use]
    pub const fn live_only() -> Self {
        Self {
            live: true,
            dvr: false,
        }
    }

    /// DVR persistence only.
    #[must_use]
    pub const fn dvr_only() -> Self {
        Self {
            live: false,
            dvr: true,
        }
    }

    /// One encode fans out to both destinations.
    #[must_use]
    pub const fn shared() -> Self {
        Self {
            live: true,
            dvr: true,
        }
    }

    /// Live path selected.
    #[must_use]
    pub const fn live(self) -> bool {
        self.live
    }

    /// DVR path selected.
    #[must_use]
    pub const fn dvr(self) -> bool {
        self.dvr
    }
}

/// Outcome of independent live and DVR cadence gates before conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CadenceDispatch {
    /// Live output without DVR persistence for this capture time.
    Live(CapturedColorFrame),
    /// DVR persistence without live output for this capture time.
    Dvr(CapturedColorFrame),
    /// One encode serves live output and DVR persistence together.
    Shared(CapturedColorFrame),
}

impl CadenceDispatch {
    /// True when a caller should run color conversion for this dispatch.
    #[must_use]
    pub const fn requests_conversion(&self) -> bool {
        true
    }

    /// Destinations that should consume one encoded picture.
    #[must_use]
    pub const fn destinations(&self) -> CadenceDestinations {
        match self {
            Self::Live(_) => CadenceDestinations::live_only(),
            Self::Dvr(_) => CadenceDestinations::dvr_only(),
            Self::Shared(_) => CadenceDestinations::shared(),
        }
    }

    /// Owned frame when conversion is required.
    #[must_use]
    pub fn into_frame(self) -> CapturedColorFrame {
        match self {
            Self::Live(frame) | Self::Dvr(frame) | Self::Shared(frame) => frame,
        }
    }

    /// Borrowed frame when conversion is required.
    #[must_use]
    pub const fn frame(&self) -> &CapturedColorFrame {
        match self {
            Self::Live(frame) | Self::Dvr(frame) | Self::Shared(frame) => frame,
        }
    }
}

/// Independent live and DVR cadence selection on capture timestamps.
///
/// Each destination keeps its own gate. Frames rejected by both gates are
/// discarded before conversion. When both gates release the same source
/// sequence, the selector emits one shared encode representation.
#[derive(Debug)]
pub struct LiveDvrCadenceSelector {
    live: FreshFrameGate,
    dvr: FreshFrameGate,
    queue_capacity: usize,
}

impl LiveDvrCadenceSelector {
    /// `live_period_ns` and `dvr_period_ns` are output spacings. `capacity`
    /// bounds selected frames waiting on each destination.
    pub fn new(
        live_period_ns: u64,
        dvr_period_ns: u64,
        capacity: usize,
    ) -> Result<Self, A2aLabError> {
        Ok(Self {
            live: FreshFrameGate::new(live_period_ns, capacity)?,
            dvr: FreshFrameGate::new(dvr_period_ns, capacity)?,
            queue_capacity: capacity,
        })
    }

    /// Live at [`OUTPUT_5FPS_PERIOD_NS`] and DVR at `dvr_period_ns`.
    pub fn live_at_5fps(dvr_period_ns: u64, capacity: usize) -> Result<Self, A2aLabError> {
        Self::new(OUTPUT_5FPS_PERIOD_NS, dvr_period_ns, capacity)
    }

    /// Replace the live output cadence and reset live selection state.
    pub fn set_live_period_ns(&mut self, period_ns: u64) -> Result<(), A2aLabError> {
        self.live = FreshFrameGate::new(period_ns, self.queue_capacity)?;
        Ok(())
    }

    /// Replace the DVR save cadence and reset DVR selection state.
    pub fn set_dvr_period_ns(&mut self, period_ns: u64) -> Result<(), A2aLabError> {
        self.dvr = FreshFrameGate::new(period_ns, self.queue_capacity)?;
        Ok(())
    }

    /// Consider one captured frame. Returns conversion dispatches only for
    /// frames selected by at least one destination.
    pub fn push(&mut self, frame: CapturedColorFrame) -> Result<Vec<CadenceDispatch>, A2aLabError> {
        self.live.push(frame.clone())?;
        self.dvr.push(frame)?;
        Ok(merge_dispatches(
            collect_selected(&mut self.live),
            collect_selected(&mut self.dvr),
        ))
    }
}

fn collect_selected(gate: &mut FreshFrameGate) -> Vec<CapturedColorFrame> {
    let mut selected = Vec::new();
    while let Some(frame) = gate.pull() {
        selected.push(frame);
    }
    selected
}

fn merge_dispatches(
    live: Vec<CapturedColorFrame>,
    dvr: Vec<CapturedColorFrame>,
) -> Vec<CadenceDispatch> {
    if live.is_empty() && dvr.is_empty() {
        return Vec::new();
    }
    let mut dvr_pending: Vec<CapturedColorFrame> = dvr;
    let mut out = Vec::with_capacity(live.len() + dvr_pending.len());
    for live_frame in live {
        if let Some(index) = dvr_pending
            .iter()
            .position(|frame| frame.identity().sequence() == live_frame.identity().sequence())
        {
            let dvr_frame = dvr_pending.remove(index);
            debug_assert_eq!(
                live_frame.timing(),
                dvr_frame.timing(),
                "shared cadence must preserve one capture timestamp"
            );
            out.push(CadenceDispatch::Shared(live_frame));
        } else {
            out.push(CadenceDispatch::Live(live_frame));
        }
    }
    for dvr_frame in dvr_pending {
        out.push(CadenceDispatch::Dvr(dvr_frame));
    }
    out
}
