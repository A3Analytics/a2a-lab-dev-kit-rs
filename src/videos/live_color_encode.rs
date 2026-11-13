//! Live color conversion, shared cadence selection, and bounded H.264 publication.

use tokio::sync::watch;

use crate::config::VideoCaptureSettings;
use crate::error::A2aLabError;
use crate::id::VideoSourceId;
use crate::time::UtcTimestamp;
use crate::videos::h264_live_catalog::SourceLiveH264Store;
use crate::videos::h264_live_epoch::H264LiveEpochWriter;
use crate::videos::{
    CadenceDestinations, CadenceDispatch, CaptureFormat, CapturedColorFrame, ColorEncodePipeline,
    ColorEncodedFrame, H264LiveAppendStats, LiveDvrCadenceSelector,
};

const PIPELINE_QUEUE: usize = 8;
const JPEG_STILL_QUALITY: u8 = 80;

pub(crate) fn encode_bitrate_bps(fps: u32) -> u32 {
    2_000_000u32.saturating_mul(fps.max(1)) / 30
}

pub(crate) fn live_period_ns_from_fps(fps: u32) -> u64 {
    1_000_000_000u64 / u64::from(fps.max(1))
}

/// Stop and settings watches for one encode pump tick loop.
#[derive(Debug)]
pub struct LiveEncodeTick<'a> {
    stop: &'a mut watch::Receiver<bool>,
    settings: &'a mut watch::Receiver<VideoCaptureSettings>,
}

impl<'a> LiveEncodeTick<'a> {
    /// Binds one iteration to shutdown and settings receivers.
    #[must_use]
    pub fn new(
        stop: &'a mut watch::Receiver<bool>,
        settings: &'a mut watch::Receiver<VideoCaptureSettings>,
    ) -> Self {
        Self { stop, settings }
    }
    /// Whether the capture loop should exit.
    #[must_use]
    pub fn stopped(&self) -> bool {
        *self.stop.borrow()
    }

    /// Applies pending settings before the next frame is processed.
    pub fn apply_pending_settings(
        &mut self,
        pump: &mut LiveColorEncodePump,
    ) -> Result<(), A2aLabError> {
        if self.settings.has_changed().is_ok() {
            let snapshot = self.settings.borrow_and_update().clone();
            pump.apply_settings(&snapshot)?;
        }
        Ok(())
    }
}

/// One MJPEG or YUY2 source with shared live/DVR cadence and a single encode path.
pub struct LiveColorEncodePump {
    pipeline: ColorEncodePipeline,
    selector: LiveDvrCadenceSelector,
    epoch: H264LiveEpochWriter,
    live: SourceLiveH264Store,
    source_id: VideoSourceId,
}

impl LiveColorEncodePump {
    /// Opens one pump backed by a bounded live store.
    pub fn open(
        source_id: VideoSourceId,
        live: SourceLiveH264Store,
        fps: u32,
        dvr_save_interval_nanos: Option<u64>,
    ) -> Result<Self, A2aLabError> {
        let live_period_ns = live_period_ns_from_fps(fps);
        let dvr_period_ns = dvr_save_interval_nanos.unwrap_or(live_period_ns);
        Ok(Self {
            pipeline: ColorEncodePipeline::at_5fps(
                encode_bitrate_bps(fps),
                PIPELINE_QUEUE,
                Some(JPEG_STILL_QUALITY),
            )?,
            selector: LiveDvrCadenceSelector::new(live_period_ns, dvr_period_ns, PIPELINE_QUEUE)?,
            epoch: H264LiveEpochWriter::new(),
            live,
            source_id,
        })
    }

    /// Aligns live and DVR cadence without recreating the pipeline.
    pub fn apply_settings(&mut self, settings: &VideoCaptureSettings) -> Result<(), A2aLabError> {
        self.selector
            .set_live_period_ns(settings.live_period_ns())?;
        self.selector.set_dvr_period_ns(settings.dvr_period_ns())?;
        Ok(())
    }

