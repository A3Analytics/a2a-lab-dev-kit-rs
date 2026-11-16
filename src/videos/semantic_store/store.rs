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
    /// `PRAGMA user_version` after the FTS5 lexical index is present and backfilled.
    pub const USER_VERSION: i64 = 1;

    /// Creates or opens `path` and applies the final semantic schema.
    ///
    /// In-place upgrade covers pre-FTS semantic files (`user_version` 0 and/or missing
    /// FTS objects). Combined DVR `index.sqlite` is never attached or migrated.
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
            .map_err(db_error)?;
        let user_version = pragma_user_version(&pool).await?;
        let fts_present = fts_objects_present(&pool).await?;
        apply_schema(&pool).await?;
        if user_version < Self::USER_VERSION || !fts_present {
            backfill_description_fts(&pool).await?;
            set_pragma_user_version(&pool, Self::USER_VERSION).await?;
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
            .map_err(db_error)
    }
}

fn db_error(error: impl std::fmt::Display) -> A2aLabError {
    A2aLabError::unavailable(format!("semantic index database: {error}"))
}

async fn apply_schema(pool: &SqlitePool) -> Result<(), A2aLabError> {
    for statement in sql_statements(SCHEMA) {
        sqlx::query(statement)
            .execute(pool)
            .await
            .map_err(db_error)?;
    }
    Ok(())
}

async fn pragma_user_version(pool: &SqlitePool) -> Result<i64, A2aLabError> {
    sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await
        .map_err(db_error)
}

async fn set_pragma_user_version(pool: &SqlitePool, version: i64) -> Result<(), A2aLabError> {
    sqlx::query(&format!("PRAGMA user_version = {version}"))
        .execute(pool)
        .await
        .map_err(db_error)?;
    Ok(())
}

async fn fts_objects_present(pool: &SqlitePool) -> Result<bool, A2aLabError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE name = 'semantic_frame_descriptions_fts'
            OR name IN (
                'semantic_frame_descriptions_fts_ai',
                'semantic_frame_descriptions_fts_ad',
                'semantic_frame_descriptions_fts_au'
            )",
    )
    .fetch_one(pool)
    .await
    .map_err(db_error)?;
    Ok(count == 4)
}

async fn backfill_description_fts(pool: &SqlitePool) -> Result<(), A2aLabError> {
    sqlx::query("DELETE FROM semantic_frame_descriptions_fts")
        .execute(pool)
        .await
        .map_err(db_error)?;
    sqlx::query(
        "INSERT INTO semantic_frame_descriptions_fts(frame_id, description_text)
         SELECT frame_id, description_text FROM semantic_frame_descriptions",
    )
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(())
}

fn sql_statements(sql: &str) -> Vec<&str> {
    let bytes = sql.as_bytes();
    let mut statements = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut in_string = false;
    let mut in_line_comment = false;
    let mut begin_depth = 0;
    let mut case_depth = 0;
    let mut saw_create = false;
    let mut in_trigger = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_line_comment {
            if byte == b'\n' {
                in_line_comment = false;
            }
            index += 1;
            continue;
        }
        if in_string {
            if byte == b'\'' {
                if index + 1 < bytes.len() && bytes[index + 1] == b'\'' {
                    index += 2;
                    continue;
                }
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'\'' {
            in_string = true;
            saw_create = false;
            index += 1;
            continue;
        }
        if byte == b'-' && index + 1 < bytes.len() && bytes[index + 1] == b'-' {
            in_line_comment = true;
            index += 2;
            continue;
        }
        if byte == b';' && begin_depth == 0 {
            let statement = sql[start..index].trim();
            if !statement.is_empty() {
                statements.push(statement);
            }
            index += 1;
            start = index;
            in_trigger = false;
            saw_create = false;
            case_depth = 0;
            continue;
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let token_start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            classify_sql_token(
                &sql[token_start..index],
                &mut saw_create,
                &mut in_trigger,
                &mut begin_depth,
                &mut case_depth,
            );
            continue;
        }
        index += 1;
    }
    let statement = sql[start..].trim();
    if !statement.is_empty() {
        statements.push(statement);
    }
    statements
}

fn classify_sql_token(
    token: &str,
    saw_create: &mut bool,
    in_trigger: &mut bool,
    begin_depth: &mut u32,
    case_depth: &mut u32,
) {
    let token = token.to_ascii_uppercase();
    if token == "CREATE" {
        *saw_create = true;
        return;
    }
    if token == "TRIGGER" && *saw_create {
        *in_trigger = true;
        *saw_create = false;
        return;
    }
    if !matches!(
        token.as_str(),
        "OR" | "IF" | "NOT" | "EXISTS" | "TEMP" | "TEMPORARY"
    ) {
        *saw_create = false;
    }
    match token.as_str() {
        "CASE" => *case_depth += 1,
        "BEGIN" if *in_trigger => *begin_depth += 1,
        "END" if *case_depth > 0 => *case_depth -= 1,
        "END" if *begin_depth > 0 => *begin_depth -= 1,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::sql_statements;

    #[test]
    fn sql_statements_keep_trigger_bodies_intact() {
        let sql = "
CREATE TABLE t (id TEXT);
CREATE TRIGGER t_ai AFTER INSERT ON t BEGIN
    INSERT INTO fts(id) VALUES (new.id);
    DELETE FROM fts WHERE id = new.id;
END;
";
        let statements = sql_statements(sql);
        assert_eq!(statements.len(), 2);
        assert!(statements[0].starts_with("CREATE TABLE"));
        assert!(statements[1].contains("INSERT INTO fts"));
        assert!(statements[1].contains("DELETE FROM fts"));
        assert!(statements[1].ends_with("END"));
    }

    #[test]
    fn sql_statements_ignore_semicolons_in_strings_and_comments() {
        let sql = "INSERT INTO t VALUES ('a;b'); -- trailing; comment\nSELECT 1;";
        let statements = sql_statements(sql);
        assert_eq!(statements[0], "INSERT INTO t VALUES ('a;b')");
        assert!(statements[1].contains("SELECT 1"));
        assert!(!statements[1].contains("VALUES"));
    }
}
