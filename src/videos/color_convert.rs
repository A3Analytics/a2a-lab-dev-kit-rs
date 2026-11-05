//! Packed YUY2 and interleaved RGB conversion through the `yuv` crate.
//!
//! YUY2 byte order is `Y0 U0 Y1 V0`. RGB and BGR become that order by planar
//! 4:2:2 and then packing. YUY2 becomes tightly packed I420 with no RGB step.
//! [`ColorMatrix::Unknown`] and [`ColorRange::Unknown`] are copied onto I420
//! and are rejected for RGB, which needs an explicit matrix and range.
//! This module does not open a camera and does not encode H.264.

use crate::error::A2aLabError;
use crate::videos::{ColorMatrix, ColorProperties, ColorRange};

/// Largest width or height accepted for an I420 picture.
///
/// OpenH264's current wrapper bound is 3840 on an edge. Larger frames are
/// rejected here so conversion does not allocate an encoder-incompatible picture.
pub const MAX_ENCODER_EDGE: u32 = 3840;

/// Borrowed packed `Y0 U0 Y1 V0` bytes, including row padding.
#[derive(Debug, Clone, Copy)]
pub struct PackedYuy2<'a> {
    bytes: &'a [u8],
    width: u32,
    height: u32,
    stride: u32,
    properties: ColorProperties,
}

impl<'a> PackedYuy2<'a> {
    /// Borrows one packed frame. `stride` is bytes per row.
    #[must_use]
    pub const fn new(
        bytes: &'a [u8],
        width: u32,
        height: u32,
        stride: u32,
        properties: ColorProperties,
    ) -> Self {
        Self {
            bytes,
            width,
            height,
            stride,
            properties,
        }
    }
}

/// Channel order of an interleaved 8-bit source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterleavedColor {
    /// `R G B`.
    Rgb,
    /// `B G R`.
    Bgr,
}

/// Borrowed interleaved RGB or BGR, including row padding.
#[derive(Debug, Clone, Copy)]
pub struct InterleavedRgb<'a> {
    bytes: &'a [u8],
    width: u32,
    height: u32,
    stride: u32,
    order: InterleavedColor,
    properties: ColorProperties,
}

impl<'a> InterleavedRgb<'a> {
    /// Borrows one interleaved frame. `stride` is bytes per row.
    #[must_use]
    pub const fn new(
        bytes: &'a [u8],
        width: u32,
        height: u32,
        stride: u32,
        order: InterleavedColor,
        properties: ColorProperties,
    ) -> Self {
        Self {
            bytes,
            width,
            height,
            stride,
            order,
            properties,
        }
    }
}

/// Tightly packed 8-bit I420. Plane order is Y, then U, then V.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedI420 {
    width: u32,
    height: u32,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    properties: ColorProperties,
}

impl OwnedI420 {
    /// Picture width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Picture height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Luma plane, `width * height` bytes.
    #[must_use]
    pub fn y(&self) -> &[u8] {
        &self.y
    }

    /// Chroma U plane, `(width / 2) * (height / 2)` bytes.
    #[must_use]
    pub fn u(&self) -> &[u8] {
        &self.u
    }

    /// Chroma V plane, `(width / 2) * (height / 2)` bytes.
    #[must_use]
    pub fn v(&self) -> &[u8] {
        &self.v
    }

    /// Matrix and range copied from the source. Unknown stays unknown.
    #[must_use]
    pub const fn properties(&self) -> ColorProperties {
        self.properties
    }
}

/// Converts packed YUY2 directly to owned I420.
pub fn yuy2_to_i420(frame: PackedYuy2<'_>) -> Result<OwnedI420, A2aLabError> {
    validate_encoder_picture(frame.width, frame.height)?;
    let row = yuy2_row_bytes(frame.width)?;
    require_stride(frame.stride, row, "stride")?;
    require_buffer(frame.bytes, frame.stride, frame.height)?;

    let mut planar =
        yuv::YuvPlanarImageMut::alloc(frame.width, frame.height, yuv::YuvChromaSubsampling::Yuv420);
    let packed = yuv::YuvPackedImage {
        yuy: frame.bytes,
        yuy_stride: frame.stride,
        width: frame.width,
        height: frame.height,
    };
    yuv::yuyv422_to_yuv420(&mut planar, &packed).map_err(|error| yuv_error(&error))?;
    Ok(OwnedI420 {
        width: frame.width,
        height: frame.height,
        y: planar.y_plane.borrow().to_vec(),
        u: planar.u_plane.borrow().to_vec(),
        v: planar.v_plane.borrow().to_vec(),
        properties: frame.properties,
    })
}