    /// Structured slow-encode probes for tests only.
    #[doc(hidden)]
    pub fn inject_encode_elapsed_ns(&mut self, elapsed_ns: u64) {
        self.pipeline.inject_encode_elapsed_ns(elapsed_ns);
    }

    pub fn source_lost(&mut self) {
        self.pipeline.source_lost();
    }

    pub fn reconnect(&mut self) {
        self.pipeline.reconnect();
        let _ = self.live.reconnect();
    }

    pub fn cancel(&mut self) {
        self.pipeline.cancel();
        let _ = self.selector.shutdown();
    }

    /// One capture iteration honoring stop and live settings watches.
    pub fn tick(
        &mut self,
        frame: CapturedColorFrame,
        control: &mut LiveEncodeTick<'_>,
    ) -> Result<Vec<PublishedH264>, A2aLabError> {
        if control.stopped() {
            self.cancel();
            return Ok(Vec::new());
        }
        control.apply_pending_settings(self)?;
        self.push_frame(frame)
    }

    /// Selects cadence, encodes once, and publishes live output when selected.
    pub fn push_frame(
        &mut self,
        frame: CapturedColorFrame,
    ) -> Result<Vec<PublishedH264>, A2aLabError> {
        let dispatches = self.selector.push(frame)?;
        let mut out = Vec::with_capacity(dispatches.len());
        for dispatch in dispatches {
            if let Some(published) = self.publish_dispatch(dispatch)? {
                out.push(published);
            }
        }
        Ok(out)
    }

    fn publish_dispatch(
        &mut self,
        dispatch: CadenceDispatch,
    ) -> Result<Option<PublishedH264>, A2aLabError> {
        let destinations = dispatch.destinations();
        let Some(encoded) = self.pipeline.encode_frame(dispatch.into_frame())? else {
            return Ok(None);
        };
        self.publish_encoded(destinations, &encoded)
    }

    fn publish_encoded(
        &mut self,
        destinations: CadenceDestinations,
        encoded: &ColorEncodedFrame,
    ) -> Result<Option<PublishedH264>, A2aLabError> {
        if !destinations.live() && !destinations.dvr() {
            return Ok(None);
        }
        let captured_at = encoded.original().timing().captured_at();
        let (track, picture) = self.epoch.ensure(&self.source_id, encoded)?;
        if destinations.live() {
            self.live
                .open_epoch_if_absent(&track, encoded.initialization().to_vec(), picture)?;
            let _stats: H264LiveAppendStats = self.live.inner().append_sample(
                &track,
                captured_at,
                encoded.sample().to_vec(),
                encoded.keyframe(),
            )?;
        }
        let jpeg = still_jpeg_bytes(encoded);
        Ok(Some(PublishedH264 {
            captured_at,
            destinations,
            initialization: encoded.initialization().to_vec(),
            sample: encoded.sample().to_vec(),
            keyframe: encoded.keyframe(),
            still_jpeg: jpeg,
        }))
    }
}

fn still_jpeg_bytes(encoded: &ColorEncodedFrame) -> Vec<u8> {
    match encoded.original().format() {
        CaptureFormat::Mjpeg => encoded.original().bytes().to_vec(),
        CaptureFormat::Yuy2 => encoded.jpeg().map(std::vec::Vec::from).unwrap_or_default(),
    }
}

/// One encoded picture and the destinations that should receive it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedH264 {
    pub captured_at: UtcTimestamp,
    pub destinations: CadenceDestinations,
    pub initialization: Vec<u8>,
    pub sample: Vec<u8>,
    pub keyframe: bool,
    pub still_jpeg: Vec<u8>,
}

impl PublishedH264 {
    /// DVR persistence should enqueue this publication.
    #[must_use]
    pub const fn fans_to_disk(&self) -> bool {
        self.destinations.dvr()
    }
}
