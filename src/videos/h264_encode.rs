//! All-IDR H.264 encoding through the bundled OpenH264 encoder.
//!
//! Every picture is forced to an IDR. Frame skip is off, the caller supplies
//! the frame rate and bitrate, and VUI matrix/range follow the I420 picture.
//! Each returned bitstream is an owned copy, so the next encode cannot alias it.
//! This module does not open a camera and does not write a DVR epoch.

use openh264::decoder::Decoder;
use openh264::encoder::{
    BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile, RateControlMode,
    VuiConfig,
};
use openh264::formats::{YUVBuffer, YUVSource};

use crate::error::A2aLabError;
use crate::videos::{CaptureIdentity, ColorMatrix, ColorProperties, ColorRange};

/// Bundled OpenH264 crate version. The codec is BSD-2-Clause and is compiled
/// from source; this crate does not download a runtime library or use FFmpeg.
pub const OPENH264_NOTICE: &str = "openh264 0.9.9 BSD-2-Clause";

/// Largest width or height accepted by this encoder.
pub const MAX_ENCODER_EDGE: u32 = 3840;

/// One tightly packed I420 picture.
#[derive(Debug, Clone, Copy)]
pub struct I420Picture<'a> {
    width: u32,
    height: u32,
    y: &'a [u8],
    u: &'a [u8],
    v: &'a [u8],
    properties: ColorProperties,
}

impl<'a> I420Picture<'a> {
    /// Borrows Y, U, and V planes. The planes must be tightly packed.
    #[must_use]
    pub const fn new(
        width: u32,
        height: u32,
        y: &'a [u8],
        u: &'a [u8],
        v: &'a [u8],
        properties: ColorProperties,
    ) -> Self {
        Self {
            width,
            height,
            y,
            u,
            v,
            properties,
        }
    }

    /// Matrix and range that the encoder writes into VUI.
    #[must_use]
    pub const fn properties(&self) -> ColorProperties {
        self.properties
    }
}

/// One owned all-IDR access unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedAccessUnit {
    bytes: Vec<u8>,
    initialization: Vec<u8>,
    keyframe: bool,
    identity: CaptureIdentity,
    timestamp_ns: u64,
}

impl EncodedAccessUnit {
    /// Annex B bytes, copied before the next encode.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// SPS/PPS copied out of this access unit.
    #[must_use]
    pub fn initialization(&self) -> &[u8] {
        &self.initialization
    }

    /// Always true for this encoder. Checked from the NAL types.
    #[must_use]
    pub const fn keyframe(&self) -> bool {
        self.keyframe
    }

    /// Identity supplied with the picture.
    #[must_use]
    pub const fn identity(&self) -> &CaptureIdentity {
        &self.identity
    }

    /// Timestamp supplied with the picture, in nanoseconds.
    #[must_use]
    pub const fn timestamp_ns(&self) -> u64 {
        self.timestamp_ns
    }
}

/// OpenH264 encoder fixed to one picture size, rate, and color description.
pub struct AllIdrEncoder {
    encoder: Encoder,
    width: u32,
    height: u32,
    fps: f32,
    properties: ColorProperties,
    closed: bool,
}

impl AllIdrEncoder {
    /// Opens an encoder. `fps` is the output rate and is not fixed at 5.
    pub fn new(
        width: u32,
        height: u32,
        fps: f32,
        bitrate_bps: u32,
        properties: ColorProperties,
    ) -> Result<Self, A2aLabError> {
        validate_picture(width, height)?;
        if fps <= 0.0 {
            return Err(A2aLabError::invalid("fps", "must be positive"));
        }
        if bitrate_bps == 0 {
            return Err(A2aLabError::invalid("bitrate", "must be positive"));
        }
        let vui = vui_for(properties)?;
        let config = EncoderConfig::new()
            .skip_frames(false)
            .bitrate(BitRate::from_bps(bitrate_bps))
            .max_frame_rate(FrameRate::from_hz(fps))
            .rate_control_mode(RateControlMode::Bitrate)
            .profile(Profile::Baseline)
            .intra_frame_period(IntraFramePeriod::from_num_frames(1))
            .num_threads(1)
            .scene_change_detect(false)
            .vui(vui);
        let encoder = Encoder::with_api_config(openh264::OpenH264API::from_source(), config)
            .map_err(encode_error)?;
        Ok(Self {
            encoder,
            width,
            height,
            fps,
            properties,
            closed: false,
        })
    }

