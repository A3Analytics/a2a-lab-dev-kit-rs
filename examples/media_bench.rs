//! Opt-in measurement of MJPEG and YUY2 conversion plus all-IDR encode.
//!
//! Not part of `mise run quality`. A slow result is printed as a failed budget
//! and does not panic. Targets that this process does not run are omitted.

use std::process::Command;
use std::time::{Duration, Instant};

use a2a_lab_dev_kit::videos::{
    AllIdrEncoder, CaptureClock, CaptureIdentity, CaptureTiming, CapturedColorFrame, ColorMatrix,
    ColorProperties, ColorRange, DecodedPicture, I420Picture, JpegDerivativeLimits,
    MjpegDecodeLimits, MjpegDecodeTarget, OwnedI420, PackedYuy2, decode_mjpeg, jpeg_derivative,
    yuy2_to_i420,
};
use a2a_lab_dev_kit::{UtcTimestamp, VideoSourceId};
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageBuffer, ImageEncoder, Rgb};

const WARMUP: usize = 2;
const SAMPLES: usize = 12;
const BUDGET: Duration = Duration::from_millis(200);

struct Stats {
    p50: Duration,
    p95: Duration,
    peak_bytes: usize,
}

fn main() {
    println!("host: {}", host());
    println!("warmup: {WARMUP}");
    println!("samples: {SAMPLES}");
    println!("target: {}", std::env::consts::ARCH);
    println!("resolution,path,stage,p50_ms,p95_ms,throughput_fps,peak_bytes,budget");
    let mut rows = Vec::new();
    for (width, height) in [(640, 480), (1280, 720), (1280, 800), (1920, 1080)] {
        rows.extend(measure_resolution(width, height));
    }
    for row in &rows {
        println!("{row}");
    }
    if let Some(dominant) = dominant(&rows) {
        println!("dominant_stage: {dominant}");
    }
}

struct Row {
    label: String,
    stage: &'static str,
    stats: Stats,
}

impl std::fmt::Display for Row {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let throughput = if self.stats.p50.is_zero() {
            0.0
        } else {
            1.0 / self.stats.p50.as_secs_f64()
        };
        let verdict = if self.stats.p95 <= BUDGET {
            "pass"
        } else {
            "fail"
        };
        write!(
            formatter,
            "{label},{stage},{p50:.3},{p95:.3},{throughput:.2},{peak},{verdict}",
            label = self.label,
            stage = self.stage,
            p50 = self.stats.p50.as_secs_f64() * 1_000.0,
            p95 = self.stats.p95.as_secs_f64() * 1_000.0,
            peak = self.stats.peak_bytes,
        )
    }
}

fn dominant(rows: &[Row]) -> Option<&str> {
    rows.iter()
        .filter(|row| {
            matches!(
                row.stage,
                "mjpeg-decode" | "yuy2-convert" | "encode" | "copy" | "jpeg-derivative"
            )
        })
        .max_by_key(|row| row.stats.p50)
        .map(|row| row.stage)
}

