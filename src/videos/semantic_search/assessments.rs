//! Bounded Jev assessments attached to semantic search hits.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::page::PageRequest;
use crate::time::TimeRange;
use crate::videos::saved_frame::SavedFrameRef;
use crate::videos::semantic_search::DecisionHistoryPolicy;
use crate::{
    FrameDecisionHistoryEntry, FrameDecisionLedgerProvider, FrameDecisionReadLimits,
    FrameDecisionReadScope, FrameDecisionRecord, ReadFrameDecisionHistoryRequest,
    decision_history_fingerprint,
};

/// Immutable Jev history and provenance bound to one saved frame revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SemanticFrameHitAssessments {
    frame: SavedFrameRef,
    history_policy: DecisionHistoryPolicy,
    entries: Vec<FrameDecisionHistoryEntry>,
    /// Stable decision-id fingerprints for explicit coverage.
    fingerprints: BTreeMap<String, String>,
    truncated: bool,
}

impl SemanticFrameHitAssessments {
    /// Validates frame association, row budget, and entry alignment.
    pub fn new(
        frame: SavedFrameRef,
        history_policy: DecisionHistoryPolicy,
        entries: Vec<FrameDecisionHistoryEntry>,
        fingerprints: BTreeMap<String, String>,
        truncated: bool,
        max_rows: u32,
    ) -> Result<Self, A2aLabError> {
        if entries.len() > usize::try_from(max_rows).unwrap_or(usize::MAX) {
            return Err(A2aLabError::invalid(
                "entries",
                format!("must contain at most {max_rows} rows"),
            ));
        }
        for entry in &entries {
            if entry.record().frame() != &frame {
                return Err(A2aLabError::invalid(
                    "entries",
                    "each record must reference the hit frame revision",
                ));
            }
        }
        Ok(Self {
            frame,
            history_policy,
            entries,
            fingerprints,
            truncated,
        })
    }

    /// Saved revision these assessments describe.
    #[must_use]
    pub const fn frame(&self) -> &SavedFrameRef {
        &self.frame
    }

    /// Reduction policy used when assembling the attachment.
    #[must_use]
    pub const fn history_policy(&self) -> DecisionHistoryPolicy {
        self.history_policy
    }

    /// Bounded history rows with per-evidence availability.
    #[must_use]
    pub fn entries(&self) -> &[FrameDecisionHistoryEntry] {
        &self.entries
    }

    /// Stable fingerprints keyed by decision id.
    #[must_use]
    pub fn fingerprints(&self) -> &BTreeMap<String, String> {
        &self.fingerprints
    }

    /// When true, additional rows exist beyond the per-hit row budget.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    /// Records referenced by `entries`.
    #[must_use]
    pub fn records(&self) -> Vec<&FrameDecisionRecord> {
        self.entries
            .iter()
            .map(FrameDecisionHistoryEntry::record)
            .collect()
    }
}

/// Loads bounded assessments for one hit when a ledger provider is configured.
pub async fn authorized_assessments_for_frame(
    provider: &dyn FrameDecisionLedgerProvider,
    frame: &SavedFrameRef,
    max_rows: u32,
) -> Result<SemanticFrameHitAssessments, A2aLabError> {
    let captured = frame.captured_at();
    let range = TimeRange::new(captured, captured.saturating_add_micros(1))?;
    let scope = FrameDecisionReadScope::new(frame.source_id().clone(), range, Some(frame.clone()))?;
    let request = ReadFrameDecisionHistoryRequest::new(
        scope,
        None,
        None,
        None,
        DecisionHistoryPolicy::FullHistory,
        FrameDecisionReadLimits::new(max_rows)?,
        PageRequest::new(None, max_rows)?,
        Vec::new(),
    )?;
    let history = provider.read_frame_decision_history(request).await?;
    let records: Vec<FrameDecisionRecord> = history
        .entries()
        .items()
        .iter()
        .map(|entry| entry.record().clone())
        .collect();
    let fingerprints = decision_history_fingerprint(&records);
    let truncated = history.entries().next_cursor().is_some();
    SemanticFrameHitAssessments::new(
        frame.clone(),
        DecisionHistoryPolicy::FullHistory,
        history.entries().items().to_vec(),
        fingerprints,
        truncated,
        max_rows,
    )
}
