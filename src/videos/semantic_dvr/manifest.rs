//! Manifest persistence against [`SemanticChunkDvrStore`].

use sqlx::Row;
use sqlx::{Sqlite, Transaction};

use crate::{
    A2aLabError, DvrStoreNamespace, DvrTrackId, FrameDescriptionProvenance, FrameEmbeddingVector,
    FrameVectorIndexAttachment, ModelSpaceIdentity, Page, SavedFrameRef, Sha256Digest,
    TextEmbeddingProvenance, UtcTimestamp, VideoSourceId,
};

use super::db::db_error;
use super::engine::SemanticChunkDvrStore;
use super::manifest_types::{
    IndexingCheckpoint, ListManifestFramesRequest, ManifestRetainedState,
    ManifestSearchCandidateFilter, ManifestSearchCandidateScan, MediaPayloadStatus,
    RegisterManifestOutcome, SemanticFrameManifestRecord, StoredFrameDescription,
    StoredFrameEmbedding, parse_manifest_cursor,
};

impl SemanticChunkDvrStore {
    /// Registers or reconciles the manifest revision for one committed DVR frame key.
    pub async fn register_manifest_at_dvr_key(
        &self,
        store: &DvrStoreNamespace,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        start_us: UtcTimestamp,
    ) -> Result<(SavedFrameRef, RegisterManifestOutcome), A2aLabError> {
        let (payload, crc, end_us) = self
            .load_dvr_payload_at_key(source_id, track_id, start_us)
            .await?;
        let frame = SavedFrameRef::from_payload_bytes(
            store.clone(),
            source_id.clone(),
            track_id.clone(),
            start_us,
            &payload,
            None,
        )?;
        let outcome = self
            .insert_manifest_revision(&frame, end_us, crc, RegisterConflictPolicy::Default)
            .await?;
        Ok((frame, outcome))
    }

    /// Persists Luna description text and provenance for one frame revision.
    pub async fn upsert_frame_description(
        &self,
        frame: &SavedFrameRef,
        text: impl AsRef<str>,
        provenance: &FrameDescriptionProvenance,
    ) -> Result<(), A2aLabError> {
        if provenance.text_hash() != &Sha256Digest::hash_bytes(text.as_ref().as_bytes()) {
            return Err(A2aLabError::invalid(
                "text_hash",
                "does not match description text",
            ));
        }
        self.require_manifest_frame(frame.frame_id().as_str())
            .await?;
        let roi = provenance.frame_roi().map(crate::FrameRoiTransform::bounds);
        sqlx::query(
            "INSERT INTO semantic_frame_descriptions (
                frame_id, description_text, description_provider, description_model,
                prompt_digest, preprocessing_digest, roi_x_min, roi_y_min, roi_x_max, roi_y_max,
                text_hash, description_generation
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(frame_id) DO UPDATE SET
                description_text = excluded.description_text,
                description_provider = excluded.description_provider,
                description_model = excluded.description_model,
                prompt_digest = excluded.prompt_digest,
                preprocessing_digest = excluded.preprocessing_digest,
                roi_x_min = excluded.roi_x_min,
                roi_y_min = excluded.roi_y_min,
                roi_x_max = excluded.roi_x_max,
                roi_y_max = excluded.roi_y_max,
                text_hash = excluded.text_hash,
                description_generation = excluded.description_generation",
        )
        .bind(frame.frame_id().as_str())
        .bind(text.as_ref())
        .bind(provenance.provider())
        .bind(provenance.resolved_model())
        .bind(provenance.prompt_digest().as_str())
        .bind(provenance.preprocessing_digest().as_str())
        .bind(roi.map(|b| f64::from(b[0])))
        .bind(roi.map(|b| f64::from(b[1])))
        .bind(roi.map(|b| f64::from(b[2])))
        .bind(roi.map(|b| f64::from(b[3])))
        .bind(provenance.text_hash().as_str())
        .bind(i64::try_from(provenance.generation().get()).unwrap_or(i64::MAX))
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        self.set_description_checkpoint(frame.frame_id().as_str(), IndexingCheckpoint::Ready)
            .await
    }

