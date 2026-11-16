//! Indexed chunk DVR paired with a separate semantic SQLite store.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use sqlx::Row;
use sqlx::sqlite::SqlitePool;

use crate::config::SemanticStreamSchedule;
use crate::error::A2aLabError;
use crate::id::{DvrStoreNamespace, VideoSourceId};
use crate::time::UtcTimestamp;
use crate::videos::semantic_store::SemanticSqliteStore;
use crate::videos::sqlite_dvr::{DvrCommittedFrame, DvrPersistHook, SqliteChunkDvrStore};
use crate::videos::sqlite_dvr::{crc32, load_payload_bytes_from_row};

use super::db::db_error;
use super::faults::SaveFaults;
use super::persist_hook::SemanticPersistHook;
use super::registration::INDEXING_PRIORITY_LIVE;

/// Restart-safe chunk DVR with semantic manifest and indexing in a second database.
#[derive(Clone)]
pub struct SemanticChunkDvrStore {
    chunk: SqliteChunkDvrStore,
    semantic: SemanticSqliteStore,
    default_namespace: Arc<RwLock<DvrStoreNamespace>>,
    pub(crate) schedules: Arc<RwLock<Vec<SemanticStreamSchedule>>>,
    pub(crate) save_faults: Arc<SaveFaults>,
    hook_error: Arc<Mutex<Option<A2aLabError>>>,
}

impl SemanticChunkDvrStore {
    /// Opens media at `root` and semantic state at `root/semantic.sqlite`.
    pub async fn open(
        root: impl AsRef<Path>,
        retain_secs: u64,
        default_namespace: DvrStoreNamespace,
    ) -> Result<Self, A2aLabError> {
        let root = root.as_ref();
        Self::open_at(
            root,
            retain_secs,
            root.join("semantic.sqlite"),
            default_namespace,
        )
        .await
    }

    /// Opens with an explicit semantic database path.
    pub async fn open_at(
        root: impl AsRef<Path>,
        retain_secs: u64,
        semantic_sqlite: impl AsRef<Path>,
        default_namespace: DvrStoreNamespace,
    ) -> Result<Self, A2aLabError> {
        if retain_secs == 0 {
            return Err(A2aLabError::invalid(
                "video_disk_retention_secs",
                "must be greater than zero",
            ));
        }
        let root = root.as_ref();
        if root.as_os_str().is_empty() {
            return Err(A2aLabError::invalid("video_dvr_root", "must not be empty"));
        }
        let store_slot: Arc<OnceLock<SemanticChunkDvrStore>> = Arc::new(OnceLock::new());
        let hook = Arc::new(SemanticPersistHook::new(Arc::clone(&store_slot)));
        let chunk = SqliteChunkDvrStore::open_with_hooks(root, retain_secs, vec![hook]).await?;
        Self::adopt_opened_at(chunk, store_slot, semantic_sqlite, default_namespace).await
    }

    /// Wraps an already-opened chunk store and opens the semantic database beside it.
    pub async fn adopt_opened(
        chunk: SqliteChunkDvrStore,
        store_slot: Arc<OnceLock<SemanticChunkDvrStore>>,
        default_namespace: DvrStoreNamespace,
    ) -> Result<Self, A2aLabError> {
        let semantic_path = chunk.root().join("semantic.sqlite");
        Self::adopt_opened_at(chunk, store_slot, semantic_path, default_namespace).await
    }

    /// Wraps media storage and opens the semantic database at `semantic_sqlite`.
    pub async fn adopt_opened_at(
        chunk: SqliteChunkDvrStore,
        store_slot: Arc<OnceLock<SemanticChunkDvrStore>>,
        semantic_sqlite: impl AsRef<Path>,
        default_namespace: DvrStoreNamespace,
    ) -> Result<Self, A2aLabError> {
        let semantic = SemanticSqliteStore::open(semantic_sqlite).await?;
        let store = Self {
            chunk,
            semantic,
            default_namespace: Arc::new(RwLock::new(default_namespace)),
            schedules: Arc::new(RwLock::new(Vec::new())),
            save_faults: Arc::new(SaveFaults::default()),
            hook_error: Arc::new(Mutex::new(None)),
        };
        let _ = store_slot.set(store.clone());
        Ok(store)
    }

    /// Persist-hook slot for declarative runtime startup.
    #[must_use]
    pub fn runtime_persist_hooks() -> (
        Vec<Arc<dyn DvrPersistHook>>,
        Arc<OnceLock<SemanticChunkDvrStore>>,
    ) {
        let store_slot = Arc::new(OnceLock::new());
        let hook = Arc::new(SemanticPersistHook::new(Arc::clone(&store_slot)));
        (vec![hook], store_slot)
    }

    #[must_use]
    pub fn chunk(&self) -> &SqliteChunkDvrStore {
        &self.chunk
    }

    #[must_use]
    pub fn semantic(&self) -> &SemanticSqliteStore {
        &self.semantic
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        self.chunk.root()
    }

