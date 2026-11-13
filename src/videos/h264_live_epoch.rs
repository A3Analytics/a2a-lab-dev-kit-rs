//! Deterministic live H.264 epoch identity from decoder initialization.

use sha2::{Digest, Sha256};

use crate::error::A2aLabError;
use crate::id::{DvrTrackId, VideoSourceId};
use crate::videos::{ColorEncodedFrame, DvrPictureProperties};

/// Stable epoch track ids from SPS/PPS initialization bytes.
#[derive(Debug, Default)]
pub struct H264LiveEpochWriter {
    track: Option<DvrTrackId>,
    initialization: Vec<u8>,
    picture: Option<DvrPictureProperties>,
}

impl H264LiveEpochWriter {
    /// Empty writer with no open epoch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Active track when initialization is unchanged.
    #[must_use]
    pub fn track_id(&self) -> Option<&DvrTrackId> {
        self.track.as_ref()
    }

    /// Resolves the epoch for one encoded picture.
    pub fn ensure(
        &mut self,
        source_id: &VideoSourceId,
        encoded: &ColorEncodedFrame,
    ) -> Result<(DvrTrackId, DvrPictureProperties), A2aLabError> {
        let init = encoded.initialization();
        if let (Some(track), Some(picture)) = (&self.track, self.picture)
            && self.initialization == init
        {
            return Ok((track.clone(), picture));
        }
        let picture =
            DvrPictureProperties::new(encoded.original().width(), encoded.original().height())?;
        let digest = Sha256::digest(init);
        let mut hex = String::with_capacity(16);
        for byte in &digest[..8] {
            use std::fmt::Write;
            let _ = write!(hex, "{byte:02x}");
        }
        let track = DvrTrackId::new(format!("h264-live-{}-{hex}", source_id.as_str()))?;
        self.track = Some(track.clone());
        self.initialization = init.to_vec();
        self.picture = Some(picture);
        Ok((track, picture))
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::videos::{
        CaptureClock, CaptureIdentity, CaptureTiming, CapturedColorFrame, ColorEncodedFrame,
        ColorMatrix, ColorProperties, ColorRange,
    };
    use crate::{UtcTimestamp, VideoSourceId};

    fn encoded(init: &[u8], width: u32, height: u32) -> ColorEncodedFrame {
        let source = VideoSourceId::new("cam").expect("source");
        let frame = CapturedColorFrame::yuy2(
            CaptureIdentity::new(source, 1),
            CaptureTiming::new(
                UtcTimestamp::parse("2020-01-01T00:00:00Z").expect("time"),
                CaptureClock::monotonic(0),
            ),
            width,
            height,
            4,
            &[16, 128, 16, 128, 16, 128, 16, 128],
            ColorProperties::new(ColorMatrix::Bt709, ColorRange::Limited),
            None,
        )
        .expect("frame");
        ColorEncodedFrame::test_fixture(frame, vec![0x65], init.to_vec(), true, None)
    }

    #[test]
    fn epoch_track_is_stable_for_one_initialization() {
        let source = VideoSourceId::new("bench").expect("source");
        let init = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42];
        let mut writer = H264LiveEpochWriter::new();
        let (first, _) = writer
            .ensure(&source, &encoded(&init, 2, 2))
            .expect("first");
        let (second, _) = writer
            .ensure(&source, &encoded(&init, 2, 2))
            .expect("second");
        assert_eq!(first, second);
    }

    #[test]
    fn epoch_track_changes_when_initialization_changes() {
        let source = VideoSourceId::new("bench").expect("source");
        let mut writer = H264LiveEpochWriter::new();
        let (first, _) = writer
            .ensure(&source, &encoded(&[1, 2, 3], 2, 2))
            .expect("first");
        let (second, _) = writer
            .ensure(&source, &encoded(&[4, 5, 6], 2, 2))
            .expect("second");
        assert_ne!(first, second);
    }
}
