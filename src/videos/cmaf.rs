//! Minimal ISOBMFF boxes for CMAF init and media segments.

use crate::error::A2aLabError;
use crate::time::{TimeRange, UtcTimestamp};
use crate::videos::codec::VideoCodec;
use crate::videos::dvr_store::{DvrMediaPayload, DvrMediaSegment, DvrMediaTrack};

const MOVIE_TIMESCALE: u32 = 1_000_000;

pub(crate) fn init_segment(track: &DvrMediaTrack) -> Result<Vec<u8>, A2aLabError> {
    let codec = track.codec();
    match codec {
        VideoCodec::Mjpeg => Err(A2aLabError::invalid(
            "codec",
            "MJPEG is not packaged as CMAF",
        )),
        VideoCodec::H264 | VideoCodec::H265 => {
            if track.codec_initialization().is_empty() {
                return Err(A2aLabError::invalid(
                    "codec_initialization",
                    "must not be empty for encoded CMAF packaging",
                ));
            }
            if track.codec_initialization().starts_with(&[0xFF, 0xD8]) {
                return Err(A2aLabError::invalid(
                    "codec_initialization",
                    "must not be JPEG for encoded CMAF packaging",
                ));
            }
            Ok(wrap_boxes(&[ftyp(), moov_init(track)?]))
        }
    }
}

pub(crate) fn media_segment(
    track: &DvrMediaTrack,
    segment: &DvrMediaSegment,
    sequence: u32,
    decode_time_us: u64,
) -> Result<Vec<u8>, A2aLabError> {
    let (codec, sample) = match segment.payload() {
        DvrMediaPayload::MjpegFrame(_) => {
            return Err(A2aLabError::invalid(
                "payload",
                "MJPEG frames are not packaged as CMAF media segments",
            ));
        }
        DvrMediaPayload::EncodedSample { codec, bytes } => (*codec, bytes.as_slice()),
    };
    if codec != track.codec() {
        return Err(A2aLabError::invalid(
            "payload",
            "segment codec must match the track codec",
        ));
    }
    let duration_us = segment_duration_us(segment.range());
    let moof = moof_fragment(sequence, decode_time_us, duration_us, sample.len());
    let mdat = mdat_box(sample);
    Ok(wrap_boxes(&[styp(), moof, mdat]))
}

pub(crate) fn fragmented_mp4_export(
    track: &DvrMediaTrack,
    segments: &[DvrMediaSegment],
) -> Result<Vec<u8>, A2aLabError> {
    let mut out = init_segment(track)?;
    let mut decode_time_us = 0_u64;
    for (index, segment) in segments.iter().enumerate() {
        let fragment = media_segment(
            track,
            segment,
            u32::try_from(index).unwrap_or(0),
            decode_time_us,
        )?;
        out.extend_from_slice(&fragment);
        decode_time_us = decode_time_us.saturating_add(segment_duration_us(segment.range()));
    }
    Ok(out)
}

pub(crate) fn segment_duration_us(range: TimeRange) -> u64 {
    u64::try_from(
        range
            .end()
            .as_micros()
            .saturating_sub(range.start().as_micros()),
    )
    .unwrap_or(0)
}

fn wrap_boxes(boxes: &[Vec<u8>]) -> Vec<u8> {
    boxes.iter().flat_map(|item| item.iter().copied()).collect()
}

fn ftyp() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"iso6");
    body.extend_from_slice(&0_u32.to_be_bytes());
    for brand in [b"iso6", b"cmfc", b"mp41"] {
        body.extend_from_slice(brand);
    }
    full_box(b"ftyp", &body)
}

fn styp() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"iso6");
    body.extend_from_slice(&0_u32.to_be_bytes());
    for brand in [b"iso6", b"cmfc", b"mp41"] {
        body.extend_from_slice(brand);
    }
    full_box(b"styp", &body)
}

fn moov_init(track_meta: &DvrMediaTrack) -> Result<Vec<u8>, A2aLabError> {
    let stsd = stsd_entry(track_meta)?;
    let mut trak_body = Vec::new();
    trak_body.extend(tkhd());
    trak_body.extend(mdia(&stsd));
    let mut moov = Vec::new();
    moov.extend(mvhd());
    moov.extend(full_box(b"trak", &trak_body));
    moov.extend(mvex());
    Ok(full_box(b"moov", &moov))
}