    /// Output frame rate supplied at construction.
    #[must_use]
    pub const fn fps(&self) -> f32 {
        self.fps
    }

    /// Color description written into the sequence.
    #[must_use]
    pub const fn properties(&self) -> ColorProperties {
        self.properties
    }

    /// Encodes one picture and returns an owned Annex B access unit.
    pub fn encode(
        &mut self,
        picture: I420Picture<'_>,
        identity: &CaptureIdentity,
        timestamp_ns: u64,
    ) -> Result<EncodedAccessUnit, A2aLabError> {
        if self.closed {
            return Err(A2aLabError::invalid("encoder", "encoder is shut down"));
        }
        if picture.width != self.width || picture.height != self.height {
            return Err(A2aLabError::invalid(
                "picture",
                "picture size does not match the encoder; open a new encoder for the new size",
            ));
        }
        if picture.properties != self.properties {
            return Err(A2aLabError::invalid(
                "picture",
                "picture color does not match the encoder VUI",
            ));
        }
        validate_planes(picture)?;
        let mut packed = Vec::with_capacity(picture.y.len() + picture.u.len() + picture.v.len());
        packed.extend_from_slice(picture.y);
        packed.extend_from_slice(picture.u);
        packed.extend_from_slice(picture.v);
        let buffer = YUVBuffer::from_vec(packed, self.width as usize, self.height as usize);
        self.encoder.force_intra_frame();
        let bitstream = self.encoder.encode(&buffer).map_err(encode_error)?;
        let owned = bitstream.to_vec();
        if owned.is_empty() || !contains_idr(&owned) || contains_non_idr_slice(&owned) {
            return Err(A2aLabError::invalid(
                "bitstream",
                "encoder did not emit an all-IDR access unit",
            ));
        }
        let initialization = parameter_sets(&owned);
        if initialization.is_empty() {
            return Err(A2aLabError::invalid(
                "bitstream",
                "access unit is missing SPS/PPS initialization",
            ));
        }
        Ok(EncodedAccessUnit {
            bytes: owned,
            initialization,
            keyframe: true,
            identity: identity.clone(),
            timestamp_ns,
        })
    }

    /// Rejects later encodes. Already returned access units stay owned by the caller.
    pub fn shutdown(&mut self) {
        self.closed = true;
    }
}

/// Decodes one owned Annex B access unit back to tight I420 luma.
pub fn decode_luma(annex_b: &[u8]) -> Result<Vec<u8>, A2aLabError> {
    let mut decoder = Decoder::new().map_err(encode_error)?;
    let picture = decoder
        .decode(annex_b)
        .map_err(encode_error)?
        .ok_or_else(|| A2aLabError::invalid("bitstream", "decoder returned no picture"))?;
    let (width, stride) = (picture.dimensions().0, picture.strides().0);
    let mut luma = Vec::with_capacity(width * picture.dimensions().1);
    for row in 0..picture.dimensions().1 {
        let start = row * stride;
        luma.extend_from_slice(&picture.y()[start..start + width]);
    }
    Ok(luma)
}

fn validate_picture(width: u32, height: u32) -> Result<(), A2aLabError> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(A2aLabError::invalid(
            "picture",
            "I420 width and height must be positive even numbers",
        ));
    }
    if width > MAX_ENCODER_EDGE || height > MAX_ENCODER_EDGE {
        return Err(A2aLabError::invalid(
            "picture",
            "picture edge exceeds the encoder limit",
        ));
    }
    Ok(())
}

