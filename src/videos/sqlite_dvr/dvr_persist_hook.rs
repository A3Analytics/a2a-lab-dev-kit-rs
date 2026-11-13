//! Post-commit DVR persistence hook seam.

use async_trait::async_trait;

use crate::error::A2aLabError;

use super::committed_frame::DvrCommittedFrame;

/// Runs after a frame row and chunk payload are committed.
#[async_trait]
pub trait DvrPersistHook: Send + Sync {
    /// Consumes one committed frame. Failures must not affect the store transaction.
    async fn after_frame_committed(&self, frame: &DvrCommittedFrame) -> Result<(), A2aLabError>;
}
