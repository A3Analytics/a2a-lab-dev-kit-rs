//! Typed append and bounded history APIs for authorized Jev decisions.

use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::A2aLabError;
use crate::id::JevEvaluationId;
use crate::page::{Page, PageRequest};
use crate::videos::frame_decision::access::{
    FrameDecisionClientRole, FrameDecisionOperation, FrameDecisionSchemaRefs,
};
use crate::videos::frame_decision::config::FrameDecisionEvaluatorCapabilities;
use crate::videos::frame_decision::ledger::FrameDecisionAppendError;
use crate::videos::frame_decision::limits::FrameDecisionReadLimits;
use crate::videos::frame_decision::predicate::{
    DecisionEvidenceItem, DecisionEvidenceKind, DecisionSubject,
};
use crate::videos::frame_decision::record::{FrameDecisionRecord, JevProducerIdentity};
use crate::videos::frame_decision::scope::FrameDecisionReadScope;
use crate::videos::semantic_search::{DecisionHistoryPolicy, SemanticSearchScope};
use crate::{DecisionOutcome, LabPredicate};

/// Binds append requests to the owned Jev evaluation id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDecisionEvaluationOwnership {
    evaluation_id: JevEvaluationId,
}

impl FrameDecisionEvaluationOwnership {
    /// Validates a non-empty evaluation id.
    #[must_use]
    pub fn new(evaluation_id: JevEvaluationId) -> Self {
        Self { evaluation_id }
    }

    /// Owned Jev evaluation id.
    #[must_use]
    pub const fn evaluation_id(&self) -> &JevEvaluationId {
        &self.evaluation_id
    }
}

/// Authorized production append payload.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AppendFrameDecisionRequest {
    record: FrameDecisionRecord,
    ownership: FrameDecisionEvaluationOwnership,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppendFrameDecisionRequestRaw {
    record: FrameDecisionRecord,
    ownership: FrameDecisionEvaluationOwnership,
}

impl AppendFrameDecisionRequest {
    /// Validates record shape, Jev provenance, and evaluation ownership.
    pub fn new(
        record: FrameDecisionRecord,
        ownership: FrameDecisionEvaluationOwnership,
    ) -> Result<Self, A2aLabError> {
        let request = Self { record, ownership };
        request.check()?;
        Ok(request)
    }

    /// Rejects forged labels, non-Jev producers, and ownership mismatch.
    pub fn check(&self) -> Result<(), A2aLabError> {
        if self.ownership.evaluation_id() != self.record.provenance().evaluation_id() {
            return Err(A2aLabError::invalid(
                "ownership",
                "evaluation_id must match record provenance",
            ));
        }
        self.record.validate()?;
        Ok(())
    }

    /// Production decision record.
    #[must_use]
    pub const fn record(&self) -> &FrameDecisionRecord {
        &self.record
    }

    /// Evaluation ownership binding.
    #[must_use]
    pub const fn ownership(&self) -> &FrameDecisionEvaluationOwnership {
        &self.ownership
    }
}

impl TryFrom<AppendFrameDecisionRequestRaw> for AppendFrameDecisionRequest {
    type Error = A2aLabError;

    fn try_from(raw: AppendFrameDecisionRequestRaw) -> Result<Self, Self::Error> {
        Self::new(raw.record, raw.ownership)
    }
}

impl<'de> Deserialize<'de> for AppendFrameDecisionRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        AppendFrameDecisionRequestRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// Append outcome preserving idempotent replay semantics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AppendFrameDecisionResult {
    record: FrameDecisionRecord,
    replayed: bool,
}

impl AppendFrameDecisionResult {
    /// Returns the stored record and whether this call was an idempotent replay.
    #[must_use]
    pub const fn new(record: FrameDecisionRecord, replayed: bool) -> Self {
        Self { record, replayed }
    }

    /// Stored or replayed decision record.
    #[must_use]
    pub const fn record(&self) -> &FrameDecisionRecord {
        &self.record
    }

    /// True when an identical payload was already stored.
    #[must_use]
    pub const fn replayed(&self) -> bool {
        self.replayed
    }
}

