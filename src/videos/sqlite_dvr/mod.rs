//! Non-optional SQLite minute-chunk DVR boundary.

mod chunk;
mod committed_frame;
mod coverage;
mod db;
mod dvr_persist_hook;
mod h264_epoch;
mod media_store;
mod payload;
mod recovery;
mod retention;
mod store;

pub use chunk::{chunk_relpath, crc32, minute_start_us};
pub use committed_frame::DvrCommittedFrame;
pub use dvr_persist_hook::DvrPersistHook;
pub(crate) use payload::load_payload_bytes_from_row;
pub use store::{DEFAULT_DISK_VIDEO_RETENTION_SECS, SqliteChunkDvrStore};
