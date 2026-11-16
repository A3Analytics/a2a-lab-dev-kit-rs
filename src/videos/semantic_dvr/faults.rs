//! Injectable faults for hardware-free save-path tests.

use std::sync::atomic::{AtomicBool, Ordering};

/// Test-only faults injected on the DVR save path.
#[derive(Debug, Default)]
pub struct SaveFaults {
    fail_payload_append: AtomicBool,
    fail_metadata_commit: AtomicBool,
}

impl SaveFaults {
    #[must_use]
    pub fn fail_payload_append(&self) -> bool {
        self.fail_payload_append.load(Ordering::Relaxed)
    }

    pub fn set_fail_payload_append(&self, value: bool) {
        self.fail_payload_append.store(value, Ordering::Relaxed);
    }

    #[must_use]
    pub fn fail_metadata_commit(&self) -> bool {
        self.fail_metadata_commit.load(Ordering::Relaxed)
    }

    pub fn set_fail_metadata_commit(&self, value: bool) {
        self.fail_metadata_commit.store(value, Ordering::Relaxed);
    }
}
