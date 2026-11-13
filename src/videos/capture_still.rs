//! Bounded JPEG bytes for warm still retention from negotiated raw capture.

use crate::error::A2aLabError;
use crate::videos::mjpeg_transport::trim_mjpeg_transport_padding;
use crate::videos::{CaptureFormat, CapturedColorFrame};

/// Returns the bounded MJPEG access unit, or encodes packed YUY2 when the `yuv` feature is on.
pub fn captured_frame_jpeg(
    frame: &CapturedColorFrame,
    max_bytes: u64,
) -> Result<Vec<u8>, A2aLabError> {
    match frame.format() {
        CaptureFormat::Mjpeg => bounded_mjpeg(frame.bytes(), max_bytes),
        CaptureFormat::Yuy2 => yuy2_jpeg(frame, max_bytes),
    }
}

fn bounded_mjpeg(bytes: &[u8], max_bytes: u64) -> Result<Vec<u8>, A2aLabError> {
    let jpeg = trim_mjpeg_transport_padding(bytes);
    let cap = usize::try_from(max_bytes).unwrap_or(usize::MAX);
    if jpeg.len() > cap {
        return Err(A2aLabError::invalid(
            "jpeg",
            "compressed JPEG exceeds the byte limit",
        ));
    }
    Ok(jpeg.to_vec())
}

#[cfg(feature = "yuv")]
fn yuy2_jpeg(frame: &CapturedColorFrame, max_bytes: u64) -> Result<Vec<u8>, A2aLabError> {
    use crate::videos::{JpegDerivativeLimits, jpeg_derivative};

    let limits = JpegDerivativeLimits::new(usize::try_from(max_bytes).unwrap_or(usize::MAX))?;
    Ok(jpeg_derivative(frame, 80, limits)?.jpeg().to_vec())
}

#[cfg(not(feature = "yuv"))]
fn yuy2_jpeg(_frame: &CapturedColorFrame, _max_bytes: u64) -> Result<Vec<u8>, A2aLabError> {
    Err(A2aLabError::unavailable(
        "yuv feature is required to encode YUY2 still JPEGs",
    ))
}

/// Minimal SOI/EOI check for pump-updated still bytes.
#[must_use]
pub fn is_bounded_jpeg(bytes: &[u8]) -> bool {
    matches!(bytes.first(), Some(0xFF))
        && bytes.get(1) == Some(&0xD8)
        && bytes.windows(2).any(|pair| pair == [0xFF, 0xD9])
}

/// Reads width and height from a JPEG start-of-frame marker.
pub fn jpeg_dimensions(data: &[u8]) -> Result<(u32, u32), A2aLabError> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err(malformed_jpeg());
    }
    let mut index = 2;
    while index < data.len() {
        let marker = read_jpeg_marker(data, &mut index)?;
        if marker == 0xD9 || marker == 0xDA {
            return Err(malformed_jpeg());
        }
        if jpeg_marker_standalone(marker) {
            continue;
        }
        let segment = read_jpeg_segment(data, &mut index)?;
        if jpeg_marker_sof(marker) {
            return jpeg_sof_size(segment);
        }
    }
    Err(malformed_jpeg())
}

fn read_jpeg_marker(data: &[u8], index: &mut usize) -> Result<u8, A2aLabError> {
    while *index < data.len() && data[*index] == 0xFF {
        *index += 1;
    }
    if *index >= data.len() {
        return Err(malformed_jpeg());
    }
    let marker = data[*index];
    *index += 1;
    Ok(marker)
}

fn read_jpeg_segment<'a>(data: &'a [u8], index: &mut usize) -> Result<&'a [u8], A2aLabError> {
    if *index + 1 >= data.len() {
        return Err(malformed_jpeg());
    }
    let length = usize::from(u16::from_be_bytes([data[*index], data[*index + 1]]));
    if length < 2 {
        return Err(malformed_jpeg());
    }
    let start = index.checked_add(2).ok_or_else(malformed_jpeg)?;
    let end = index.checked_add(length).ok_or_else(malformed_jpeg)?;
    if end > data.len() {
        return Err(malformed_jpeg());
    }
    *index = end;
    Ok(&data[start..end])
}

fn jpeg_sof_size(payload: &[u8]) -> Result<(u32, u32), A2aLabError> {
    if payload.len() < 5 {
        return Err(malformed_jpeg());
    }
    let height = u16::from_be_bytes([payload[1], payload[2]]);
    let width = u16::from_be_bytes([payload[3], payload[4]]);
    if width == 0 || height == 0 {
        return Err(A2aLabError::invalid(
            "camera",
            "JPEG dimensions must be positive",
        ));
    }
    Ok((u32::from(width), u32::from(height)))
}

fn jpeg_marker_standalone(marker: u8) -> bool {
    marker == 0x01 || (0xD0..=0xD7).contains(&marker)
}

fn jpeg_marker_sof(marker: u8) -> bool {
    matches!(
        marker,
        0xC0 | 0xC1 | 0xC2 | 0xC3 | 0xC5 | 0xC6 | 0xC7 | 0xC9 | 0xCA | 0xCB | 0xCD | 0xCE | 0xCF
    )
}

fn malformed_jpeg() -> A2aLabError {
    A2aLabError::invalid("camera", "malformed JPEG")
}