fn mvhd() -> Vec<u8> {
    let mut content = vec![0_u8; 4];
    content.extend_from_slice(&0_u32.to_be_bytes());
    content.extend_from_slice(&0_u32.to_be_bytes());
    content.extend_from_slice(&MOVIE_TIMESCALE.to_be_bytes());
    content.extend_from_slice(&0_u32.to_be_bytes());
    content.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    content.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    content.extend_from_slice(&2_u32.to_be_bytes());
    content.extend_from_slice(&0_u32.to_be_bytes());
    full_box(b"mvhd", &content)
}

fn tkhd() -> Vec<u8> {
    let mut content = vec![0_u8; 4];
    content.extend_from_slice(&0_u32.to_be_bytes());
    content.extend_from_slice(&0_u32.to_be_bytes());
    content.extend_from_slice(&1_u32.to_be_bytes());
    content.extend_from_slice(&0_u32.to_be_bytes());
    content.extend_from_slice(&0_u32.to_be_bytes());
    content.extend_from_slice(&0_u32.to_be_bytes());
    content.extend_from_slice(&0_u16.to_be_bytes());
    content.extend_from_slice(&0_u16.to_be_bytes());
    content.extend_from_slice(&0_u16.to_be_bytes());
    content.extend_from_slice(&0_u16.to_be_bytes());
    content.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    content.extend_from_slice(&640_u32.to_be_bytes());
    content.extend_from_slice(&480_u32.to_be_bytes());
    full_box(b"tkhd", &content)
}

fn mdia(stsd: &[u8]) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend(mdhd());
    content.extend(hdlr());
    content.extend(minf(stsd));
    full_box(b"mdia", &content)
}

fn mdhd() -> Vec<u8> {
    let mut content = vec![0_u8; 4];
    content.extend_from_slice(&0_u32.to_be_bytes());
    content.extend_from_slice(&0_u32.to_be_bytes());
    content.extend_from_slice(&MOVIE_TIMESCALE.to_be_bytes());
    content.extend_from_slice(&0_u32.to_be_bytes());
    content.extend_from_slice(&0x55C4_u32.to_be_bytes());
    content.extend_from_slice(&0_u32.to_be_bytes());
    full_box(b"mdhd", &content)
}

fn hdlr() -> Vec<u8> {
    let mut content = vec![0_u8; 4];
    content.extend_from_slice(&0_u32.to_be_bytes());
    content.extend_from_slice(b"vide");
    content.extend_from_slice(&[0; 12]);
    content.extend_from_slice(b"VideoHandler");
    content.push(0);
    full_box(b"hdlr", &content)
}

fn minf(stsd: &[u8]) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend(full_box(b"vmhd", &[0; 8]));
    let dref = full_box(b"url ", &[0; 4]);
    content.extend(full_box(
        b"dinf",
        &full_box(b"dref", &{
            let mut body = 0_u32.to_be_bytes().to_vec();
            body.extend_from_slice(&1_u32.to_be_bytes());
            body.extend(dref);
            body
        }),
    ));
    content.extend(stbl(stsd));
    full_box(b"minf", &content)
}

fn stbl(stsd: &[u8]) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend(full_box(b"stsd", &{
        let mut body = 0_u32.to_be_bytes().to_vec();
        body.extend_from_slice(&1_u32.to_be_bytes());
        body.extend_from_slice(stsd);
        body
    }));
    content.extend(full_box(b"stts", &0_u32.to_be_bytes()));
    content.extend(full_box(b"stsc", &0_u32.to_be_bytes()));
    content.extend(full_box(b"stsz", &{
        let mut body = 0_u32.to_be_bytes().to_vec();
        body.extend_from_slice(&0_u32.to_be_bytes());
        body.extend_from_slice(&0_u32.to_be_bytes());
        body
    }));
    content.extend(full_box(b"stco", &0_u32.to_be_bytes()));
    full_box(b"stbl", &content)
}

fn stsd_entry(track: &DvrMediaTrack) -> Result<Vec<u8>, A2aLabError> {
    codec_sample_entry(track)
}

