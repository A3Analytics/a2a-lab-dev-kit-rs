//! Generic semantic search over [`crate::videos::SemanticChunkDvrStore`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;

use crate::config::SemanticIndexingConfig;
use crate::error::A2aLabError;
use crate::page::slice_page;
use crate::time::UtcTimestamp;
use crate::videos::digest::Sha256Digest;
use crate::videos::index_provenance::FrameVectorIndexAttachment;
use crate::videos::model_space::ModelSpaceIdentity;
use crate::videos::semantic_dvr::{
    IndexingCheckpoint, ManifestSearchCandidateFilter, MediaPayloadStatus, SemanticChunkDvrStore,
    SemanticFrameManifestRecord, neighbor_window,
};
use crate::videos::semantic_indexing::{
    TextEmbeddingKey, TextEmbeddingProvider, TextEmbeddingRequest,
    deterministic_indexing_providers, text_embedding_limits, validate_embedding_batch,
};
use crate::videos::semantic_search::provenance::SemanticProvenanceSelector;
use crate::videos::semantic_search::provider::SemanticVideoSearchProvider;
use crate::videos::semantic_search::query::SemanticSearchQuery;
use crate::videos::semantic_search::ranking::{
    admits_hybrid_candidate, cmp_fused_rank, fts5_match_query, has_alphanumeric_token,
    lexical_strength, rank_value, reciprocal_rank_fusion, retrieval_score,
};
use crate::videos::semantic_search::request::{
    GetSemanticIndexingCoverageRequest, SearchSemanticFramesRequest,
};
use crate::videos::semantic_search::result::{
    SemanticFrameHit, SemanticFrameIndexState, SemanticFrameMediaAvailability,
    SemanticFramePlaybackNeighbors, SemanticFrameSearchResult, SemanticIndexingCoverage,
};

struct RankedHit {
    lexical_strength: f32,
    rrf: f32,
    record: SemanticFrameManifestRecord,
    attachment: FrameVectorIndexAttachment,
    text: String,
}

/// Store-backed search using one configured embedding identity for index and query.
pub struct ChunkDvrSemanticSearchProvider<E> {
    store: SemanticChunkDvrStore,
    embedder: Arc<E>,
    semantic: SemanticIndexingConfig,
    provenance: SemanticProvenanceSelector,
}

