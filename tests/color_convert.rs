//! YUY2 and RGB conversion through the optional `yuv` feature.

#![cfg(feature = "yuv")]

use a2a_lab_dev_kit::A2aLabError;
use a2a_lab_dev_kit::videos::{
    ColorMatrix, ColorProperties, ColorRange, InterleavedColor, InterleavedRgb, MAX_ENCODER_EDGE,
    OwnedI420, PackedYuy2, rgb_to_yuy2, yuy2_to_i420,
};

const TOLERANCE: i16 = 4;

fn props(matrix: ColorMatrix, range: ColorRange) -> ColorProperties {
    ColorProperties::new(matrix, range)
}

fn limited(matrix: ColorMatrix) -> ColorProperties {
    props(matrix, ColorRange::Limited)
}

fn assert_invalid(error: A2aLabError, field: &str) {
    match error {
        A2aLabError::Invalid { field: actual, .. } => assert_eq!(actual, field),
        other => panic!("expected invalid {field}, got {other:?}"),
    }
}

fn near(actual: u8, expected: f32) {
    let delta = f32::from(actual) - expected;
    assert!(
        delta.abs() <= f32::from(TOLERANCE),
        "actual {actual} expected {expected:.2}"
    );
}

fn bt601_limited(red: f32, green: f32, blue: f32) -> (f32, f32, f32) {
    yuv(red, green, blue, 0.299, 0.587, 0.114, true)
}

fn bt709_limited(red: f32, green: f32, blue: f32) -> (f32, f32, f32) {
    yuv(red, green, blue, 0.2126, 0.7152, 0.0722, true)
}

fn bt601_full(red: f32, green: f32, blue: f32) -> (f32, f32, f32) {
    yuv(red, green, blue, 0.299, 0.587, 0.114, false)
}

fn yuv(
    red: f32,
    green: f32,
    blue: f32,
    kr: f32,
    kg: f32,
    kb: f32,
    limited: bool,
) -> (f32, f32, f32) {
    let red = red / 255.0;
    let green = green / 255.0;
    let blue = blue / 255.0;
    let luma = kr * red + kg * green + kb * blue;
    let chroma_u = (blue - luma) / (2.0 * (1.0 - kb));
    let chroma_v = (red - luma) / (2.0 * (1.0 - kr));
    if limited {
        (
            16.0 + 219.0 * luma,
            128.0 + 224.0 * chroma_u,
            128.0 + 224.0 * chroma_v,
        )
    } else {
        (
            255.0 * luma,
            128.0 + 255.0 * chroma_u,
            128.0 + 255.0 * chroma_v,
        )
    }
}

fn solid_rgb(red: u8, green: u8, blue: u8, width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((width * height * 3) as usize);
    for _ in 0..width * height {
        bytes.extend([red, green, blue]);
    }
    bytes
}

fn packed_yuy2(y0: u8, u: u8, y1: u8, v: u8, pairs: usize) -> Vec<u8> {
    [y0, u, y1, v].repeat(pairs)
}

#[test]
fn yuy2_becomes_planar_i420_without_reordering_chroma() {
    let bytes = packed_yuy2(20, 40, 60, 80, 4);
    let picture = yuy2_to_i420(PackedYuy2::new(
        &bytes,
        4,
        2,
        8,
        props(ColorMatrix::Unknown, ColorRange::Unknown),
    ))
    .expect("i420");
    assert_eq!(picture.y(), &[20, 60, 20, 60, 20, 60, 20, 60]);
    assert_eq!(picture.u(), &[40, 40]);
    assert_eq!(picture.v(), &[80, 80]);
    assert_eq!(picture.properties().matrix(), ColorMatrix::Unknown);
    assert_eq!(picture.properties().range(), ColorRange::Unknown);
    assert_eq!(picture.width(), 4);
    assert_eq!(picture.height(), 2);
}

#[test]
fn padded_yuy2_stride_ignores_row_padding() {
    let mut bytes = Vec::new();
    for _ in 0..2 {
        bytes.extend([16, 128, 16, 128]);
        bytes.extend([0xAA, 0xAA, 0xAA, 0xAA]);
    }
    let picture = yuy2_to_i420(PackedYuy2::new(
        &bytes,
        2,
        2,
        8,
        limited(ColorMatrix::Bt601),
    ))
    .expect("padded");
    assert_eq!(picture.y(), &[16, 16, 16, 16]);
    assert_eq!(picture.u(), &[128]);
    assert_eq!(picture.v(), &[128]);
    assert_eq!(picture.properties().matrix(), ColorMatrix::Bt601);
}

