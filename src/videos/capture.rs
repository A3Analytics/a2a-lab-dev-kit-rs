//! Owned color frames captured from a webcam or vendor adapter.
//!
//! Packed YUY2 and YUYV are the same `Y0 U0 Y1 V0` layout. These raw formats
//! are not delivery [`crate::videos::VideoCodec`] values and are not part of
//! the A2A, MCP, or SiLA contracts. This module does not convert pixels. A
//! later converter copies [`CapturedColorFrame::timing`] from the source
//! snapshot instead of replacing the original UTC capture time.

use std::collections::VecDeque;

use crate::error::A2aLabError;
use crate::id::VideoSourceId;
use crate::time::UtcTimestamp;

/// Raw color layout supplied by a capture device.
///
/// This is not a [`crate::videos::VideoCodec`]. `Yuy2` and [`Self::YUYV`] are one
/// packed order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptureFormat {
    /// Packed 4:2:2 bytes `Y0 U0 Y1 V0`.
    Yuy2,
    /// One encoded JPEG access unit from an MJPEG source.
    Mjpeg,
}

impl CaptureFormat {
    /// YUYV alias of [`Self::Yuy2`]. Both names mean `Y0 U0 Y1 V0`.
    pub const YUYV: Self = Self::Yuy2;
}

/// Color matrix recorded with a frame.
///
/// [`Self::Unknown`] means the backend did not say. This type does not substitute
/// BT.601 or any other matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorMatrix {
    /// ITU-R BT.601.
    Bt601,
    /// ITU-R BT.709.
    Bt709,
    /// ITU-R BT.2020.
    Bt2020,
    /// The capture backend did not provide a matrix.
    Unknown,
}

/// Sample range recorded with a frame.
///
/// [`Self::Unknown`] means the backend did not say. This type does not substitute
/// limited or full range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorRange {
    /// Studio / limited range.
    Limited,
    /// Full range.
    Full,
    /// The capture backend did not provide a range.
    Unknown,
}

/// Explicit matrix and range. Callers choose [`ColorMatrix::Unknown`] when absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorProperties {
    matrix: ColorMatrix,
    range: ColorRange,
}

impl ColorProperties {
    /// Records the backend's matrix and range, including unknown.
    #[must_use]
    pub const fn new(matrix: ColorMatrix, range: ColorRange) -> Self {
        Self { matrix, range }
    }

    /// Color matrix supplied by the backend.
    #[must_use]
    pub const fn matrix(self) -> ColorMatrix {
        self.matrix
    }

    /// Sample range supplied by the backend.
    #[must_use]
    pub const fn range(self) -> ColorRange {
        self.range
    }
}

/// Source and sequence identity that stays on the owned frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureIdentity {
    source_id: VideoSourceId,
    sequence: u64,
}

impl CaptureIdentity {
    /// Pairs a video source with the backend's frame sequence number.
    #[must_use]
    pub const fn new(source_id: VideoSourceId, sequence: u64) -> Self {
        Self {
            source_id,
            sequence,
        }
    }

    /// Source that produced the frame.
    #[must_use]
    pub const fn source_id(&self) -> &VideoSourceId {
        &self.source_id
    }

    /// Backend sequence number for this source.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
}

/// Monotonic sample, a named clock, or both.
///
/// At least one of the two is required. Neither field is invented when absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureClock {
    identity: Option<String>,
    monotonic_ns: Option<u64>,
}

impl CaptureClock {
    /// A sample from an unnamed monotonic clock.
    #[must_use]
    pub const fn monotonic(monotonic_ns: u64) -> Self {
        Self {
            identity: None,
            monotonic_ns: Some(monotonic_ns),
        }
    }

    /// A named capture clock, with a monotonic sample when the backend has one.
    pub fn identified(identity: &str, monotonic_ns: Option<u64>) -> Result<Self, A2aLabError> {
        if identity.is_empty() {
            return Err(A2aLabError::invalid("clock", "identity must be non-empty"));
        }
        Ok(Self {
            identity: Some(identity.to_owned()),
            monotonic_ns,
        })
    }

    /// Clock name, when the backend provided one.
    #[must_use]
    pub fn identity(&self) -> Option<&str> {
        self.identity.as_deref()
    }

    /// Monotonic nanoseconds on [`Self::identity`], when the backend provided them.
    #[must_use]
    pub const fn monotonic_ns(&self) -> Option<u64> {
        self.monotonic_ns
    }
}

/// Original UTC capture time plus the backend clock sample.
///
/// Pixel conversion is not performed here. Copy this value onto derived pictures
/// instead of stamping decode or encode time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureTiming {
    captured_at: UtcTimestamp,
    clock: CaptureClock,
}

impl CaptureTiming {
    /// Stores the source UTC time and clock sample together.
    #[must_use]
    pub const fn new(captured_at: UtcTimestamp, clock: CaptureClock) -> Self {
        Self { captured_at, clock }
    }

