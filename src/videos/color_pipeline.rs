//! Selected MJPEG and YUY2 frames become owned all-IDR H.264 samples.
//!
//! The fresh-frame gate runs first. Conversion and encoding see only the
//! frames that gate selects. Applications choose the source and store the
//! sample; this module does not open a camera or write a recording.

use std::collections::VecDeque;

use crate::error::A2aLabError;
use crate::videos::mjpeg_decode::DecodedPicture;
use crate::videos::{
    AllIdrEncoder, CaptureFormat, CapturedColorFrame, ColorProperties, FreshFrameGate, I420Picture,
    JpegDerivativeLimits, MjpegDecodeLimits, MjpegDecodeTarget, OwnedI420, PackedYuy2,
    decode_mjpeg, frame_h264_picture, jpeg_derivative, yuy2_to_i420,
};

/// Counts for one capture-to-sample pipeline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColorEncodeCounters {
    converted: u64,
    encoded: u64,
    dropped: u64,
    malformed: u64,
    source_lost: u64,
    encoder_failures: u64,
    reconnects: u64,
    cancelled: u64,
}

impl ColorEncodeCounters {
    /// Frames that passed the gate and were converted.
    #[must_use]
    pub const fn converted(self) -> u64 {
        self.converted
    }

    /// Frames that produced an H.264 sample.
    #[must_use]
    pub const fn encoded(self) -> u64 {
        self.encoded
    }

    /// Frames rejected by the gate or dropped because the output queue was full.
    #[must_use]
    pub const fn dropped(self) -> u64 {
        self.dropped
    }

    /// Frames that failed to decode or convert.
    #[must_use]
    pub const fn malformed(self) -> u64 {
        self.malformed
    }

    /// Frames offered while the source was lost.
    #[must_use]
    pub const fn source_lost(self) -> u64 {
        self.source_lost
    }

    /// Selected frames the encoder refused.
    #[must_use]
    pub const fn encoder_failures(self) -> u64 {
        self.encoder_failures
    }

    /// Times a new encoder was opened after a size or color change, or after reconnect.
    #[must_use]
    pub const fn reconnects(self) -> u64 {
        self.reconnects
    }

    /// Frames refused after cancellation.
    #[must_use]
    pub const fn cancelled(self) -> u64 {
        self.cancelled
    }
}

/// One selected frame, its H.264 sample, and an optional JPEG of that same frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorEncodedFrame {
    original: CapturedColorFrame,
    sample: Vec<u8>,
    initialization: Vec<u8>,
    keyframe: bool,
    jpeg: Option<Vec<u8>>,
}

impl ColorEncodedFrame {
    /// Captured frame the sample was encoded from.
    #[must_use]
    pub const fn original(&self) -> &CapturedColorFrame {
        &self.original
    }

    /// Length-prefixed H.264 picture. Parameter sets are not in these bytes.
    #[must_use]
    pub fn sample(&self) -> &[u8] {
        &self.sample
    }

    /// Annex B SPS/PPS for this sample.
    #[must_use]
    pub fn initialization(&self) -> &[u8] {
        &self.initialization
    }

    /// True when the sample contains an IDR.
    #[must_use]
    pub const fn keyframe(&self) -> bool {
        self.keyframe
    }

    /// JPEG of this selected frame, present only when the pipeline was asked for one.
    #[must_use]
    pub fn jpeg(&self) -> Option<&[u8]> {
        self.jpeg.as_deref()
    }
}

struct OpenEncoder {
    encoder: AllIdrEncoder,
    width: u32,
    height: u32,
    properties: ColorProperties,
}

/// Bounded MJPEG/YUY2 path to all-IDR H.264 at a fixed output rate.
pub struct ColorEncodePipeline {
    gate: FreshFrameGate,
    output: VecDeque<ColorEncodedFrame>,
    capacity: usize,
    bitrate_bps: u32,
    fps: f32,
    jpeg_quality: Option<u8>,
    encoder: Option<OpenEncoder>,
    counters: ColorEncodeCounters,
    source_lost: bool,
    faulted: bool,
    cancelled: bool,
}