/// Converts interleaved RGB or BGR to packed YUY2 at the same dimensions.
pub fn rgb_to_yuy2(frame: InterleavedRgb<'_>) -> Result<Vec<u8>, A2aLabError> {
    validate_extent(frame.width, frame.height)?;
    require_edge(frame.width, frame.height)?;
    let row = yuy2_row_bytes(frame.width)?;
    let rgb_row = rgb_row_bytes(frame.width)?;
    require_stride(frame.stride, rgb_row, "stride")?;
    require_buffer(frame.bytes, frame.stride, frame.height)?;
    let (range, matrix) = known_color(frame.properties)?;

    let mut planar =
        yuv::YuvPlanarImageMut::alloc(frame.width, frame.height, yuv::YuvChromaSubsampling::Yuv422);
    let convert = match frame.order {
        InterleavedColor::Rgb => yuv::rgb_to_yuv422,
        InterleavedColor::Bgr => yuv::bgr_to_yuv422,
    };
    convert(
        &mut planar,
        frame.bytes,
        frame.stride,
        range,
        matrix,
        yuv::YuvConversionMode::Balanced,
    )
    .map_err(|error| yuv_error(&error))?;

    let mut packed = yuv::YuvPackedImageMut {
        yuy: yuv::BufferStoreMut::Owned(vec![
            0;
            row * usize::try_from(frame.height).expect("height")
        ]),
        yuy_stride: u32::try_from(row).expect("row"),
        width: frame.width,
        height: frame.height,
    };
    yuv::yuv422_to_yuyv422(&mut packed, &planar.to_fixed()).map_err(|error| yuv_error(&error))?;
    match packed.yuy {
        yuv::BufferStoreMut::Owned(bytes) => Ok(bytes),
        yuv::BufferStoreMut::Borrowed(_) => Err(A2aLabError::invalid(
            "pixels",
            "packed YUY2 conversion did not own its output",
        )),
    }
}

fn known_color(
    properties: ColorProperties,
) -> Result<(yuv::YuvRange, yuv::YuvStandardMatrix), A2aLabError> {
    let range = match properties.range() {
        ColorRange::Limited => yuv::YuvRange::Limited,
        ColorRange::Full => yuv::YuvRange::Full,
        ColorRange::Unknown => {
            return Err(A2aLabError::invalid(
                "range",
                "RGB conversion requires an explicit limited or full range",
            ));
        }
    };
    let matrix = match properties.matrix() {
        ColorMatrix::Bt601 => yuv::YuvStandardMatrix::Bt601,
        ColorMatrix::Bt709 => yuv::YuvStandardMatrix::Bt709,
        ColorMatrix::Bt2020 => yuv::YuvStandardMatrix::Bt2020,
        ColorMatrix::Unknown => {
            return Err(A2aLabError::invalid(
                "matrix",
                "RGB conversion requires an explicit color matrix",
            ));
        }
    };
    Ok((range, matrix))
}

fn validate_encoder_picture(width: u32, height: u32) -> Result<(), A2aLabError> {
    validate_extent(width, height)?;
    require_edge(width, height)?;
    if !height.is_multiple_of(2) {
        return Err(A2aLabError::invalid(
            "height",
            "I420 encoder height must be even",
        ));
    }
    Ok(())
}

fn validate_extent(width: u32, height: u32) -> Result<(), A2aLabError> {
    if width == 0 {
        return Err(A2aLabError::invalid("width", "must be positive"));
    }
    if height == 0 {
        return Err(A2aLabError::invalid("height", "must be positive"));
    }
    Ok(())
}

fn require_edge(width: u32, height: u32) -> Result<(), A2aLabError> {
    if width > MAX_ENCODER_EDGE {
        return Err(A2aLabError::invalid(
            "width",
            format!("must be at most {MAX_ENCODER_EDGE}"),
        ));
    }
    if height > MAX_ENCODER_EDGE {
        return Err(A2aLabError::invalid(
            "height",
            format!("must be at most {MAX_ENCODER_EDGE}"),
        ));
    }
    Ok(())
}

fn yuy2_row_bytes(width: u32) -> Result<usize, A2aLabError> {
    if !width.is_multiple_of(2) {
        return Err(A2aLabError::invalid(
            "width",
            "packed YUY2 width must be even",
        ));
    }
    usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(2))
        .ok_or_else(|| A2aLabError::invalid("width", "packed YUY2 row length overflows"))
}

fn rgb_row_bytes(width: u32) -> Result<u32, A2aLabError> {
    width
        .checked_mul(3)
        .ok_or_else(|| A2aLabError::invalid("width", "RGB row length overflows"))
}

fn require_stride(
    stride: u32,
    minimum: impl TryInto<u32>,
    field: &'static str,
) -> Result<(), A2aLabError> {
    let minimum = minimum
        .try_into()
        .map_err(|_| A2aLabError::invalid(field, "row length overflows"))?;
    if stride < minimum {
        return Err(A2aLabError::invalid(
            field,
            "must cover the active pixels in the row",
        ));
    }
    Ok(())
}

fn require_buffer(bytes: &[u8], stride: u32, height: u32) -> Result<(), A2aLabError> {
    let needed = stride
        .checked_mul(height)
        .ok_or_else(|| A2aLabError::invalid("stride", "stride times height overflows"))?;
    if bytes.len() < usize::try_from(needed).expect("u32 fits usize") {
        return Err(A2aLabError::invalid(
            "bytes",
            "buffer is shorter than stride times height",
        ));
    }
    Ok(())
}

fn yuv_error(error: &yuv::YuvError) -> A2aLabError {
    A2aLabError::invalid("pixels", error.to_string())
}
