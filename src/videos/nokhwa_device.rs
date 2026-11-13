//! Optional Nokhwa native-backend adapter.
//!
//! Raw `frame_raw` bytes are copied into the shared capture contract. This
//! module does not call Nokhwa's decoded RGB path.

pub use crate::videos::nokhwa_frame::normalize_raw_camera_sample;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use nokhwa::utils::{
    CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
};
use nokhwa::{Camera, nokhwa_initialize};

use crate::error::A2aLabError;
use crate::videos::{
    AdvertisedCaptureProfile, CaptureFormat, CaptureMode, CaptureRate, ColorCamera,
    RawCameraSample, SelectedCaptureProfile,
};

static NOKHWA_OPEN: OnceLock<Mutex<()>> = OnceLock::new();

/// Fourcc order for automatic device open when format is omitted.
pub const AUTO_PROBE_FOURCCS: [FrameFormat; 2] = [FrameFormat::MJPEG, FrameFormat::YUYV];

/// Resolution and rate ladder used after each fourcc in [`AUTO_PROBE_FOURCCS`].
pub const AUTO_PROBE_LADDER: [(u32, u32, u32); 3] =
    [(1920, 1080, 30), (1280, 720, 30), (640, 480, 30)];

/// Serializes native camera configuration so concurrent opens cannot race backends.
pub fn with_nokhwa_open_lock<T>(operation: impl FnOnce() -> T) -> T {
    let _guard = NOKHWA_OPEN
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    operation()
}

/// Open Nokhwa camera using the platform's native input backend.
pub struct NokhwaCamera {
    index: u32,
    camera: Camera,
}

impl NokhwaCamera {
    /// Opens a camera index with the auto fourcc/resolution ladder.
    pub fn open_index(index: u32) -> Result<Self, A2aLabError> {
        with_nokhwa_open_lock(|| {
            nokhwa_initialize(|_granted| {});
            let camera = auto_probe_camera(index)?;
            Ok(Self { index, camera })
        })
    }
}

/// Lists MJPEG/YUYV profiles for one native index without opening a stream.
pub fn nokhwa_profiles_for_index(index: u32) -> Result<Vec<AdvertisedCaptureProfile>, A2aLabError> {
    with_nokhwa_open_lock(|| {
        nokhwa_initialize(|_granted| {});
        let requested =
            RequestedFormat::with_formats(RequestedFormatType::None, &AUTO_PROBE_FOURCCS);
        let mut camera = Camera::new(CameraIndex::Index(index), requested).map_err(camera_error)?;
        advertised_profiles(&mut camera)
    })
}

fn advertised_profiles(camera: &mut Camera) -> Result<Vec<AdvertisedCaptureProfile>, A2aLabError> {
    let formats = camera.compatible_camera_formats().map_err(camera_error)?;
    if !formats.is_empty() {
        return Ok(profiles_from_nokhwa(&formats));
    }
    let fourccs = camera.compatible_fourcc().map_err(camera_error)?;
    Ok(recover_inverted_fourcc_profiles(camera, &fourccs))
}

impl ColorCamera for NokhwaCamera {
    fn advertised(&mut self) -> Result<Vec<AdvertisedCaptureProfile>, A2aLabError> {
        advertised_profiles(&mut self.camera)
    }

    fn open(
        &mut self,
        profile: &SelectedCaptureProfile,
    ) -> Result<SelectedCaptureProfile, A2aLabError> {
        with_nokhwa_open_lock(|| {
            let format = camera_format(profile)?;
            self.camera = open_exact_camera(self.index, format)?;
            self.camera.open_stream().map_err(camera_error)?;
            let actual = self.camera.camera_format();
            negotiated_profile(profile, actual)
        })
    }

    fn raw_frame(&mut self) -> Result<RawCameraSample, A2aLabError> {
        let format = self.camera.camera_format();
        let bytes = self.camera.frame_raw().map_err(camera_error)?.into_owned();
        sample_from_nokhwa(format, bytes)
    }

    fn stop(&mut self) -> Result<(), A2aLabError> {
        self.camera.stop_stream().map_err(camera_error)
    }
}

/// Maps advertised Nokhwa formats. Layouts other than MJPEG and YUYV are omitted.
#[must_use]
pub fn profiles_from_nokhwa(formats: &[CameraFormat]) -> Vec<AdvertisedCaptureProfile> {
    formats
        .iter()
        .filter_map(|format| profile_from_nokhwa(*format))
        .collect()
}

