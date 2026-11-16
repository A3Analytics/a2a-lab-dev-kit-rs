//! Semantic manifest registration on [`DvrPersistHook`].

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use crate::error::A2aLabError;
use crate::videos::sqlite_dvr::{DvrCommittedFrame, DvrPersistHook};

use super::engine::SemanticChunkDvrStore;

/// Post-commit semantic sidecar wired at DVR open.
pub struct SemanticPersistHook {
    store: Arc<OnceLock<SemanticChunkDvrStore>>,
}

impl SemanticPersistHook {
    #[must_use]
    pub fn new(store: Arc<OnceLock<SemanticChunkDvrStore>>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl DvrPersistHook for SemanticPersistHook {
    async fn after_frame_committed(&self, frame: &DvrCommittedFrame) -> Result<(), A2aLabError> {
        let Some(store) = self.store.get() else {
            return Err(A2aLabError::unavailable(
                "semantic DVR hook before store init",
            ));
        };
        match store.on_frame_committed(frame).await {
            Ok(()) => Ok(()),
            Err(error) => {
                store.record_hook_failure(error.clone());
                Err(error)
            }
        }
    }
}
