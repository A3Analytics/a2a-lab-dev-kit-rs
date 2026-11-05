//! Real MJPEG and YUY2 fixtures through `OpenH264`, finite MP4, and HLS.
//!
//! No camera and no `FFmpeg`. Playback modes are fixture checks: every frame
//! stays available at 0.5x, one frame seeks without its neighbors, live and
//! finite playlists differ by the end tag, and 8x uses independently decodable
//! keyframes.

#![cfg(all(feature = "yuv", feature = "openh264"))]

use a2a_lab_dev_kit::videos::{
    CaptureClock, CaptureIdentity, CaptureTiming, CapturedColorFrame, ColorEncodePipeline,
    ColorEncodedFrame, ColorMatrix, ColorProperties, ColorRange, DvrPictureProperties,
    EpochPlaylistKind, OUTPUT_5FPS_PERIOD_NS, decode_luma,
};
use a2a_lab_dev_kit::{
    A2aLabError, DvrTrackId, MJPEG_MEDIA_TYPE, MemoryVideos, TimeRange, UtcTimestamp, VideoCodec,
    VideoSource, VideoSourceId, export_finite_h264_epochs, package_h264_epochs,
};
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageBuffer, ImageEncoder, Rgb};

fn stamp(ms: u64) -> UtcTimestamp {
    let seconds = ms / 1000;
    let millis = ms % 1000;
    UtcTimestamp::parse(&format!("2024-01-01T00:00:{seconds:02}.{millis:03}Z")).expect("stamp")
}

fn range_ms(start: u64, end: u64) -> TimeRange {
    TimeRange::new(stamp(start), stamp(end)).expect("range")
}

fn color(matrix: ColorMatrix, range: ColorRange) -> ColorProperties {
    ColorProperties::new(matrix, range)
}

fn yuy2_frame(sequence: u64, time_ns: u64, y: u8) -> CapturedColorFrame {
    let mut packed = Vec::with_capacity(16 * 32);
    for _ in 0..(16 * 16 / 2) {
        packed.extend_from_slice(&[y, 128, y, 128]);
    }
    CapturedColorFrame::yuy2(
        CaptureIdentity::new(VideoSourceId::new("cam").expect("source"), sequence),
        CaptureTiming::new(stamp(time_ns / 1_000_000), CaptureClock::monotonic(time_ns)),
        16,
        16,
        32,
        &packed,
        color(ColorMatrix::Bt601, ColorRange::Limited),
        None,
    )
    .expect("yuy2")
}

fn jpeg_frame(sequence: u64, time_ns: u64, y: u8) -> CapturedColorFrame {
    let image: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_raw(16, 16, vec![y; 16 * 16 * 3]).expect("image");
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 80)
        .write_image(image.as_raw(), 16, 16, ExtendedColorType::Rgb8)
        .expect("jpeg");
    CapturedColorFrame::mjpeg(
        CaptureIdentity::new(VideoSourceId::new("cam").expect("source"), sequence),
        CaptureTiming::new(stamp(time_ns / 1_000_000), CaptureClock::monotonic(time_ns)),
        16,
        16,
        &jpeg,
        color(ColorMatrix::Bt601, ColorRange::Full),
    )
    .expect("mjpeg")
}

fn encode_five(mjpeg: bool) -> Vec<ColorEncodedFrame> {
    let mut pipeline = ColorEncodePipeline::at_5fps(1_000_000, 8, None).expect("pipeline");
    for index in 0..5_u64 {
        let time = index * OUTPUT_5FPS_PERIOD_NS;
        let y = if index == 0 { 40 } else { 200 };
        let frame = if mjpeg {
            jpeg_frame(index, time, y)
        } else {
            yuy2_frame(index, time, y)
        };
        pipeline.push(frame).expect("push");
    }
    let mut frames = Vec::new();
    while let Some(frame) = pipeline.pull() {
        frames.push(frame);
    }
    frames
}

fn annex_b(initialization: &[u8], sample: &[u8]) -> Vec<u8> {
    let mut annex_b = initialization.to_vec();
    let mut index = 0_usize;
    while index + 4 <= sample.len() {
        let length = usize::try_from(u32::from_be_bytes([
            sample[index],
            sample[index + 1],
            sample[index + 2],
            sample[index + 3],
        ]))
        .expect("length");
        index += 4;
        annex_b.extend_from_slice(&[0, 0, 0, 1]);
        annex_b.extend_from_slice(&sample[index..index + length]);
        index += length;
    }
    annex_b
}

fn mean(samples: &[u8]) -> f32 {
    let sum: u32 = samples.iter().map(|sample| u32::from(*sample)).sum();
    #[allow(clippy::cast_precision_loss)]
    {
        sum as f32 / samples.len() as f32
    }
}

fn luma(initialization: &[u8], sample: &[u8]) -> Vec<u8> {
    decode_luma(&annex_b(initialization, sample)).expect("decode")
}

