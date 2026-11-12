//! Bounded in-memory H.264 live retention independent of DVR persistence.
//!
//! Samples stay in RAM with capture timestamps and immutable epoch identity.
//! Explicit sample, byte, and time bounds evict the oldest retained media without
//! blocking on disk DVR queues.

use std::sync::RwLock;

use crate::error::A2aLabError;
use crate::id::DvrTrackId;
use crate::time::UtcTimestamp;
use crate::videos::{DvrPictureProperties, VideoCodec};

/// Positive bounds for one live store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct H264LiveStoreLimits {
    max_samples: usize,
    max_bytes: usize,
    retention_us: u64,
}

impl H264LiveStoreLimits {
    /// Creates limits when every bound is positive.
    pub fn new(
        max_samples: usize,
        max_bytes: usize,
        retention_us: u64,
    ) -> Result<Self, A2aLabError> {
        if max_samples == 0 {
            return Err(A2aLabError::invalid("max_samples", "must be positive"));
        }
        if max_bytes == 0 {
            return Err(A2aLabError::invalid("max_bytes", "must be positive"));
        }
        if retention_us == 0 {
            return Err(A2aLabError::invalid("retention_us", "must be positive"));
        }
        Ok(Self {
            max_samples,
            max_bytes,
            retention_us,
        })
    }

    /// Maximum retained encoded samples.
    #[must_use]
    pub const fn max_samples(self) -> usize {
        self.max_samples
    }

    /// Maximum retained encoded bytes across all samples.
    #[must_use]
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }

    /// Oldest sample may trail the newest by at most this many microseconds.
    #[must_use]
    pub const fn retention_us(self) -> u64 {
        self.retention_us
    }
}

/// Immutable H.264 decoder configuration for one live epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H264LiveEpoch {
    track_id: DvrTrackId,
    initialization: Vec<u8>,
    picture: DvrPictureProperties,
}

impl H264LiveEpoch {
    /// Track identity for this epoch.
    #[must_use]
    pub fn track_id(&self) -> &DvrTrackId {
        &self.track_id
    }

    /// Annex B SPS/PPS bound to this epoch.
    #[must_use]
    pub fn initialization(&self) -> &[u8] {
        &self.initialization
    }

    /// Declared picture size for this epoch.
    #[must_use]
    pub const fn picture(&self) -> DvrPictureProperties {
        self.picture
    }
}

/// One encoded live sample and its epoch association.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H264LiveSample {
    track_id: DvrTrackId,
    captured_at: UtcTimestamp,
    bytes: Vec<u8>,
    keyframe: bool,
}

impl H264LiveSample {
    /// Epoch that owns decoder initialization for this sample.
    #[must_use]
    pub fn track_id(&self) -> &DvrTrackId {
        &self.track_id
    }

    /// Original UTC capture instant.
    #[must_use]
    pub fn captured_at(&self) -> UtcTimestamp {
        self.captured_at
    }

    /// Length-prefixed or annex-B media bytes for this picture.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Whether this picture is a keyframe.
    #[must_use]
    pub const fn keyframe(&self) -> bool {
        self.keyframe
    }
}

/// Accounting for one bounded append.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct H264LiveAppendStats {
    evicted_samples: usize,
    evicted_bytes: usize,
}

impl H264LiveAppendStats {
    /// Samples removed to restore bounds.
    #[must_use]
    pub const fn evicted_samples(self) -> usize {
        self.evicted_samples
    }

    /// Payload bytes removed to restore bounds.
    #[must_use]
    pub const fn evicted_bytes(self) -> usize {
        self.evicted_bytes
    }

    fn record_eviction(&mut self, bytes: usize) {
        self.evicted_samples += 1;
        self.evicted_bytes += bytes;
    }
}

/// Snapshot of retained live media safe for concurrent readers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H264LiveSnapshot {
    epochs: Vec<H264LiveEpoch>,
    samples: Vec<H264LiveSample>,
    retained_bytes: usize,
}

impl H264LiveSnapshot {
    /// Opened configuration epochs.
    #[must_use]
    pub fn epochs(&self) -> &[H264LiveEpoch] {
        &self.epochs
    }

    /// Retained samples in capture-time order.
    #[must_use]
    pub fn samples(&self) -> &[H264LiveSample] {
        &self.samples
    }

    /// Sum of [`H264LiveSample::bytes`] lengths.
    #[must_use]
    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

#[derive(Debug)]
struct H264LiveState {
    limits: H264LiveStoreLimits,
    epochs: Vec<H264LiveEpoch>,
    samples: Vec<H264LiveSample>,
    retained_bytes: usize,
}

/// Thread-safe bounded store for encoded live H.264.
#[derive(Debug)]
pub struct H264LiveStore {
    inner: RwLock<H264LiveState>,
}

impl H264LiveStore {
    /// Empty store with fixed bounds.
    #[must_use]
    pub fn new(limits: H264LiveStoreLimits) -> Self {
        Self {
            inner: RwLock::new(H264LiveState {
                limits,
                epochs: Vec::new(),
                samples: Vec::new(),
                retained_bytes: 0,
            }),
        }
    }

