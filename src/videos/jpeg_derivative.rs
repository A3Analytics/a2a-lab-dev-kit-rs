//! JPEG derivatives for raw YUY2 frames.
//!
//! An original camera JPEG is returned unchanged. Raw frames are converted
//! with the `yuv` crate and encoded with `image`'s JPEG encoder. Callers own
//! any cache and request one identity at a time.

use std::io::Cursor;

use image::ExtendedColorType;
use image::codecs::jpeg::JpegEncoder;

use crate::error::A2aLabError;
use crate::videos::color_convert::known_color;
use crate::videos::{CaptureFormat, CaptureIdentity, CapturedColorFrame};

/// Inclusive JPEG quality accepted by the encoder.
pub const MIN_JPEG_QUALITY: u8 = 1;
pub const MAX_JPEG_QUALITY: u8 = 100;

/// Limits for one derivative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegDerivativeLimits {
    max_bytes: usize,
}

impl JpegDerivativeLimits {
    /// Rejects a zero byte cap.
    pub fn new(max_bytes: usize) -> Result<Self, A2aLabError> {
        if max_bytes == 0 {
            return Err(A2aLabError::invalid("max_bytes", "must be positive"));
        }
        Ok(Self { max_bytes })
    }
}

/// JPEG bytes tied to the source frame identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegDerivative {
    identity: CaptureIdentity,
    width: u32,
    height: u32,
    jpeg: Vec<u8>,
    original: bool,
}

impl JpegDerivative {
    /// Source identity copied from the captured frame.
    #[must_use]
    pub const fn identity(&self) -> &CaptureIdentity {
        &self.identity
    }

    /// Width of the JPEG.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height of the JPEG.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Encoded or original JPEG bytes.
    #[must_use]
    pub fn jpeg(&self) -> &[u8] {
        &self.jpeg
    }

    /// Whether these bytes are the camera's original JPEG.
    #[must_use]
    pub const fn original(&self) -> bool {
        self.original
    }
}

/// Caller-owned cache keyed by capture identity and quality.
#[derive(Debug, Default, Clone)]
pub struct JpegDerivativeCache {
    entries: Vec<CachedJpeg>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedJpeg {
    identity: CaptureIdentity,
    quality: u8,
    derivative: JpegDerivative,
}

impl JpegDerivativeCache {
    /// Empty cache. Nothing is encoded until a frame is requested.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Number of stored derivatives.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache has no stored derivatives.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the stored derivative or encodes this one frame.
    pub fn get_or_create(
        &mut self,
        frame: &CapturedColorFrame,
        quality: u8,
        limits: JpegDerivativeLimits,
    ) -> Result<&JpegDerivative, A2aLabError> {
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.identity == *frame.identity() && entry.quality == quality)
        {
            return Ok(&self.entries[index].derivative);
        }
        let derivative = jpeg_derivative(frame, quality, limits)?;
        self.entries.push(CachedJpeg {
            identity: frame.identity().clone(),
            quality,
            derivative,
        });
        Ok(&self.entries.last().expect("just inserted").derivative)
    }
}

/// Builds one JPEG derivative. Does not look at any other frame.
pub fn jpeg_derivative(
    frame: &CapturedColorFrame,
    quality: u8,
    limits: JpegDerivativeLimits,
) -> Result<JpegDerivative, A2aLabError> {
    if !(MIN_JPEG_QUALITY..=MAX_JPEG_QUALITY).contains(&quality) {
        return Err(A2aLabError::invalid(
            "quality",
            "JPEG quality must be from 1 through 100",
        ));
    }
    if let Some(original) = frame.original_jpeg() {
        return bounded(frame, original.to_vec(), true, limits);
    }
    if frame.format() != CaptureFormat::Yuy2 {
        return Err(A2aLabError::invalid(
            "frame",
            "JPEG derivative needs YUY2 or an original JPEG",
        ));
    }
    let (range, matrix) = known_color(frame.color())?;
    let rgb_stride = frame
        .width()
        .checked_mul(3)
        .ok_or_else(|| A2aLabError::invalid("width", "RGB row size overflow"))?;
    let rgb_len = usize::try_from(rgb_stride)
        .ok()
        .and_then(|row| {
            usize::try_from(frame.height())
                .ok()
                .and_then(|height| row.checked_mul(height))
        })
        .ok_or_else(|| A2aLabError::invalid("width", "RGB buffer size overflow"))?;
    let mut rgb = vec![0_u8; rgb_len];
    let stride = u32::try_from(frame.stride().unwrap_or(0))
        .map_err(|_| A2aLabError::invalid("stride", "YUY2 stride exceeds u32"))?;
    let image = yuv::YuvPackedImage {
        yuy: frame.bytes(),
        yuy_stride: stride,
        width: frame.width(),
        height: frame.height(),
    };
    yuv::yuyv422_to_rgb(&image, &mut rgb, rgb_stride, range, matrix)
        .map_err(|error| A2aLabError::invalid("color", error.to_string()))?;
    let mut cursor = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut cursor, quality)
        .encode(&rgb, frame.width(), frame.height(), ExtendedColorType::Rgb8)
        .map_err(|error| A2aLabError::invalid("jpeg", error.to_string()))?;
    bounded(frame, cursor.into_inner(), false, limits)
}

fn bounded(
    frame: &CapturedColorFrame,
    jpeg: Vec<u8>,
    original: bool,
    limits: JpegDerivativeLimits,
) -> Result<JpegDerivative, A2aLabError> {
    if jpeg.len() > limits.max_bytes {
        return Err(A2aLabError::invalid(
            "jpeg",
            "JPEG derivative exceeds the byte limit",
        ));
    }
    Ok(JpegDerivative {
        identity: frame.identity().clone(),
        width: frame.width(),
        height: frame.height(),
        jpeg,
        original,
    })
}
