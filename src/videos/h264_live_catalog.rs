//! Shared registration for bounded per-source live H.264 stores.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::error::A2aLabError;
use crate::id::VideoSourceId;
use crate::videos::{DvrPictureProperties, H264LiveStore, H264LiveStoreLimits};

const MIN_LIVE_SAMPLES: u64 = 32;

fn live_store_limits(
    retain_secs: u64,
    fps: u32,
    max_sample_bytes: u64,
) -> Result<H264LiveStoreLimits, A2aLabError> {
    let retention_us = retain_secs.saturating_mul(1_000_000);
    let max_samples = usize::try_from(
        retain_secs
            .saturating_mul(u64::from(fps.max(1)))
            .saturating_add(16)
            .max(MIN_LIVE_SAMPLES),
    )
    .unwrap_or(usize::MAX);
    let max_bytes = max_sample_bytes
        .saturating_mul(u64::try_from(max_samples).unwrap_or(u64::MAX))
        .max(max_sample_bytes)
        .min(512 * 1024 * 1024) as usize;
    H264LiveStoreLimits::new(max_samples, max_bytes, retention_us)
}

/// Bounded live retention for one source id.
#[derive(Clone, Debug)]
pub struct SourceLiveH264Store {
    store: Arc<H264LiveStore>,
}

impl SourceLiveH264Store {
    /// Opens limits from retention, cadence, and sample size.
    pub fn open(retain_secs: u64, fps: u32, max_sample_bytes: u64) -> Result<Self, A2aLabError> {
        Ok(Self {
            store: Arc::new(H264LiveStore::new(live_store_limits(
                retain_secs,
                fps,
                max_sample_bytes,
            )?)),
        })
    }

    /// Shared handle for readers and non-blocking appenders.
    #[must_use]
    pub fn inner(&self) -> &Arc<H264LiveStore> {
        &self.store
    }

    /// Clears retained samples after source recovery.
    pub fn reconnect(&self) -> Result<(), A2aLabError> {
        self.store.reconnect()
    }

    /// Opens an epoch when absent. Live append never waits on DVR writers.
    pub fn open_epoch_if_absent(
        &self,
        track_id: &crate::id::DvrTrackId,
        initialization: Vec<u8>,
        picture: DvrPictureProperties,
    ) -> Result<(), A2aLabError> {
        let snapshot = self.store.snapshot()?;
        if snapshot
            .epochs()
            .iter()
            .any(|epoch| epoch.track_id() == track_id)
        {
            return Ok(());
        }
        self.store
            .open_epoch(track_id.clone(), initialization, picture)
    }
}

/// Process-wide map of live stores keyed by source id.
#[derive(Clone, Debug, Default)]
pub struct LiveH264Catalog {
    stores: Arc<Mutex<HashMap<String, SourceLiveH264Store>>>,
}

impl LiveH264Catalog {
    /// Empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a new bounded store for one source id.
    pub fn register(
        &self,
        source_id: &VideoSourceId,
        retain_secs: u64,
        fps: u32,
        max_sample_bytes: u64,
    ) -> Result<SourceLiveH264Store, A2aLabError> {
        let entry = SourceLiveH264Store::open(retain_secs, fps, max_sample_bytes)?;
        self.stores
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(source_id.as_str().to_owned(), entry.clone());
        Ok(entry)
    }

    /// Returns a registered store when one exists.
    #[must_use]
    pub fn get(&self, source_id: &VideoSourceId) -> Option<SourceLiveH264Store> {
        self.stores
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(source_id.as_str())
            .cloned()
    }
}
