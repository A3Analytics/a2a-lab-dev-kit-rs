//! Generic semantic search over [`crate::videos::SemanticChunkDvrStore`].

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
    neighbor_window,
};
use crate::videos::semantic_indexing::{
    TextEmbeddingKey, TextEmbeddingProvider, TextEmbeddingRequest,
    deterministic_indexing_providers, text_embedding_limits, validate_embedding_batch,
};
use crate::videos::semantic_search::provenance::SemanticProvenanceSelector;
use crate::videos::semantic_search::provider::SemanticVideoSearchProvider;
use crate::videos::semantic_search::query::SemanticSearchQuery;
use crate::videos::semantic_search::ranking::{rank_value, retrieval_score};
use crate::videos::semantic_search::request::{
    GetSemanticIndexingCoverageRequest, SearchSemanticFramesRequest,
};
use crate::videos::semantic_search::result::{
    SemanticFrameHit, SemanticFrameIndexState, SemanticFrameMediaAvailability,
    SemanticFramePlaybackNeighbors, SemanticFrameSearchResult, SemanticIndexingCoverage,
};

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
    ) -> Result<
        (
            Vec<(
                f32,
                crate::videos::semantic_dvr::SemanticFrameManifestRecord,
                FrameVectorIndexAttachment,
                String,
            )>,
            bool,
        ),
        A2aLabError,
    > {
        let (start_us, end_us) = match request.scope().range() {
            Some(range) => (range.start().as_micros(), range.end().as_micros()),
            None => (i64::MIN / 2, i64::MAX / 2),
        };
        let filter = ManifestSearchCandidateFilter {
            description_generation: self.provenance.description_generation().get(),
            model_space: space.clone(),
        };
        let want = request.limits().candidate_limit();
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
        let mut ranked = Vec::new();
        for record in records {
            if record.payload_status() != MediaPayloadStatus::Available {
                continue;
            }
            let Some(description) = record.description() else {
                continue;
            };
            let Some(stored) = record.embedding() else {
                continue;
            };
            let Some(vector) = stored.vector() else {
                continue;
            };
            let attachment = FrameVectorIndexAttachment::new(
                space.clone(),
                description.provenance().clone(),
                stored.provenance().clone(),
                vector.clone(),
            )?;
            if !self.provenance.accepts_attachment(&attachment) {
                continue;
            }
            let rank = rank_value(space.metric(), query, vector.values())?;
            let text = description.text().to_owned();
            ranked.push((rank, record, attachment, text));
        }
        ranked.sort_by(|left, right| {
            right.0.total_cmp(&left.0).then_with(|| {
                left.1
                    .frame()
                    .captured_at()
                    .cmp(&right.1.frame().captured_at())
                    .then_with(|| {
                        left.1
                            .frame()
                            .frame_id()
                            .as_str()
                            .cmp(right.1.frame().frame_id().as_str())
                    })
            })
        });
        ranked.truncate(usize::try_from(request.limits().top_k()).unwrap_or(usize::MAX));
        Ok((ranked, truncated))
    }

    async fn hit_from_ranked(
        &self,
        request: &SearchSemanticFramesRequest,
        space: &ModelSpaceIdentity,
        query: &[f32],
        record: crate::videos::semantic_dvr::SemanticFrameManifestRecord,
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
        for (_, record, attachment, text) in ranked {
            hits.push(
                self.hit_from_ranked(&request, &space, &query, record, attachment, text)
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