impl<E> ChunkDvrSemanticSearchProvider<E>
where
    E: TextEmbeddingProvider,
{
    /// Binds workers and search to one validated embedding identity.
    pub async fn new(
        store: SemanticChunkDvrStore,
        semantic: SemanticIndexingConfig,
        embedder: E,
    ) -> Result<Self, A2aLabError> {
        semantic.validate_shared_embedding_identity()?;
        let embeddings = semantic
            .embeddings
            .as_ref()
            .ok_or_else(|| A2aLabError::unavailable("semantic vector search is disabled"))?;
        let identity = embeddings.model_space_identity()?;
        if embedder.config().model_space() != &identity
            || embedder.config().provider() != identity.provider()
            || embedder.config().model() != identity.model()
        {
            return Err(A2aLabError::invalid(
                "semantic_indexing.embeddings",
                "index and query embedding identity must match the configured model space",
            ));
        }
        store
            .persist_embedding_config_identity(&identity, UtcTimestamp::now().as_micros())
            .await?;
        let provenance = semantic.active_search_provenance()?;
        Ok(Self {
            store,
            embedder: Arc::new(embedder),
            semantic,
            provenance,
        })
    }

    fn require_shared_identity(
        &self,
        requested: &SemanticProvenanceSelector,
    ) -> Result<&ModelSpaceIdentity, A2aLabError> {
        if requested != &self.provenance {
            return Err(A2aLabError::invalid(
                "provenance",
                "query provenance must match the configured index embedding identity",
            ));
        }
        Ok(self.provenance.model_space())
    }

    async fn embed_query(
        &self,
        request: &SearchSemanticFramesRequest,
    ) -> Result<Vec<f32>, A2aLabError> {
        match request.query() {
            SemanticSearchQuery::Text { text } => {
                let limits = text_embedding_limits(&self.semantic.limits);
                let key = TextEmbeddingKey::SearchQuery(Sha256Digest::hash_bytes(text.as_bytes()));
                let item = TextEmbeddingRequest::new(
                    limits,
                    key,
                    text.clone(),
                    u32::try_from(text.len()).unwrap_or(u32::MAX),
                )
                .map_err(|failure| A2aLabError::invalid("query", failure.to_string()))?;
                let responses = self
                    .embedder
                    .embed_texts(vec![item.clone()])
                    .await
                    .map_err(|failure| A2aLabError::unavailable(failure.to_string()))?;
                let validated = validate_embedding_batch(
                    self.embedder.config(),
                    limits,
                    std::slice::from_ref(&item),
                    &responses,
                )
                .map_err(|failure| A2aLabError::invalid("query", failure.to_string()))?;
                Ok(validated[0].vector().values().to_vec())
            }
            SemanticSearchQuery::ReferenceFrame { frame } => {
                let manifest = self
                    .store
                    .get_manifest_frame(frame.frame_id().as_str())
                    .await?;
                let embedding = manifest.embedding().and_then(|item| item.vector());
                let Some(vector) = embedding else {
                    return Err(A2aLabError::invalid(
                        "reference_frame",
                        "reference revision is not indexed in the active model space",
                    ));
                };
                Ok(vector.values().to_vec())
            }
        }
    }

    async fn rank_ready_candidates(
        &self,
        request: &SearchSemanticFramesRequest,
        space: &ModelSpaceIdentity,
        query: &[f32],
    ) -> Result<(Vec<RankedHit>, bool), A2aLabError> {
        let (vector_records, vector_truncated) =
            self.scan_playable_candidates(request, space).await?;
        let lexical_query = lexical_query_text(request.query());
        let (lexical_records, lexical_truncated) = match lexical_query.and_then(fts5_match_query) {
            Some(match_query) => {
                self.scan_playable_lexical_candidates(request, space, &match_query)
                    .await?
            }
            None => (Vec::new(), false),
        };
        let truncated = vector_truncated || lexical_truncated;
        let lexical_ranks = one_based_lane_ranks(&lexical_records);
        let vector_ids: HashSet<String> = vector_records
            .iter()
            .map(|record| record.frame().frame_id().as_str().to_owned())
            .collect();
        let mut seen = HashSet::new();
        let mut records = Vec::new();
        for record in lexical_records {
            if seen.insert(record.frame().frame_id().as_str().to_owned()) {
                records.push(record);
            }
        }
        for record in vector_records {
            if seen.insert(record.frame().frame_id().as_str().to_owned()) {
                records.push(record);
            }
        }
        let score_floor = self.semantic.limits.search.score_floor();
        let mut scored = Vec::new();
        for record in records {
            let lexical_lane = lexical_ranks.contains_key(record.frame().frame_id().as_str());
            if let Some(hit) = self.scored_hit(
                space,
                query,
                lexical_query,
                score_floor,
                lexical_lane,
                record,
            )? {
                scored.push(hit);
            }
        }
        let vector_ranks = vector_lane_ranks(&scored, &vector_ids);
        let mut ranked = Vec::new();
        for (_, hit) in scored {
            let frame_id = hit.record.frame().frame_id().as_str();
            ranked.push(RankedHit {
                rrf: reciprocal_rank_fusion(
                    lexical_ranks.get(frame_id).copied(),
                    vector_ranks.get(frame_id).copied(),
                ),
                ..hit
            });
        }
        sort_ranked_hits(&mut ranked);
        ranked.truncate(usize::try_from(request.limits().top_k()).unwrap_or(usize::MAX));
        Ok((ranked, truncated))
    }

    fn search_scan_args(
        &self,
        request: &SearchSemanticFramesRequest,
        space: &ModelSpaceIdentity,
    ) -> (i64, i64, ManifestSearchCandidateFilter, u32) {
        let (start_us, end_us) = match request.scope().range() {
            Some(range) => (range.start().as_micros(), range.end().as_micros()),
            None => (i64::MIN / 2, i64::MAX / 2),
        };
        let filter = ManifestSearchCandidateFilter {
            description_generation: self.provenance.description_generation().get(),
            model_space: space.clone(),
        };
        (start_us, end_us, filter, request.limits().candidate_limit())
    }

    async fn scan_playable_candidates(
        &self,
        request: &SearchSemanticFramesRequest,
        space: &ModelSpaceIdentity,
    ) -> Result<(Vec<SemanticFrameManifestRecord>, bool), A2aLabError> {
        let (start_us, end_us, filter, want) = self.search_scan_args(request, space);
        let mut records = Vec::new();
        let mut after: Option<(String, i64)> = None;
        let mut truncated = false;
        loop {
            let scan = self
                .store
                .scan_semantic_search_candidates_after(
                    request.scope().source_id().as_str(),
                    start_us,
                    end_us,
                    &filter,
                    want,
                    after
                        .as_ref()
                        .map(|(frame_id, captured)| (frame_id.as_str(), *captured)),
                )
                .await?;
            let last = scan.records.last().map(|record| {
                (
                    record.frame().frame_id().as_str().to_owned(),
                    record.frame().captured_at().as_micros(),
                )
            });
            truncated |= scan.truncated;
            let playable = self
                .store
                .reconcile_search_media_availability(scan.records)
                .await?;
            records.extend(playable);
            if records.len() >= usize::try_from(want).unwrap_or(usize::MAX) || last.is_none() {
                break;
            }
            if !scan.truncated {
                truncated = false;
                break;
            }
            after = last;
        }
        records.truncate(usize::try_from(want).unwrap_or(usize::MAX));
        Ok((records, truncated))
    }

    async fn scan_playable_lexical_candidates(
        &self,
        request: &SearchSemanticFramesRequest,
        space: &ModelSpaceIdentity,
        match_query: &str,
    ) -> Result<(Vec<SemanticFrameManifestRecord>, bool), A2aLabError> {
        let (start_us, end_us, filter, want) = self.search_scan_args(request, space);
        let mut records = Vec::new();
        let mut offset = 0_u32;
        let mut truncated = false;
        loop {
            let scan = self
                .store
                .scan_semantic_lexical_candidates(
                    request.scope().source_id().as_str(),
                    start_us,
                    end_us,
                    &filter,
                    match_query,
                    want,
                    offset,
                )
                .await?;
            let fetched = u32::try_from(scan.records.len()).unwrap_or(u32::MAX);
            truncated |= scan.truncated;
            let playable = self
                .store
                .reconcile_search_media_availability(scan.records)
                .await?;
            records.extend(playable);
            if records.len() >= usize::try_from(want).unwrap_or(usize::MAX) || fetched == 0 {
                break;
            }
            if !scan.truncated {
                truncated = false;
                break;
            }
            offset = offset.saturating_add(fetched);
        }
        records.truncate(usize::try_from(want).unwrap_or(usize::MAX));
        Ok((records, truncated))
    }

    fn scored_hit(
        &self,
        space: &ModelSpaceIdentity,
        query: &[f32],
        lexical_query: Option<&str>,
        score_floor: f32,
        lexical_lane: bool,
        record: SemanticFrameManifestRecord,
    ) -> Result<Option<(f32, RankedHit)>, A2aLabError> {
        if record.payload_status() != MediaPayloadStatus::Available {
            return Ok(None);
        }
        let Some(description) = record.description() else {
            return Ok(None);
        };
        let Some(stored) = record.embedding() else {
            return Ok(None);
        };
        let Some(vector) = stored.vector() else {
            return Ok(None);
        };
        let attachment = FrameVectorIndexAttachment::new(
            space.clone(),
            description.provenance().clone(),
            stored.provenance().clone(),
            vector.clone(),
        )?;
        if !self.provenance.accepts_attachment(&attachment) {
            return Ok(None);
        }
        let semantic_rank = rank_value(space.metric(), query, vector.values())?;
        let lexical = lexical_query.map_or(0.0, |text| lexical_strength(text, description.text()));
        if !lexical_lane && !admits_hybrid_candidate(semantic_rank, lexical, score_floor) {
            return Ok(None);
        }
        let text = description.text().to_owned();
        Ok(Some((
            semantic_rank,
            RankedHit {
                lexical_strength: lexical,
                rrf: 0.0,
                record,
                attachment,
                text,
            },
        )))
    }

    async fn hit_from_ranked(
        &self,
        request: &SearchSemanticFramesRequest,
        space: &ModelSpaceIdentity,
        query: &[f32],
        record: SemanticFrameManifestRecord,
        attachment: FrameVectorIndexAttachment,
        text: String,
    ) -> Result<SemanticFrameHit, A2aLabError> {
        let neighbors_page = self
            .store
            .list_semantic_playback_neighbors(
                record.frame().source_id().as_str(),
                record.frame().captured_at().as_micros(),
                request.limits().max_neighbor_frames(),
            )
            .await?;
        let neighbor_frames = neighbors_page.items().to_vec();
        let window = neighbor_window(&neighbor_frames, record.frame().captured_at());
        let neighbors = SemanticFramePlaybackNeighbors::new(
            window,
            neighbor_frames,
            request.limits().max_neighbor_frames(),
        )?;
        let media = match record.payload_status() {
            MediaPayloadStatus::Available => SemanticFrameMediaAvailability::Ready,
            MediaPayloadStatus::Missing => SemanticFrameMediaAvailability::Unavailable {
                reason: "payload_missing".to_owned(),
            },
            MediaPayloadStatus::Corrupt => SemanticFrameMediaAvailability::Unavailable {
                reason: "payload_corrupt".to_owned(),
            },
        };
        let indexed = record.description().is_some()
            && record.embedding().and_then(|item| item.vector()).is_some();
        let index = if indexed {
            SemanticFrameIndexState::Ready
        } else if record.description_checkpoint() == IndexingCheckpoint::Failed
            || record.embedding_checkpoint() == IndexingCheckpoint::Failed
        {
            SemanticFrameIndexState::Failed
        } else {
            SemanticFrameIndexState::Pending
        };
        let score = retrieval_score(space.metric(), query, attachment.vector().values())?;
        SemanticFrameHit::new(
            record.frame().clone(),
            text,
            &attachment,
            score,
            index,
            media,
            neighbors,
            None,
        )
    }
}

