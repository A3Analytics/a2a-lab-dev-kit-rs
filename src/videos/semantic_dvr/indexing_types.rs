//! Durable indexing job types.

use crate::error::A2aLabError;
use sqlx::{Sqlite, Transaction};

use super::db::db_error;

/// Observable indexing job lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexingJobState {
    Pending,
    Leased,
    Described,
    VectorPending,
    Ready,
    RetryableFailed,
    TerminalFailed,
    Expired,
    Skipped,
}

impl IndexingJobState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Described => "described",
            Self::VectorPending => "vector_pending",
            Self::Ready => "ready",
            Self::RetryableFailed => "retryable_failed",
            Self::TerminalFailed => "terminal_failed",
            Self::Expired => "expired",
            Self::Skipped => "skipped",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, A2aLabError> {
        match value {
            "pending" => Ok(Self::Pending),
            "leased" => Ok(Self::Leased),
            "described" => Ok(Self::Described),
            "vector_pending" => Ok(Self::VectorPending),
            "ready" => Ok(Self::Ready),
            "retryable_failed" => Ok(Self::RetryableFailed),
            "terminal_failed" => Ok(Self::TerminalFailed),
            "expired" => Ok(Self::Expired),
            "skipped" => Ok(Self::Skipped),
            _ => Err(A2aLabError::invalid(
                "job_state",
                format!("unknown {value}"),
            )),
        }
    }
}

/// One durable job row joined with runtime state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexingJobRecord {
    pub(crate) job_id: String,
    pub(crate) frame_id: String,
    pub(crate) job_kind: String,
    pub(crate) state: IndexingJobState,
    pub(crate) revision_generation: u32,
    pub(crate) lease_owner: Option<String>,
    pub(crate) lease_expires_at_us: Option<i64>,
    pub(crate) description_generation: Option<u64>,
    pub(crate) description_attempts: u32,
    pub(crate) embedding_attempts: u32,
    pub(crate) last_retry_reason: Option<String>,
    pub(crate) next_retry_at_us: i64,
    pub(crate) last_error_at_us: Option<i64>,
    pub(crate) completed_at_us: Option<i64>,
}

impl IndexingJobRecord {
    #[must_use]
    pub fn job_id(&self) -> &str {
        &self.job_id
    }

    #[must_use]
    pub fn frame_id(&self) -> &str {
        &self.frame_id
    }

    #[must_use]
    pub fn job_kind(&self) -> &str {
        &self.job_kind
    }

    #[must_use]
    pub const fn state(&self) -> IndexingJobState {
        self.state
    }

    #[must_use]
    pub const fn revision_generation(&self) -> u32 {
        self.revision_generation
    }

    #[must_use]
    pub fn lease_owner(&self) -> Option<&str> {
        self.lease_owner.as_deref()
    }

    #[must_use]
    pub const fn description_attempts(&self) -> u32 {
        self.description_attempts
    }

    #[must_use]
    pub const fn embedding_attempts(&self) -> u32 {
        self.embedding_attempts
    }

    #[must_use]
    pub fn last_retry_reason(&self) -> Option<&str> {
        self.last_retry_reason.as_deref()
    }

    #[must_use]
    pub const fn next_retry_at_us(&self) -> i64 {
        self.next_retry_at_us
    }

    #[must_use]
    pub fn description_generation(&self) -> Option<u64> {
        self.description_generation
    }
}

/// Bounded work-queue snapshot for one worker tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexingWorkerQueueStats {
    pub active_jobs: u64,
    /// Advisory only: queue depth exceeded `max_queue_backlog`; workers must keep processing.
    pub backlog_over_limit: bool,
}

pub(crate) async fn ensure_job_runtime_tx(
    tx: &mut Transaction<'_, Sqlite>,
    job_id: &str,
) -> Result<(), A2aLabError> {
    sqlx::query(
        "INSERT INTO semantic_indexing_job_runtime (job_id)
         VALUES (?1)
         ON CONFLICT(job_id) DO NOTHING",
    )
    .bind(job_id)
    .execute(&mut **tx)
    .await
    .map_err(db_error)?;
    Ok(())
}
