//! Jev decision predicates for post-filtering retrieval candidates.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::{DecisionOutcome, DecisionSubject, JevProducerIdentity, LabPredicate};

/// How decision history is reduced before semantic ranking completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum DecisionHistoryPolicy {
    /// Latest authorized Jev production decision per predicate and subject.
    #[default]
    LatestAuthorizedProduction,
    /// Latest record per predicate and subject across all producers.
    LatestPerPredicateSubject,
    /// All matching history rows; may truncate when candidate budget is exhausted.
    FullHistory,
}

/// Filters retrieval candidates using persisted Jev decisions only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DecisionSearchFilter {
    predicate: LabPredicate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subject: Option<DecisionSubject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    outcome: Option<DecisionOutcome>,
    producer: JevProducerIdentity,
    #[serde(default)]
    history_policy: DecisionHistoryPolicy,
}

impl DecisionSearchFilter {
    /// Validates predicate, authorized producer, and optional subject/outcome.
    pub fn new(
        predicate: LabPredicate,
        subject: Option<DecisionSubject>,
        outcome: Option<DecisionOutcome>,
        producer: JevProducerIdentity,
        history_policy: DecisionHistoryPolicy,
    ) -> Result<Self, A2aLabError> {
        producer.validate()?;
        if let Some(subject) = &subject
            && !subject.matches_predicate(&predicate)
        {
            return Err(A2aLabError::invalid(
                "subject",
                "is incompatible with the declared predicate",
            ));
        }
        Ok(Self {
            predicate,
            subject,
            outcome,
            producer,
            history_policy,
        })
    }

    /// Predicate id and version.
    #[must_use]
    pub const fn predicate(&self) -> &LabPredicate {
        &self.predicate
    }

    /// Optional exact subject match.
    #[must_use]
    pub const fn subject(&self) -> Option<&DecisionSubject> {
        self.subject.as_ref()
    }

    /// Optional decision outcome filter.
    #[must_use]
    pub const fn outcome(&self) -> Option<DecisionOutcome> {
        self.outcome
    }

    /// Authorized Jev producer stream.
    #[must_use]
    pub const fn producer(&self) -> &JevProducerIdentity {
        &self.producer
    }

    /// History reduction policy.
    #[must_use]
    pub const fn history_policy(&self) -> DecisionHistoryPolicy {
        self.history_policy
    }
}
