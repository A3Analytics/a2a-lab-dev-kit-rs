//! Frame-decision ledger and semantic search routing for [`super::A2aLabService`].

use std::sync::Arc;

use crate::error::A2aLabError;
use crate::page::Page;
use crate::videos::{
    AppendFrameDecisionRequest, FrameDecisionLedgerProvider, GetFrameDecisionCapabilitiesRequest,
    ReadFrameDecisionHistoryRequest, SearchSemanticFramesRequest, SemanticFrameSearchResult,
    SemanticSearchCapabilities, authorized_assessments_for_frame, frame_decision_unavailable,
    semantic_search_unavailable,
};

use super::{A2aLabCommand, A2aLabResult, A2aLabService, TaskSnapshot};

impl<L, M, W, I, V> A2aLabService<L, M, W, I, V>
where
    L: crate::logs::LogProvider,
    M: crate::metrics::MetricProvider,
    W: crate::tasks::TaskProvider,
    I: crate::images::ImageProvider,
    V: crate::videos::VideoProvider,
{
    pub(super) fn frame_decisions(
        &self,
    ) -> Result<&Arc<dyn FrameDecisionLedgerProvider>, A2aLabError> {
        self.frame_decisions
            .as_ref()
            .ok_or_else(frame_decision_unavailable)
    }

    pub(super) async fn get_frame_decision_capabilities(
        &self,
        request: GetFrameDecisionCapabilitiesRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        let capabilities = self
            .frame_decisions()?
            .get_frame_decision_capabilities(request)
            .await?;
        self.store_snapshot(A2aLabResult::GetFrameDecisionCapabilities(capabilities))
            .await
    }

    pub(super) async fn append_frame_decision(
        &self,
        request: AppendFrameDecisionRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        request.check()?;
        let result = self
            .frame_decisions()?
            .append_frame_decision(request)
            .await?;
        self.store_snapshot(A2aLabResult::AppendFrameDecision(result))
            .await
    }

    pub(super) async fn read_frame_decision_history(
        &self,
        request: ReadFrameDecisionHistoryRequest,
    ) -> Result<TaskSnapshot, A2aLabError> {
        request.check()?;
        let result = self
            .frame_decisions()?
            .read_frame_decision_history(request)
            .await?;
        self.store_snapshot(A2aLabResult::ReadFrameDecisionHistory(result))
            .await
    }

    pub(super) async fn attach_search_assessments(
        &self,
        request: &SearchSemanticFramesRequest,
        result: SemanticFrameSearchResult,
    ) -> Result<SemanticFrameSearchResult, A2aLabError> {
        let Some(ledger) = self.frame_decisions.as_ref() else {
            return Ok(result);
        };
        let max_rows = request.limits().max_assessment_rows_per_hit();
        let mut items = result.hits().items().to_vec();
        for hit in &mut items {
            if hit.authorized_assessments().is_some() {
                continue;
            }
            let assessments =
                authorized_assessments_for_frame(ledger.as_ref(), hit.frame(), max_rows).await?;
            *hit = hit.clone().with_authorized_assessments(Some(assessments))?;
        }
        Ok(SemanticFrameSearchResult::new(
            Page::new(items, result.hits().next_cursor().map(str::to_owned)),
            result.incomplete_indexing(),
            result.candidate_budget_exhausted(),
        ))
    }

    pub(super) async fn run_semantic_search(
        &self,
        command: A2aLabCommand,
    ) -> Result<TaskSnapshot, A2aLabError> {
        match command {
            A2aLabCommand::GetSemanticSearchCapabilities(_) => {
                let capabilities = match self.semantic_search.as_ref() {
                    Some(provider) => {
                        SemanticSearchCapabilities::registered(provider.active_provenance().ok())
                    }
                    None => SemanticSearchCapabilities::unregistered(),
                };
                self.store_snapshot(A2aLabResult::GetSemanticSearchCapabilities(capabilities))
                    .await
            }
            A2aLabCommand::SearchSemanticFrames(request) => {
                request.check()?;
                let result = self
                    .semantic_search
                    .as_ref()
                    .ok_or_else(semantic_search_unavailable)?
                    .search_semantic_frames(request.clone())
                    .await?;
                let result = self.attach_search_assessments(&request, result).await?;
                self.store_snapshot(A2aLabResult::SearchSemanticFrames(result))
                    .await
            }
            A2aLabCommand::GetSemanticIndexingCoverage(request) => {
                let result = self
                    .semantic_search
                    .as_ref()
                    .ok_or_else(semantic_search_unavailable)?
                    .get_semantic_indexing_coverage(request)
                    .await?;
                self.store_snapshot(A2aLabResult::GetSemanticIndexingCoverage(result))
                    .await
            }
            _ => Err(A2aLabError::protocol("unexpected semantic search command")),
        }
    }
}
