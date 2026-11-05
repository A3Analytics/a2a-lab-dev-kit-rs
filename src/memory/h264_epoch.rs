//! H.264 epoch insertion kept out of the memory video provider file.

use crate::error::A2aLabError;
use crate::id::{DvrTrackId, VideoSourceId};
use crate::videos::{DvrPictureProperties, VideoCodec};

use super::videos::{MemoryVideos, StoredDvrTrack, require_source};

impl MemoryVideos {
    /// Opens an H.264 epoch. The same track id cannot replace initialization or picture size.
    pub async fn insert_h264_epoch(
        &self,
        source_id: VideoSourceId,
        track_id: DvrTrackId,
        codec_initialization: Vec<u8>,
        picture: DvrPictureProperties,
    ) -> Result<(), A2aLabError> {
        if codec_initialization.is_empty() {
            return Err(A2aLabError::invalid(
                "codec_initialization",
                "an H.264 epoch requires decoder initialization",
            ));
        }
        let mut state = self.lock().await;
        require_source(&state, &source_id)?;
        if state
            .dvr_tracks
            .iter()
            .any(|track| track.source_id == source_id && track.id == track_id)
        {
            return Err(A2aLabError::invalid(
                "id",
                "H.264 epoch initialization and picture properties are immutable",
            ));
        }
        state.dvr_tracks.push(StoredDvrTrack {
            source_id,
            id: track_id,
            codec: VideoCodec::H264,
            codec_initialization,
            picture: Some(picture),
        });
        Ok(())
    }
}