impl ChunkDvrSemanticSearchProvider<crate::videos::semantic_indexing::FakeTextEmbeddingProvider> {
    /// Deterministic search provider from validated semantic configuration.
    pub async fn deterministic(
        store: SemanticChunkDvrStore,
        semantic: SemanticIndexingConfig,
    ) -> Result<Self, A2aLabError> {
        let providers = deterministic_indexing_providers(&semantic)?;
        Self::new(store, semantic, providers.embedding).await
    }
}

#[async_trait]
impl<E> SemanticVideoSearchProvider for ChunkDvrSemanticSearchProvider<E>
where
    E: TextEmbeddingProvider + 'static,
{
    async fn search_semantic_frames(
        &self,
        request: SearchSemanticFramesRequest,
    ) -> Result<SemanticFrameSearchResult, A2aLabError> {
        request.check()?;
        let space = self.require_shared_identity(request.provenance())?.clone();
        let query = self.embed_query(&request).await?;
        let (ranked, truncated) = self.rank_ready_candidates(&request, &space, &query).await?;
        let mut hits = Vec::new();
        for hit in ranked {
            hits.push(
                self.hit_from_ranked(
                    &request,
                    &space,
                    &query,
                    hit.record,
                    hit.attachment,
                    hit.text,
                )
                .await?,
            );
        }
        let page = slice_page(&hits, request.page())?;
        let (counts, _) = self
            .store
            .semantic_indexing_coverage_snapshot(request.scope())
            .await?;
        Ok(SemanticFrameSearchResult::new(
            page,
            counts.pending() > 0,
            truncated,
        ))
    }

    async fn get_semantic_indexing_coverage(
        &self,
        request: GetSemanticIndexingCoverageRequest,
    ) -> Result<SemanticIndexingCoverage, A2aLabError> {
        let space = self.require_shared_identity(request.provenance())?;
        let (counts, freshness) = self
            .store
            .semantic_indexing_coverage_snapshot(request.scope())
            .await?;
        let complete = counts.pending() == 0 && counts.failed() == 0;
        Ok(SemanticIndexingCoverage::new(
            request.scope().clone(),
            space.clone(),
            counts,
            freshness,
            complete,
        ))
    }

    fn active_provenance(&self) -> Result<SemanticProvenanceSelector, A2aLabError> {
        Ok(self.provenance.clone())
    }
}

