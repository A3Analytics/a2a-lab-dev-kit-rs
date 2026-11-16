//! Semantic chunk DVR persistence (TASK-148).

use a2a_lab_dev_kit::{
    DEFAULT_RECONCILE_BATCH, DvrMediaStore, DvrStoreNamespace, EmbeddingMetric,
    FrameDescriptionProvenance, FrameEmbeddingVector, FrameRoiTransform, IndexingCheckpoint,
    ListManifestFramesRequest, ManifestPageRequest, ManifestRetainedState, MediaPayloadStatus,
    MemoryVideos, ModelSpaceIdentity, ProvenanceGeneration, RegisterManifestOutcome,
    SemanticChunkDvrStore, Sha256Digest, TextEmbeddingProvenance, UtcTimestamp,
    VectorNormalization, VideoSourceId, chunk_relpath, minute_start_us,
};

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "a2a-lab-devkit-semantic-dvr-{name}-{}",
        std::process::id()
    ))
}

fn store_ns() -> DvrStoreNamespace {
    DvrStoreNamespace::new("test.video-dvr").unwrap()
}

fn jpeg(tag: u8) -> Vec<u8> {
    vec![0xFF, 0xD8, tag, 0xD9]
}

async fn open_store(name: &str) -> (std::path::PathBuf, SemanticChunkDvrStore) {
    let dir = temp_root(name);
    let _ = std::fs::remove_dir_all(&dir);
    let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    (dir, store)
}

fn fresh_capture(offset_us: i64) -> UtcTimestamp {
    UtcTimestamp::now().saturating_add_micros(offset_us)
}

fn sample_description(text: &str) -> FrameDescriptionProvenance {
    FrameDescriptionProvenance::new(
        "bedrock",
        "global.openai.gpt-5.6-luna",
        Sha256Digest::hash_bytes(b"prompt-v1"),
        Sha256Digest::hash_bytes(b"preprocess-v1"),
        Some(FrameRoiTransform::new([0.0, 0.0, 1.0, 1.0]).unwrap()),
        Sha256Digest::hash_bytes(text.as_bytes()),
        ProvenanceGeneration::new(1).unwrap(),
    )
    .unwrap()
}

fn sample_model_space() -> ModelSpaceIdentity {
    ModelSpaceIdentity::new(
        "bedrock",
        "text-embed-v1",
        4,
        VectorNormalization::L2Unit,
        EmbeddingMetric::Cosine,
        ProvenanceGeneration::new(1).unwrap(),
    )
    .unwrap()
}

fn sample_embedding(text: &str) -> TextEmbeddingProvenance {
    TextEmbeddingProvenance::new(
        "bedrock",
        "text-embed-v1",
        4,
        VectorNormalization::L2Unit,
        EmbeddingMetric::Cosine,
        ProvenanceGeneration::new(1).unwrap(),
        Sha256Digest::hash_bytes(text.as_bytes()),
    )
    .unwrap()
}

fn sample_vector() -> FrameEmbeddingVector {
    FrameEmbeddingVector::new(vec![0.1, 0.2, 0.3, 0.4], 4).unwrap()
}