fn validate_planes(picture: I420Picture<'_>) -> Result<(), A2aLabError> {
    let y_len = usize::try_from(picture.width)
        .ok()
        .and_then(|width| {
            usize::try_from(picture.height)
                .ok()
                .map(|height| width * height)
        })
        .ok_or_else(|| A2aLabError::invalid("picture", "plane size overflow"))?;
    let chroma = y_len / 4;
    if picture.y.len() != y_len || picture.u.len() != chroma || picture.v.len() != chroma {
        return Err(A2aLabError::invalid(
            "picture",
            "I420 plane lengths do not match the dimensions",
        ));
    }
    Ok(())
}

fn vui_for(properties: ColorProperties) -> Result<VuiConfig, A2aLabError> {
    let full = match properties.range() {
        ColorRange::Full => true,
        ColorRange::Limited => false,
        ColorRange::Unknown => {
            return Err(A2aLabError::invalid(
                "color",
                "encoder VUI needs an explicit color range",
            ));
        }
    };
    let config = match properties.matrix() {
        ColorMatrix::Bt601 => VuiConfig::bt601(),
        ColorMatrix::Bt709 => VuiConfig::bt709(),
        ColorMatrix::Bt2020 => VuiConfig::bt2020(),
        ColorMatrix::Unknown => {
            return Err(A2aLabError::invalid(
                "color",
                "encoder VUI needs an explicit color matrix",
            ));
        }
    };
    Ok(config.full_range(full))
}

fn encode_error(error: impl std::fmt::Display) -> A2aLabError {
    A2aLabError::invalid("encoder", error.to_string())
}

fn parameter_sets(annex_b: &[u8]) -> Vec<u8> {
    let mut sets = Vec::new();
    for (start, nal_type) in nal_headers(annex_b) {
        if nal_type == 7 || nal_type == 8 {
            let next = nal_headers(annex_b)
                .find(|(offset, _)| *offset > start)
                .map_or(annex_b.len(), |(offset, _)| {
                    offset.saturating_sub(start_code_len(annex_b, offset))
                });
            let prefix = start.saturating_sub(start_code_len(annex_b, start));
            sets.extend_from_slice(&annex_b[prefix..next]);
        }
    }
    sets
}

fn start_code_len(annex_b: &[u8], header: usize) -> usize {
    if header >= 4 && annex_b[header - 4..header] == [0, 0, 0, 1] {
        4
    } else if header >= 3 && annex_b[header - 3..header] == [0, 0, 1] {
        3
    } else {
        0
    }
}

fn nal_headers(annex_b: &[u8]) -> impl Iterator<Item = (usize, u8)> + '_ {
    annex_b
        .windows(4)
        .enumerate()
        .filter_map(|(index, window)| {
            let header = if window.starts_with(&[0, 0, 0, 1]) {
                index + 4
            } else if window.starts_with(&[0, 0, 1]) {
                index + 3
            } else {
                return None;
            };
            annex_b.get(header).map(|byte| (header, byte & 0x1F))
        })
}

fn contains_idr(annex_b: &[u8]) -> bool {
    nal_types(annex_b).contains(&5)
}

fn contains_non_idr_slice(annex_b: &[u8]) -> bool {
    nal_types(annex_b).contains(&1)
}

fn nal_types(annex_b: &[u8]) -> Vec<u8> {
    let mut types = Vec::new();
    let mut index = 0;
    while index + 3 < annex_b.len() {
        let start = if annex_b[index..].starts_with(&[0, 0, 0, 1]) {
            index + 4
        } else if annex_b[index..].starts_with(&[0, 0, 1]) {
            index + 3
        } else {
            index += 1;
            continue;
        };
        if start < annex_b.len() {
            types.push(annex_b[start] & 0x1F);
        }
        index = start.saturating_add(1);
    }
    types
}
