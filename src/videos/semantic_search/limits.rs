//! Finite bounds for semantic frame search and coverage queries.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;

/// Largest ranked hit count a caller may request.
pub const MAX_SEMANTIC_TOP_K: u32 = 100;

/// Largest ANN candidate pool a search may scan before post-filters.
pub const MAX_SEMANTIC_CANDIDATES: u32 = 10_000;

/// Maximum neighbor frames returned for bounded playback context.
pub const MAX_SEMANTIC_NEIGHBOR_FRAMES: u32 = 30;

/// Largest Jev history row budget attached to each semantic hit.
pub const MAX_SEMANTIC_HIT_ASSESSMENT_ROWS: u32 = 100;

/// Default per-hit Jev history row budget for interactive search.
pub const DEFAULT_SEMANTIC_HIT_ASSESSMENT_ROWS: u32 = 20;

/// Default ranked hit cap for interactive search.
pub const DEFAULT_SEMANTIC_TOP_K: u32 = 10;

/// Default ANN candidate budget for pilot deployments.
pub const DEFAULT_SEMANTIC_CANDIDATES: u32 = 1_000;

/// Bounded search budgets carried on requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SemanticSearchLimits {
    top_k: u32,
    candidate_limit: u32,
    max_neighbor_frames: u32,
    #[serde(default = "default_max_assessment_rows_per_hit")]
    max_assessment_rows_per_hit: u32,
}

impl SemanticSearchLimits {
    /// Pilot defaults within documented ceilings.
    #[must_use]
    pub const fn pilot() -> Self {
        Self {
            top_k: DEFAULT_SEMANTIC_TOP_K,
            candidate_limit: DEFAULT_SEMANTIC_CANDIDATES,
            max_neighbor_frames: 5,
            max_assessment_rows_per_hit: DEFAULT_SEMANTIC_HIT_ASSESSMENT_ROWS,
        }
    }

    /// Creates limits after checking each ceiling.
    pub fn new(
        top_k: u32,
        candidate_limit: u32,
        max_neighbor_frames: u32,
    ) -> Result<Self, A2aLabError> {
        Self::with_assessment_rows(top_k, candidate_limit, max_neighbor_frames, None)
    }

    /// Creates limits including an optional per-hit assessment row budget.
    pub fn with_assessment_rows(
        top_k: u32,
        candidate_limit: u32,
        max_neighbor_frames: u32,
        max_assessment_rows_per_hit: Option<u32>,
    ) -> Result<Self, A2aLabError> {
        let limits = Self {
            top_k,
            candidate_limit,
            max_neighbor_frames,
            max_assessment_rows_per_hit: max_assessment_rows_per_hit
                .unwrap_or(DEFAULT_SEMANTIC_HIT_ASSESSMENT_ROWS),
        };
        limits.check()?;
        Ok(limits)
    }

    /// Rejects zero or above-ceiling values.
    pub fn check(&self) -> Result<(), A2aLabError> {
        check_positive_bounded("top_k", self.top_k, MAX_SEMANTIC_TOP_K)?;
        check_positive_bounded(
            "candidate_limit",
            self.candidate_limit,
            MAX_SEMANTIC_CANDIDATES,
        )?;
        check_positive_bounded(
            "max_neighbor_frames",
            self.max_neighbor_frames,
            MAX_SEMANTIC_NEIGHBOR_FRAMES,
        )?;
        check_positive_bounded(
            "max_assessment_rows_per_hit",
            self.max_assessment_rows_per_hit,
            MAX_SEMANTIC_HIT_ASSESSMENT_ROWS,
        )?;
        Ok(())
    }

    /// Maximum ranked hits returned across all pages.
    #[must_use]
    pub const fn top_k(self) -> u32 {
        self.top_k
    }

    /// Maximum ANN candidates considered before decision filters and rescoring.
    #[must_use]
    pub const fn candidate_limit(self) -> u32 {
        self.candidate_limit
    }

    /// Maximum temporal neighbor frames attached to each hit.
    #[must_use]
    pub const fn max_neighbor_frames(self) -> u32 {
        self.max_neighbor_frames
    }

    /// Maximum Jev history rows attached to each returned hit.
    #[must_use]
    pub const fn max_assessment_rows_per_hit(self) -> u32 {
        self.max_assessment_rows_per_hit
    }
}

const fn default_max_assessment_rows_per_hit() -> u32 {
    DEFAULT_SEMANTIC_HIT_ASSESSMENT_ROWS
}

fn check_positive_bounded(field: &'static str, value: u32, max: u32) -> Result<(), A2aLabError> {
    if value == 0 || value > max {
        return Err(A2aLabError::invalid(field, format!("must be 1..={max}")));
    }
    Ok(())
}
