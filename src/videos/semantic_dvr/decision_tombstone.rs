use sqlx::{Sqlite, Transaction};

use crate::error::A2aLabError;

use super::db::db_error;
use super::engine::SemanticChunkDvrStore;

impl SemanticChunkDvrStore {
    pub(crate) async fn tombstone_decisions_for_frame_tx(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        frame_id: &str,
        retained_state: &str,
        tombstoned_at_us: i64,
    ) -> Result<(), A2aLabError> {
        sqlx::query(
            "INSERT INTO frame_decision_tombstones (frame_id, retained_state, tombstoned_at_us)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(frame_id) DO UPDATE SET
                retained_state = excluded.retained_state,
                tombstoned_at_us = excluded.tombstoned_at_us",
        )
        .bind(frame_id)
        .bind(retained_state)
        .bind(tombstoned_at_us)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
        sqlx::query("UPDATE frame_decisions SET tombstoned = 1 WHERE frame_id = ?1")
            .bind(frame_id)
            .execute(&mut **tx)
            .await
            .map_err(db_error)?;
        Ok(())
    }
}
