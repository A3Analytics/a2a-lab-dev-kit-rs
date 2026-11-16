//! Versioned FTS5 lexical index on semantic SQLite (TASK-157).

use a2a_lab_dev_kit::SemanticSqliteStore;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::time::Duration;

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "a2a-lab-devkit-semantic-fts-{name}-{}.sqlite",
        std::process::id()
    ))
}

async fn connect_raw(path: &std::path::Path) -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .busy_timeout(Duration::from_secs(5));
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap()
}

async fn insert_description(pool: &SqlitePool, frame_id: &str, text: &str) {
    sqlx::query(
        "INSERT INTO semantic_frame_revisions (
            frame_id, store_namespace, source_id, track_id, start_us, end_us,
            payload_sha256, payload_crc32, retained_state, revision_seq, registered_at_us
         ) VALUES (?1, 'ns', 'cam-a', 'mjpeg', 1, 2, 'digest', 0, 'retained', 1, 0)",
    )
    .bind(frame_id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO semantic_frame_descriptions (
            frame_id, description_text, description_provider, description_model,
            prompt_digest, preprocessing_digest, text_hash, description_generation
         ) VALUES (?1, ?2, 'bedrock', 'model', 'p', 'q', 'h', 1)",
    )
    .bind(frame_id)
    .bind(text)
    .execute(pool)
    .await
    .unwrap();
}

async fn match_frame_ids(pool: &SqlitePool, token: &str) -> Vec<String> {
    sqlx::query(
        "SELECT frame_id FROM semantic_frame_descriptions_fts WHERE description_text MATCH ?1",
    )
    .bind(token)
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.get("frame_id"))
    .collect()
}

async fn user_version(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn fts_row_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM semantic_frame_descriptions_fts")
        .fetch_one(pool)
        .await
        .unwrap()
}

const PRE_FTS_SCHEMA: &str = "
CREATE TABLE semantic_frame_revisions (
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
CREATE TABLE semantic_frame_descriptions (
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
";

async fn write_legacy_file(path: &std::path::Path, frame_id: &str, text: &str) {
    let _ = std::fs::remove_file(path);
    let pool = connect_raw(path).await;
    for statement in PRE_FTS_SCHEMA.split(';') {
        let statement = statement.trim();
        if statement.is_empty() {
            continue;
        }
        sqlx::query(statement).execute(&pool).await.unwrap();
    }
    insert_description(&pool, frame_id, text).await;
    sqlx::query("PRAGMA user_version = 0")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

#[tokio::test]
async fn fresh_file_creates_unicode_fts_and_triggers() {
    let path = temp_path("fresh");
    let _ = std::fs::remove_file(&path);
    let store = SemanticSqliteStore::open(&path).await.unwrap();
    let tokenizer: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE name = 'semantic_frame_descriptions_fts'",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert!(tokenizer.to_ascii_lowercase().contains("unicode61"));
    let triggers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger'
         AND name IN (
            'semantic_frame_descriptions_fts_ai',
            'semantic_frame_descriptions_fts_ad',
            'semantic_frame_descriptions_fts_au'
         )",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(triggers, 3);
    assert_eq!(
        user_version(store.pool()).await,
        SemanticSqliteStore::USER_VERSION
    );
    insert_description(store.pool(), "frame-fresh", "pipette tip attached").await;
    assert_eq!(
        match_frame_ids(store.pool(), "pipette").await,
        vec!["frame-fresh"]
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn legacy_file_upgrades_backfills_and_reopen_is_idempotent() {
    let path = temp_path("legacy");
    write_legacy_file(&path, "frame-old", "incubator door ajar").await;
    let store = SemanticSqliteStore::open(&path).await.unwrap();
    assert_eq!(
        user_version(store.pool()).await,
        SemanticSqliteStore::USER_VERSION
    );
    assert_eq!(
        match_frame_ids(store.pool(), "incubator").await,
        vec!["frame-old"]
    );
    assert_eq!(fts_row_count(store.pool()).await, 1);
    store.pool().close().await;

    let reopened = SemanticSqliteStore::open(&path).await.unwrap();
    assert_eq!(fts_row_count(reopened.pool()).await, 1);
    assert_eq!(
        match_frame_ids(reopened.pool(), "incubator").await,
        vec!["frame-old"]
    );
    assert_eq!(
        user_version(reopened.pool()).await,
        SemanticSqliteStore::USER_VERSION
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn missing_fts_objects_backfill_without_attach_or_dvr_migration() {
    let dir = std::env::temp_dir().join(format!(
        "a2a-lab-devkit-semantic-fts-dvr-{}",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&dir);
    let index = dir.join("index.sqlite");
    std::fs::write(&index, b"dvr-index-untouched").unwrap();
    let path = dir.join("semantic.sqlite");
    write_legacy_file(&path, "frame-gap", "plate misaligned on deck").await;
    let store = SemanticSqliteStore::open(&path).await.unwrap();
    assert_eq!(
        match_frame_ids(store.pool(), "misaligned").await,
        vec!["frame-gap"]
    );
    assert_eq!(std::fs::read(&index).unwrap(), b"dvr-index-untouched");
    let attached: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_database_list")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(attached, 1);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&index);
    let _ = std::fs::remove_dir(&dir);
}

#[tokio::test]
async fn description_triggers_sync_insert_update_delete() {
    let path = temp_path("triggers");
    let _ = std::fs::remove_file(&path);
    let store = SemanticSqliteStore::open(&path).await.unwrap();
    insert_description(store.pool(), "frame-sync", "empty deck slot").await;
    assert_eq!(
        match_frame_ids(store.pool(), "deck").await,
        vec!["frame-sync"]
    );

    sqlx::query(
        "UPDATE semantic_frame_descriptions
         SET description_text = 'pipette rack loaded'
         WHERE frame_id = 'frame-sync'",
    )
    .execute(store.pool())
    .await
    .unwrap();
    assert!(match_frame_ids(store.pool(), "deck").await.is_empty());
    assert_eq!(
        match_frame_ids(store.pool(), "pipette").await,
        vec!["frame-sync"]
    );

    sqlx::query("DELETE FROM semantic_frame_descriptions WHERE frame_id = 'frame-sync'")
        .execute(store.pool())
        .await
        .unwrap();
    assert!(match_frame_ids(store.pool(), "pipette").await.is_empty());
    assert_eq!(fts_row_count(store.pool()).await, 0);
    let _ = std::fs::remove_file(&path);
}
