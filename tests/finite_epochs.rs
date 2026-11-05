//! Finite H.264 export keeps each epoch initialization with its samples.

use a2a_lab_dev_kit::{
    A2aLabError, DvrMediaPackager, DvrPackagerConfig, DvrPictureProperties, DvrTrackId,
    MJPEG_MEDIA_TYPE, MemoryVideos, TimeRange, UtcTimestamp, VideoAvailability, VideoCodec,
    VideoSource, VideoSourceId, export_finite_h264_epochs,
};

const INIT_A: &[u8] = &[
    0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, 0x00, 0x00, 0x00, 0x01, 0x68, 0xCE,
];
const INIT_B: &[u8] = &[
    0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1F, 0x00, 0x00, 0x00, 0x01, 0x68, 0xCF,
];

fn source() -> VideoSourceId {
    VideoSourceId::new("cam").expect("source")
}

fn track(id: &str) -> DvrTrackId {
    DvrTrackId::new(id).expect("track")
}

fn range(start: &str, end: &str) -> TimeRange {
    TimeRange::new(
        UtcTimestamp::parse(start).expect("start"),
        UtcTimestamp::parse(end).expect("end"),
    )
    .expect("range")
}

async fn store() -> MemoryVideos {
    let videos = MemoryVideos::new();
    videos
        .insert_source(
            VideoSource::new(
                source(),
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
}

async fn epoch(videos: &MemoryVideos, id: &str, init: &[u8], width: u32, height: u32) {
    videos
        .insert_h264_epoch(
            source(),
            track(id),
            init.to_vec(),
            DvrPictureProperties::new(width, height).expect("picture"),
        )
        .await
        .expect("epoch");
}

async fn segment(
    videos: &MemoryVideos,
    id: &str,
    start: &str,
    end: &str,
    sample: u8,
    keyframe: bool,
) {
    segment_bytes(
        videos,
        id,
        start,
        end,
        vec![0x00, 0x00, 0x00, 0x02, 0x65, sample],
        keyframe,
    )
    .await;
}

async fn segment_bytes(
    videos: &MemoryVideos,
    id: &str,
    start: &str,
    end: &str,
    sample: Vec<u8>,
    keyframe: bool,
) {
    videos
        .insert_dvr_segment(
            source(),
            track(id),
            range(start, end),
            keyframe,
            VideoCodec::H264,
            sample,
        )
        .await
        .expect("segment");
}

#[tokio::test]
async fn one_epoch_uses_its_retained_initialization() {
    let videos = store().await;
    epoch(&videos, "epoch-a", INIT_A, 640, 480).await;
    segment(
        &videos,
        "epoch-a",
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        0x11,
        true,
    )
    .await;
    segment(
        &videos,
        "epoch-a",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:02Z",
        0x22,
        false,
    )
    .await;
    let window = range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z");
    let clips = export_finite_h264_epochs(&videos, &source(), window)
        .await
        .expect("export");
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].initialization(), INIT_A);
    assert_eq!(clips[0].range(), window);
    assert!(
        clips[0]
            .mp4()
            .windows(4)
            .any(|nal| nal == [0x67, 0x42, 0x00, 0x1E])
    );
    assert!(clips[0].mp4().windows(2).any(|nal| nal == [0x65, 0x11]));
    assert!(clips[0].mp4().windows(2).any(|nal| nal == [0x65, 0x22]));
    let single = DvrMediaPackager::new(DvrPackagerConfig::default())
        .export_finite_mp4(&videos, &source(), &track("epoch-a"), window)
        .await
        .expect("single");
    assert_eq!(clips[0].mp4(), single.bytes());
}

