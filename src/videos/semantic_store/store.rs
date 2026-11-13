//! Fresh semantic SQLite file. Callers do not migrate combined DVR databases.

use std::collections::HashSet;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

use crate::error::A2aLabError;

const SCHEMA: &str = include_str!("schema.sql");

/// Jobs, manifests, embeddings, and decisions for one lab.
#[derive(Clone)]
pub struct SemanticSqliteStore {
    path: PathBuf,
    pool: SqlitePool,
}

impl SemanticSqliteStore {
    /// Creates or opens `path` and applies the final semantic schema.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, A2aLabError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(A2aLabError::invalid(
                "semantic_sqlite_path",
                "must not be empty",
            ));
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| {
                A2aLabError::unavailable(format!("create {}: {error}", parent.display()))
            })?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|error| {
                A2aLabError::unavailable(format!("semantic index database: {error}"))
            })?;
        for statement in SCHEMA.split(';') {
            let statement = statement.trim();
            if statement.is_empty() {
                continue;
            }
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| {
                    A2aLabError::unavailable(format!("semantic index database: {error}"))
                })?;
        }
        Ok(Self {
            path: path.to_path_buf(),
            pool,
        })
    }

    /// Semantic database path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Pool for semantic tables only.
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Current search rows whose media keys are in `keys`.
    pub async fn current_revision_keys(
        &self,
        keys: &[(String, String, i64)],
    ) -> Result<HashSet<(String, String, i64)>, A2aLabError> {
        if keys.is_empty() {
            return Ok(HashSet::new());
        }
        let mut sql = String::from(
            "SELECT source_id, track_id, start_us
             FROM semantic_frame_revisions
             WHERE search_current = 1 AND (",
        );
        for (index, _) in keys.iter().enumerate() {
            if index > 0 {
                sql.push_str(" OR ");
            }
            let base = index * 3;
            let _ = write!(
                sql,
                "(source_id = ?{} AND track_id = ?{} AND start_us = ?{})",
                base + 1,
                base + 2,
                base + 3
            );
        }
        sql.push(')');
        let mut query = sqlx::query(&sql);
        for (source_id, track_id, start_us) in keys {
            query = query.bind(source_id).bind(track_id).bind(start_us);
        }
        let rows = query.fetch_all(&self.pool).await.map_err(|error| {
            A2aLabError::unavailable(format!("semantic index database: {error}"))
        })?;
        rows.into_iter()
            .map(|row| {
                Ok((
                    row.try_get("source_id")?,
                    row.try_get("track_id")?,
                    row.try_get("start_us")?,
                ))
            })
            .collect::<Result<HashSet<_>, sqlx::Error>>()
            .map_err(|error| A2aLabError::unavailable(format!("semantic index database: {error}")))
    }
}