    /// Persists validated text-embedding provenance, vector payload, and checkpoint.
    pub async fn upsert_frame_embedding(
        &self,
        frame: &SavedFrameRef,
        model_space: &ModelSpaceIdentity,
        provenance: &TextEmbeddingProvenance,
        vector: &FrameEmbeddingVector,
        checkpoint: IndexingCheckpoint,
    ) -> Result<(), A2aLabError> {
        if checkpoint == IndexingCheckpoint::Ready {
            let description = self
                .load_description(frame.frame_id().as_str())
                .await?
                .ok_or_else(|| {
                    A2aLabError::invalid(
                        "description",
                        "ready embedding requires persisted description",
                    )
                })?;
            FrameVectorIndexAttachment::new(
                model_space.clone(),
                description.provenance().clone(),
                provenance.clone(),
                vector.clone(),
            )?;
        } else if !provenance.matches_model_space(model_space) {
            return Err(A2aLabError::invalid(
                "embedding",
                "provenance does not match the declared model space",
            ));
        }
        self.require_manifest_frame(frame.frame_id().as_str())
            .await?;
        let blob = encode_embedding_vector(vector);
        sqlx::query(
            "INSERT INTO semantic_frame_embeddings (
                frame_id, embedding_provider, embedding_model, dimensions, normalization, metric,
                embedding_generation, model_space_generation, source_text_hash, embedding_vector
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(frame_id) DO UPDATE SET
                embedding_provider = excluded.embedding_provider,
                embedding_model = excluded.embedding_model,
                dimensions = excluded.dimensions,
                normalization = excluded.normalization,
                metric = excluded.metric,
                embedding_generation = excluded.embedding_generation,
                model_space_generation = excluded.model_space_generation,
                source_text_hash = excluded.source_text_hash,
                embedding_vector = excluded.embedding_vector",
        )
        .bind(frame.frame_id().as_str())
        .bind(provenance.provider())
        .bind(provenance.model())
        .bind(i64::from(provenance.dimensions()))
        .bind(normalization_label(provenance.normalization()))
        .bind(metric_label(provenance.metric()))
        .bind(i64::try_from(provenance.generation().get()).unwrap_or(i64::MAX))
        .bind(i64::try_from(model_space.generation().get()).unwrap_or(i64::MAX))
        .bind(provenance.source_text_hash().as_str())
        .bind(blob)
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        self.set_embedding_checkpoint(frame.frame_id().as_str(), checkpoint)
            .await
    }

    /// Lists ready searchable revisions for semantic ranking scans.
    pub async fn scan_semantic_search_candidates(
        &self,
        source_id: &str,
        range_start_us: i64,
        range_end_us: i64,
        filter: &ManifestSearchCandidateFilter,
        scan_limit: u32,
    ) -> Result<ManifestSearchCandidateScan, A2aLabError> {
        self.scan_semantic_search_candidates_after(
            source_id,
            range_start_us,
            range_end_us,
            filter,
            scan_limit,
            None,
        )
        .await
    }

    pub async fn scan_semantic_search_candidates_after(
        &self,
        source_id: &str,
        range_start_us: i64,
        range_end_us: i64,
        filter: &ManifestSearchCandidateFilter,
        scan_limit: u32,
        after: Option<(&str, i64)>,
    ) -> Result<ManifestSearchCandidateScan, A2aLabError> {
        if scan_limit == 0 {
            return Err(A2aLabError::invalid(
                "candidate_limit",
                "must be greater than zero",
            ));
        }
        let space = &filter.model_space;
        let generation = i64::try_from(filter.description_generation).unwrap_or(i64::MAX);
        let fetch_limit = i64::from(scan_limit.saturating_add(1));
        let after_id = after.map(|(frame_id, _)| frame_id);
        let after_us = after.map(|(_, start_us)| start_us);
        let rows = sqlx::query(
            "SELECT rev.frame_id, rev.store_namespace, rev.source_id, rev.track_id, rev.start_us,
                    rev.end_us, rev.payload_sha256, rev.payload_crc32, rev.capture_group,
                    rev.retained_state, rev.search_current, rev.revision_seq,
                    rev.description_checkpoint, rev.embedding_checkpoint
             FROM semantic_frame_revisions rev
             INNER JOIN semantic_frame_descriptions desc ON desc.frame_id = rev.frame_id
             INNER JOIN semantic_frame_embeddings emb ON emb.frame_id = rev.frame_id
             WHERE rev.source_id = ?1
               AND rev.start_us >= ?2
               AND rev.start_us < ?3
               AND rev.search_current = 1
               AND rev.retained_state = 'retained'
               AND desc.description_generation = ?4
               AND emb.embedding_provider = ?5
               AND emb.embedding_model = ?6
               AND emb.dimensions = ?7
               AND emb.normalization = ?8
               AND emb.metric = ?9
               AND emb.embedding_vector IS NOT NULL
               AND length(emb.embedding_vector) > 0
               AND (
                 ?11 IS NULL
                 OR rev.start_us < ?12
                 OR (rev.start_us = ?12 AND rev.frame_id < ?11)
               )
             ORDER BY rev.start_us DESC, rev.frame_id DESC
             LIMIT ?10",
        )
        .bind(source_id)
        .bind(range_start_us)
        .bind(range_end_us)
        .bind(generation)
        .bind(space.provider())
        .bind(space.model())
        .bind(i64::from(space.dimensions()))
        .bind(normalization_label(space.normalization()))
        .bind(metric_label(space.metric()))
        .bind(fetch_limit)
        .bind(after_id)
        .bind(after_us)
        .fetch_all(self.semantic_pool())
        .await
        .map_err(db_error)?;
        let scan_usize = usize::try_from(scan_limit).unwrap_or(usize::MAX);
        let truncated = rows.len() > scan_usize;
        let mut records = Vec::new();
        for row in rows.iter().take(scan_usize) {
            records.push(self.record_from_row(row).await?);
        }
        Ok(ManifestSearchCandidateScan { records, truncated })
    }

