CREATE TABLE IF NOT EXISTS semantic_frame_revisions (
    frame_id TEXT NOT NULL,
    store_namespace TEXT NOT NULL,
    source_id TEXT NOT NULL,
    track_id TEXT NOT NULL,
    start_us INTEGER NOT NULL,
    end_us INTEGER NOT NULL,
    payload_sha256 TEXT NOT NULL,
    payload_crc32 INTEGER NOT NULL,
    capture_group TEXT,
    retained_state TEXT NOT NULL,
    search_current INTEGER NOT NULL DEFAULT 1,
    revision_seq INTEGER NOT NULL,
    description_checkpoint TEXT NOT NULL DEFAULT 'pending',
    embedding_checkpoint TEXT NOT NULL DEFAULT 'pending',
    registered_at_us INTEGER NOT NULL,
    PRIMARY KEY (frame_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS semantic_frame_revisions_dvr_key_digest
    ON semantic_frame_revisions (source_id, track_id, start_us, payload_sha256);

CREATE INDEX IF NOT EXISTS semantic_frame_revisions_current_enum
    ON semantic_frame_revisions (source_id, track_id, start_us, frame_id)
    WHERE search_current = 1;

CREATE INDEX IF NOT EXISTS semantic_frame_revisions_history_enum
    ON semantic_frame_revisions (source_id, track_id, start_us, revision_seq, frame_id);

CREATE TABLE IF NOT EXISTS semantic_frame_descriptions (
    frame_id TEXT NOT NULL,
    description_text TEXT NOT NULL,
    description_provider TEXT NOT NULL,
    description_model TEXT NOT NULL,
    prompt_digest TEXT NOT NULL,
    preprocessing_digest TEXT NOT NULL,
    roi_x_min REAL,
    roi_y_min REAL,
    roi_x_max REAL,
    roi_y_max REAL,
    text_hash TEXT NOT NULL,
    description_generation INTEGER NOT NULL,
    PRIMARY KEY (frame_id),
    FOREIGN KEY (frame_id) REFERENCES semantic_frame_revisions (frame_id)
);

CREATE TABLE IF NOT EXISTS semantic_frame_embeddings (
    frame_id TEXT NOT NULL,
    embedding_provider TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    dimensions INTEGER NOT NULL,
    normalization TEXT NOT NULL,
    metric TEXT NOT NULL,
    embedding_generation INTEGER NOT NULL,
    model_space_generation INTEGER NOT NULL,
    source_text_hash TEXT NOT NULL,
    embedding_vector BLOB,
    PRIMARY KEY (frame_id),
    FOREIGN KEY (frame_id) REFERENCES semantic_frame_revisions (frame_id)
);

CREATE TABLE IF NOT EXISTS semantic_indexing_jobs (
    job_id TEXT NOT NULL,
    frame_id TEXT NOT NULL,
    job_kind TEXT NOT NULL DEFAULT 'frame_indexing',
    created_at_us INTEGER NOT NULL,
    enqueue_priority INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (job_id),
    UNIQUE (frame_id, job_kind),
    FOREIGN KEY (frame_id) REFERENCES semantic_frame_revisions (frame_id)
);

CREATE INDEX IF NOT EXISTS semantic_indexing_jobs_frame
    ON semantic_indexing_jobs (frame_id);

CREATE INDEX IF NOT EXISTS semantic_indexing_jobs_claim_order
    ON semantic_indexing_jobs (enqueue_priority, created_at_us, job_id);

CREATE TABLE IF NOT EXISTS semantic_indexing_job_runtime (
    job_id TEXT NOT NULL,
    job_state TEXT NOT NULL DEFAULT 'pending',
    revision_generation INTEGER NOT NULL DEFAULT 0,
    lease_owner TEXT,
    lease_expires_at_us INTEGER,
    description_generation INTEGER,
    description_attempts INTEGER NOT NULL DEFAULT 0,
    embedding_attempts INTEGER NOT NULL DEFAULT 0,
    last_retry_reason TEXT,
    next_retry_at_us INTEGER NOT NULL DEFAULT 0,
    last_error_at_us INTEGER,
    completed_at_us INTEGER,
    PRIMARY KEY (job_id),
    FOREIGN KEY (job_id) REFERENCES semantic_indexing_jobs (job_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS semantic_indexing_job_runtime_work_queue
    ON semantic_indexing_job_runtime (job_state, next_retry_at_us);

CREATE TABLE IF NOT EXISTS semantic_indexing_worker_meta (
    meta_key TEXT NOT NULL PRIMARY KEY,
    meta_value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS semantic_stage_eligibility (
    source_id TEXT NOT NULL,
    track_id TEXT NOT NULL,
    stage TEXT NOT NULL,
    bucket_start_us INTEGER NOT NULL,
    interval_us INTEGER,
    winner_frame_id TEXT NOT NULL,
    winner_start_us INTEGER NOT NULL,
    enqueue_priority INTEGER NOT NULL,
    created_at_us INTEGER NOT NULL,
    updated_at_us INTEGER NOT NULL,
    PRIMARY KEY (source_id, track_id, stage, bucket_start_us),
    FOREIGN KEY (winner_frame_id) REFERENCES semantic_frame_revisions (frame_id)
);

CREATE INDEX IF NOT EXISTS semantic_stage_eligibility_winner
    ON semantic_stage_eligibility (winner_frame_id);

CREATE TABLE IF NOT EXISTS semantic_reconcile_cursors (
    source_id TEXT NOT NULL,
    track_id TEXT NOT NULL,
    last_start_us INTEGER NOT NULL,
    PRIMARY KEY (source_id, track_id)
);

CREATE TABLE IF NOT EXISTS semantic_embedding_config_identity (
    singleton_id INTEGER NOT NULL PRIMARY KEY CHECK (singleton_id = 1),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimensions INTEGER NOT NULL,
    normalization TEXT NOT NULL,
    metric TEXT NOT NULL,
    config_digest TEXT NOT NULL,
    model_space_generation INTEGER NOT NULL DEFAULT 0,
    updated_at_us INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS frame_decisions (
    decision_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    frame_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    captured_at_us INTEGER NOT NULL,
    predicate_id TEXT NOT NULL,
    predicate_version INTEGER NOT NULL,
    subject_digest TEXT NOT NULL,
    producer_id TEXT NOT NULL,
    model_revision TEXT NOT NULL,
    evaluated_at_us INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    payload_fingerprint TEXT NOT NULL,
    supersedes TEXT,
    append_seq INTEGER NOT NULL,
    tombstoned INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (decision_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS frame_decisions_idempotency
    ON frame_decisions (idempotency_key);

CREATE INDEX IF NOT EXISTS frame_decisions_frame_append
    ON frame_decisions (frame_id, append_seq);

CREATE TABLE IF NOT EXISTS frame_decision_tombstones (
    frame_id TEXT NOT NULL,
    retained_state TEXT NOT NULL,
    tombstoned_at_us INTEGER NOT NULL,
    PRIMARY KEY (frame_id)
);

CREATE TABLE IF NOT EXISTS human_corpus_labels (
    label_id TEXT NOT NULL,
    frame_id TEXT NOT NULL,
    predicate_id TEXT NOT NULL,
    label_json TEXT NOT NULL,
    labeled_at_us INTEGER NOT NULL,
    PRIMARY KEY (label_id)
);

CREATE TABLE IF NOT EXISTS frame_decision_evaluation_jobs (
    job_id TEXT NOT NULL,
    frame_id TEXT NOT NULL,
    evaluation_id TEXT NOT NULL,
    predicate_id TEXT NOT NULL,
    predicate_version INTEGER NOT NULL,
    subject_digest TEXT NOT NULL,
    config_digest TEXT NOT NULL,
    producer_revision TEXT NOT NULL,
    coverage_policy TEXT NOT NULL,
    created_at_us INTEGER NOT NULL,
    PRIMARY KEY (job_id),
    UNIQUE (
        evaluation_id,
        predicate_id,
        predicate_version,
        subject_digest,
        config_digest,
        producer_revision
    ),
    FOREIGN KEY (frame_id) REFERENCES semantic_frame_revisions (frame_id)
);

CREATE TABLE IF NOT EXISTS frame_decision_evaluation_job_runtime (
    job_id TEXT NOT NULL,
    job_state TEXT NOT NULL DEFAULT 'pending',
    coverage TEXT,
    gap_reason TEXT,
    revision_generation INTEGER NOT NULL DEFAULT 0,
    lease_owner TEXT,
    lease_expires_at_us INTEGER,
    evaluation_attempts INTEGER NOT NULL DEFAULT 0,
    last_retry_reason TEXT,
    next_retry_at_us INTEGER NOT NULL DEFAULT 0,
    last_error_at_us INTEGER,
    completed_at_us INTEGER,
    cancelled_at_us INTEGER,
    idempotency_key TEXT,
    decision_id TEXT,
    PRIMARY KEY (job_id),
    FOREIGN KEY (job_id) REFERENCES frame_decision_evaluation_jobs (job_id) ON DELETE CASCADE
);