fn one_based_lane_ranks(records: &[SemanticFrameManifestRecord]) -> HashMap<String, u32> {
    one_based_lane_ranks_from_ids(
        records
            .iter()
            .map(|record| record.frame().frame_id().as_str().to_owned()),
    )
}

fn vector_lane_ranks(
    scored: &[(f32, RankedHit)],
    vector_ids: &HashSet<String>,
) -> HashMap<String, u32> {
    let mut lane: Vec<(f32, i64, String)> = scored
        .iter()
        .filter(|(_, hit)| vector_ids.contains(hit.record.frame().frame_id().as_str()))
        .map(|(semantic_rank, hit)| {
            (
                *semantic_rank,
                hit.record.frame().captured_at().as_micros(),
                hit.record.frame().frame_id().as_str().to_owned(),
            )
        })
        .collect();
    lane.sort_by(|left, right| {
        right
            .0
            .total_cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    one_based_lane_ranks_from_ids(lane.into_iter().map(|(_, _, frame_id)| frame_id))
}

fn one_based_lane_ranks_from_ids(ids: impl IntoIterator<Item = String>) -> HashMap<String, u32> {
    ids.into_iter()
        .enumerate()
        .map(|(index, frame_id)| (frame_id, u32::try_from(index + 1).unwrap_or(u32::MAX)))
        .collect()
}

fn sort_ranked_hits(ranked: &mut [RankedHit]) {
    ranked.sort_by(|left, right| {
        cmp_fused_rank(
            left.lexical_strength,
            left.rrf,
            right.lexical_strength,
            right.rrf,
        )
        .then_with(|| {
            left.record
                .frame()
                .captured_at()
                .cmp(&right.record.frame().captured_at())
                .then_with(|| {
                    left.record
                        .frame()
                        .frame_id()
                        .as_str()
                        .cmp(right.record.frame().frame_id().as_str())
                })
        })
    });
}

fn lexical_query_text(query: &SemanticSearchQuery) -> Option<&str> {
    match query {
        SemanticSearchQuery::Text { text } => {
            let trimmed = text.trim();
            has_alphanumeric_token(trimmed).then_some(trimmed)
        }
        SemanticSearchQuery::ReferenceFrame { .. } => None,
    }
}