fn codec_sample_entry(track: &DvrMediaTrack) -> Result<Vec<u8>, A2aLabError> {
    let init = track.codec_initialization();
    match track.codec() {
        VideoCodec::H264 => {
            let avcc = annex_b_to_avcc(init)?;
            let mut sample = Vec::new();
            sample.extend_from_slice(&[0; 6]);
            sample.extend_from_slice(&1_u16.to_be_bytes());
            sample.extend_from_slice(b"avc1");
            sample.extend_from_slice(&[0; 16]);
            sample.extend_from_slice(&640_u16.to_be_bytes());
            sample.extend_from_slice(&480_u16.to_be_bytes());
            sample.extend_from_slice(&[0; 14]);
            sample.extend_from_slice(&24_u16.to_be_bytes());
            sample.extend_from_slice(&[0xFF, 0xFF]);
            sample.extend(full_box(b"avcC", &avcc));
            Ok(full_box(b"avc1", &sample))
        }
        VideoCodec::H265 => {
            let hvcc = annex_b_to_hvcc(init)?;
            let mut sample = Vec::new();
            sample.extend_from_slice(&[0; 6]);
            sample.extend_from_slice(&1_u16.to_be_bytes());
            sample.extend_from_slice(b"hvc1");
            sample.extend_from_slice(&[0; 16]);
            sample.extend_from_slice(&640_u16.to_be_bytes());
            sample.extend_from_slice(&480_u16.to_be_bytes());
            sample.extend_from_slice(&[0; 14]);
            sample.extend_from_slice(&24_u16.to_be_bytes());
            sample.extend_from_slice(&[0xFF, 0xFF]);
            sample.extend(full_box(b"hvcC", &hvcc));
            Ok(full_box(b"hvc1", &sample))
        }
        VideoCodec::Mjpeg => Err(A2aLabError::invalid(
            "codec",
            "MJPEG is not packaged as CMAF",
        )),
    }
}

fn annex_b_to_avcc(init: &[u8]) -> Result<Vec<u8>, A2aLabError> {
    let nals = annex_b_nals(init)?;
    let sps = nals
        .iter()
        .find(|nal| nal.first().is_some_and(|header| header & 0x1F == 7))
        .ok_or_else(|| {
            A2aLabError::invalid(
                "codec_initialization",
                "Annex B init must include an SPS NAL",
            )
        })?;
    let pps_nals: Vec<_> = nals
        .iter()
        .filter(|nal| nal.first().is_some_and(|header| header & 0x1F == 8))
        .collect();
    let profile = sps.get(1).copied().unwrap_or(0x42);
    let compat = sps.get(2).copied().unwrap_or(0);
    let level = sps.get(3).copied().unwrap_or(0x1E);
    let mut avcc = vec![1, profile, compat, level, 0xFC | 3];
    avcc.push(0xE1);
    avcc.push(u8::try_from(sps.len()).map_err(|_| {
        A2aLabError::invalid("codec_initialization", "SPS NAL is too large for avcC")
    })?);
    avcc.extend_from_slice(sps);
    avcc.push(
        u8::try_from(pps_nals.len()).map_err(|_| {
            A2aLabError::invalid("codec_initialization", "too many PPS NALs for avcC")
        })?,
    );
    for pps in pps_nals {
        avcc.push(u8::try_from(pps.len()).map_err(|_| {
            A2aLabError::invalid("codec_initialization", "PPS NAL is too large for avcC")
        })?);
        avcc.extend_from_slice(pps);
    }
    Ok(avcc)
}

fn annex_b_nals(init: &[u8]) -> Result<Vec<Vec<u8>>, A2aLabError> {
    let mut nals = Vec::new();
    let mut index = 0_usize;
    while index < init.len() {
        let Some((start, advance)) = annex_b_start_code(&init[index..]) else {
            index += 1;
            continue;
        };
        index += advance;
        let payload_start = index + start;
        index = payload_start;
        while index < init.len() && annex_b_start_code(&init[index..]).is_none() {
            index += 1;
        }
        if payload_start < index {
            nals.push(init[payload_start..index].to_vec());
        }
    }
    if nals.is_empty() {
        return Err(A2aLabError::invalid(
            "codec_initialization",
            "must be Annex B H.264/H.265 initialization",
        ));
    }
    Ok(nals)
}

fn annex_b_start_code(bytes: &[u8]) -> Option<(usize, usize)> {
    if bytes.len() >= 4 && bytes[..4] == [0, 0, 0, 1] {
        Some((0, 4))
    } else if bytes.len() >= 3 && bytes[..3] == [0, 0, 1] {
        Some((0, 3))
    } else {
        None
    }
}