impl ColorEncodePipeline {
    /// Five output frames per second. `capacity` bounds selected samples waiting for the consumer.
    pub fn at_5fps(
        bitrate_bps: u32,
        capacity: usize,
        jpeg_quality: Option<u8>,
    ) -> Result<Self, A2aLabError> {
        if jpeg_quality.is_some_and(|quality| !(1..=100).contains(&quality)) {
            return Err(A2aLabError::invalid(
                "quality",
                "JPEG quality must be from 1 through 100",
            ));
        }
        Ok(Self {
            gate: FreshFrameGate::at_5fps(capacity)?,
            output: VecDeque::new(),
            capacity,
            bitrate_bps,
            fps: 5.0,
            jpeg_quality,
            encoder: None,
            counters: ColorEncodeCounters::default(),
            source_lost: false,
            faulted: false,
            cancelled: false,
        })
    }

    /// Drop and error counts.
    #[must_use]
    pub const fn counters(&self) -> ColorEncodeCounters {
        self.counters
    }

    /// Samples waiting for a slow consumer. Never more than the configured capacity.
    #[must_use]
    pub fn queued(&self) -> usize {
        self.output.len()
    }

    /// The source stopped. Later frames are counted and are not converted.
    pub fn source_lost(&mut self) {
        self.source_lost = true;
    }

    /// The encoder cannot accept another picture until [`Self::reconnect`].
    pub fn fault_encoder(&mut self) {
        if let Some(open) = &mut self.encoder {
            open.encoder.shutdown();
        }
        self.faulted = true;
    }

    /// Opens a new encoder on the next selected frame after source loss or an encoder fault.
    pub fn reconnect(&mut self) {
        self.source_lost = false;
        self.faulted = false;
        self.encoder = None;
        self.counters.reconnects = self.counters.reconnects.saturating_add(1);
    }

