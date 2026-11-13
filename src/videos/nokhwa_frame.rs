//! Normalizes undecoded Nokhwa buffers to the shared raw capture contract.

use crate::error::A2aLabError;
use crate::videos::{
    CaptureFormat, RawCameraSample, mjpeg_transport::trim_mjpeg_transport_padding,
};

/// Packs YUY2 rows to `width * 2` bytes per row and drops trailing row padding.
pub fn normalize_yuy2_buffer(
    width: u32,
    height: u32,
    mut bytes: Vec<u8>,
) -> Result<(u32, Vec<u8>), A2aLabError> {
    let row = yuy2_row_bytes(width)?;
    let rows = usize::try_from(height)
        .map_err(|_| A2aLabError::invalid("stride", "YUY2 height exceeds usize"))?;
    if rows == 0 {
        return Err(A2aLabError::invalid(
            "stride",
            "YUY2 height must be positive",
        ));
    }
    if bytes.len().is_multiple_of(rows) {
        let inferred = bytes.len() / rows;
        if inferred > row {
            let mut packed =
                Vec::with_capacity(row.checked_mul(rows).ok_or_else(|| {
                    A2aLabError::invalid("stride", "YUY2 packed buffer overflows")
                })?);
            for row_index in 0..rows {
                let start = row_index
                    .checked_mul(inferred)
                    .ok_or_else(|| A2aLabError::invalid("stride", "YUY2 row offset overflows"))?;
                packed.extend_from_slice(&bytes[start..start + row]);
            }
            let stride = u32::try_from(row)
                .map_err(|_| A2aLabError::invalid("stride", "YUY2 stride exceeds u32"))?;
            return Ok((stride, packed));
        }
        if inferred >= row {
            let needed = row
                .checked_mul(rows)
                .ok_or_else(|| A2aLabError::invalid("stride", "YUY2 buffer size overflow"))?;
            if bytes.len() > needed {
                bytes.truncate(needed);
            }
            let stride = u32::try_from(row)
                .map_err(|_| A2aLabError::invalid("stride", "YUY2 stride exceeds u32"))?;
            return Ok((stride, bytes));
        }
    }
    truncate_trailing_yuy2_rows(width, height, row, rows, bytes)
}

/// Applies format-specific normalization for a Nokhwa raw frame.
pub fn normalize_raw_camera_sample(
    format: CaptureFormat,
    width: u32,
    height: u32,
    bytes: Vec<u8>,
) -> Result<RawCameraSample, A2aLabError> {
    match format {
        CaptureFormat::Mjpeg => {
            let trimmed = trim_mjpeg_transport_padding(&bytes);
            Ok(RawCameraSample::new(
                CaptureFormat::Mjpeg,
                width,
                height,
                0,
                trimmed.to_vec(),
            ))
        }
        CaptureFormat::Yuy2 => {
            let (stride, packed) = normalize_yuy2_buffer(width, height, bytes)?;
            Ok(RawCameraSample::new(
                CaptureFormat::Yuy2,
                width,
                height,
                stride,
                packed,
            ))
        }
    }
}

fn truncate_trailing_yuy2_rows(
    width: u32,
    height: u32,
    row: usize,
    rows: usize,
    mut bytes: Vec<u8>,
) -> Result<(u32, Vec<u8>), A2aLabError> {
    let needed = row
        .checked_mul(rows)
        .ok_or_else(|| A2aLabError::invalid("stride", "YUY2 buffer size overflow"))?;
    if row == 0 || bytes.len() < needed {
        return Err(A2aLabError::invalid(
            "stride",
            format!("YUY2 buffer is {} bytes for {width}x{height}", bytes.len()),
        ));
    }
    bytes.truncate(needed);
    let stride = u32::try_from(row)
        .map_err(|_| A2aLabError::invalid("stride", "YUY2 stride exceeds u32"))?;
    Ok((stride, bytes))
}

fn yuy2_row_bytes(width: u32) -> Result<usize, A2aLabError> {
    usize::try_from(width)
        .ok()
        .and_then(|value| value.checked_mul(2))
        .ok_or_else(|| A2aLabError::invalid("stride", "YUY2 row size overflow"))
}

#[cfg(test)]
mod tests {
    use super::{normalize_raw_camera_sample, normalize_yuy2_buffer, trim_mjpeg_transport_padding};
    use crate::videos::CaptureFormat;

    #[test]
    fn trims_v4l2_padding_after_the_scan_end() {
        let mut jpeg = vec![
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x02, 0xFF, 0xDA, 0x00, 0x02, 0x12, 0xFF, 0x00, 0x34,
            0xFF, 0xD9,
        ];
        let end = jpeg.len();
        jpeg.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(trim_mjpeg_transport_padding(&jpeg), &jpeg[..end]);
    }

    #[test]
    fn keeps_bytes_that_are_not_a_jpeg() {
        let bytes = [1, 2, 3, 4];
        assert_eq!(trim_mjpeg_transport_padding(&bytes), &bytes);
    }

    #[test]
    fn yuy2_drops_a_trailing_padding_row() {
        let row = 1280 * 2;
        let bytes = vec![7_u8; row * 721];
        let (stride, trimmed) = normalize_yuy2_buffer(1280, 720, bytes).unwrap();
        assert_eq!(stride, u32::try_from(row).unwrap());
        assert_eq!(trimmed.len(), row * 720);
        assert!(trimmed.iter().all(|value| *value == 7));
    }

    #[test]
    fn yuy2_packs_per_row_stride_padding() {
        let width = 4;
        let height = 2;
        let row = width * 2;
        let mut bytes = Vec::new();
        for row_index in 0..height {
            let marker = u8::try_from(row_index + 1).unwrap();
            bytes.extend(std::iter::repeat_n(marker, row as usize));
            bytes.extend(std::iter::repeat_n(0, 4));
            let _ = row_index;
        }
        let sample = normalize_raw_camera_sample(CaptureFormat::Yuy2, width, height, bytes)
            .expect("normalize");
        assert_eq!(sample.stride(), row);
        assert_eq!(sample.bytes().len(), usize::try_from(row * height).unwrap());
        assert_eq!(sample.bytes()[0], 1);
        assert_eq!(sample.bytes()[row as usize], 2);
    }

    #[test]
    fn mjpeg_sample_trims_transport_padding() {
        let mut jpeg = vec![
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11,
            0x00, 0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0xFF, 0xD9,
        ];
        let exact = jpeg.clone();
        jpeg.extend_from_slice(&[9, 9, 9]);
        let sample =
            normalize_raw_camera_sample(CaptureFormat::Mjpeg, 1, 1, jpeg).expect("normalize");
        assert_eq!(sample.bytes(), exact.as_slice());
    }
}