async fn store_epoch(frames: &[ColorEncodedFrame], track_id: &str) -> MemoryVideos {
    let videos = MemoryVideos::new();
    let source = VideoSourceId::new("cam").expect("source");
    videos
        .insert_source(
            VideoSource::new(
                source.clone(),
                "cam",
                "camera",
                None,
                None,
                1,
                1,
                5,
                MJPEG_MEDIA_TYPE,
            )
            .expect("source"),
        )
        .await
        .expect("insert source");
    videos
        .insert_h264_epoch(
            source.clone(),
            DvrTrackId::new(track_id).expect("track"),
            frames[0].initialization().to_vec(),
            DvrPictureProperties::new(16, 16).expect("picture"),
        )
        .await
        .expect("epoch");
    for (index, frame) in frames.iter().enumerate() {
        let start = index as u64 * 200;
        videos
            .insert_dvr_segment(
                source.clone(),
                DvrTrackId::new(track_id).expect("track"),
                range_ms(start, start + 200),
                frame.keyframe(),
                VideoCodec::H264,
                frame.sample().to_vec(),
            )
            .await
            .expect("segment");
    }
    videos
}

#[tokio::test]
async fn both_formats_make_decodable_finite_and_live_media() {
    for mjpeg in [false, true] {
        let frames = encode_five(mjpeg);
        assert_eq!(frames.len(), 5);
        let first = luma(frames[0].initialization(), frames[0].sample());
        let last = luma(frames[4].initialization(), frames[4].sample());
        assert_eq!(first.len(), 16 * 16);
        assert!((mean(&first) - 40.0).abs() < 25.0);
        assert!((mean(&last) - 200.0).abs() < 25.0);
        assert!(mean(&last) > mean(&first) + 50.0);
        let middle = luma(frames[2].initialization(), frames[2].sample());
        assert_eq!(middle.len(), 16 * 16);
        assert_eq!(
            frames[2].original().timing().clock().monotonic_ns(),
            Some(2 * OUTPUT_5FPS_PERIOD_NS)
        );

        let videos = store_epoch(&frames, "epoch").await;
        let source = VideoSourceId::new("cam").expect("source");
        let window = range_ms(0, 1_000);
        let clips = export_finite_h264_epochs(&videos, &source, window)
            .await
            .expect("mp4");
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].initialization(), frames[0].initialization());
        assert_eq!(clips[0].range(), window);
        for frame in &frames {
            assert!(
                clips[0]
                    .mp4()
                    .windows(frame.sample().len())
                    .any(|bytes| bytes == frame.sample())
            );
            assert_eq!(
                luma(clips[0].initialization(), frame.sample()).len(),
                16 * 16
            );
        }

        let finite = package_h264_epochs(&videos, &source, window, EpochPlaylistKind::Finite)
            .await
            .expect("hls");
        let live = package_h264_epochs(&videos, &source, window, EpochPlaylistKind::Live)
            .await
            .expect("live");
        assert!(finite.playlist().contains("#EXT-X-ENDLIST"));
        assert!(!live.playlist().contains("#EXT-X-ENDLIST"));
        assert_eq!(finite.segments().len(), 5);
        assert_eq!(finite.segments()[2].program_date_time(), stamp(400));
        let trick = finite.trick_playlist().expect("8x");
        assert_eq!(trick.matches("#EXTINF").count(), 5);
        for frame in &frames {
            assert!(frame.keyframe());
            let alone = luma(frame.initialization(), frame.sample());
            assert_eq!(alone.len(), 16 * 16);
        }
    }
}

#[tokio::test]
async fn a_configuration_change_leaves_the_gap_empty() {
    let first = encode_five(false);
    let second = encode_five(true);
    assert_ne!(first[0].initialization(), second[0].initialization());
    let videos = store_epoch(&first, "epoch-a").await;
    let source = VideoSourceId::new("cam").expect("source");
    videos
        .insert_h264_epoch(
            source.clone(),
            DvrTrackId::new("epoch-b").expect("track"),
            second[0].initialization().to_vec(),
            DvrPictureProperties::new(16, 16).expect("picture"),
        )
        .await
        .expect("epoch b");
    for (index, frame) in second.iter().enumerate() {
        let start = 2_000 + index as u64 * 200;
        videos
            .insert_dvr_segment(
                source.clone(),
                DvrTrackId::new("epoch-b").expect("track"),
                range_ms(start, start + 200),
                true,
                VideoCodec::H264,
                frame.sample().to_vec(),
            )
            .await
            .expect("segment");
    }
    let clips = export_finite_h264_epochs(&videos, &source, range_ms(0, 3_000))
        .await
        .expect("export");
    assert_eq!(clips.len(), 2);
    assert!(clips[0].range().end() < clips[1].range().start());
    assert_ne!(clips[0].initialization(), clips[1].initialization());
    let gap = export_finite_h264_epochs(&videos, &source, range_ms(1_000, 2_000)).await;
    assert!(matches!(
        gap,
        Err(A2aLabError::Video {
            availability: a2a_lab_dev_kit::VideoAvailability::NoRecording,
            ..
        })
    ));
    let finite = package_h264_epochs(
        &videos,
        &source,
        range_ms(0, 3_000),
        EpochPlaylistKind::Finite,
    )
    .await
    .expect("hls");
    assert!(finite.playlist().contains("#EXT-X-DISCONTINUITY"));
    assert_eq!(finite.inits().len(), 2);
}