fn measure_resolution(width: u32, height: u32) -> Vec<Row> {
    let jpeg = jpeg_bytes(width, height, 40);
    let yuy2 = yuy2_bytes(width, height, 40);
    let frame = mjpeg_frame(width, height, &jpeg);
    let decoded = decode_mjpeg(
        &frame,
        MjpegDecodeTarget::I420,
        MjpegDecodeLimits::standard(),
    )
    .expect("warmup decode");
    let DecodedPicture::I420(picture) = decoded.picture() else {
        panic!("expected I420");
    };
    let mut encoder =
        AllIdrEncoder::new(width, height, 5.0, 2_000_000, picture.properties()).expect("encoder");
    let identity = CaptureIdentity::new(VideoSourceId::new("bench").expect("source"), 1);
    let mut decode_times = Vec::new();
    let mut encode_times = Vec::new();
    let mut copy_times = Vec::new();
    let mut peak = picture.y().len() + picture.u().len() + picture.v().len();
    for index in 0..(WARMUP + SAMPLES) {
        let started = Instant::now();
        let decoded = decode_mjpeg(
            &frame,
            MjpegDecodeTarget::I420,
            MjpegDecodeLimits::standard(),
        )
        .expect("decode");
        let decode_elapsed = started.elapsed();
        let DecodedPicture::I420(picture) = decoded.picture() else {
            panic!("expected I420");
        };
        peak = peak.max(picture.y().len() + picture.u().len() + picture.v().len());
        let copied = Instant::now();
        let _planes = [
            picture.y().to_vec(),
            picture.u().to_vec(),
            picture.v().to_vec(),
        ];
        let copy_elapsed = copied.elapsed();
        let started = Instant::now();
        encoder
            .encode(
                I420Picture::new(
                    picture.width(),
                    picture.height(),
                    picture.y(),
                    picture.u(),
                    picture.v(),
                    picture.properties(),
                ),
                &identity,
                u64::try_from(index).unwrap_or(0),
            )
            .expect("encode");
        let encode_elapsed = started.elapsed();
        if index >= WARMUP {
            decode_times.push(decode_elapsed);
            encode_times.push(encode_elapsed);
            copy_times.push(copy_elapsed);
        }
    }
    let label = format!("{width}x{height},mjpeg-i420");
    let mut rows = vec![
        row(&label, "mjpeg-decode", &decode_times, peak),
        row(&label, "encode", &encode_times, peak),
        row(&label, "copy", &copy_times, peak),
        row(
            &label,
            "total",
            &sum_times(&decode_times, &encode_times),
            peak,
        ),
    ];
    rows.extend(orbbec_path(width, height, &jpeg));
    rows.push(derivative_row(width, height, &yuy2));
    rows
}

fn orbbec_path(width: u32, height: u32, jpeg: &[u8]) -> Vec<Row> {
    let frame = mjpeg_frame(width, height, jpeg);
    let decoded = decode_mjpeg(
        &frame,
        MjpegDecodeTarget::Yuy2,
        MjpegDecodeLimits::standard(),
    )
    .expect("yuy2 decode");
    let DecodedPicture::Yuy2(packed) = decoded.picture() else {
        panic!("expected YUY2");
    };
    let i420 = yuy2_to_i420(PackedYuy2::new(
        packed,
        width,
        height,
        width * 2,
        decoded.properties(),
    ))
    .expect("convert");
    let mut encoder =
        AllIdrEncoder::new(width, height, 5.0, 2_000_000, i420.properties()).expect("encoder");
    let identity = CaptureIdentity::new(VideoSourceId::new("bench").expect("source"), 1);
    let mut decode_times = Vec::new();
    let mut convert_times = Vec::new();
    let mut encode_times = Vec::new();
    let mut peak = packed.len();
    for index in 0..(WARMUP + SAMPLES) {
        let started = Instant::now();
        let decoded = decode_mjpeg(
            &frame,
            MjpegDecodeTarget::Yuy2,
            MjpegDecodeLimits::standard(),
        )
        .expect("decode");
        let decode_elapsed = started.elapsed();
        let DecodedPicture::Yuy2(packed) = decoded.picture() else {
            panic!("expected YUY2");
        };
        peak = peak.max(packed.len());
        let started = Instant::now();
        let picture = yuy2_to_i420(PackedYuy2::new(
            packed,
            width,
            height,
            width * 2,
            decoded.properties(),
        ))
        .expect("convert");
        let convert_elapsed = started.elapsed();
        peak = peak.max(picture.y().len() + picture.u().len() + picture.v().len() + packed.len());
        let started = Instant::now();
        encoder
            .encode(
                picture_view(&picture),
                &identity,
                u64::try_from(index).unwrap_or(0),
            )
            .expect("encode");
        if index >= WARMUP {
            decode_times.push(decode_elapsed);
            convert_times.push(convert_elapsed);
            encode_times.push(started.elapsed());
        }
    }
    let label = format!("{width}x{height},mjpeg-yuy2-i420");
    let mut total = sum_times(&decode_times, &convert_times);
    for (index, sample) in encode_times.iter().enumerate() {
        total[index] += *sample;
    }
    vec![
        row(&label, "mjpeg-decode", &decode_times, peak),
        row(&label, "yuy2-convert", &convert_times, peak),
        row(&label, "encode", &encode_times, peak),
        row(&label, "total", &total, peak),
    ]
}

