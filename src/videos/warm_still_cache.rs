//! Thread-safe newest JPEG still per configured video source.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Newest retained still for one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmStill {
    sequence: u64,
    jpeg: Vec<u8>,
}

impl WarmStill {
    /// Backend sequence copied from the drained color frame.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// JPEG bytes retained for still capture.
    #[must_use]
    pub fn jpeg(&self) -> &[u8] {
        &self.jpeg
    }
}

#[derive(Debug, Default)]
struct Slot {
    alive: bool,
    sequence: u64,
    jpeg: Vec<u8>,
}

/// Shared warm still storage updated by capture drain loops.
#[derive(Debug, Default, Clone)]
pub struct WarmStillCache {
    slots: Arc<Mutex<HashMap<String, Slot>>>,
}

impl WarmStillCache {
    /// Empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores `jpeg` when the source is active and `sequence` is newer.
    pub fn remember(&self, source_id: &str, sequence: u64, jpeg: Vec<u8>) {
        let Ok(mut slots) = self.slots.lock() else {
            return;
        };
        let slot = slots.entry(source_id.to_owned()).or_insert_with(|| Slot {
            alive: true,
            ..Slot::default()
        });
        if !slot.alive || sequence < slot.sequence {
            return;
        }
        slot.sequence = sequence;
        slot.jpeg = jpeg;
    }

    /// Newest valid still for `source_id`.
    #[must_use]
    pub fn latest(&self, source_id: &str) -> Option<WarmStill> {
        let slots = self.slots.lock().ok()?;
        let slot = slots.get(source_id)?;
        if !slot.alive || slot.jpeg.is_empty() {
            return None;
        }
        Some(WarmStill {
            sequence: slot.sequence,
            jpeg: slot.jpeg.clone(),
        })
    }

    /// Clears retained bytes without accepting further updates until [`Self::reset_source`].
    pub fn invalidate(&self, source_id: &str) {
        let Ok(mut slots) = self.slots.lock() else {
            return;
        };
        if let Some(slot) = slots.get_mut(source_id) {
            slot.alive = false;
            slot.jpeg.clear();
        }
    }

    /// Drops remembered still data for one configured source.
    pub fn clear_source(&self, source_id: &str) {
        let Ok(mut slots) = self.slots.lock() else {
            return;
        };
        slots.remove(source_id);
    }

    /// Clears every remembered still buffer. Tests use this for isolation.
    pub fn clear_all(&self) {
        let Ok(mut slots) = self.slots.lock() else {
            return;
        };
        slots.clear();
    }

    /// Clears stale retention after reconnect and accepts frames again.
    pub fn reset_source(&self, source_id: &str) {
        let Ok(mut slots) = self.slots.lock() else {
            return;
        };
        slots.insert(
            source_id.to_owned(),
            Slot {
                alive: true,
                sequence: 0,
                jpeg: Vec::new(),
            },
        );
    }
}