fn recover_inverted_fourcc_profiles(
    camera: &mut Camera,
    fourccs: &[FrameFormat],
) -> Vec<AdvertisedCaptureProfile> {
    let mut profiles = Vec::new();
    for fourcc in AUTO_PROBE_FOURCCS {
        if !fourccs.contains(&fourcc) {
            continue;
        }
        let probe = inverted_probe_fourcc(fourcc);
        let Ok(listed) = camera.compatible_list_by_resolution(probe) else {
            continue;
        };
        profiles.extend(profiles_for_fourcc(fourcc, &listed));
    }
    profiles
}

fn inverted_probe_fourcc(fourcc: FrameFormat) -> FrameFormat {
    if fourcc == FrameFormat::MJPEG {
        FrameFormat::YUYV
    } else {
        FrameFormat::MJPEG
    }
}

fn profiles_for_fourcc(
    fourcc: FrameFormat,
    resolutions: &HashMap<Resolution, Vec<u32>>,
) -> Vec<AdvertisedCaptureProfile> {
    let mut formats = Vec::new();
    for (resolution, rates) in resolutions {
        let mut rates = rates.clone();
        rates.sort_unstable();
        rates.dedup();
        for fps in rates {
            formats.push(CameraFormat::new_from(
                resolution.width(),
                resolution.height(),
                fourcc,
                fps,
            ));
        }
    }
    profiles_from_nokhwa(&formats)
}

fn auto_probe_camera(index: u32) -> Result<Camera, A2aLabError> {
    let mut last = None;
    for fourcc in AUTO_PROBE_FOURCCS {
        for &(width, height, fps) in &AUTO_PROBE_LADDER {
            let format = CameraFormat::new_from(width, height, fourcc, fps);
            match open_exact_camera(index, format) {
                Ok(camera) => return Ok(camera),
                Err(error) => last = Some(error),
            }
        }
    }
    Err(last.unwrap_or_else(|| {
        camera_error("auto probe could not open a 1080p-or-smaller MJPEG/YUYV mode")
    }))
}

fn open_exact_camera(index: u32, format: CameraFormat) -> Result<Camera, A2aLabError> {
    nokhwa_initialize(|_granted| {});
    let formats = [format.format()];
    let requested = RequestedFormat::with_formats(RequestedFormatType::Exact(format), &formats);
    Camera::new(CameraIndex::Index(index), requested).map_err(camera_error)
}

fn profile_from_nokhwa(format: CameraFormat) -> Option<AdvertisedCaptureProfile> {
    let capture = match format.format() {
        FrameFormat::MJPEG => CaptureFormat::Mjpeg,
        FrameFormat::YUYV => CaptureFormat::Yuy2,
        _ => return None,
    };
    let mode = CaptureMode::new(capture, format.width(), format.height()).ok()?;
    let rate = CaptureRate::fps(format.frame_rate()).ok()?;
    Some(AdvertisedCaptureProfile::new(mode, rate))
}

fn sample_from_nokhwa(
    format: CameraFormat,
    bytes: Vec<u8>,
) -> Result<RawCameraSample, A2aLabError> {
    let capture = match format.format() {
        FrameFormat::MJPEG => CaptureFormat::Mjpeg,
        FrameFormat::YUYV => CaptureFormat::Yuy2,
        other => {
            return Err(A2aLabError::invalid(
                "format",
                format!("unsupported raw camera layout {other}"),
            ));
        }
    };
    normalize_raw_camera_sample(capture, format.width(), format.height(), bytes)
}

fn camera_format(profile: &SelectedCaptureProfile) -> Result<CameraFormat, A2aLabError> {
    let format = match profile.mode().format() {
        CaptureFormat::Mjpeg => FrameFormat::MJPEG,
        CaptureFormat::Yuy2 => FrameFormat::YUYV,
    };
    if profile.capture_rate().denominator() != 1 {
        return Err(A2aLabError::invalid(
            "rate",
            "Nokhwa advertises whole-number frame rates",
        ));
    }
    Ok(CameraFormat::new_from(
        profile.mode().width(),
        profile.mode().height(),
        format,
        profile.capture_rate().numerator(),
    ))
}

