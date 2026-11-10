//! Protected append/read boundary for immutable Jev frame decisions.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::error::A2aLabError;
use crate::id::{DecisionIdempotencyKey, FrameDecisionId, SavedFrameId};
use crate::videos::frame_decision::predicate::LabPredicate;
use crate::videos::frame_decision::record::FrameDecisionRecord;
use crate::videos::saved_frame::SavedFrameRef;

/// Append conflict or authorization failure for the protected boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameDecisionAppendError {
    /// Same idempotency key with a different payload.
    IdempotencyConflict {
        key: DecisionIdempotencyKey,
        existing: FrameDecisionId,
    },
    /// Supersession link pointed at a missing or incompatible record.
    InvalidSupersession {
        prior: FrameDecisionId,
        message: String,
    },
    /// Record failed validation for the ledger policy.
    Validation(A2aLabError),
}

impl FrameDecisionAppendError {
    /// Stable reason code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::IdempotencyConflict { .. } => "idempotency_conflict",
            Self::InvalidSupersession { .. } => "invalid_supersession",
            Self::Validation(_) => "validation",
        }
    }
}

/// In-memory protected ledger validating Jev-only production appends.
#[derive(Debug, Default)]
pub struct FrameDecisionLedger {
    by_idempotency: HashMap<DecisionIdempotencyKey, FrameDecisionId>,
    records: HashMap<FrameDecisionId, FrameDecisionRecord>,
    append_order: Vec<FrameDecisionId>,
}

impl FrameDecisionLedger {
    /// Empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append-only write path for validated production records.
    pub fn append(
        &mut self,
        record: FrameDecisionRecord,
    ) -> Result<&FrameDecisionRecord, FrameDecisionAppendError> {
        record
            .validate()
            .map_err(FrameDecisionAppendError::Validation)?;
        if let Some(prior_id) = record.supersedes() {
            self.validate_supersession(&record, prior_id)?;
        }

        if let Some(existing_id) = self.by_idempotency.get(record.idempotency_key()) {
            let existing = self
                .records
                .get(existing_id)
                .expect("index contains decision id");
            if existing.idempotent_fingerprint().ok() == record.idempotent_fingerprint().ok() {
                return Ok(existing);
            }
            return Err(FrameDecisionAppendError::IdempotencyConflict {
                key: record.idempotency_key().clone(),
                existing: existing_id.clone(),
            });
        }

        let id = record.decision_id().clone();
        self.by_idempotency
            .insert(record.idempotency_key().clone(), id.clone());
        self.records.insert(id.clone(), record);
        self.append_order.push(id);
        Ok(self
            .records
            .get(self.append_order.last().expect("pushed"))
            .unwrap())
    }

    /// Read by stable decision id.
    #[must_use]
    pub fn get(&self, decision_id: &FrameDecisionId) -> Option<&FrameDecisionRecord> {
        self.records.get(decision_id)
    }

    /// Read by idempotency key.
    #[must_use]
    pub fn get_by_idempotency(&self, key: &DecisionIdempotencyKey) -> Option<&FrameDecisionRecord> {
        self.by_idempotency
            .get(key)
            .and_then(|id| self.records.get(id))
    }

    /// Append order for all retained decisions.
    #[must_use]
    pub fn history(&self) -> &[FrameDecisionId] {
        &self.append_order
    }

    /// All decisions for one saved frame revision, in append order.
    #[must_use]
    pub fn history_for_frame(&self, frame_id: &SavedFrameId) -> Vec<&FrameDecisionRecord> {
        self.append_order
            .iter()
            .filter_map(|id| self.records.get(id))
            .filter(|record| record.frame().frame_id() == frame_id)
            .collect()
    }

    /// Predicate history for one frame revision.
    #[must_use]
    pub fn history_for_predicate(
        &self,
        frame: &SavedFrameRef,
        predicate: &LabPredicate,
    ) -> Vec<&FrameDecisionRecord> {
        self.history_for_frame(frame.frame_id())
            .into_iter()
            .filter(|record| record.predicate() == predicate)
            .collect()
    }

    fn validate_supersession(
        &self,
        record: &FrameDecisionRecord,
        prior_id: &FrameDecisionId,
    ) -> Result<(), FrameDecisionAppendError> {
        let prior = self.records.get(prior_id).ok_or_else(|| {
            FrameDecisionAppendError::InvalidSupersession {
                prior: prior_id.clone(),
                message: "superseded decision was not found".to_owned(),
            }
        })?;
        if prior.frame().frame_id() != record.frame().frame_id() {
            return Err(FrameDecisionAppendError::InvalidSupersession {
                prior: prior_id.clone(),
                message: "supersession must stay on the same frame revision".to_owned(),
            });
        }
        if prior.predicate() != record.predicate() {
            return Err(FrameDecisionAppendError::InvalidSupersession {
                prior: prior_id.clone(),
                message: "supersession must keep the same predicate".to_owned(),
            });
        }
        if prior.provenance().producer() != record.provenance().producer() {
            return Err(FrameDecisionAppendError::InvalidSupersession {
                prior: prior_id.clone(),
                message: "supersession must stay within the same Jev producer stream".to_owned(),
            });
        }
        if !acyclic_supersession_chain(self, prior_id) {
            return Err(FrameDecisionAppendError::InvalidSupersession {
                prior: prior_id.clone(),
                message: "supersession chain would form a cycle".to_owned(),
            });
        }
        Ok(())
    }
}

fn acyclic_supersession_chain(ledger: &FrameDecisionLedger, start: &FrameDecisionId) -> bool {
    let mut seen = HashSet::new();
    let mut current = Some(start);
    while let Some(id) = current {
        if !seen.insert(id.clone()) {
            return false;
        }
        current = ledger.get(id).and_then(|record| record.supersedes());
    }
    true
}

/// Groups decisions by predicate for coverage summaries.
#[must_use]
pub fn group_history_by_predicate(
    records: &[&FrameDecisionRecord],
) -> BTreeMap<String, Vec<FrameDecisionId>> {
    let mut grouped = BTreeMap::new();
    for record in records {
        grouped
            .entry(record.predicate().id().to_owned())
            .or_insert_with(Vec::new)
            .push(record.decision_id().clone());
    }
    grouped
}