    /// BM25 FTS candidate scan using the same identity predicates as vector search.
    pub async fn scan_semantic_lexical_candidates(
        &self,
        source_id: &str,
        range_start_us: i64,
        range_end_us: i64,
        filter: &ManifestSearchCandidateFilter,
        match_query: &str,
        scan_limit: u32,
        offset: u32,
    ) -> Result<ManifestSearchCandidateScan, A2aLabError> {
        if scan_limit == 0 {
            return Err(A2aLabError::invalid(
                "candidate_limit",
                "must be greater than zero",
            ));
        }
        let space = &filter.model_space;
        let generation = i64::try_from(filter.description_generation).unwrap_or(i64::MAX);
        let fetch_limit = i64::from(scan_limit.saturating_add(1));
        let rows = sqlx::query(
            "SELECT rev.frame_id, rev.store_namespace, rev.source_id, rev.track_id, rev.start_us,
                    rev.end_us, rev.payload_sha256, rev.payload_crc32, rev.capture_group,
                    rev.retained_state, rev.search_current, rev.revision_seq,
                    rev.description_checkpoint, rev.embedding_checkpoint
             FROM semantic_frame_descriptions_fts fts
             INNER JOIN semantic_frame_revisions rev ON rev.frame_id = fts.frame_id
             INNER JOIN semantic_frame_descriptions desc ON desc.frame_id = rev.frame_id
             INNER JOIN semantic_frame_embeddings emb ON emb.frame_id = rev.frame_id
             WHERE fts.description_text MATCH ?1
               AND rev.source_id = ?2
               AND rev.start_us >= ?3
               AND rev.start_us < ?4
               AND rev.search_current = 1
               AND rev.retained_state = 'retained'
               AND desc.description_generation = ?5
               AND emb.embedding_provider = ?6
               AND emb.embedding_model = ?7
               AND emb.dimensions = ?8
               AND emb.normalization = ?9
               AND emb.metric = ?10
               AND emb.embedding_vector IS NOT NULL
               AND length(emb.embedding_vector) > 0
             ORDER BY bm25(semantic_frame_descriptions_fts), rev.start_us DESC, rev.frame_id DESC
             LIMIT ?11 OFFSET ?12",
        )
        .bind(match_query)
        .bind(source_id)
        .bind(range_start_us)
        .bind(range_end_us)
        .bind(generation)
        .bind(space.provider())
        .bind(space.model())
        .bind(i64::from(space.dimensions()))
        .bind(normalization_label(space.normalization()))
        .bind(metric_label(space.metric()))
        .bind(fetch_limit)
        .bind(i64::from(offset))
        .fetch_all(self.semantic_pool())
        .await
        .map_err(db_error)?;
        let scan_usize = usize::try_from(scan_limit).unwrap_or(usize::MAX);
        let truncated = rows.len() > scan_usize;
        let mut records = Vec::new();
        for row in rows.iter().take(scan_usize) {
            records.push(self.record_from_row(row).await?);
        }
        Ok(ManifestSearchCandidateScan { records, truncated })
    }