    /// Rejects later frames. Already queued samples stay pullable.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.counters.cancelled = self
            .counters
            .cancelled
            .saturating_add(u64::try_from(self.gate.shutdown().len()).unwrap_or(0));
    }

    /// Selects, converts, and encodes. Unselected frames are not converted.
    pub fn push(&mut self, frame: CapturedColorFrame) -> Result<(), A2aLabError> {
        if self.cancelled {
            self.counters.cancelled = self.counters.cancelled.saturating_add(1);
            return Ok(());
        }
        if self.source_lost {
            self.counters.source_lost = self.counters.source_lost.saturating_add(1);
            return Ok(());
        }
        let rejected = match self.gate.push(frame) {
            Ok(rejected) => rejected,
            Err(error) => {
                self.counters.malformed = self.counters.malformed.saturating_add(1);
                return Err(error);
            }
        };
        self.counters.dropped = self
            .counters
            .dropped
            .saturating_add(u64::try_from(rejected.len()).unwrap_or(0));
        while let Some(selected) = self.gate.pull() {
            if let Some(encoded) = self.encode_selected(selected)? {
                if self.output.len() == self.capacity {
                    self.output.pop_front();
                    self.counters.dropped = self.counters.dropped.saturating_add(1);
                }
                self.output.push_back(encoded);
            }
        }
        Ok(())
    }

    /// Oldest encoded sample still queued.
    pub fn pull(&mut self) -> Option<ColorEncodedFrame> {
        self.output.pop_front()
    }

    /// Converts and encodes one caller-selected frame without the internal cadence gate.
    pub fn encode_frame(
        &mut self,
        frame: CapturedColorFrame,
    ) -> Result<Option<ColorEncodedFrame>, A2aLabError> {
        self.encode_selected(frame)
    }

    fn encode_selected(
        &mut self,
        frame: CapturedColorFrame,
    ) -> Result<Option<ColorEncodedFrame>, A2aLabError> {
        let Some(picture) = self.convert(&frame)? else {
            return Ok(None);
        };
        self.counters.converted = self.counters.converted.saturating_add(1);
        if self.faulted {
            self.counters.encoder_failures = self.counters.encoder_failures.saturating_add(1);
            return Ok(None);
        }
        self.ensure_encoder(picture.width(), picture.height(), picture.properties())?;
        let timestamp_ns =
            frame.timing().clock().monotonic_ns().ok_or_else(|| {
                A2aLabError::invalid("clock", "selected frame has no monotonic time")
            })?;
        let open = self
            .encoder
            .as_mut()
            .ok_or_else(|| A2aLabError::invalid("encoder", "encoder is not open"))?;
        let Ok(access_unit) = open.encoder.encode(
            I420Picture::new(
                picture.width(),
                picture.height(),
                picture.y(),
                picture.u(),
                picture.v(),
                picture.properties(),
            ),
            frame.identity(),
            timestamp_ns,
        ) else {
            self.counters.encoder_failures = self.counters.encoder_failures.saturating_add(1);
            return Ok(None);
        };
        let framed = frame_h264_picture(access_unit.bytes())?;
        let jpeg = self.jpeg_of(&frame)?;
        self.counters.encoded = self.counters.encoded.saturating_add(1);
        Ok(Some(ColorEncodedFrame {
            original: frame,
            sample: framed.sample().to_vec(),
            initialization: framed.initialization().to_vec(),
            keyframe: framed.keyframe(),
            jpeg,
        }))
    }

    fn convert(&mut self, frame: &CapturedColorFrame) -> Result<Option<OwnedI420>, A2aLabError> {
        match frame.format() {
            CaptureFormat::Yuy2 => {
                let stride = frame.stride().ok_or_else(|| {
                    A2aLabError::invalid("stride", "YUY2 frame is missing its stride")
                })?;
                let stride = u32::try_from(stride).map_err(|_| {
                    A2aLabError::invalid("stride", "YUY2 stride does not fit the converter")
                })?;
                let packed = PackedYuy2::new(
                    frame.bytes(),
                    frame.width(),
                    frame.height(),
                    stride,
                    frame.color(),
                );
                if let Ok(picture) = yuy2_to_i420(packed) {
                    Ok(Some(picture))
                } else {
                    self.note_malformed();
                    Ok(None)
                }
            }
            CaptureFormat::Mjpeg => {
                let decoded = decode_mjpeg(
                    frame,
                    MjpegDecodeTarget::I420,
                    MjpegDecodeLimits::standard(),
                );
                let Ok(decoded) = decoded else {
                    self.note_malformed();
                    return Ok(None);
                };
                match decoded.picture() {
                    DecodedPicture::I420(picture) => Ok(Some(picture.clone())),
                    DecodedPicture::Yuy2(_) => Err(A2aLabError::invalid(
                        "frame",
                        "MJPEG pipeline requested I420",
                    )),
                }
            }
        }
    }

    fn note_malformed(&mut self) {
        self.counters.malformed = self.counters.malformed.saturating_add(1);
    }

    fn ensure_encoder(
        &mut self,
        width: u32,
        height: u32,
        properties: ColorProperties,
    ) -> Result<(), A2aLabError> {
        let changed = self.encoder.as_ref().is_some_and(|open| {
            open.width != width || open.height != height || open.properties != properties
        });
        if self.encoder.is_none() || changed {
            if changed {
                self.counters.reconnects = self.counters.reconnects.saturating_add(1);
            }
            self.encoder = Some(OpenEncoder {
                encoder: AllIdrEncoder::new(width, height, self.fps, self.bitrate_bps, properties)?,
                width,
                height,
                properties,
            });
        }
        Ok(())
    }

    fn jpeg_of(&self, frame: &CapturedColorFrame) -> Result<Option<Vec<u8>>, A2aLabError> {
        let Some(quality) = self.jpeg_quality else {
            return Ok(None);
        };
        let derivative =
            jpeg_derivative(frame, quality, JpegDerivativeLimits::new(8 * 1024 * 1024)?)?;
        Ok(Some(derivative.jpeg().to_vec()))
    }
}