fn annex_b_to_hvcc(init: &[u8]) -> Result<Vec<u8>, A2aLabError> {
    let nal = annex_b_nal(init)?;
    let mut hvcc = vec![
        1,
        nal.get(1).copied().unwrap_or(1),
        nal.get(2).copied().unwrap_or(0x60),
        nal.get(3).copied().unwrap_or(0),
        0xFC | 0xF,
        0xF0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    hvcc[22] = 1;
    hvcc.push(0x20);
    hvcc.extend_from_slice(&[0, 1]);
    hvcc.extend_from_slice(
        &(u16::try_from(nal.len()).map_err(|_| {
            A2aLabError::invalid("codec_initialization", "VPS/SPS NAL is too large for hvcC")
        })?)
        .to_be_bytes(),
    );
    hvcc.extend_from_slice(nal);
    Ok(hvcc)
}

fn annex_b_nal(init: &[u8]) -> Result<&[u8], A2aLabError> {
    if init.len() >= 4 && init[..4] == [0, 0, 0, 1] {
        Ok(&init[4..])
    } else if init.len() >= 3 && init[..3] == [0, 0, 1] {
        Ok(&init[3..])
    } else {
        Err(A2aLabError::invalid(
            "codec_initialization",
            "must be Annex B H.264/H.265 initialization",
        ))
    }
}

fn mvex() -> Vec<u8> {
    let mut trex = vec![0_u8; 4];
    trex.extend_from_slice(&1_u32.to_be_bytes());
    trex.extend_from_slice(&1_u32.to_be_bytes());
    trex.extend_from_slice(&0_u32.to_be_bytes());
    trex.extend_from_slice(&0_u32.to_be_bytes());
    trex.extend_from_slice(&0_u32.to_be_bytes());
    trex.extend_from_slice(&0_u32.to_be_bytes());
    full_box(b"mvex", &full_box(b"trex", &trex))
}

fn moof_fragment(
    sequence: u32,
    decode_time_us: u64,
    duration_us: u64,
    sample_size: usize,
) -> Vec<u8> {
    let sample_duration = u32::try_from(duration_us).unwrap_or(1);
    let sample_size_u32 = u32::try_from(sample_size).unwrap_or(u32::MAX);
    let mut mfhd = vec![0_u8; 4];
    mfhd.extend_from_slice(&sequence.to_be_bytes());
    let tfdt = full_box(b"tfdt", &{
        let mut body = vec![1_u8];
        body.extend_from_slice(&[0; 3]);
        body.extend_from_slice(&decode_time_us.to_be_bytes());
        body
    });
    let placeholder = assemble_moof(&mfhd, &tfdt, 8, sample_duration, sample_size_u32);
    let data_offset = u32::try_from(placeholder.len().saturating_add(8)).unwrap_or(u32::MAX);
    assemble_moof(&mfhd, &tfdt, data_offset, sample_duration, sample_size_u32)
}

fn assemble_moof(
    mfhd: &[u8],
    tfdt: &[u8],
    data_offset: u32,
    sample_duration: u32,
    sample_size: u32,
) -> Vec<u8> {
    let mut trun = vec![0_u8; 4];
    trun.extend_from_slice(&0x0000_0301_u32.to_be_bytes());
    trun.extend_from_slice(&1_u32.to_be_bytes());
    trun.extend_from_slice(&data_offset.to_be_bytes());
    trun.extend_from_slice(&sample_duration.to_be_bytes());
    trun.extend_from_slice(&sample_size.to_be_bytes());
    let mut tfhd = vec![0_u8; 4];
    tfhd.extend_from_slice(&0x0002_0000_u32.to_be_bytes());
    tfhd.extend_from_slice(&1_u32.to_be_bytes());
    let mut traf = Vec::new();
    traf.extend(full_box(b"tfhd", &tfhd));
    traf.extend_from_slice(tfdt);
    traf.extend(full_box(b"trun", &trun));
    let mut moof = Vec::new();
    moof.extend(full_box(b"mfhd", mfhd));
    moof.extend(full_box(b"traf", &traf));
    full_box(b"moof", &moof)
}

fn mdat_box(sample: &[u8]) -> Vec<u8> {
    full_box(b"mdat", sample)
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn full_box(fourcc: &[u8; 4], content: &[u8]) -> Vec<u8> {
    let size = u32::try_from(8 + content.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(fourcc);
    out.extend_from_slice(content);
    out
}

pub(crate) fn program_date_time(at: UtcTimestamp) -> String {
    at.to_rfc3339()
}