#[tokio::test]
async fn a_gap_stays_between_same_size_epochs() {
    let videos = store().await;
    epoch(&videos, "epoch-a", INIT_A, 640, 480).await;
    epoch(&videos, "epoch-b", INIT_B, 640, 480).await;
    segment(
        &videos,
        "epoch-a",
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        0x11,
        true,
    )
    .await;
    segment(
        &videos,
        "epoch-b",
        "2024-01-01T00:00:05Z",
        "2024-01-01T00:00:06Z",
        0x33,
        true,
    )
    .await;
    let clips = export_finite_h264_epochs(
        &videos,
        &source(),
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:06Z"),
    )
    .await
    .expect("export");
    assert_eq!(clips.len(), 2);
    assert_eq!(clips[0].initialization(), INIT_A);
    assert_eq!(clips[1].initialization(), INIT_B);
    assert!(clips[0].range().end() < clips[1].range().start());
    assert!(
        clips[0]
            .mp4()
            .windows(4)
            .any(|nal| nal == [0x67, 0x42, 0x00, 0x1E])
    );
    assert!(
        !clips[0]
            .mp4()
            .windows(4)
            .any(|nal| nal == [0x67, 0x42, 0x00, 0x1F])
    );
    assert!(
        clips[1]
            .mp4()
            .windows(4)
            .any(|nal| nal == [0x67, 0x42, 0x00, 0x1F])
    );
    assert!(!clips[1].mp4().windows(2).any(|nal| nal == [0x65, 0x11]));
}

#[tokio::test]
async fn a_dimension_change_publishes_nothing() {
    let videos = store().await;
    epoch(&videos, "epoch-a", INIT_A, 640, 480).await;
    epoch(&videos, "epoch-b", INIT_B, 1280, 720).await;
    segment(
        &videos,
        "epoch-a",
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        0x11,
        true,
    )
    .await;
    segment(
        &videos,
        "epoch-b",
        "2024-01-01T00:00:01Z",
        "2024-01-01T00:00:02Z",
        0x33,
        true,
    )
    .await;
    let error = export_finite_h264_epochs(
        &videos,
        &source(),
        range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
    )
    .await
    .expect_err("dimensions");
    assert!(matches!(
        error,
        A2aLabError::Invalid {
            field: "picture",
            ..
        }
    ));
}

#[tokio::test]
async fn an_empty_window_is_not_a_clip() {
    let videos = store().await;
    epoch(&videos, "epoch-a", INIT_A, 640, 480).await;
    segment(
        &videos,
        "epoch-a",
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:01Z",
        0x11,
        true,
    )
    .await;
    let error = export_finite_h264_epochs(
        &videos,
        &source(),
        range("2024-01-01T01:00:00Z", "2024-01-01T01:00:01Z"),
    )
    .await
    .expect_err("empty");
    assert!(matches!(
        error,
        A2aLabError::Video {
            availability: VideoAvailability::NoRecording,
            ..
        }
    ));
}

#[cfg(feature = "openh264")]
mod real_bitstreams {
    use openh264::OpenH264API;
    use openh264::decoder::Decoder;
    use openh264::encoder::{
        BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile, RateControlMode,
        VuiConfig,
    };
    use openh264::formats::{YUVBuffer, YUVSource};

    use super::{epoch, range, segment_bytes, source, store};

    fn encode(value: u8, vui: VuiConfig) -> (Vec<u8>, Vec<u8>) {
        let config = EncoderConfig::new()
            .skip_frames(false)
            .bitrate(BitRate::from_bps(500_000))
            .max_frame_rate(FrameRate::from_hz(5.0))
            .rate_control_mode(RateControlMode::Bitrate)
            .profile(Profile::Baseline)
            .intra_frame_period(IntraFramePeriod::from_num_frames(1))
            .vui(vui);
        let mut encoder =
            Encoder::with_api_config(OpenH264API::from_source(), config).expect("encoder");
        let mut yuv = vec![value; 16 * 16];
        yuv.extend(std::iter::repeat_n(128, 8 * 8 * 2));
        let picture = YUVBuffer::from_vec(yuv, 16, 16);
        let annex_b = encoder.encode(&picture).expect("encode").to_vec();
        split_picture(&annex_b)
    }

