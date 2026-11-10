//! Immutable Jev-only frame decision records, evaluator contracts, and ledger boundary.

mod access;
mod api;
mod config;
mod failure;
mod fakes;
mod history;
mod ledger;
mod ledger_provider;
mod limits;
mod predicate;
mod provider;
mod record;
mod request;
mod result;
mod routing;
mod scope;

pub use access::{FrameDecisionClientRole, FrameDecisionOperation, FrameDecisionSchemaRefs};
pub use api::{
    AppendFrameDecisionRequest, AppendFrameDecisionResult, DecisionEvidenceAvailability,
    FrameDecisionCapabilities, FrameDecisionEvaluationOwnership, FrameDecisionHistoryEntry,
    FrameDecisionHistoryResult, GetFrameDecisionCapabilitiesRequest,
    ReadFrameDecisionHistoryRequest, TckMalformedFrameDecisionRequests, map_append_error,
};
pub use config::{
    FrameDecisionEvaluatorCapabilities, FrameDecisionEvaluatorLimits,
    FrameDecisionEvaluatorProviderConfig, JEV_EVALUATOR_PROVIDER_ID, JEV_MAX_EDGE_PIXELS,
    JEV_MAX_IMAGE_BYTES, JEV_MAX_REQUEST_BYTES, JEV_MIN_EDGE_PIXELS,
    RegisteredFrameDecisionEvaluator, RejectedDecisionEvaluatorBackend,
    parse_decision_evaluator_provider, reject_decision_evaluator_backend,
};
pub use failure::FrameDecisionEvaluationFailure;
pub use fakes::{FakeFrameDecisionEvaluator, FakeJevBehavior};
pub use ledger::{FrameDecisionAppendError, FrameDecisionLedger, group_history_by_predicate};
pub use ledger_provider::{
    FakeFrameDecisionLedgerProvider, FrameDecisionLedgerProvider, frame_decision_unavailable,
};
pub use limits::{
    DEFAULT_FRAME_DECISION_HISTORY_ROWS, FrameDecisionReadLimits, MAX_FRAME_DECISION_HISTORY_ROWS,
};
pub use predicate::{
    DecisionEvidenceItem, DecisionEvidenceKind, DecisionSubject, DecisionUnknownReason,
    ForeignObjectSubject, LabPredicate, PREDICATE_FOREIGN_OBJECT_PRESENT,
    PREDICATE_PIPETTE_TIP_ATTACHED, PREDICATE_PLATE_MISALIGNED, PipetteChannelSubject,
    PlateAlignmentReference, PlateSlotSubject,
};
pub use provider::FrameDecisionEvaluator;
pub use record::{
    DEFAULT_JEV_MODEL_REVISION, DecisionEvaluationCoverage, DecisionOutcome, FrameDecisionRecord,
    JEV_PRODUCER_ID, JevEvaluationProvenance, JevProducerIdentity, JevStructuredOutput,
    LabProbabilityVector, RejectedDecisionProvenance, decision_history_fingerprint,
    derive_decision_id, reject_decision_provenance,
};
pub use request::{
    ForbiddenDecisionInput, FrameDecisionEvaluationRequest, OperatorDecisionCriteria,
    OperatorReferenceImage, default_pinned_jev_revision, guard_against_description_evidence,
    reject_forbidden_decision_input,
};
pub use result::{
    DecisionThresholdPolicy, KeyedFrameDecisionEvaluation, interpret_structured_outcome,
    unsupported_predicate_result, validate_evaluation_batch,
};
pub use routing::{FrameDecisionEvaluationRouter, FrameDecisionRoutingAudit};
pub use scope::FrameDecisionReadScope;