/// Evidence availability reported without mutating stored outcomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionEvidenceAvailability {
    /// Evidence bytes remain resolvable for this item.
    Ready,
    /// Evidence is missing or unavailable; the decision outcome is unchanged.
    Unavailable {
        /// Stable explanation for operators.
        reason: String,
    },
}

/// One history row plus per-evidence availability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDecisionHistoryEntry {
    record: FrameDecisionRecord,
    evidence: Vec<DecisionEvidenceAvailability>,
}

impl FrameDecisionHistoryEntry {
    /// Builds an entry with explicit evidence availability reports.
    #[must_use]
    pub fn new(record: FrameDecisionRecord, evidence: Vec<DecisionEvidenceAvailability>) -> Self {
        Self { record, evidence }
    }

    /// Immutable decision record.
    #[must_use]
    pub const fn record(&self) -> &FrameDecisionRecord {
        &self.record
    }

    /// Availability aligned with `record.evidence()` order.
    #[must_use]
    pub fn evidence_availability(&self) -> &[DecisionEvidenceAvailability] {
        &self.evidence
    }
}

/// Bounded history read request.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ReadFrameDecisionHistoryRequest {
    scope: FrameDecisionReadScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    predicate: Option<LabPredicate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subject: Option<DecisionSubject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    producer: Option<JevProducerIdentity>,
    history_policy: DecisionHistoryPolicy,
    limits: FrameDecisionReadLimits,
    page: PageRequest,
    #[serde(default)]
    report_unavailable_evidence: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadFrameDecisionHistoryRequestRaw {
    scope: FrameDecisionReadScope,
    #[serde(default)]
    predicate: Option<LabPredicate>,
    #[serde(default)]
    subject: Option<DecisionSubject>,
    #[serde(default)]
    producer: Option<JevProducerIdentity>,
    history_policy: DecisionHistoryPolicy,
    limits: FrameDecisionReadLimits,
    page: PageRequest,
    #[serde(default)]
    report_unavailable_evidence: Vec<String>,
}

impl ReadFrameDecisionHistoryRequest {
    /// Validates scope, filters, paging, and producer identity.
    pub fn new(
        scope: FrameDecisionReadScope,
        predicate: Option<LabPredicate>,
        subject: Option<DecisionSubject>,
        producer: Option<JevProducerIdentity>,
        history_policy: DecisionHistoryPolicy,
        limits: FrameDecisionReadLimits,
        page: PageRequest,
        report_unavailable_evidence: Vec<String>,
    ) -> Result<Self, A2aLabError> {
        let request = Self {
            scope,
            predicate,
            subject,
            producer,
            history_policy,
            limits,
            page,
            report_unavailable_evidence,
        };
        request.check()?;
        Ok(request)
    }

    /// Rejects malformed scope, subject/predicate mismatch, and page overflow.
    pub fn check(&self) -> Result<(), A2aLabError> {
        self.scope.range().check()?;
        self.limits.check()?;
        self.page.check()?;
        if self.page.limit() > self.limits.max_rows() {
            return Err(A2aLabError::invalid(
                "page.limit",
                "must not exceed max_rows",
            ));
        }
        if let Some(producer) = &self.producer {
            producer.validate()?;
        }
        if let (Some(predicate), Some(subject)) = (&self.predicate, &self.subject)
            && !subject.matches_predicate(predicate)
        {
            return Err(A2aLabError::invalid(
                "subject",
                "is incompatible with the declared predicate",
            ));
        }
        Ok(())
    }

    /// Bounded source/time/frame scope.
    #[must_use]
    pub const fn scope(&self) -> &FrameDecisionReadScope {
        &self.scope
    }

    /// Optional predicate filter.
    #[must_use]
    pub const fn predicate(&self) -> Option<&LabPredicate> {
        self.predicate.as_ref()
    }

    /// Optional subject filter.
    #[must_use]
    pub const fn subject(&self) -> Option<&DecisionSubject> {
        self.subject.as_ref()
    }

    /// Optional authorized producer filter.
    #[must_use]
    pub const fn producer(&self) -> Option<&JevProducerIdentity> {
        self.producer.as_ref()
    }