    fn split_picture(annex_b: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut initialization = Vec::new();
        let mut sample = Vec::new();
        for nal in nals(annex_b) {
            let nal_type = nal[0] & 0x1F;
            if nal_type == 7 || nal_type == 8 {
                initialization.extend_from_slice(&[0, 0, 0, 1]);
                initialization.extend_from_slice(nal);
            } else {
                let length = u32::try_from(nal.len()).expect("length");
                sample.extend_from_slice(&length.to_be_bytes());
                sample.extend_from_slice(nal);
            }
        }
        (initialization, sample)
    }

    fn nals(bytes: &[u8]) -> Vec<&[u8]> {
        let mut starts = Vec::new();
        let mut index = 0_usize;
        while index + 3 <= bytes.len() {
            if bytes[index..].starts_with(&[0, 0, 0, 1]) {
                starts.push(index + 4);
                index += 4;
            } else if bytes[index..].starts_with(&[0, 0, 1]) {
                starts.push(index + 3);
                index += 3;
            } else {
                index += 1;
            }
        }
        starts
            .iter()
            .enumerate()
            .map(|(position, start)| {
                let end = starts
                    .get(position + 1)
                    .copied()
                    .map_or(bytes.len(), |next| {
                        if next >= 4 && bytes[next - 4..next] == [0, 0, 0, 1] {
                            next - 4
                        } else {
                            next - 3
                        }
                    });
                &bytes[*start..end]
            })
            .collect()
    }

    fn mean(samples: &[u8]) -> f32 {
        let sum: u32 = samples.iter().map(|sample| u32::from(*sample)).sum();
        #[allow(clippy::cast_precision_loss)]
        {
            sum as f32 / samples.len() as f32
        }
    }

    fn decoded_luma(initialization: &[u8], sample: &[u8]) -> Vec<u8> {
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
        let mut decoder = Decoder::new().expect("decoder");
        let picture = decoder.decode(&annex_b).expect("decode").expect("picture");
        let (width, stride) = (picture.dimensions().0, picture.strides().0);
        let mut luma = Vec::new();
        for row in 0..picture.dimensions().1 {
            let start = row * stride;
            luma.extend_from_slice(&picture.y()[start..start + width]);
        }
        luma
    }

    #[tokio::test]
    async fn same_size_epochs_decode_with_their_own_configuration() {
        let (init_a, sample_a) = encode(40, VuiConfig::bt601());
        let (init_b, sample_b) = encode(200, VuiConfig::bt709_full());
        assert_ne!(init_a, init_b);
        let videos = store().await;
        epoch(&videos, "epoch-a", &init_a, 16, 16).await;
        epoch(&videos, "epoch-b", &init_b, 16, 16).await;
        segment_bytes(
            &videos,
            "epoch-a",
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:01Z",
            sample_a.clone(),
            true,
        )
        .await;
        segment_bytes(
            &videos,
            "epoch-b",
            "2024-01-01T00:00:02Z",
            "2024-01-01T00:00:03Z",
            sample_b.clone(),
            true,
        )
        .await;
        let clips = a2a_lab_dev_kit::export_finite_h264_epochs(
            &videos,
            &source(),
            range("2024-01-01T00:00:00Z", "2024-01-01T00:00:03Z"),
        )
        .await
        .expect("export");
        assert_eq!(clips.len(), 2);
        assert!(clips[0].range().end() < clips[1].range().start());
        for (clip, sample, expected) in
            [(&clips[0], &sample_a, 40.0), (&clips[1], &sample_b, 200.0)]
        {
            assert!(
                clip.mp4()
                    .windows(sample.len())
                    .any(|window| window == sample)
            );
            let retained = if (expected - 40.0_f32).abs() < f32::EPSILON {
                init_a.as_slice()
            } else {
                init_b.as_slice()
            };
            assert_eq!(clip.initialization(), retained);
            let luma = decoded_luma(clip.initialization(), sample);
            assert!((mean(&luma) - expected).abs() < 20.0);
        }
    }
}