#[test]
fn neutral_gray_and_color_bars_follow_matrix_and_range() {
    let gray = solid_rgb(128, 128, 128, 2, 2);
    let limited_gray = rgb_to_yuy2(InterleavedRgb::new(
        &gray,
        2,
        2,
        6,
        InterleavedColor::Rgb,
        limited(ColorMatrix::Bt601),
    ))
    .expect("gray");
    let (y, u, v) = bt601_limited(128.0, 128.0, 128.0);
    near(limited_gray[0], y);
    near(limited_gray[1], u);
    near(limited_gray[2], y);
    near(limited_gray[3], v);
    near(limited_gray[1], 128.0);
    near(limited_gray[3], 128.0);

    let white = solid_rgb(255, 255, 255, 2, 2);
    let full = rgb_to_yuy2(InterleavedRgb::new(
        &white,
        2,
        2,
        6,
        InterleavedColor::Rgb,
        props(ColorMatrix::Bt601, ColorRange::Full),
    ))
    .expect("full white");
    let limited_white = rgb_to_yuy2(InterleavedRgb::new(
        &white,
        2,
        2,
        6,
        InterleavedColor::Rgb,
        limited(ColorMatrix::Bt601),
    ))
    .expect("limited white");
    let (full_y, _, _) = bt601_full(255.0, 255.0, 255.0);
    let (limited_white_y, _, _) = bt601_limited(255.0, 255.0, 255.0);
    near(full[0], full_y);
    near(limited_white[0], limited_white_y);
    assert!((i16::from(full[0]) - i16::from(limited_white[0])).abs() > TOLERANCE);

    let green = solid_rgb(0, 255, 0, 2, 1);
    let bt601 = rgb_to_yuy2(InterleavedRgb::new(
        &green,
        2,
        1,
        6,
        InterleavedColor::Rgb,
        limited(ColorMatrix::Bt601),
    ))
    .expect("601");
    let bt709 = rgb_to_yuy2(InterleavedRgb::new(
        &green,
        2,
        1,
        6,
        InterleavedColor::Rgb,
        limited(ColorMatrix::Bt709),
    ))
    .expect("709");
    let sample_601 = bt601_limited(0.0, 255.0, 0.0);
    near(bt601[0], sample_601.0);
    near(bt601[1], sample_601.1);
    near(bt601[3], sample_601.2);
    let sample_709 = bt709_limited(0.0, 255.0, 0.0);
    near(bt709[0], sample_709.0);
    near(bt709[1], sample_709.1);
    near(bt709[3], sample_709.2);
    assert!((i16::from(bt601[0]) - i16::from(bt709[0])).abs() > TOLERANCE);

    let red = [255, 0, 0, 255, 0, 0];
    let packed = rgb_to_yuy2(InterleavedRgb::new(
        &red,
        2,
        1,
        6,
        InterleavedColor::Rgb,
        limited(ColorMatrix::Bt601),
    ))
    .expect("red");
    let bgr = [0, 0, 255, 0, 0, 255];
    let packed_bgr = rgb_to_yuy2(InterleavedRgb::new(
        &bgr,
        2,
        1,
        6,
        InterleavedColor::Bgr,
        limited(ColorMatrix::Bt601),
    ))
    .expect("bgr red");
    let (red_y, red_u, red_v) = bt601_limited(255.0, 0.0, 0.0);
    near(packed[0], red_y);
    near(packed[1], red_u);
    near(packed[2], red_y);
    near(packed[3], red_v);
    assert_eq!(packed, packed_bgr);
    assert_eq!(packed.len(), 4);
}

#[test]
fn geometry_and_allocation_failures_precede_conversion() {
    let odd = PackedYuy2::new(&[0, 0], 1, 2, 2, limited(ColorMatrix::Bt709));
    assert_invalid(yuy2_to_i420(odd).expect_err("odd width"), "width");

    let odd_height = PackedYuy2::new(&[0, 128, 0, 128], 2, 1, 4, limited(ColorMatrix::Bt709));
    assert_invalid(yuy2_to_i420(odd_height).expect_err("odd height"), "height");

    let short = PackedYuy2::new(&[0, 128], 2, 2, 4, limited(ColorMatrix::Bt709));
    assert_invalid(yuy2_to_i420(short).expect_err("short"), "bytes");

    let overflow = PackedYuy2::new(
        &[0, 128, 0, 128],
        2,
        2,
        u32::MAX,
        limited(ColorMatrix::Bt709),
    );
    assert_invalid(yuy2_to_i420(overflow).expect_err("overflow"), "stride");

    let wide = PackedYuy2::new(&[], MAX_ENCODER_EDGE + 2, 2, 8, limited(ColorMatrix::Bt709));
    assert_invalid(yuy2_to_i420(wide).expect_err("edge"), "width");

    let unknown = InterleavedRgb::new(
        &[0, 0, 0, 0, 0, 0],
        2,
        1,
        6,
        InterleavedColor::Rgb,
        props(ColorMatrix::Unknown, ColorRange::Limited),
    );
    assert_invalid(rgb_to_yuy2(unknown).expect_err("matrix"), "matrix");

    let picture: Result<OwnedI420, _> =
        yuy2_to_i420(PackedYuy2::new(&[], 0, 2, 0, limited(ColorMatrix::Bt601)));
    assert_invalid(picture.expect_err("zero"), "width");
}