    /// Explicit history reduction policy.
    #[must_use]
    pub const fn history_policy(&self) -> DecisionHistoryPolicy {
        self.history_policy
    }

    /// Row budget.
    #[must_use]
    pub const fn limits(&self) -> FrameDecisionReadLimits {
        self.limits
    }

    /// Result paging.
    #[must_use]
    pub const fn page(&self) -> &PageRequest {
        &self.page
    }

    /// Evidence item keys that should be reported unavailable without changing outcomes.
    #[must_use]
    pub fn report_unavailable_evidence(&self) -> &[String] {
        &self.report_unavailable_evidence
    }
}

impl TryFrom<ReadFrameDecisionHistoryRequestRaw> for ReadFrameDecisionHistoryRequest {
    type Error = A2aLabError;

    fn try_from(raw: ReadFrameDecisionHistoryRequestRaw) -> Result<Self, Self::Error> {
        Self::new(
            raw.scope,
            raw.predicate,
            raw.subject,
            raw.producer,
            raw.history_policy,
            raw.limits,
            raw.page,
            raw.report_unavailable_evidence,
        )
    }
}

impl<'de> Deserialize<'de> for ReadFrameDecisionHistoryRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ReadFrameDecisionHistoryRequestRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// One page of decision history plus filter metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDecisionHistoryResult {
    scope: FrameDecisionReadScope,
    history_policy: DecisionHistoryPolicy,
    entries: Page<FrameDecisionHistoryEntry>,
}

impl FrameDecisionHistoryResult {
    /// Returns one page of history entries.
    #[must_use]
    pub fn new(
        scope: FrameDecisionReadScope,
        history_policy: DecisionHistoryPolicy,
        entries: Page<FrameDecisionHistoryEntry>,
    ) -> Self {
        Self {
            scope,
            history_policy,
            entries,
        }
    }

    /// Request scope echoed for traceability.
    #[must_use]
    pub const fn scope(&self) -> &FrameDecisionReadScope {
        &self.scope
    }

    /// Applied reduction policy.
    #[must_use]
    pub const fn history_policy(&self) -> DecisionHistoryPolicy {
        self.history_policy
    }

    /// History page.
    #[must_use]
    pub const fn entries(&self) -> &Page<FrameDecisionHistoryEntry> {
        &self.entries
    }
}

/// Empty capability discovery request.
///
/// Uses an empty object on the wire so MCP `inputSchema` roots as `object`, not `null`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct GetFrameDecisionCapabilitiesRequest {}

impl GetFrameDecisionCapabilitiesRequest {
    /// Default empty request accepted by A2A and MCP adapters.
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }
}

/// Capability discovery for append/read transport APIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameDecisionCapabilities {
    registered: bool,
    client_role: FrameDecisionClientRole,
    operations: Vec<FrameDecisionOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    evaluator: Option<FrameDecisionEvaluatorCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    schemas: Option<FrameDecisionSchemaRefs>,
}

impl FrameDecisionCapabilities {
    /// Unregistered provider: no schemas or operations.
    #[must_use]
    pub fn unregistered(client_role: FrameDecisionClientRole) -> Self {
        Self {
            registered: false,
            client_role,
            operations: Vec::new(),
            evaluator: None,
            schemas: None,
        }
    }

    /// Registered provider exposing schemas for the caller role.
    #[must_use]
    pub fn registered(
        client_role: FrameDecisionClientRole,
        evaluator: FrameDecisionEvaluatorCapabilities,
    ) -> Self {
        let mut operations = vec![FrameDecisionOperation::ReadFrameDecisionHistory];
        if client_role == FrameDecisionClientRole::AuthorizedProducer {
            operations.push(FrameDecisionOperation::AppendFrameDecision);
        }
        Self {
            registered: true,
            client_role,
            operations,
            evaluator: Some(evaluator),
            schemas: Some(FrameDecisionSchemaRefs::registered()),
        }
    }

    /// Whether a ledger provider is configured.
    #[must_use]
    pub const fn is_registered(&self) -> bool {
        self.registered
    }

    /// Resolved caller role.
    #[must_use]
    pub const fn client_role(&self) -> FrameDecisionClientRole {
        self.client_role
    }