    /// UTC time the device captured the frame.
    #[must_use]
    pub const fn captured_at(&self) -> UtcTimestamp {
        self.captured_at
    }

    /// Monotonic sample or named clock from the same capture.
    #[must_use]
    pub const fn clock(&self) -> &CaptureClock {
        &self.clock
    }
}

/// One advertised raw mode. Dimensions are pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CaptureMode {
    format: CaptureFormat,
    width: u32,
    height: u32,
}

impl CaptureMode {
    /// Accepts a positive mode. Packed YUY2 width must be even.
    pub fn new(format: CaptureFormat, width: u32, height: u32) -> Result<Self, A2aLabError> {
        validate_extent(width, height)?;
        if format == CaptureFormat::Yuy2 {
            yuy2_row_bytes(width)?;
        }
        Ok(Self {
            format,
            width,
            height,
        })
    }

    /// Raw capture format.
    #[must_use]
    pub const fn format(self) -> CaptureFormat {
        self.format
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    fn matches(self, frame: &CapturedColorFrame) -> bool {
        self.format == frame.format()
            && self.width == frame.width()
            && self.height == frame.height()
    }
}

/// Formats a hardware-free source can produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureCapabilities {
    modes: Vec<CaptureMode>,
}

impl CaptureCapabilities {
    /// Accepts a non-empty mode list with no duplicates.
    ///
    /// [`CaptureFormat::Yuy2`] and [`CaptureFormat::YUYV`] are the same mode.
    pub fn new(modes: Vec<CaptureMode>) -> Result<Self, A2aLabError> {
        if modes.is_empty() {
            return Err(A2aLabError::invalid(
                "capabilities",
                "must list at least one capture mode",
            ));
        }
        for (index, mode) in modes.iter().enumerate() {
            if modes[..index].contains(mode) {
                return Err(A2aLabError::invalid(
                    "capabilities",
                    "must not repeat a capture mode",
                ));
            }
        }
        Ok(Self { modes })
    }

    /// Advertised capture modes.
    #[must_use]
    pub fn modes(&self) -> &[CaptureMode] {
        &self.modes
    }

    fn allows(&self, frame: &CapturedColorFrame) -> bool {
        self.modes.iter().any(|mode| mode.matches(frame))
    }
}

/// Hardware-free capture seam.
///
/// Implementations own any device. This crate does not open a camera.
pub trait ColorCaptureSource {
    /// Modes this source can produce.
    fn capabilities(&self) -> &CaptureCapabilities;

    /// Next owned color frame.
    fn pull_frame(&mut self) -> Result<CapturedColorFrame, A2aLabError>;
}

/// Queues frames that were already captured. It does not open a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryColorCapture {
    capabilities: CaptureCapabilities,
    pending: VecDeque<CapturedColorFrame>,
}

impl MemoryColorCapture {
    /// Queues `frames` when each one matches an advertised mode.
    pub fn new(
        capabilities: CaptureCapabilities,
        frames: Vec<CapturedColorFrame>,
    ) -> Result<Self, A2aLabError> {
        if let Some(frame) = frames.iter().find(|frame| !capabilities.allows(frame)) {
            return Err(A2aLabError::invalid(
                "frame",
                format!(
                    "{} {}x{} is not advertised",
                    format_name(frame.format()),
                    frame.width(),
                    frame.height()
                ),
            ));
        }
        Ok(Self {
            capabilities,
            pending: VecDeque::from(frames),
        })
    }
}

impl ColorCaptureSource for MemoryColorCapture {
    fn capabilities(&self) -> &CaptureCapabilities {
        &self.capabilities
    }

    fn pull_frame(&mut self) -> Result<CapturedColorFrame, A2aLabError> {
        self.pending
            .pop_front()
            .ok_or_else(|| A2aLabError::unavailable("color capture has no frame"))
    }
}

/// Owned MJPEG or packed YUY2 snapshot.
///
/// Bytes are copied at construction. Reusing the producer buffer does not change
/// the frame. [`Self::timing`] remains the source capture time; this type has no
/// pixel conversion and no method that replaces that timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedColorFrame {
    identity: CaptureIdentity,
    timing: CaptureTiming,
    width: u32,
    height: u32,
    color: ColorProperties,
    payload: ColorPayload,
    original_jpeg: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ColorPayload {
    Yuy2 { stride: usize, bytes: Vec<u8> },
    Mjpeg { jpeg: Vec<u8> },
}

