//! Bounded JPEG decode for a selected MJPEG frame.
//!
//! Decoding uses `image` (zune-jpeg) and the `yuv` adapters. The original JPEG
//! bytes, capture identity, and capture timestamp stay on the result. When the
//! frame does not name a matrix or range, JPEG's defined BT.601 full-range
//! interpretation is applied and recorded on the decoded picture. This module
//! does not rescale and does not replace the capture clock.

use std::io::Cursor;

use image::{DynamicImage, ImageFormat, ImageReader, Limits};

use crate::error::A2aLabError;
use crate::videos::{
    CaptureFormat, CaptureIdentity, CaptureTiming, CapturedColorFrame, ColorMatrix,
    ColorProperties, ColorRange, InterleavedColor, InterleavedRgb, OwnedI420, rgb_to_i420,
    rgb_to_yuy2,
};

/// Largest accepted JPEG payload, in bytes.
pub const MAX_MJPEG_BYTES: usize = 8 * 1024 * 1024;

/// Which decoded layout a caller needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MjpegDecodeTarget {
    /// Planar I420 for the encoder.
    I420,
    /// Packed YUY2 for explicit color normalization.
    Yuy2,
}

/// Limits checked before the decoder allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MjpegDecodeLimits {
    max_bytes: usize,
    max_edge: u32,
}

impl MjpegDecodeLimits {
    /// Rejects a zero byte cap or a zero edge.
    pub fn new(max_bytes: usize, max_edge: u32) -> Result<Self, A2aLabError> {
        if max_bytes == 0 {
            return Err(A2aLabError::invalid("max_bytes", "must be positive"));
        }
        if max_edge == 0 {
            return Err(A2aLabError::invalid("max_edge", "must be positive"));
        }
        Ok(Self {
            max_bytes,
            max_edge,
        })
    }

    /// Default cap of [`MAX_MJPEG_BYTES`] and the encoder edge.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            max_bytes: MAX_MJPEG_BYTES,
            max_edge: crate::videos::MAX_ENCODER_EDGE,
        }
    }
}

/// Decoded color plus the untouched JPEG and the source identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedMjpeg {
    identity: CaptureIdentity,
    timing: CaptureTiming,
    jpeg: Vec<u8>,
    width: u32,
    height: u32,
    properties: ColorProperties,
    picture: DecodedPicture,
}

/// Owned pixels at the JPEG's original dimensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedPicture {
    /// Tight I420 planes.
    I420(OwnedI420),
    /// Packed `Y0 U0 Y1 V0`.
    Yuy2(Vec<u8>),
}

impl DecodedMjpeg {
    /// Source identity copied from the captured frame.
    #[must_use]
    pub const fn identity(&self) -> &CaptureIdentity {
        &self.identity
    }

    /// Original capture time. This is not the decode time.
    #[must_use]
    pub const fn timing(&self) -> &CaptureTiming {
        &self.timing
    }

    /// JPEG bytes copied from the captured frame.
    #[must_use]
    pub fn jpeg(&self) -> &[u8] {
        &self.jpeg
    }

    /// Decoded width. Matches the captured frame.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Decoded height. Matches the captured frame.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Matrix and range used for the conversion, including the JPEG fallback.
    #[must_use]
    pub const fn properties(&self) -> ColorProperties {
        self.properties
    }

    /// Decoded pixels.
    #[must_use]
    pub const fn picture(&self) -> &DecodedPicture {
        &self.picture
    }
}

/// Decodes one selected MJPEG frame. Raw YUY2 frames are rejected.
pub fn decode_mjpeg(
    frame: &CapturedColorFrame,
    target: MjpegDecodeTarget,
    limits: MjpegDecodeLimits,
) -> Result<DecodedMjpeg, A2aLabError> {
    if frame.format() != CaptureFormat::Mjpeg {
        return Err(A2aLabError::invalid(
            "frame",
            "MJPEG decode requires an MJPEG capture frame",
        ));
    }
    let jpeg = frame.bytes();
    if jpeg.len() > limits.max_bytes {
        return Err(A2aLabError::invalid(
            "jpeg",
            "compressed JPEG exceeds the byte limit",
        ));
    }
    if frame.width() > limits.max_edge || frame.height() > limits.max_edge {
        return Err(A2aLabError::invalid(
            "jpeg",
            "declared JPEG dimensions exceed the edge limit",
        ));
    }
    let image = read_jpeg(jpeg, limits, frame.width(), frame.height())?;
    let properties = jpeg_color(frame.color());
    let rgb = image.into_rgb8();
    if rgb.width() != frame.width() || rgb.height() != frame.height() {
        return Err(A2aLabError::invalid(
            "jpeg",
            "decoded dimensions do not match the captured frame",
        ));
    }
    let interleaved = InterleavedRgb::new(
        rgb.as_raw(),
        frame.width(),
        frame.height(),
        frame.width().saturating_mul(3),
        InterleavedColor::Rgb,
        properties,
    );
    let picture = match target {
        MjpegDecodeTarget::I420 => DecodedPicture::I420(rgb_to_i420(interleaved)?),
        MjpegDecodeTarget::Yuy2 => DecodedPicture::Yuy2(rgb_to_yuy2(interleaved)?),
    };
    Ok(DecodedMjpeg {
        identity: frame.identity().clone(),
        timing: frame.timing().clone(),
        jpeg: jpeg.to_vec(),
        width: frame.width(),
        height: frame.height(),
        properties,
        picture,
    })
}

fn read_jpeg(
    jpeg: &[u8],
    limits: MjpegDecodeLimits,
    width: u32,
    height: u32,
) -> Result<DynamicImage, A2aLabError> {
    let mut reader = ImageReader::new(Cursor::new(jpeg));
    reader.set_format(ImageFormat::Jpeg);
    let mut image_limits = Limits::default();
    image_limits.max_image_width = Some(limits.max_edge);
    image_limits.max_image_height = Some(limits.max_edge);
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    image_limits.max_alloc = Some(pixels.saturating_mul(4).max(1));
    reader.limits(image_limits);
    reader
        .decode()
        .map_err(|error| A2aLabError::invalid("jpeg", error.to_string()))
}

/// JPEG's specified default is BT.601 full range when the camera did not say.
fn jpeg_color(color: ColorProperties) -> ColorProperties {
    let matrix = match color.matrix() {
        ColorMatrix::Unknown => ColorMatrix::Bt601,
        matrix => matrix,
    };
    let range = match color.range() {
        ColorRange::Unknown => ColorRange::Full,
        range => range,
    };
    ColorProperties::new(matrix, range)
}