fn derivative_row(width: u32, height: u32, packed: &[u8]) -> Row {
    let limits = JpegDerivativeLimits::new(32 * 1024 * 1024).expect("limits");
    let mut times = Vec::new();
    let mut peak = 0_usize;
    for index in 0..(WARMUP + SAMPLES) {
        let unique = yuy2_frame_at(width, height, packed, u64::try_from(index).unwrap_or(0));
        let started = Instant::now();
        let derivative = jpeg_derivative(&unique, 80, limits).expect("jpeg");
        let elapsed = started.elapsed();
        peak = peak.max(derivative.jpeg().len());
        if index >= WARMUP {
            times.push(elapsed);
        }
    }
    row(
        &format!("{width}x{height},jpeg-derivative"),
        "jpeg-derivative",
        &times,
        peak,
    )
}

fn picture_view(picture: &OwnedI420) -> I420Picture<'_> {
    I420Picture::new(
        picture.width(),
        picture.height(),
        picture.y(),
        picture.u(),
        picture.v(),
        picture.properties(),
    )
}

fn row(label: &str, stage: &'static str, samples: &[Duration], peak_bytes: usize) -> Row {
    Row {
        label: label.to_owned(),
        stage,
        stats: Stats {
            p50: percentile(samples, 50),
            p95: percentile(samples, 95),
            peak_bytes,
        },
    }
}

fn sum_times(left: &[Duration], right: &[Duration]) -> Vec<Duration> {
    left.iter()
        .zip(right)
        .map(|(one, other)| *one + *other)
        .collect()
}

fn percentile(samples: &[Duration], percent: usize) -> Duration {
    let mut ordered = samples.to_vec();
    ordered.sort();
    let index = (ordered.len().saturating_mul(percent) / 100).min(ordered.len().saturating_sub(1));
    ordered[index]
}

fn jpeg_bytes(width: u32, height: u32, value: u8) -> Vec<u8> {
    let pixels =
        vec![value; usize::try_from(width).unwrap_or(0) * usize::try_from(height).unwrap_or(0) * 3];
    let image: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width, height, pixels).expect("image");
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 80)
        .write_image(image.as_raw(), width, height, ExtendedColorType::Rgb8)
        .expect("jpeg");
    jpeg
}

fn yuy2_bytes(width: u32, height: u32, value: u8) -> Vec<u8> {
    let pixels = usize::try_from(width).unwrap_or(0) * usize::try_from(height).unwrap_or(0);
    let mut packed = Vec::with_capacity(pixels * 2);
    for _ in 0..(pixels / 2) {
        packed.extend_from_slice(&[value, 128, value, 128]);
    }
    packed
}

fn mjpeg_frame(width: u32, height: u32, jpeg: &[u8]) -> CapturedColorFrame {
    CapturedColorFrame::mjpeg(
        CaptureIdentity::new(VideoSourceId::new("bench").expect("source"), 1),
        CaptureTiming::new(
            UtcTimestamp::parse("2024-01-01T00:00:00Z").expect("time"),
            CaptureClock::monotonic(0),
        ),
        width,
        height,
        jpeg,
        color(),
    )
    .expect("frame")
}

fn yuy2_frame_at(width: u32, height: u32, packed: &[u8], sequence: u64) -> CapturedColorFrame {
    CapturedColorFrame::yuy2(
        CaptureIdentity::new(VideoSourceId::new("bench").expect("source"), sequence),
        CaptureTiming::new(
            UtcTimestamp::parse("2024-01-01T00:00:00Z").expect("time"),
            CaptureClock::monotonic(sequence.saturating_mul(200_000_000)),
        ),
        width,
        height,
        usize::try_from(width).unwrap_or(0).saturating_mul(2),
        packed,
        color(),
        None,
    )
    .expect("frame")
}

fn color() -> ColorProperties {
    ColorProperties::new(ColorMatrix::Bt601, ColorRange::Full)
}

fn host() -> String {
    let cpu = Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map_or_else(
            || "unreported".to_owned(),
            |output| String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        );
    format!(
        "{arch}-{os} {cpu}",
        arch = std::env::consts::ARCH,
        os = std::env::consts::OS
    )
}
