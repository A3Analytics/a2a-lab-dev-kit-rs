//! Bounded limits for frame decision history reads.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;

/// Maximum rows one history page may return.
pub const MAX_FRAME_DECISION_HISTORY_ROWS: u32 = 1_000;

/// Default row cap for pilot history reads.
pub const DEFAULT_FRAME_DECISION_HISTORY_ROWS: u32 = 100;

/// Read budgets for history queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDecisionReadLimits {
    max_rows: u32,
}

impl FrameDecisionReadLimits {
    /// Pilot defaults within documented ceilings.
    #[must_use]
    pub const fn pilot() -> Self {
        Self {
            max_rows: DEFAULT_FRAME_DECISION_HISTORY_ROWS,
        }
    }

    /// Creates limits after checking the ceiling.
    pub fn new(max_rows: u32) -> Result<Self, A2aLabError> {
        let limits = Self { max_rows };
        limits.check()?;
        Ok(limits)
    }

    /// Rejects zero or above-ceiling values.
    pub fn check(&self) -> Result<(), A2aLabError> {
        if self.max_rows == 0 || self.max_rows > MAX_FRAME_DECISION_HISTORY_ROWS {
            return Err(A2aLabError::invalid(
                "max_rows",
                format!("must be 1..={MAX_FRAME_DECISION_HISTORY_ROWS}"),
            ));
        }
        Ok(())
    }

    /// Maximum history rows per page.
    #[must_use]
    pub const fn max_rows(self) -> u32 {
        self.max_rows
    }
}