    /// Refreshes candidate media status from retained DVR bytes before ranking.
    pub async fn reconcile_search_media_availability(
        &self,
        records: Vec<SemanticFrameManifestRecord>,
    ) -> Result<Vec<SemanticFrameManifestRecord>, A2aLabError> {
        let keys: Vec<(String, String, i64)> = records.iter().map(dvr_media_key).collect();
        let retained = self.retained_media_keys_chunked(&keys).await?;
        let mut playable = Vec::new();
        for mut record in records {
            if !record.search_current()
                || record.retained_state() != ManifestRetainedState::Retained
            {
                continue;
            }
            let key = dvr_media_key(&record);
            if !retained.contains(&key) {
                record.payload_status = MediaPayloadStatus::Missing;
                continue;
            }
            record.payload_status = self
                .resolve_payload_status(record.frame(), record.search_current())
                .await?;
            if record.payload_status == MediaPayloadStatus::Available {
                playable.push(record);
            }
        }
        Ok(playable)
    }

    async fn retained_media_keys_chunked(
        &self,
        keys: &[(String, String, i64)],
    ) -> Result<std::collections::HashSet<(String, String, i64)>, A2aLabError> {
        const BATCH: usize = 32;
        let mut retained = std::collections::HashSet::new();
        for chunk in keys.chunks(BATCH) {
            retained.extend(self.chunk().retained_media_keys(chunk).await?);
        }
        Ok(retained)
    }

    /// Lists manifest revisions with deterministic keyset paging.
    pub async fn list_manifest_frames(
        &self,
        request: &ListManifestFramesRequest,
    ) -> Result<Page<SemanticFrameManifestRecord>, A2aLabError> {
        request.page().check()?;
        let (cursor_start, cursor_frame) = match request.page().cursor() {
            None => (None, None),
            Some(cursor) => {
                let (start, frame) = parse_manifest_cursor(cursor)?;
                (Some(start), Some(frame))
            }
        };
        let window_start = request
            .window()
            .map_or(i64::MIN, |range| range.start().as_micros());
        let window_end = request
            .window()
            .map_or(i64::MAX, |range| range.end().as_micros());
        let limit = i64::from(request.page().limit());
        let current_only = i64::from(request.current_search_only());
        let rows = sqlx::query(
            "SELECT frame_id, store_namespace, source_id, track_id, start_us, end_us,
                    payload_sha256, payload_crc32, capture_group, retained_state, search_current,
                    revision_seq, description_checkpoint, embedding_checkpoint
             FROM semantic_frame_revisions
             WHERE source_id = ?1 AND track_id = ?2
               AND start_us >= ?3 AND start_us < ?4
               AND (?5 = 0 OR search_current = 1)
               AND (
                 ?6 IS NULL OR start_us > ?6 OR (start_us = ?6 AND frame_id > ?7)
               )
             ORDER BY start_us ASC, frame_id ASC
             LIMIT ?8",
        )
        .bind(request.source_id().as_str())
        .bind(request.track_id().as_str())
        .bind(window_start)
        .bind(window_end)
        .bind(current_only)
        .bind(cursor_start)
        .bind(cursor_frame)
        .bind(limit + 1)
        .fetch_all(self.semantic_pool())
        .await
        .map_err(db_error)?;
        let limit_usize = usize::try_from(limit).unwrap_or(usize::MAX);
        let mut items = Vec::new();
        for row in rows.iter().take(limit_usize) {
            items.push(self.record_from_row(row).await?);
        }
        let next_cursor = (rows.len() > limit_usize).then(|| {
            let last = &items[items.len() - 1];
            format!(
                "{}|{}",
                last.frame().captured_at().as_micros(),
                last.frame().frame_id().as_str()
            )
        });
        Ok(Page::new(items, next_cursor))
    }

    /// Loads one manifest revision.
    pub async fn get_manifest_frame(
        &self,
        frame_id: &str,
    ) -> Result<SemanticFrameManifestRecord, A2aLabError> {
        let row = sqlx::query(
            "SELECT frame_id, store_namespace, source_id, track_id, start_us, end_us,
                    payload_sha256, payload_crc32, capture_group, retained_state, search_current,
                    revision_seq, description_checkpoint, embedding_checkpoint
             FROM semantic_frame_revisions WHERE frame_id = ?1",
        )
        .bind(frame_id)
        .fetch_optional(self.semantic_pool())
        .await
        .map_err(db_error)?;
        let Some(row) = row else {
            return Err(A2aLabError::not_found("semantic frame manifest", frame_id));
        };
        self.record_from_row(&row).await
    }
}

enum RegisterConflictPolicy {
    Default,
}

