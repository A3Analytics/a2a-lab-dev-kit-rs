//! Annex B picture framing for existing DVR samples.
//!
//! One picture becomes one owned length-prefixed sample. SPS and PPS stay in
//! the initialization and are not mixed into the media payload. This module
//! does not encode, decode, or convert color.

use crate::error::A2aLabError;
use crate::videos::{DvrMediaPayload, VideoCodec};

/// Owned H.264 picture split into initialization and one media sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H264PictureSample {
    initialization: Vec<u8>,
    sample: Vec<u8>,
    keyframe: bool,
}

impl H264PictureSample {
    /// Annex B SPS/PPS for this picture's configuration.
    #[must_use]
    pub fn initialization(&self) -> &[u8] {
        &self.initialization
    }

    /// 4-byte big-endian length-prefixed NALs for this picture.
    #[must_use]
    pub fn sample(&self) -> &[u8] {
        &self.sample
    }

    /// True when the picture contains a parsed IDR NAL.
    #[must_use]
    pub const fn keyframe(&self) -> bool {
        self.keyframe
    }

    /// DVR payload containing only the media sample.
    #[must_use]
    pub fn payload(&self) -> DvrMediaPayload {
        DvrMediaPayload::EncodedSample {
            codec: VideoCodec::H264,
            bytes: self.sample.clone(),
        }
    }
}

/// Frames one Annex B picture. Empty input and a picture with no slice fail.
pub fn frame_h264_picture(annex_b: &[u8]) -> Result<H264PictureSample, A2aLabError> {
    let nals = split_annex_b(annex_b)?;
    let mut initialization = Vec::new();
    let mut sample = Vec::new();
    let mut keyframe = false;
    let mut slices = 0_u32;
    for nal in nals {
        if nal.is_empty() {
            return Err(A2aLabError::invalid("annex_b", "H.264 NAL is empty"));
        }
        let nal_type = nal[0] & 0x1F;
        match nal_type {
            7 | 8 => {
                initialization.extend_from_slice(&[0, 0, 0, 1]);
                initialization.extend_from_slice(nal);
            }
            5 => {
                keyframe = true;
                slices += 1;
                push_length_prefixed(&mut sample, nal)?;
            }
            1 => {
                slices += 1;
                push_length_prefixed(&mut sample, nal)?;
            }
            _ => push_length_prefixed(&mut sample, nal)?,
        }
    }
    if slices == 0 || sample.is_empty() {
        return Err(A2aLabError::invalid(
            "annex_b",
            "H.264 picture does not contain a slice",
        ));
    }
    Ok(H264PictureSample {
        initialization,
        sample,
        keyframe,
    })
}

fn push_length_prefixed(sample: &mut Vec<u8>, nal: &[u8]) -> Result<(), A2aLabError> {
    let length = u32::try_from(nal.len())
        .map_err(|_| A2aLabError::invalid("annex_b", "H.264 NAL exceeds 4-byte length"))?;
    sample.extend_from_slice(&length.to_be_bytes());
    sample.extend_from_slice(nal);
    Ok(())
}

fn split_annex_b(bytes: &[u8]) -> Result<Vec<&[u8]>, A2aLabError> {
    let mut headers = Vec::new();
    let mut index = 0_usize;
    while index + 3 <= bytes.len() {
        if bytes[index..].starts_with(&[0, 0, 0, 1]) {
            headers.push(index + 4);
            index += 4;
        } else if bytes[index..].starts_with(&[0, 0, 1]) {
            headers.push(index + 3);
            index += 3;
        } else {
            index += 1;
        }
    }
    if headers.is_empty() {
        return Err(A2aLabError::invalid(
            "annex_b",
            "H.264 picture is not Annex B",
        ));
    }
    let mut nals = Vec::new();
    for (position, start) in headers.iter().copied().enumerate() {
        let end = match headers.get(position + 1).copied() {
            Some(next) if next >= 4 && bytes[next - 4..next] == [0, 0, 0, 1] => next - 4,
            Some(next) => next - 3,
            None => bytes.len(),
        };
        if end <= start {
            return Err(A2aLabError::invalid("annex_b", "H.264 NAL is empty"));
        }
        nals.push(&bytes[start..end]);
    }
    Ok(nals)
}
