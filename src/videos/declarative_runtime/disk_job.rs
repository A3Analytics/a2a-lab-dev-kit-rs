//! Async DVR persistence jobs decoupled from the capture thread.

use tokio::sync::mpsc;

use crate::id::VideoSourceId;
use crate::time::UtcTimestamp;
use crate::videos::SqliteChunkDvrStore;

#[derive(Debug)]
pub(crate) enum DiskJob {
    H264 {
        source_id: VideoSourceId,
        captured_at: UtcTimestamp,
        frame_period_us: u64,
        keyframe: bool,
        codec_init: Vec<u8>,
        sample: Vec<u8>,
    },
}

pub(crate) fn spawn_disk_writer(dvr: SqliteChunkDvrStore) -> mpsc::Sender<DiskJob> {
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move {
        while let Some(job) = rx.recv().await {
            if let DiskJob::H264 {
                source_id,
                captured_at,
                frame_period_us,
                keyframe,
                codec_init,
                sample,
            } = job
                && let Err(error) = dvr
                    .append_h264_sample(
                        &source_id,
                        captured_at,
                        frame_period_us,
                        keyframe,
                        &codec_init,
                        &sample,
                    )
                    .await
            {
                eprintln!("video DVR: {error}");
            }
        }
    });
    tx
}

pub(crate) fn try_queue_h264(
    tx: &mpsc::Sender<DiskJob>,
    source_id: &VideoSourceId,
    captured_at: UtcTimestamp,
    frame_period_us: u64,
    keyframe: bool,
    codec_init: Vec<u8>,
    sample: Vec<u8>,
) {
    let job = DiskJob::H264 {
        source_id: source_id.clone(),
        captured_at,
        frame_period_us,
        keyframe,
        codec_init,
        sample,
    };
    if let Err(error) = tx.try_send(job) {
        match error {
            mpsc::error::TrySendError::Full(_) => {
                eprintln!(
                    "video DVR: persistence queue full for {}",
                    source_id.as_str()
                );
            }
            mpsc::error::TrySendError::Closed(_) => {
                eprintln!("video DVR: persistence writer stopped");
            }
        }
    }
}