impl SemanticChunkDvrStore {
    async fn insert_manifest_revision(
        &self,
        frame: &SavedFrameRef,
        end_us: UtcTimestamp,
        payload_crc32: u32,
        _policy: RegisterConflictPolicy,
    ) -> Result<RegisterManifestOutcome, A2aLabError> {
        let registered_at = UtcTimestamp::now();
        let mut tx = self.semantic_pool().begin().await.map_err(db_error)?;
        let outcome = self
            .register_manifest_revision_tx(
                &mut tx,
                frame,
                end_us,
                payload_crc32,
                registered_at.as_micros(),
            )
            .await?;
        tx.commit().await.map_err(db_error)?;
        Ok(outcome)
    }

    pub(crate) async fn register_manifest_revision_tx(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        frame: &SavedFrameRef,
        end_us: UtcTimestamp,
        payload_crc32: u32,
        registered_at_us: i64,
    ) -> Result<RegisterManifestOutcome, A2aLabError> {
        let frame_id = frame.frame_id().as_str();
        let existing_id: Option<String> =
            sqlx::query_scalar("SELECT frame_id FROM semantic_frame_revisions WHERE frame_id = ?1")
                .bind(frame_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(db_error)?;
        if existing_id.is_some() {
            return Ok(RegisterManifestOutcome::Idempotent);
        }
        let digest = frame.payload_digest().as_str();
        let duplicate: Option<String> = sqlx::query_scalar(
            "SELECT frame_id FROM semantic_frame_revisions
             WHERE source_id = ?1 AND track_id = ?2 AND start_us = ?3 AND payload_sha256 = ?4",
        )
        .bind(frame.source_id().as_str())
        .bind(frame.track_id().as_str())
        .bind(frame.captured_at().as_micros())
        .bind(digest)
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_error)?;
        if duplicate.is_some() {
            return Ok(RegisterManifestOutcome::Idempotent);
        }
        let superseded_frames: Vec<String> = sqlx::query_scalar(
            "SELECT frame_id FROM semantic_frame_revisions
             WHERE source_id = ?1 AND track_id = ?2 AND start_us = ?3 AND search_current = 1",
        )
        .bind(frame.source_id().as_str())
        .bind(frame.track_id().as_str())
        .bind(frame.captured_at().as_micros())
        .fetch_all(&mut **tx)
        .await
        .map_err(db_error)?;
        let replaced = sqlx::query(
            "UPDATE semantic_frame_revisions
             SET search_current = 0, retained_state = 'superseded'
             WHERE source_id = ?1 AND track_id = ?2 AND start_us = ?3 AND search_current = 1",
        )
        .bind(frame.source_id().as_str())
        .bind(frame.track_id().as_str())
        .bind(frame.captured_at().as_micros())
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
        for prior_frame in superseded_frames {
            self.tombstone_decisions_for_frame_tx(tx, &prior_frame, "superseded", registered_at_us)
                .await?;
        }
        let current: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(revision_seq) FROM semantic_frame_revisions
             WHERE source_id = ?1 AND track_id = ?2 AND start_us = ?3",
        )
        .bind(frame.source_id().as_str())
        .bind(frame.track_id().as_str())
        .bind(frame.captured_at().as_micros())
        .fetch_one(&mut **tx)
        .await
        .map_err(db_error)?;
        let revision_seq = u32::try_from(current.unwrap_or(0) + 1).unwrap_or(1);
        sqlx::query(
            "INSERT INTO semantic_frame_revisions (
                frame_id, store_namespace, source_id, track_id, start_us, end_us, payload_sha256,
                payload_crc32, capture_group, retained_state, search_current, revision_seq,
                description_checkpoint, embedding_checkpoint, registered_at_us
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'retained', 1, ?10, 'pending', 'pending', ?11)",
        )
        .bind(frame_id)
        .bind(frame.store().as_str())
        .bind(frame.source_id().as_str())
        .bind(frame.track_id().as_str())
        .bind(frame.captured_at().as_micros())
        .bind(end_us.as_micros())
        .bind(digest)
        .bind(i64::from(payload_crc32))
        .bind(frame.capture_group().map(crate::CaptureGroupId::as_str))
        .bind(i64::from(revision_seq))
        .bind(registered_at_us)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
        Ok(if replaced.rows_affected() > 0 {
            RegisterManifestOutcome::ReplacedPrior
        } else {
            RegisterManifestOutcome::Registered
        })
    }

    async fn load_dvr_payload_at_key(
        &self,
        source_id: &VideoSourceId,
        track_id: &DvrTrackId,
        start_us: UtcTimestamp,
    ) -> Result<(Vec<u8>, u32, UtcTimestamp), A2aLabError> {
        self.load_retained_payload_at_key(source_id, track_id, start_us)
            .await
    }

    async fn record_from_row(
        &self,
        row: &sqlx::sqlite::SqliteRow,
    ) -> Result<SemanticFrameManifestRecord, A2aLabError> {
        let frame = saved_frame_from_row(row)?;
        let end_us: i64 = row.try_get("end_us").map_err(db_error)?;
        let retained_state = parse_retained_state(
            &row.try_get::<String, _>("retained_state")
                .map_err(db_error)?,
        )?;
        let search_current: i64 = row.try_get("search_current").map_err(db_error)?;
        let revision_seq: i64 = row.try_get("revision_seq").map_err(db_error)?;
        let description_checkpoint = parse_checkpoint(
            &row.try_get::<String, _>("description_checkpoint")
                .map_err(db_error)?,
        )?;
        let embedding_checkpoint = parse_checkpoint(
            &row.try_get::<String, _>("embedding_checkpoint")
                .map_err(db_error)?,
        )?;
        let frame_id: String = row.try_get("frame_id").map_err(db_error)?;
        let description = self.load_description(&frame_id).await?;
        let embedding = self.load_embedding(&frame_id).await?;
        let payload_status = self
            .resolve_payload_status(&frame, search_current != 0)
            .await?;
        Ok(SemanticFrameManifestRecord {
            frame,
            end_us: UtcTimestamp::from_unix_microseconds(end_us)?,
            retained_state,
            search_current: search_current != 0,
            revision_seq: u32::try_from(revision_seq.max(0)).unwrap_or(0),
            description_checkpoint,
            embedding_checkpoint,
            description,
            embedding,
            payload_status,
        })
    }

    async fn resolve_payload_status(
        &self,
        frame: &SavedFrameRef,
        search_current: bool,
    ) -> Result<MediaPayloadStatus, A2aLabError> {
        let bytes = match self
            .load_retained_payload_at_key(frame.source_id(), frame.track_id(), frame.captured_at())
            .await
        {
            Ok((payload, _, _)) => payload,
            Err(error) => {
                let message = error.to_string();
                if message.contains("checksum mismatch") {
                    return Ok(MediaPayloadStatus::Corrupt);
                }
                return Ok(MediaPayloadStatus::Missing);
            }
        };
        let digest = Sha256Digest::hash_bytes(&bytes);
        if digest != *frame.payload_digest() {
            return if search_current {
                Ok(MediaPayloadStatus::Corrupt)
            } else {
                Ok(MediaPayloadStatus::Missing)
            };
        }
        Ok(MediaPayloadStatus::Available)
    }

    #[allow(clippy::cast_possible_truncation)]
    async fn load_description(
        &self,
        frame_id: &str,
    ) -> Result<Option<StoredFrameDescription>, A2aLabError> {
        let row = sqlx::query(
            "SELECT description_text, description_provider, description_model, prompt_digest,
                    preprocessing_digest, roi_x_min, roi_y_min, roi_x_max, roi_y_max, text_hash,
                    description_generation
             FROM semantic_frame_descriptions WHERE frame_id = ?1",
        )
        .bind(frame_id)
        .fetch_optional(self.semantic_pool())
        .await
        .map_err(db_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let text: String = row.try_get("description_text").map_err(db_error)?;
        let roi = match (
            row.try_get::<Option<f64>, _>("roi_x_min")
                .map_err(db_error)?,
            row.try_get::<Option<f64>, _>("roi_y_min")
                .map_err(db_error)?,
            row.try_get::<Option<f64>, _>("roi_x_max")
                .map_err(db_error)?,
            row.try_get::<Option<f64>, _>("roi_y_max")
                .map_err(db_error)?,
        ) {
            (Some(x_min), Some(y_min), Some(x_max), Some(y_max)) => {
                Some(crate::FrameRoiTransform::new([
                    x_min as f32,
                    y_min as f32,
                    x_max as f32,
                    y_max as f32,
                ])?)
            }
            _ => None,
        };
        let provenance = FrameDescriptionProvenance::new(
            row.try_get::<String, _>("description_provider")
                .map_err(db_error)?,
            row.try_get::<String, _>("description_model")
                .map_err(db_error)?,
            Sha256Digest::parse(
                row.try_get::<String, _>("prompt_digest")
                    .map_err(db_error)?
                    .as_str(),
            )?,
            Sha256Digest::parse(
                row.try_get::<String, _>("preprocessing_digest")
                    .map_err(db_error)?
                    .as_str(),
            )?,
            roi,
            Sha256Digest::parse(
                row.try_get::<String, _>("text_hash")
                    .map_err(db_error)?
                    .as_str(),
            )?,
            crate::ProvenanceGeneration::new(
                u64::try_from(
                    row.try_get::<i64, _>("description_generation")
                        .map_err(db_error)?
                        .max(0),
                )
                .unwrap_or(0),
            )?,
        )?;
        Ok(Some(StoredFrameDescription { text, provenance }))
    }

    async fn load_embedding(
        &self,
        frame_id: &str,
    ) -> Result<Option<StoredFrameEmbedding>, A2aLabError> {
        let row = sqlx::query(
            "SELECT embedding_provider, embedding_model, dimensions, normalization, metric,
                    embedding_generation, model_space_generation, source_text_hash, embedding_vector
             FROM semantic_frame_embeddings WHERE frame_id = ?1",
        )
        .bind(frame_id)
        .fetch_optional(self.semantic_pool())
        .await
        .map_err(db_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let provenance = TextEmbeddingProvenance::new(
            row.try_get::<String, _>("embedding_provider")
                .map_err(db_error)?,
            row.try_get::<String, _>("embedding_model")
                .map_err(db_error)?,
            u32::try_from(
                row.try_get::<i64, _>("dimensions")
                    .map_err(db_error)?
                    .max(0),
            )
            .unwrap_or(0),
            parse_normalization(
                &row.try_get::<String, _>("normalization")
                    .map_err(db_error)?,
            )?,
            parse_metric(&row.try_get::<String, _>("metric").map_err(db_error)?)?,
            crate::ProvenanceGeneration::new(
                u64::try_from(
                    row.try_get::<i64, _>("embedding_generation")
                        .map_err(db_error)?
                        .max(0),
                )
                .unwrap_or(0),
            )?,
            Sha256Digest::parse(
                row.try_get::<String, _>("source_text_hash")
                    .map_err(db_error)?
                    .as_str(),
            )?,
        )?;
        let dimensions = provenance.dimensions();
        let vector = match row
            .try_get::<Option<Vec<u8>>, _>("embedding_vector")
            .map_err(db_error)?
        {
            None => None,
            Some(bytes) if bytes.is_empty() => None,
            Some(bytes) => Some(decode_embedding_vector(&bytes, dimensions)?),
        };
        Ok(Some(StoredFrameEmbedding { provenance, vector }))
    }

    pub(crate) async fn require_manifest_frame(&self, frame_id: &str) -> Result<(), A2aLabError> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM semantic_frame_revisions WHERE frame_id = ?1")
                .bind(frame_id)
                .fetch_one(self.semantic_pool())
                .await
                .map_err(db_error)?;
        if count == 0 {
            return Err(A2aLabError::not_found("semantic frame manifest", frame_id));
        }
        Ok(())
    }

    async fn set_description_checkpoint(
        &self,
        frame_id: &str,
        checkpoint: IndexingCheckpoint,
    ) -> Result<(), A2aLabError> {
        sqlx::query(
            "UPDATE semantic_frame_revisions SET description_checkpoint = ?2 WHERE frame_id = ?1",
        )
        .bind(frame_id)
        .bind(checkpoint_label(checkpoint))
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn set_embedding_checkpoint(
        &self,
        frame_id: &str,
        checkpoint: IndexingCheckpoint,
    ) -> Result<(), A2aLabError> {
        sqlx::query(
            "UPDATE semantic_frame_revisions SET embedding_checkpoint = ?2 WHERE frame_id = ?1",
        )
        .bind(frame_id)
        .bind(checkpoint_label(checkpoint))
        .execute(self.semantic_pool())
        .await
        .map_err(db_error)?;
        Ok(())
    }
}

pub(crate) fn saved_frame_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<SavedFrameRef, A2aLabError> {
    let store = DvrStoreNamespace::new(
        row.try_get::<String, _>("store_namespace")
            .map_err(db_error)?,
    )?;
    let source_id = VideoSourceId::new(row.try_get::<String, _>("source_id").map_err(db_error)?)?;
    let track_id = DvrTrackId::new(row.try_get::<String, _>("track_id").map_err(db_error)?)?;
    let captured_at =
        UtcTimestamp::from_unix_microseconds(row.try_get::<i64, _>("start_us").map_err(db_error)?)?;
    let digest = Sha256Digest::parse(
        row.try_get::<String, _>("payload_sha256")
            .map_err(db_error)?
            .as_str(),
    )?;
    let capture_group = match row
        .try_get::<Option<String>, _>("capture_group")
        .map_err(db_error)?
    {
        Some(value) => Some(crate::CaptureGroupId::new(value)?),
        None => None,
    };
    SavedFrameRef::new(
        store,
        source_id,
        track_id,
        captured_at,
        digest,
        capture_group,
    )
}

fn checkpoint_label(checkpoint: IndexingCheckpoint) -> &'static str {
    match checkpoint {
        IndexingCheckpoint::Pending => "pending",
        IndexingCheckpoint::Ready => "ready",
        IndexingCheckpoint::Failed => "failed",
    }
}

