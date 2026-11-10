//! Optional ledger provider for append and bounded history APIs.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::error::A2aLabError;
use crate::page::Page;
use crate::videos::frame_decision::access::{FrameDecisionClientRole, FrameDecisionOperation};
use crate::videos::frame_decision::api::{
    AppendFrameDecisionRequest, AppendFrameDecisionResult, FrameDecisionCapabilities,
    FrameDecisionHistoryEntry, FrameDecisionHistoryResult, GetFrameDecisionCapabilitiesRequest,
    ReadFrameDecisionHistoryRequest, evidence_availability_for, map_append_error,
};
use crate::videos::frame_decision::config::{
    FrameDecisionEvaluatorLimits, FrameDecisionEvaluatorProviderConfig,
    RegisteredFrameDecisionEvaluator,
};
use crate::videos::frame_decision::history::{filter_records, reduce_history};
use crate::videos::frame_decision::ledger::FrameDecisionLedger;
use crate::videos::frame_decision::record::FrameDecisionRecord;

/// Append/read boundary for configured Jev ledger backends.
#[async_trait]
pub trait FrameDecisionLedgerProvider: Send + Sync {
    /// Resolved caller role for capability discovery.
    fn client_role(&self) -> FrameDecisionClientRole;

    /// Advertises append/read schemas for the active role.
    async fn get_frame_decision_capabilities(
        &self,
        _request: GetFrameDecisionCapabilitiesRequest,
    ) -> Result<FrameDecisionCapabilities, A2aLabError>;

    /// Appends one authorized Jev production record.
    async fn append_frame_decision(
        &self,
        request: AppendFrameDecisionRequest,
    ) -> Result<AppendFrameDecisionResult, A2aLabError>;

    /// Reads bounded decision history inside one scope.
    async fn read_frame_decision_history(
        &self,
        request: ReadFrameDecisionHistoryRequest,
    ) -> Result<FrameDecisionHistoryResult, A2aLabError>;
}

/// Stable unavailable error for absent frame decision APIs.
#[must_use]
pub fn frame_decision_unavailable() -> A2aLabError {
    A2aLabError::unavailable("frame decision ledger is not available")
}

/// In-memory provider for contract tests and local labs.
#[derive(Debug, Clone)]
pub struct FakeFrameDecisionLedgerProvider {
    role: FrameDecisionClientRole,
    registered: Option<RegisteredFrameDecisionEvaluator>,
    ledger: Arc<Mutex<FrameDecisionLedger>>,
}

impl FakeFrameDecisionLedgerProvider {
    /// Read-only consumer with no append authority.
    pub fn read_only_consumer(
        config: &FrameDecisionEvaluatorProviderConfig,
        limits: FrameDecisionEvaluatorLimits,
    ) -> Result<Self, A2aLabError> {
        Ok(Self {
            role: FrameDecisionClientRole::ReadOnlyConsumer,
            registered: Some(RegisteredFrameDecisionEvaluator::register(config, limits)?),
            ledger: Arc::new(Mutex::new(FrameDecisionLedger::new())),
        })
    }

    /// Authorized producer that may append and read.
    pub fn authorized_producer(
        config: &FrameDecisionEvaluatorProviderConfig,
        limits: FrameDecisionEvaluatorLimits,
    ) -> Result<Self, A2aLabError> {
        Ok(Self {
            role: FrameDecisionClientRole::AuthorizedProducer,
            registered: Some(RegisteredFrameDecisionEvaluator::register(config, limits)?),
            ledger: Arc::new(Mutex::new(FrameDecisionLedger::new())),
        })
    }

    /// Unregistered provider used for unavailable capability checks.
    #[must_use]
    pub fn unregistered(role: FrameDecisionClientRole) -> Self {
        Self {
            role,
            registered: None,
            ledger: Arc::new(Mutex::new(FrameDecisionLedger::new())),
        }
    }

    /// Seeds one record without transport authorization checks.
    pub fn seed(&self, record: FrameDecisionRecord) -> Result<(), A2aLabError> {
        self.ledger
            .lock()
            .map_err(|_| A2aLabError::unavailable("ledger lock poisoned"))?
            .append(record)
            .map(|_| ())
            .map_err(map_append_error)
    }

    fn registered_capabilities(&self) -> FrameDecisionCapabilities {
        let Some(registered) = &self.registered else {
            return FrameDecisionCapabilities::unregistered(self.role);
        };
        FrameDecisionCapabilities::registered(self.role, registered.capabilities().clone())
    }
}

#[async_trait]
impl FrameDecisionLedgerProvider for FakeFrameDecisionLedgerProvider {
    fn client_role(&self) -> FrameDecisionClientRole {
        self.role
    }

    async fn get_frame_decision_capabilities(
        &self,
        _request: GetFrameDecisionCapabilitiesRequest,
    ) -> Result<FrameDecisionCapabilities, A2aLabError> {
        Ok(self.registered_capabilities())
    }

    async fn append_frame_decision(
        &self,
        request: AppendFrameDecisionRequest,
    ) -> Result<AppendFrameDecisionResult, A2aLabError> {
        if self.registered.is_none() {
            return Err(frame_decision_unavailable());
        }
        if self.role != FrameDecisionClientRole::AuthorizedProducer {
            return Err(A2aLabError::invalid(
                "client_role",
                "append requires an authorized Jev producer",
            ));
        }
        if !self
            .registered_capabilities()
            .operations()
            .contains(&FrameDecisionOperation::AppendFrameDecision)
        {
            return Err(A2aLabError::invalid(
                "operation",
                "append is not advertised for this caller",
            ));
        }
        request.check()?;
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| A2aLabError::unavailable("ledger lock poisoned"))?;
        let prior = ledger
            .get_by_idempotency(request.record().idempotency_key())
            .map(|record| record.decision_id().clone());
        let stored = ledger
            .append(request.record().clone())
            .map_err(map_append_error)?;
        let replayed = prior.is_some_and(|id| id == *stored.decision_id());
        Ok(AppendFrameDecisionResult::new(stored.clone(), replayed))
    }

    async fn read_frame_decision_history(
        &self,
        request: ReadFrameDecisionHistoryRequest,
    ) -> Result<FrameDecisionHistoryResult, A2aLabError> {
        if self.registered.is_none() {
            return Err(frame_decision_unavailable());
        }
        request.check()?;
        let ledger = self
            .ledger
            .lock()
            .map_err(|_| A2aLabError::unavailable("ledger lock poisoned"))?;
        let ordered: Vec<_> = ledger
            .history()
            .iter()
            .filter_map(|id| ledger.get(id))
            .collect();
        let filtered = filter_records(
            ordered.into_iter(),
            request.scope(),
            request.predicate(),
            request.subject(),
            request.producer(),
            None,
        );
        let reduced = reduce_history(filtered, request.history_policy(), request.producer());
        let total = reduced.len();
        let offset = request.page().offset()?;
        let limit = request.page().limit() as usize;
        let page_items: Vec<FrameDecisionHistoryEntry> = reduced
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|record| {
                let evidence = record
                    .evidence()
                    .iter()
                    .map(|item| {
                        evidence_availability_for(item, request.report_unavailable_evidence())
                    })
                    .collect();
                FrameDecisionHistoryEntry::new(record.clone(), evidence)
            })
            .collect();
        let next_offset = offset + page_items.len();
        let cursor = if next_offset < total {
            Some(next_offset.to_string())
        } else {
            None
        };
        Ok(FrameDecisionHistoryResult::new(
            request.scope().clone(),
            request.history_policy(),
            Page::new(page_items, cursor),
        ))
    }
}
