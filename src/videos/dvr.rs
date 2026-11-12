//! Bounded in-memory queue for sparse disk DVR persistence writers.
//!
//! A full or stopped queue applies backpressure only to DVR writers. Live
//! delivery uses [`super::live::H264LiveStore`] on a separate path.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::A2aLabError;
use crate::time::UtcTimestamp;

/// One DVR-bound encoded sample waiting for disk persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DvrQueuedSample {
    captured_at: UtcTimestamp,
    bytes: Vec<u8>,
}

impl DvrQueuedSample {
    /// UTC capture instant.
    #[must_use]
    pub fn captured_at(&self) -> UtcTimestamp {
        self.captured_at
    }

    /// Encoded bytes retained for disk write.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Result of a non-blocking DVR enqueue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DvrEnqueueStatus {
    /// Sample is queued for persistence.
    Accepted,
    /// Queue capacity is exhausted.
    Full,
    /// Persistence is stopped and rejects new samples.
    Stopped,
}

/// Bounded DVR writer queue that can saturate independently of live retention.
#[derive(Debug)]
pub struct DvrPersistenceQueue {
    capacity: usize,
    stopped: AtomicBool,
    inner: Mutex<Vec<DvrQueuedSample>>,
}

impl DvrPersistenceQueue {
    /// Creates a queue when capacity is positive.
    pub fn new(capacity: usize) -> Result<Self, A2aLabError> {
        if capacity == 0 {
            return Err(A2aLabError::invalid("capacity", "must be positive"));
        }
        Ok(Self {
            capacity,
            stopped: AtomicBool::new(false),
            inner: Mutex::new(Vec::new()),
        })
    }

    /// Stops accepting DVR samples until [`Self::start`] runs again.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    /// Resumes acceptance after [`Self::stop`].
    pub fn start(&self) {
        self.stopped.store(false, Ordering::Relaxed);
    }

    /// Reports whether the queue is stopped.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    /// Attempts to enqueue without blocking when the queue is full or stopped.
    pub fn try_enqueue(
        &self,
        captured_at: UtcTimestamp,
        bytes: Vec<u8>,
    ) -> Result<DvrEnqueueStatus, A2aLabError> {
        if self.is_stopped() {
            return Ok(DvrEnqueueStatus::Stopped);
        }
        let mut queue = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if queue.len() >= self.capacity {
            return Ok(DvrEnqueueStatus::Full);
        }
        queue.push(DvrQueuedSample { captured_at, bytes });
        Ok(DvrEnqueueStatus::Accepted)
    }

    /// Number of samples waiting for disk persistence.
    pub fn depth(&self) -> Result<usize, A2aLabError> {
        let queue = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(queue.len())
    }

    /// Drains one queued sample, usually after a simulated disk write.
    pub fn pop_front(&self) -> Result<Option<DvrQueuedSample>, A2aLabError> {
        let mut queue = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if queue.is_empty() {
            return Ok(None);
        }
        Ok(Some(queue.remove(0)))
    }

    /// Whether no DVR samples are waiting for persistence.
    pub fn is_sparse(&self) -> Result<bool, A2aLabError> {
        Ok(self.depth()? == 0)
    }
}