fn parse_checkpoint(value: &str) -> Result<IndexingCheckpoint, A2aLabError> {
    match value {
        "pending" => Ok(IndexingCheckpoint::Pending),
        "ready" => Ok(IndexingCheckpoint::Ready),
        "failed" => Ok(IndexingCheckpoint::Failed),
        _ => Err(A2aLabError::invalid(
            "checkpoint",
            "unknown semantic indexing checkpoint",
        )),
    }
}

fn dvr_media_key(record: &SemanticFrameManifestRecord) -> (String, String, i64) {
    (
        record.frame().source_id().as_str().to_owned(),
        record.frame().track_id().as_str().to_owned(),
        record.frame().captured_at().as_micros(),
    )
}

fn parse_retained_state(value: &str) -> Result<ManifestRetainedState, A2aLabError> {
    match value {
        "retained" => Ok(ManifestRetainedState::Retained),
        "expired" => Ok(ManifestRetainedState::Expired),
        "superseded" => Ok(ManifestRetainedState::Superseded),
        _ => Err(A2aLabError::invalid(
            "retained_state",
            "unknown manifest retained state",
        )),
    }
}

pub(crate) fn normalization_label(value: crate::VectorNormalization) -> &'static str {
    match value {
        crate::VectorNormalization::L2Unit => "l2_unit",
        crate::VectorNormalization::None => "none",
    }
}