    /// Advertised append/read operations for this role.
    #[must_use]
    pub fn operations(&self) -> &[FrameDecisionOperation] {
        &self.operations
    }

    /// Registered Jev evaluator metadata when available.
    #[must_use]
    pub const fn evaluator(&self) -> Option<&FrameDecisionEvaluatorCapabilities> {
        self.evaluator.as_ref()
    }

    /// Append/read JSON Schema type names when registered.
    #[must_use]
    pub const fn schemas(&self) -> Option<&FrameDecisionSchemaRefs> {
        self.schemas.as_ref()
    }
}

/// Maps append validation failures into stable lab errors.
#[must_use]
pub fn map_append_error(error: FrameDecisionAppendError) -> A2aLabError {
    match error {
        FrameDecisionAppendError::IdempotencyConflict { key, existing } => A2aLabError::invalid(
            "idempotency_key",
            format!(
                "conflicts with existing decision {} for key {}",
                existing.as_str(),
                key.as_str()
            ),
        ),
        FrameDecisionAppendError::InvalidSupersession { prior, message } => {
            A2aLabError::invalid("supersedes", format!("prior {}: {message}", prior.as_str()))
        }
        FrameDecisionAppendError::Validation(inner) => inner,
    }
}

/// Evidence availability for one indexed evidence item.
#[must_use]
pub fn evidence_availability_for(
    item: &DecisionEvidenceItem,
    unavailable_keys: &[String],
) -> DecisionEvidenceAvailability {
    let key = evidence_report_key(item);
    if unavailable_keys.iter().any(|candidate| candidate == &key) {
        DecisionEvidenceAvailability::Unavailable {
            reason: "retained evidence bytes are not currently resolvable".to_owned(),
        }
    } else {
        DecisionEvidenceAvailability::Ready
    }
}

fn evidence_report_key(item: &DecisionEvidenceItem) -> String {
    let kind = match item.kind() {
        DecisionEvidenceKind::SavedFrameRevision => "saved_frame_revision",
        DecisionEvidenceKind::ReferenceImage => "reference_image",
    };
    format!("{}:{kind}", item.frame_id().as_str())
}

/// Constructors reserved for malformed frame decision conformance fixtures.
pub struct TckMalformedFrameDecisionRequests;

impl TckMalformedFrameDecisionRequests {
    /// Read request with zero page limit.
    #[must_use]
    pub fn history_zero_page_limit(
        scope: FrameDecisionReadScope,
    ) -> ReadFrameDecisionHistoryRequest {
        ReadFrameDecisionHistoryRequest {
            scope,
            predicate: None,
            subject: None,
            producer: None,
            history_policy: DecisionHistoryPolicy::FullHistory,
            limits: FrameDecisionReadLimits::pilot(),
            page: PageRequest::tck_zero_limit(),
            report_unavailable_evidence: Vec::new(),
        }
    }

    /// Read request whose page limit exceeds `max_rows`.
    #[must_use]
    pub fn history_page_exceeds_max_rows(
        scope: FrameDecisionReadScope,
    ) -> ReadFrameDecisionHistoryRequest {
        ReadFrameDecisionHistoryRequest {
            scope,
            predicate: None,
            subject: None,
            producer: None,
            history_policy: DecisionHistoryPolicy::FullHistory,
            limits: FrameDecisionReadLimits::new(10).unwrap(),
            page: PageRequest::new(None, 50).unwrap(),
            report_unavailable_evidence: Vec::new(),
        }
    }

    /// Semantic scope reused to build an invalid time range for decision history.
    #[must_use]
    pub fn history_reversed_range(
        semantic_scope: &SemanticSearchScope,
    ) -> ReadFrameDecisionHistoryRequest {
        let scope = FrameDecisionReadScope::tck_reversed_range(
            semantic_scope.source_id().clone(),
            semantic_scope.range(),
        );
        Self::history_zero_page_limit(scope)
    }
}

/// Optional outcome filter used when reducing latest-per-producer views.
#[must_use]
#[allow(dead_code)]
pub fn outcome_matches(record: &FrameDecisionRecord, outcome: Option<DecisionOutcome>) -> bool {
    outcome.is_none_or(|expected| record.outcome() == expected)
}