    #[must_use]
    pub fn media_pool(&self) -> &SqlitePool {
        self.chunk.media_pool()
    }

    #[must_use]
    pub fn semantic_pool(&self) -> &SqlitePool {
        self.semantic.pool()
    }

    /// Injectable faults for hardware-free save-path tests.
    #[must_use]
    pub fn save_faults(&self) -> &Arc<SaveFaults> {
        &self.save_faults
    }

    pub fn set_default_namespace(&self, namespace: DvrStoreNamespace) {
        if let Ok(mut guard) = self.default_namespace.write() {
            *guard = namespace;
        }
    }

    fn default_namespace(&self) -> DvrStoreNamespace {
        self.default_namespace
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn permits_indexing_source(_source_id: &str) -> bool {
        true
    }

    pub async fn append_mjpeg_frame(
        &self,
        source_id: &VideoSourceId,
        captured_at: UtcTimestamp,
        frame_period_us: u64,
        jpeg: &[u8],
    ) -> Result<(), A2aLabError> {
        if self.save_faults.fail_payload_append() {
            return Err(A2aLabError::unavailable(
                "semantic DVR test fault: payload append blocked",
            ));
        }
        *self
            .hook_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.chunk
            .append_mjpeg_frame(source_id, captured_at, frame_period_us, jpeg)
            .await?;
        self.propagate_hook_failure()
    }

    pub async fn append_h264_sample(
        &self,
        source_id: &VideoSourceId,
        captured_at: UtcTimestamp,
        frame_period_us: u64,
        keyframe: bool,
        codec_init: &[u8],
        sample: &[u8],
    ) -> Result<(), A2aLabError> {
        if self.save_faults.fail_payload_append() {
            return Err(A2aLabError::unavailable(
                "semantic DVR test fault: payload append blocked",
            ));
        }
        *self
            .hook_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.chunk
            .append_h264_sample(
                source_id,
                captured_at,
                frame_period_us,
                keyframe,
                codec_init,
                sample,
            )
            .await?;
        self.propagate_hook_failure()
    }

    fn propagate_hook_failure(&self) -> Result<(), A2aLabError> {
        self.hook_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .map_or(Ok(()), Err)
    }

    pub(crate) async fn on_frame_committed(
        &self,
        frame: &DvrCommittedFrame,
    ) -> Result<(), A2aLabError> {
        if self.save_faults.fail_metadata_commit() {
            return Err(A2aLabError::unavailable(
                "semantic DVR test fault: metadata commit blocked",
            ));
        }
        let row = self
            .chunk_frame_row(frame.source_id(), frame.track_id(), frame.start())
            .await?;
        let bytes = load_payload_bytes_from_row(self.chunk().root(), &row).await?;
        let checksum = crc32(&bytes);
        if checksum != frame.payload_crc32() {
            return Err(A2aLabError::unavailable(
                "committed frame checksum mismatch",
            ));
        }
        let end = UtcTimestamp::from_unix_microseconds(row.try_get("end_us").map_err(db_error)?)?;
        let store_ns = self.default_namespace();
        let saved = crate::SavedFrameRef::from_payload_bytes(
            store_ns,
            frame.source_id().clone(),
            frame.track_id().clone(),
            frame.start(),
            &bytes,
            None,
        )?;
        let registered_at = UtcTimestamp::now();
        let mut tx = self.semantic_pool().begin().await.map_err(db_error)?;
        self.register_manifest_revision_tx(
            &mut tx,
            &saved,
            end,
            checksum,
            registered_at.as_micros(),
        )
        .await?;
        if Self::permits_indexing_source(frame.source_id().as_str()) {
            self.materialize_frame_eligibility_tx(
                &mut tx,
                frame.source_id().as_str(),
                frame.track_id().as_str(),
                saved.frame_id().as_str(),
                frame.start().as_micros(),
                INDEXING_PRIORITY_LIVE,
                registered_at.as_micros(),
            )
            .await?;
        }
        tx.commit().await.map_err(db_error)?;
        Ok(())
    }

    pub(crate) fn record_hook_failure(&self, error: A2aLabError) {
        *self
            .hook_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error);
    }

    pub(crate) async fn list_ready_media_keys(
        &self,
        limit: i64,
    ) -> Result<Vec<(String, String, i64)>, A2aLabError> {
        self.chunk.list_ready_media_keys(limit).await
    }

    pub(crate) async fn retained_media_keys(
        &self,
        keys: &[(String, String, i64)],
    ) -> Result<std::collections::HashSet<(String, String, i64)>, A2aLabError> {
        self.chunk.retained_media_keys(keys).await
    }

    pub(crate) async fn current_semantic_revision_keys(
        &self,
        keys: &[(String, String, i64)],
    ) -> Result<std::collections::HashSet<(String, String, i64)>, A2aLabError> {
        self.semantic.current_revision_keys(keys).await
    }
}