impl CapturedColorFrame {
    /// Copies a packed YUY2 frame, including supported row padding.
    ///
    /// `stride` is the byte distance from one row to the next and must hold an
    /// even-width `Y0 U0 Y1 V0` row. `original_jpeg` is retained when the camera
    /// also delivered the JPEG that this raw frame came from.
    pub fn yuy2(
        identity: CaptureIdentity,
        timing: CaptureTiming,
        width: u32,
        height: u32,
        stride: usize,
        packed: &[u8],
        color: ColorProperties,
        original_jpeg: Option<&[u8]>,
    ) -> Result<Self, A2aLabError> {
        validate_extent(width, height)?;
        let row_bytes = yuy2_row_bytes(width)?;
        if stride < row_bytes {
            return Err(A2aLabError::invalid(
                "stride",
                "shorter than the packed YUY2 row",
            ));
        }
        let needed = packed_byte_len(stride, height)?;
        if packed.len() < needed {
            return Err(A2aLabError::invalid("bytes", "truncated raw rows"));
        }
        Ok(Self {
            identity,
            timing,
            width,
            height,
            color,
            payload: ColorPayload::Yuy2 {
                stride,
                bytes: packed[..needed].to_vec(),
            },
            original_jpeg: optional_jpeg(original_jpeg)?,
        })
    }

    /// Copies one encoded JPEG access unit. The JPEG is the original encoded image.
    pub fn mjpeg(
        identity: CaptureIdentity,
        timing: CaptureTiming,
        width: u32,
        height: u32,
        jpeg: &[u8],
        color: ColorProperties,
    ) -> Result<Self, A2aLabError> {
        validate_extent(width, height)?;
        let jpeg = owned_jpeg(jpeg)?;
        Ok(Self {
            identity,
            timing,
            width,
            height,
            color,
            payload: ColorPayload::Mjpeg { jpeg },
            original_jpeg: None,
        })
    }

    /// Raw capture format. Never a delivery [`crate::videos::VideoCodec`].
    #[must_use]
    pub const fn format(&self) -> CaptureFormat {
        match self.payload {
            ColorPayload::Yuy2 { .. } => CaptureFormat::Yuy2,
            ColorPayload::Mjpeg { .. } => CaptureFormat::Mjpeg,
        }
    }

    /// Source and sequence identity.
    #[must_use]
    pub const fn identity(&self) -> &CaptureIdentity {
        &self.identity
    }

    /// Source UTC time and capture clock.
    #[must_use]
    pub const fn timing(&self) -> &CaptureTiming {
        &self.timing
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Packed row stride, in bytes. MJPEG has no stride.
    #[must_use]
    pub const fn stride(&self) -> Option<usize> {
        match self.payload {
            ColorPayload::Yuy2 { stride, .. } => Some(stride),
            ColorPayload::Mjpeg { .. } => None,
        }
    }

    /// Matrix and range exactly as supplied. Unknown stays unknown.
    #[must_use]
    pub const fn color(&self) -> ColorProperties {
        self.color
    }

    /// Owned packed YUY2 bytes, including row padding, or the encoded JPEG.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match &self.payload {
            ColorPayload::Yuy2 { bytes, .. } => bytes,
            ColorPayload::Mjpeg { jpeg } => jpeg,
        }
    }

    /// Original encoded JPEG when the capture included one.
    ///
    /// For MJPEG this is [`Self::bytes`]. For YUY2 it is the camera JPEG retained
    /// beside the packed frame, if any.
    #[must_use]
    pub fn original_jpeg(&self) -> Option<&[u8]> {
        match &self.payload {
            ColorPayload::Mjpeg { jpeg } => Some(jpeg),
            ColorPayload::Yuy2 { .. } => self.original_jpeg.as_deref(),
        }
    }
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

fn yuy2_row_bytes(width: u32) -> Result<usize, A2aLabError> {
    if !width.is_multiple_of(2) {
        return Err(A2aLabError::invalid(
            "width",
            "packed YUY2 width must be even",
        ));
    }
    let width = usize::try_from(width)
        .map_err(|_| A2aLabError::invalid("width", "packed YUY2 row length overflows"))?;
    width
        .checked_mul(2)
        .ok_or_else(|| A2aLabError::invalid("width", "packed YUY2 row length overflows"))
}

fn packed_byte_len(stride: usize, height: u32) -> Result<usize, A2aLabError> {
    let height = usize::try_from(height)
        .map_err(|_| A2aLabError::invalid("stride", "stride times height overflows"))?;
    stride
        .checked_mul(height)
        .ok_or_else(|| A2aLabError::invalid("stride", "stride times height overflows"))
}

fn owned_jpeg(jpeg: &[u8]) -> Result<Vec<u8>, A2aLabError> {
    if jpeg.is_empty() {
        return Err(A2aLabError::invalid("jpeg", "must be non-empty"));
    }
    Ok(jpeg.to_vec())
}

fn optional_jpeg(jpeg: Option<&[u8]>) -> Result<Option<Vec<u8>>, A2aLabError> {
    jpeg.map(owned_jpeg).transpose()
}

fn format_name(format: CaptureFormat) -> &'static str {
    match format {
        CaptureFormat::Yuy2 => "YUY2",
        CaptureFormat::Mjpeg => "MJPEG",
    }
}