#[tokio::test]
async fn fresh_semantic_schema_separate_from_media() {
    let dir = temp_root("schema");
    let _ = std::fs::remove_dir_all(&dir);
    let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    let media_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN
         ('dvr_frames', 'dvr_chunks')",
    )
    .fetch_one(store.media_pool())
    .await
    .unwrap();
    assert_eq!(media_tables, 2);
    let semantic_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name =
         'semantic_frame_revisions'",
    )
    .fetch_one(store.semantic_pool())
    .await
    .unwrap();
    assert_eq!(semantic_tables, 1);
    assert!(dir.join("index.sqlite").exists());
    assert!(dir.join("semantic.sqlite").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn duplicate_registration_is_idempotent() {
    let (dir, store) = open_store("dup").await;
    let source = VideoSourceId::new("cam-a").unwrap();
    let track = MemoryVideos::mjpeg_track_id(&source).unwrap();
    let capture = fresh_capture(1_000_000);
    store
        .append_mjpeg_frame(&source, capture, 1_000_000, &jpeg(2))
        .await
        .unwrap();
    assert_eq!(store.count_indexing_jobs().await.unwrap(), 2);
    let (frame, first) = store
        .register_manifest_at_dvr_key(&store_ns(), &source, &track, capture)
        .await
        .unwrap();
    assert_eq!(first, RegisterManifestOutcome::Idempotent);
    let current = store
        .get_manifest_frame(frame.frame_id().as_str())
        .await
        .unwrap();
    assert!(current.search_current());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn metadata_commit_fault_retains_media_without_semantic_orphans() {
    let (_dir, store) = open_store("meta-fault").await;
    let source = VideoSourceId::new("camera").unwrap();
    let capture = fresh_capture(0);
    store.save_faults().set_fail_metadata_commit(true);
    let result = store
        .append_mjpeg_frame(&source, capture, 1_000_000, &jpeg(2))
        .await;
    assert!(result.is_err());
    assert_eq!(store.count_indexing_jobs().await.unwrap(), 0);
    let media_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM dvr_frames WHERE source_id = ?1 AND start_us = ?2",
    )
    .bind(source.as_str())
    .bind(capture.as_micros())
    .fetch_one(store.media_pool())
    .await
    .unwrap();
    assert_eq!(media_count, 1);
    store.save_faults().set_fail_metadata_commit(false);
    let report = store
        .reconcile_indexing_jobs(&store_ns(), DEFAULT_RECONCILE_BATCH)
        .await
        .unwrap();
    assert_eq!(report.jobs_created(), 2);
}

#[tokio::test]
async fn embedding_vector_survives_reopen() {
    let (dir, store) = open_store("vector-reopen").await;
    let source = VideoSourceId::new("cam-a").unwrap();
    let track = MemoryVideos::mjpeg_track_id(&source).unwrap();
    let capture = fresh_capture(50_000_000);
    store
        .append_mjpeg_frame(&source, capture, 1_000_000, &jpeg(50))
        .await
        .unwrap();
    let (frame, _) = store
        .register_manifest_at_dvr_key(&store_ns(), &source, &track, capture)
        .await
        .unwrap();
    let text = "tips aligned on deck";
    let vector = sample_vector();
    store
        .upsert_frame_description(&frame, text, &sample_description(text))
        .await
        .unwrap();
    store
        .upsert_frame_embedding(
            &frame,
            &sample_model_space(),
            &sample_embedding(text),
            &vector,
            IndexingCheckpoint::Ready,
        )
        .await
        .unwrap();
    drop(store);
    let reopened = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    let record = reopened
        .get_manifest_frame(frame.frame_id().as_str())
        .await
        .unwrap();
    assert_eq!(record.embedding().and_then(|e| e.vector()), Some(&vector));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn payload_status_reports_missing_and_corrupt_distinctly() {
    let (dir, store) = open_store("payload").await;
    let source = VideoSourceId::new("cam-a").unwrap();
    let track = MemoryVideos::mjpeg_track_id(&source).unwrap();
    let capture = fresh_capture(30_000_000);
    store
        .append_mjpeg_frame(&source, capture, 1_000_000, &jpeg(30))
        .await
        .unwrap();
    let (frame, _) = store
        .register_manifest_at_dvr_key(&store_ns(), &source, &track, capture)
        .await
        .unwrap();
    assert_eq!(
        store
            .get_manifest_frame(frame.frame_id().as_str())
            .await
            .unwrap()
            .payload_status(),
        MediaPayloadStatus::Available
    );
    let minute = minute_start_us(capture);
    let chunk_relpath = chunk_relpath(source.as_str(), track.as_str(), minute);
    let chunk_path = store.root().join(&chunk_relpath);
    let path = if chunk_path.exists() {
        chunk_path
    } else {
        store.root().join(format!("{chunk_relpath}.tmp"))
    };
    let mut bytes = std::fs::read(&path).unwrap();
    if let Some(byte) = bytes.first_mut() {
        *byte ^= 0xFF;
    }
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(
        store
            .get_manifest_frame(frame.frame_id().as_str())
            .await
            .unwrap()
            .payload_status(),
        MediaPayloadStatus::Corrupt
    );
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        store
            .get_manifest_frame(frame.frame_id().as_str())
            .await
            .unwrap()
            .payload_status(),
        MediaPayloadStatus::Missing
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn restart_preserves_jobs_and_manifest() {
    let dir = temp_root("restart");
    let _ = std::fs::remove_dir_all(&dir);
    let source = VideoSourceId::new("camera").unwrap();
    let track = MemoryVideos::mjpeg_track_id(&source).unwrap();
    let capture = fresh_capture(0);
    let frame_id = {
        let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
            .await
            .unwrap();
        store
            .append_mjpeg_frame(&source, capture, 1_000_000, &jpeg(4))
            .await
            .unwrap();
        let (frame, _, _) = store
            .ensure_indexing_at_dvr_key(&store_ns(), &source, &track, capture)
            .await
            .unwrap();
        frame.frame_id().as_str().to_owned()
    };
    let store = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    assert_eq!(store.count_indexing_jobs().await.unwrap(), 2);
    assert!(store.indexing_job_exists(&frame_id).await.unwrap());
    let record = store.get_manifest_frame(&frame_id).await.unwrap();
    assert_eq!(record.retained_state(), ManifestRetainedState::Retained);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn expired_lease_is_reclaimable_without_duplicate_jobs() {
    let (_dir, store) = open_store("lease").await;
    let source = VideoSourceId::new("cam-a").unwrap();
    let capture = fresh_capture(0);
    store
        .append_mjpeg_frame(&source, capture, 1_000_000, &jpeg(8))
        .await
        .unwrap();
    assert_eq!(store.count_indexing_jobs().await.unwrap(), 2);
    let jobs = store
        .claim_indexing_jobs(
            "worker-a",
            1,
            capture.as_micros() + 1_000_000,
            capture.as_micros(),
        )
        .await
        .unwrap();
    assert_eq!(jobs.len(), 1);
    let job_id = jobs[0].job_id().to_owned();
    let stale = capture.as_micros() + 2_000_000;
    let reclaimed = store.reclaim_expired_indexing_leases(stale).await.unwrap();
    assert!(reclaimed >= 1);
    let job = store.get_indexing_job(&job_id).await.unwrap();
    assert!(job.lease_owner().is_none());
    assert_eq!(store.count_indexing_jobs().await.unwrap(), 2);
    let claimed_again = store
        .claim_indexing_jobs("worker-b", 1, stale + 1_000_000, stale)
        .await
        .unwrap();
    assert_eq!(claimed_again.len(), 1);
    assert_eq!(claimed_again[0].job_id(), job_id);
}

#[tokio::test]
async fn media_coverage_unchanged_after_semantic_reopen() {
    let (dir, store) = open_store("media").await;
    let source = VideoSourceId::new("cam-a").unwrap();
    let capture = fresh_capture(0);
    store
        .append_mjpeg_frame(&source, capture, 1_000_000, &jpeg(1))
        .await
        .unwrap();
    let before = store.chunk().media_coverage(&source).await.unwrap();
    drop(store);
    let reopened = SemanticChunkDvrStore::open(&dir, 86_400, store_ns())
        .await
        .unwrap();
    let after = reopened.chunk().media_coverage(&source).await.unwrap();
    assert_eq!(before.intervals().len(), after.intervals().len());
    let track = MemoryVideos::mjpeg_track_id(&source).unwrap();
    let request = ListManifestFramesRequest::new(
        source.clone(),
        track,
        None,
        true,
        ManifestPageRequest::new(None, 10).unwrap(),
    )
    .unwrap();
    assert_eq!(
        reopened
            .list_manifest_frames(&request)
            .await
            .unwrap()
            .items()
            .len(),
        1
    );
    let _ = std::fs::remove_dir_all(&dir);
}