    /// Opens one H.264 epoch. The same track id cannot replace initialization.
    pub fn open_epoch(
        &self,
        track_id: DvrTrackId,
        codec_initialization: Vec<u8>,
        picture: DvrPictureProperties,
    ) -> Result<(), A2aLabError> {
        if codec_initialization.is_empty() {
            return Err(A2aLabError::invalid(
                "codec_initialization",
                "an H.264 epoch requires decoder initialization",
            ));
        }
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.epochs.iter().any(|epoch| epoch.track_id == track_id) {
            return Err(A2aLabError::invalid(
                "id",
                "H.264 epoch initialization and picture properties are immutable",
            ));
        }
        state.epochs.push(H264LiveEpoch {
            track_id,
            initialization: codec_initialization,
            picture,
        });
        Ok(())
    }

    /// Retains one encoded sample and evicts the oldest media until bounds hold.
    pub fn append_sample(
        &self,
        track_id: &DvrTrackId,
        captured_at: UtcTimestamp,
        bytes: Vec<u8>,
        keyframe: bool,
    ) -> Result<H264LiveAppendStats, A2aLabError> {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.epochs.iter().any(|epoch| &epoch.track_id == track_id) {
            return Err(A2aLabError::invalid(
                "track_id",
                "live sample references an unknown H.264 epoch",
            ));
        }
        if bytes.len() > state.limits.max_bytes() {
            return Err(A2aLabError::invalid(
                "bytes",
                "one live sample exceeds the configured byte bound",
            ));
        }
        state.samples.push(H264LiveSample {
            track_id: track_id.clone(),
            captured_at,
            bytes,
            keyframe,
        });
        state.retained_bytes = state.samples.iter().map(|sample| sample.bytes.len()).sum();
        Ok(enforce_bounds(&mut state))
    }

    /// Clears retained samples after source recovery while leaving epochs in place.
    pub fn reconnect(&self) -> Result<(), A2aLabError> {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.samples.clear();
        state.retained_bytes = 0;
        Ok(())
    }

    /// Returns a copy of epochs and samples for readers.
    pub fn snapshot(&self) -> Result<H264LiveSnapshot, A2aLabError> {
        let state = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(H264LiveSnapshot {
            epochs: state.epochs.clone(),
            samples: state.samples.clone(),
            retained_bytes: state.retained_bytes,
        })
    }

    /// Finds the epoch associated with one track id.
    pub fn epoch(&self, track_id: &DvrTrackId) -> Result<Option<H264LiveEpoch>, A2aLabError> {
        let state = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(state
            .epochs
            .iter()
            .find(|epoch| &epoch.track_id == track_id)
            .cloned())
    }

    /// Stored codec for every live epoch.
    #[must_use]
    pub const fn codec() -> VideoCodec {
        VideoCodec::H264
    }
}

fn enforce_bounds(state: &mut H264LiveState) -> H264LiveAppendStats {
    let mut eviction = H264LiveAppendStats::default();
    while bound_violation(state).is_some() {
        let oldest = state.samples.remove(0);
        state.retained_bytes = state.retained_bytes.saturating_sub(oldest.bytes.len());
        eviction.record_eviction(oldest.bytes.len());
    }
    eviction
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundViolation {
    SampleCount,
    ByteCount,
    Retention,
}

fn bound_violation(state: &H264LiveState) -> Option<BoundViolation> {
    if state.samples.len() > state.limits.max_samples() {
        return Some(BoundViolation::SampleCount);
    }
    if state.retained_bytes > state.limits.max_bytes() {
        return Some(BoundViolation::ByteCount);
    }
    let newest = state.samples.last()?;
    let floor = newest
        .captured_at
        .saturating_add_micros(-i64::try_from(state.limits.retention_us()).unwrap_or(i64::MAX));
    if state
        .samples
        .first()
        .is_some_and(|oldest| oldest.captured_at < floor)
    {
        return Some(BoundViolation::Retention);
    }
    None
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    const INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67];

    fn track(id: &str) -> DvrTrackId {
        DvrTrackId::new(id).expect("track")
    }

    fn store() -> H264LiveStore {
        H264LiveStore::new(H264LiveStoreLimits::new(3, 30, 5_000_000).expect("limits"))
    }

    #[test]
    fn epoch_is_immutable() {
        let store = store();
        let picture = DvrPictureProperties::new(640, 480).expect("picture");
        store
            .open_epoch(track("a"), INIT.to_vec(), picture)
            .expect("open");
        let changed = DvrPictureProperties::new(1280, 720).expect("picture");
        let error = store
            .open_epoch(track("a"), INIT.to_vec(), changed)
            .expect_err("immutable");
        assert!(matches!(error, A2aLabError::Invalid { field: "id", .. }));
    }
}
