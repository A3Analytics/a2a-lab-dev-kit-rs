//! Annex B pictures become owned length-prefixed DVR samples.

use a2a_lab_dev_kit::{A2aLabError, DvrMediaPayload, VideoCodec, frame_h264_picture};

fn four_byte(nals: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for nal in nals {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
    }
    out
}

#[test]
fn one_picture_keeps_parameter_sets_out_of_the_sample() {
    let sps = [0x67_u8, 0x42, 0x00];
    let pps = [0x68_u8, 0xCE];
    let sei = [0x06_u8, 0x05];
    let slice_a = [0x65_u8, 0x11, 0x22];
    let slice_b = [0x65_u8, 0x33];
    let mut annex_b = vec![0, 0, 1];
    annex_b.extend_from_slice(&sps);
    annex_b.extend_from_slice(&[0, 0, 0, 1]);
    annex_b.extend_from_slice(&pps);
    annex_b.extend_from_slice(&[0, 0, 0, 1]);
    annex_b.extend_from_slice(&sei);
    annex_b.extend_from_slice(&[0, 0, 1]);
    annex_b.extend_from_slice(&slice_a);
    annex_b.extend_from_slice(&[0, 0, 0, 1]);
    annex_b.extend_from_slice(&slice_b);
    let framed = frame_h264_picture(&annex_b).expect("frame");
    assert!(framed.keyframe());
    assert_eq!(framed.initialization(), four_byte(&[&sps, &pps]).as_slice());
    assert!(framed.sample().windows(3).any(|nal| nal == slice_a));
    assert!(framed.sample().windows(2).any(|nal| nal == slice_b));
    assert!(framed.sample().windows(2).any(|nal| nal == sei));
    assert!(!framed.sample().windows(3).any(|nal| nal == sps));
    match framed.payload() {
        DvrMediaPayload::EncodedSample { codec, bytes } => {
            assert_eq!(codec, VideoCodec::H264);
            assert_eq!(bytes, framed.sample());
        }
        DvrMediaPayload::MjpegFrame(_) => panic!("sample must stay H.264"),
    }
}

#[test]
fn a_non_idr_slice_is_not_a_keyframe() {
    let framed = frame_h264_picture(&four_byte(&[&[0x67, 0x42], &[0x68, 0xCE], &[0x41, 0x01]]))
        .expect("frame");
    assert!(!framed.keyframe());
}

#[test]
fn empty_and_slice_less_input_fail() {
    assert!(matches!(
        frame_h264_picture(&[]),
        Err(A2aLabError::Invalid {
            field: "annex_b",
            ..
        })
    ));
    assert!(matches!(
        frame_h264_picture(&four_byte(&[&[0x67, 0x42], &[0x68, 0xCE]])),
        Err(A2aLabError::Invalid {
            field: "annex_b",
            ..
        })
    ));
    assert!(matches!(
        frame_h264_picture(&[0x65, 0x00]),
        Err(A2aLabError::Invalid {
            field: "annex_b",
            ..
        })
    ));
}

#[cfg(feature = "openh264")]
mod round_trip {
    use a2a_lab_dev_kit::VideoSourceId;
    use a2a_lab_dev_kit::videos::{
        AllIdrEncoder, CaptureIdentity, ColorMatrix, ColorProperties, ColorRange, I420Picture,
        decode_luma, frame_h264_picture,
    };

    fn color(matrix: ColorMatrix, range: ColorRange) -> ColorProperties {
        ColorProperties::new(matrix, range)
    }

    fn gray(value: u8) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        (vec![value; 16 * 16], vec![128; 8 * 8], vec![128; 8 * 8])
    }

    fn mean(samples: &[u8]) -> f32 {
        let sum: u32 = samples.iter().map(|sample| u32::from(*sample)).sum();
        #[allow(clippy::cast_precision_loss)]
        {
            sum as f32 / samples.len() as f32
        }
    }

    fn annex_b_from_framed(framed: &a2a_lab_dev_kit::H264PictureSample) -> Vec<u8> {
        let mut annex_b = framed.initialization().to_vec();
        let sample = framed.sample();
        let mut index = 0_usize;
        while index + 4 <= sample.len() {
            let length = u32::from_be_bytes([
                sample[index],
                sample[index + 1],
                sample[index + 2],
                sample[index + 3],
            ]);
            let length = usize::try_from(length).expect("length");
            index += 4;
            annex_b.extend_from_slice(&[0, 0, 0, 1]);
            annex_b.extend_from_slice(&sample[index..index + length]);
            index += length;
        }
        annex_b
    }

    #[test]
    fn framed_samples_decode_and_outlive_the_next_encode() {
        let limited = color(ColorMatrix::Bt601, ColorRange::Limited);
        let mut encoder = AllIdrEncoder::new(16, 16, 5.0, 1_000_000, limited).expect("encoder");
        let identity = CaptureIdentity::new(VideoSourceId::new("cam").expect("source"), 1);
        let (y, u, v) = gray(40);
        let first = encoder
            .encode(
                I420Picture::new(16, 16, &y, &u, &v, limited),
                &identity,
                1_000,
            )
            .expect("first");
        let framed = frame_h264_picture(first.bytes()).expect("frame");
        let owned = framed.sample().to_vec();
        let (y2, u2, v2) = gray(200);
        let second = encoder
            .encode(
                I420Picture::new(16, 16, &y2, &u2, &v2, limited),
                &identity,
                2_000,
            )
            .expect("second");
        assert_eq!(framed.sample(), owned.as_slice());
        assert_ne!(first.bytes(), second.bytes());
        let decoded = decode_luma(&annex_b_from_framed(&framed)).expect("decode");
        assert!((mean(&decoded) - 40.0).abs() < 20.0);

        let full = color(ColorMatrix::Bt709, ColorRange::Full);
        let mut other = AllIdrEncoder::new(16, 16, 5.0, 1_000_000, full).expect("other");
        let changed = other
            .encode(I420Picture::new(16, 16, &y, &u, &v, full), &identity, 3_000)
            .expect("changed");
        let framed_changed = frame_h264_picture(changed.bytes()).expect("frame changed");
        assert_ne!(framed.initialization(), framed_changed.initialization());
        assert!(
            !framed
                .sample()
                .windows(framed_changed.initialization().len())
                .any(|window| window == framed_changed.initialization())
        );
    }
}
