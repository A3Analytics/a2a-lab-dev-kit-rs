//! Camera selection handed to each capture worker.

use crate::videos::ColorCamera;

/// Camera opened on the worker thread so non-`Send` backends stay contained.
pub(crate) enum WorkerCamera {
    /// Injected backend for hardware-free tests.
    Injected(Box<dyn ColorCamera + Send>),
    /// Native index opened inside the worker when `nokhwa` is enabled.
    #[cfg(feature = "nokhwa")]
    NativeIndex(u32),
    /// Discovery failed or the selector did not resolve.
    Absent,
}