pub(crate) fn metric_label(value: crate::EmbeddingMetric) -> &'static str {
    match value {
        crate::EmbeddingMetric::Cosine => "cosine",
        crate::EmbeddingMetric::Dot => "dot",
        crate::EmbeddingMetric::Euclidean => "euclidean",
    }
}

pub(crate) fn parse_normalization(value: &str) -> Result<crate::VectorNormalization, A2aLabError> {
    match value {
        "l2_unit" => Ok(crate::VectorNormalization::L2Unit),
        "none" => Ok(crate::VectorNormalization::None),
        _ => Err(A2aLabError::invalid("normalization", "unknown value")),
    }
}

pub(crate) fn parse_metric(value: &str) -> Result<crate::EmbeddingMetric, A2aLabError> {
    match value {
        "cosine" => Ok(crate::EmbeddingMetric::Cosine),
        "dot" => Ok(crate::EmbeddingMetric::Dot),
        "euclidean" => Ok(crate::EmbeddingMetric::Euclidean),
        _ => Err(A2aLabError::invalid("metric", "unknown value")),
    }
}

fn encode_embedding_vector(vector: &FrameEmbeddingVector) -> Vec<u8> {
    vector
        .values()
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn decode_embedding_vector(
    bytes: &[u8],
    dimensions: u32,
) -> Result<FrameEmbeddingVector, A2aLabError> {
    let expected_bytes = usize::try_from(dimensions)
        .map_err(|_| A2aLabError::invalid("dimensions", "vector dimension count overflowed usize"))?
        .saturating_mul(std::mem::size_of::<f32>());
    if bytes.len() != expected_bytes {
        return Err(A2aLabError::invalid(
            "embedding_vector",
            format!("blob length must be {expected_bytes} bytes for {dimensions} dimensions"),
        ));
    }
    let mut values = Vec::with_capacity(usize::try_from(dimensions).unwrap_or(0));
    for chunk in bytes.as_chunks::<4>().0 {
        values.push(f32::from_le_bytes(*chunk));
    }
    FrameEmbeddingVector::new(values, dimensions)
}
