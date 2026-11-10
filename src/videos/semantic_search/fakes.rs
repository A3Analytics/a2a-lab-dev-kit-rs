//! Deterministic semantic search doubles for contract tests.

use std::collections::HashMap;

use async_trait::async_trait;

use crate::error::A2aLabError;
use crate::page::slice_page;
use crate::videos::index_provenance::FrameVectorIndexAttachment;
use crate::videos::semantic_search::provider::SemanticVideoSearchProvider;
use crate::videos::semantic_search::query::SemanticSearchQuery;
use crate::videos::semantic_search::request::{
    GetSemanticIndexingCoverageRequest, SearchSemanticFramesRequest,
};
use crate::videos::semantic_search::result::{
    SemanticFrameHit, SemanticFrameIndexState, SemanticFrameMediaAvailability,
    SemanticFramePlaybackNeighbors, SemanticFrameSearchResult, SemanticIndexingCoverage,
    SemanticIndexingFreshness, SemanticIndexingStateCounts, SemanticRetrievalScore,
};

/// Prepared hit keyed by `frame_id` for deterministic tests.
#[derive(Debug, Clone)]
pub struct FakeSemanticHit {
    /// Complete hit payload.
    pub hit: SemanticFrameHit,
    /// Attachment used to validate provenance selectors.
    pub attachment: FrameVectorIndexAttachment,
    /// Description text matching the attachment digest pipeline in tests.
    pub description_text: String,
}

/// In-memory semantic search provider with explicit capability.
#[derive(Debug, Default)]
pub struct FakeSemanticVideoSearchProvider {
    available: bool,
    hits: HashMap<String, FakeSemanticHit>,
    coverage: Option<SemanticIndexingCoverage>,
}

impl FakeSemanticVideoSearchProvider {
    /// Provider that reports unavailable for capability-absent tests.
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            available: false,
            hits: HashMap::new(),
            coverage: None,
        }
    }

    /// Provider that serves indexed hits.
    #[must_use]
    pub fn available() -> Self {
        Self {
            available: true,
            hits: HashMap::new(),
            coverage: None,
        }
    }

    /// Registers one hit keyed by frame id.
    pub fn insert_hit(&mut self, prepared: FakeSemanticHit) {
        let key = prepared.hit.frame().frame_id().as_str().to_owned();
        self.hits.insert(key, prepared);
    }

    /// Sets the coverage snapshot returned by [`Self::get_semantic_indexing_coverage`].
    pub fn set_coverage(&mut self, coverage: SemanticIndexingCoverage) {
        self.coverage = Some(coverage);
    }
}

#[async_trait]
impl SemanticVideoSearchProvider for FakeSemanticVideoSearchProvider {
    async fn search_semantic_frames(
        &self,
        request: SearchSemanticFramesRequest,
    ) -> Result<SemanticFrameSearchResult, A2aLabError> {
        if !self.available {
            return Err(super::provider::semantic_search_unavailable());
        }
        request.check()?;
        let selector = request.provenance();
        let mut ranked = Vec::new();
        for prepared in self.hits.values() {
            if prepared.hit.frame().source_id() != request.scope().source_id() {
                continue;
            }
            let captured = prepared.hit.captured_at();
            if captured < request.scope().range().start()
                || captured >= request.scope().range().end()
            {
                continue;
            }
            if !selector.accepts_attachment(&prepared.attachment) {
                continue;
            }
            if let SemanticSearchQuery::Text { text } = request.query()
                && !prepared.description_text.contains(text.trim())
            {
                continue;
            }
            if let SemanticSearchQuery::ReferenceFrame { frame } = request.query()
                && frame.frame_id() != prepared.hit.frame().frame_id()
            {
                continue;
            }
            ranked.push(prepared.hit.clone());
        }
        ranked.sort_by(|left, right| {
            right
                .retrieval()
                .value()
                .total_cmp(&left.retrieval().value())
        });
        ranked.truncate(usize::try_from(request.limits().top_k()).unwrap_or(usize::MAX));
        let page = slice_page(&ranked, request.page())?;
        let incomplete = self
            .coverage
            .as_ref()
            .is_some_and(|coverage| !coverage.scope_complete() || coverage.counts().pending() > 0);
        Ok(SemanticFrameSearchResult::new(page, incomplete, false))
    }

    async fn get_semantic_indexing_coverage(
        &self,
        request: GetSemanticIndexingCoverageRequest,
    ) -> Result<SemanticIndexingCoverage, A2aLabError> {
        if !self.available {
            return Err(super::provider::semantic_search_unavailable());
        }
        if let Some(coverage) = &self.coverage
            && coverage.scope() == request.scope()
            && coverage.model_space() == request.provenance().model_space()
        {
            return Ok(coverage.clone());
        }
        let counts = SemanticIndexingStateCounts::new(0, self.hits.len() as u64, 0, 0);
        Ok(SemanticIndexingCoverage::new(
            request.scope().clone(),
            request.provenance().model_space().clone(),
            counts,
            SemanticIndexingFreshness::default(),
            true,
        ))
    }
}

/// Builds a deterministic hit for fake providers.
pub fn fake_semantic_hit(
    frame: crate::videos::saved_frame::SavedFrameRef,
    attachment: FrameVectorIndexAttachment,
    description: &str,
    score: f32,
    neighbors: SemanticFramePlaybackNeighbors,
) -> Result<FakeSemanticHit, A2aLabError> {
    let retrieval = SemanticRetrievalScore::new(score, attachment.model_space().metric())?;
    let hit = SemanticFrameHit::new(
        frame,
        description,
        &attachment,
        retrieval,
        SemanticFrameIndexState::Ready,
        SemanticFrameMediaAvailability::Ready,
        neighbors,
    )?;
    Ok(FakeSemanticHit {
        hit: hit.clone(),
        attachment,
        description_text: description.to_owned(),
    })
}