fn negotiated_profile(
    requested: &SelectedCaptureProfile,
    actual: CameraFormat,
) -> Result<SelectedCaptureProfile, A2aLabError> {
    let Some(profile) = profile_from_nokhwa(actual) else {
        return Err(A2aLabError::invalid(
            "format",
            "device negotiated an unsupported raw layout",
        ));
    };
    Ok(SelectedCaptureProfile::negotiated(
        profile.mode(),
        profile.capture_rate(),
        requested.output_rate(),
    ))
}

fn camera_error(error: impl std::fmt::Display) -> A2aLabError {
    A2aLabError::invalid("camera", error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    use nokhwa::utils::{FrameFormat, Resolution};

    use nokhwa::utils::FrameFormat as NokhwaFrameFormat;

    use super::{
        AUTO_PROBE_FOURCCS, AUTO_PROBE_LADDER, camera_format, inverted_probe_fourcc,
        profiles_for_fourcc, with_nokhwa_open_lock,
    };
    use crate::videos::{CaptureFormat, CaptureMode, CaptureRate, SelectedCaptureProfile};

    fn selected_profile(
        format: CaptureFormat,
        width: u32,
        height: u32,
        fps: u32,
    ) -> SelectedCaptureProfile {
        SelectedCaptureProfile::negotiated(
            CaptureMode::new(format, width, height).expect("mode"),
            CaptureRate::fps(fps).expect("capture"),
            CaptureRate::fps(5).expect("output"),
        )
    }

    #[test]
    fn exact_reopen_keeps_requested_camera_format() {
        let profile = selected_profile(CaptureFormat::Yuy2, 640, 480, 15);
        let format = camera_format(&profile).expect("format");
        assert_eq!(format.width(), 640);
        assert_eq!(format.height(), 480);
        assert_eq!(format.format(), NokhwaFrameFormat::YUYV);
        assert_eq!(format.frame_rate(), 15);
    }

    #[test]
    fn auto_probe_prefers_mjpeg_before_yuyv_and_sizes_in_order() {
        assert_eq!(AUTO_PROBE_FOURCCS[0], FrameFormat::MJPEG);
        assert_eq!(AUTO_PROBE_FOURCCS[1], FrameFormat::YUYV);
        assert_eq!(AUTO_PROBE_LADDER[0], (1920, 1080, 30));
        assert_eq!(AUTO_PROBE_LADDER[1], (1280, 720, 30));
        assert_eq!(AUTO_PROBE_LADDER[2], (640, 480, 30));
    }

    #[test]
    fn inverted_fourcc_probe_swaps_each_layout() {
        assert_eq!(inverted_probe_fourcc(FrameFormat::MJPEG), FrameFormat::YUYV);
        assert_eq!(inverted_probe_fourcc(FrameFormat::YUYV), FrameFormat::MJPEG);
    }

    #[test]
    fn recovered_fourcc_list_keeps_distinct_rates() {
        let mut listed = HashMap::new();
        listed.insert(Resolution::new(1280, 720), vec![30, 15, 30]);
        let profiles = profiles_for_fourcc(FrameFormat::YUYV, &listed);
        assert_eq!(profiles.len(), 2);
        assert!(profiles.iter().all(|profile| {
            profile.mode().format() == CaptureFormat::Yuy2
                && profile.mode().width() == 1280
                && profile.mode().height() == 720
        }));
        assert!(
            profiles
                .iter()
                .any(|profile| profile.capture_rate() == CaptureRate::fps(30).unwrap())
        );
        assert!(
            profiles
                .iter()
                .any(|profile| profile.capture_rate() == CaptureRate::fps(15).unwrap())
        );
    }

    #[test]
    fn open_lock_serializes_overlapping_holders() {
        static ACTIVE: AtomicUsize = AtomicUsize::new(0);
        static PEAK: AtomicUsize = AtomicUsize::new(0);

        let handles: Vec<_> = (0..8)
            .map(|_| {
                thread::spawn(|| {
                    with_nokhwa_open_lock(|| {
                        let current = ACTIVE.fetch_add(1, Ordering::SeqCst) + 1;
                        PEAK.fetch_max(current, Ordering::SeqCst);
                        thread::sleep(Duration::from_millis(10));
                        ACTIVE.fetch_sub(1, Ordering::SeqCst);
                    });
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("thread");
        }
        assert_eq!(PEAK.load(Ordering::SeqCst), 1);
    }
}
