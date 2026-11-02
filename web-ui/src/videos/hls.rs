//! HLS media-playlist parsing and init-segment codec selection.

use crate::videos::speed::safe_relative_asset;

#[derive(Clone, Debug, PartialEq)]
pub struct SegmentLine {
    pub uri: String,
    pub duration_secs: f64,
    pub program_date_time: Option<String>,
    pub discontinuity_before: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedPlaylist {
    pub map_uri: Option<String>,
    pub segments: Vec<SegmentLine>,
    pub end_list: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecMime {
    Avc1 { codecs: String },
    Hvc1 { codecs: String },
}

impl CodecMime {
    pub fn as_source_buffer_type(&self) -> String {
        match self {
            Self::Avc1 { codecs } | Self::Hvc1 { codecs } => {
                format!(r#"video/mp4; codecs="{codecs}""#)
            }
        }
    }

    pub fn mime_candidates(&self) -> Vec<String> {
        let mut out = match self {
            Self::Avc1 { .. } => vec![
                self.as_source_buffer_type(),
                r#"video/mp4; codecs="avc1.42E01E""#.to_owned(),
                r#"video/mp4; codecs="avc1.42001E""#.to_owned(),
                r#"video/mp4; codecs="avc1.4D401E""#.to_owned(),
            ],
            Self::Hvc1 { .. } => vec![
                self.as_source_buffer_type(),
                r#"video/mp4; codecs="hvc1.1.6.L93.B0""#.to_owned(),
            ],
        };
        out.dedup();
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecProbe {
    Supported(CodecMime),
    Unsupported,
}

pub fn parse_playlist(text: &str) -> Option<ParsedPlaylist> {
    let mut map_uri = None;
    let mut segments = Vec::new();
    let mut end_list = false;
    let mut pending_pdt = None;
    let mut pending_duration = None;
    let mut next_discontinuity = false;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line == "#EXT-X-ENDLIST" {
            end_list = true;
            continue;
        }
        if line == "#EXT-X-DISCONTINUITY" {
            next_discontinuity = true;
            continue;
        }
        if let Some(uri) = line
            .strip_prefix("#EXT-X-MAP:URI=\"")
            .and_then(|rest| rest.strip_suffix('"'))
        {
            if safe_relative_asset(uri) {
                map_uri = Some(uri.to_owned());
            }
            continue;
        }
        if let Some(stamp) = line.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:") {
            pending_pdt = Some(stamp.to_owned());
            continue;
        }
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            let number = info.split(',').next()?.trim();
            pending_duration = number.parse().ok();
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if !safe_relative_asset(line) {
            return None;
        }
        segments.push(SegmentLine {
            uri: line.to_owned(),
            duration_secs: pending_duration.take().unwrap_or(0.0),
            program_date_time: pending_pdt.take(),
            discontinuity_before: next_discontinuity,
        });
        next_discontinuity = false;
    }
    Some(ParsedPlaylist {
        map_uri,
        segments,
        end_list,
    })
}

pub fn gateway_dir(path: &str) -> &str {
    path.rfind('/').map_or(path, |index| &path[..=index])
}

pub fn join_gateway_asset(base: &str, asset: &str) -> Option<String> {
    if !safe_relative_asset(asset) {
        return None;
    }
    let trimmed = base.trim_end_matches('/');
    Some(format!("{trimmed}/{asset}"))
}

pub fn resolve_playlist_asset(playlist_path: &str, relative: &str) -> Option<String> {
    if !safe_relative_asset(relative) {
        return None;
    }
    Some(format!("{}{relative}", gateway_dir(playlist_path)))
}

pub fn probe_codec_from_init(init: &[u8]) -> CodecProbe {
    if let Some(mime) = avc1_mime(init) {
        return CodecProbe::Supported(mime);
    }
    if let Some(mime) = hvc1_mime(init) {
        return CodecProbe::Supported(mime);
    }
    CodecProbe::Unsupported
}

fn avc1_mime(init: &[u8]) -> Option<CodecMime> {
    let avcc = find_box_payload(init, b"avcC")?;
    if avcc.len() < 4 {
        return None;
    }
    let profile = avcc[1];
    let compat = avcc[2];
    let level = avcc[3];
    Some(CodecMime::Avc1 {
        codecs: format!("avc1.{profile:02X}{compat:02X}{level:02X}"),
    })
}

fn hvc1_mime(init: &[u8]) -> Option<CodecMime> {
    let hvcc = find_box_payload(init, b"hvcC")?;
    if hvcc.len() < 4 {
        return None;
    }
    let profile = hvcc[1];
    let compat = (hvcc[2] >> 4) & 0x0F;
    let level = hvcc[12];
    Some(CodecMime::Hvc1 {
        codecs: format!("hvc1.{profile}.{compat}.{level:02X}.B0"),
    })
}

fn find_box_payload<'a>(bytes: &'a [u8], target: &[u8; 4]) -> Option<&'a [u8]> {
    let mut offset = 0_usize;
    while offset + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().ok()?) as usize;
        if size < 8 {
            offset += 8;
            continue;
        }
        let Some(end) = offset.checked_add(size) else {
            offset += 8;
            continue;
        };
        if end > bytes.len() {
            break;
        }
        let kind = &bytes[offset + 4..offset + 8];
        let payload = &bytes[offset + 8..end];
        if kind == target {
            return Some(payload);
        }
        if let Some(found) = find_box_payload(payload, target) {
            return Some(found);
        }
        offset = end;
    }
    scan_box_payload(bytes, target)
}

fn scan_box_payload<'a>(bytes: &'a [u8], target: &[u8; 4]) -> Option<&'a [u8]> {
    bytes.iter().enumerate().find_map(|(offset, _)| {
        if offset + 8 > bytes.len() || &bytes[offset + 4..offset + 8] != target {
            return None;
        }
        let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().ok()?) as usize;
        if size < 8 {
            return None;
        }
        let end = offset.checked_add(size)?;
        (end <= bytes.len()).then(|| &bytes[offset + 8..end])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_marks_leading_middle_consecutive_and_live_edge_gaps() {
        let clip = r#"#EXTM3U
#EXT-X-MAP:URI="init.mp4"
#EXT-X-DISCONTINUITY
#EXT-X-PROGRAM-DATE-TIME:2024-01-01T00:00:10.000000Z
#EXTINF:1.000,
lead.m4s
#EXT-X-PROGRAM-DATE-TIME:2024-01-01T00:00:11.000000Z
#EXTINF:1.000,
mid-a.m4s
#EXT-X-DISCONTINUITY
#EXT-X-PROGRAM-DATE-TIME:2024-01-01T00:00:20.000000Z
#EXTINF:1.000,
mid-b.m4s
#EXT-X-DISCONTINUITY
#EXT-X-PROGRAM-DATE-TIME:2024-01-01T00:00:30.000000Z
#EXTINF:1.000,
consec-a.m4s
#EXT-X-DISCONTINUITY
#EXT-X-PROGRAM-DATE-TIME:2024-01-01T00:00:40.000000Z
#EXTINF:1.000,
consec-b.m4s
#EXT-X-ENDLIST
"#;
        let parsed = parse_playlist(clip).unwrap();
        let skips: Vec<_> = parsed
            .segments
            .iter()
            .filter(|segment| segment.discontinuity_before)
            .map(|segment| segment.uri.as_str())
            .collect();
        assert_eq!(
            skips,
            ["lead.m4s", "mid-b.m4s", "consec-a.m4s", "consec-b.m4s"]
        );
        let window = parse_playlist(
            r#"#EXTM3U
#EXT-X-MAP:URI="init.mp4"
#EXT-X-PROGRAM-DATE-TIME:2024-01-01T00:02:00.000000Z
#EXTINF:1.000,
window.m4s
#EXT-X-ENDLIST
"#,
        )
        .unwrap();
        assert!(!window.segments[0].discontinuity_before);
        let live_first = parse_playlist(
            r#"#EXTM3U
#EXT-X-MAP:URI="init.mp4"
#EXTINF:1.000,
live-0.m4s
"#,
        )
        .unwrap();
        assert!(!live_first.segments[0].discontinuity_before);
        let live_edge = parse_playlist(
            r#"#EXTM3U
#EXT-X-MAP:URI="init.mp4"
#EXTINF:1.000,
live-0.m4s
#EXT-X-DISCONTINUITY
#EXTINF:1.000,
live-1.m4s
"#,
        )
        .unwrap();
        assert!(live_edge.segments[1].discontinuity_before);
        assert!(!live_edge.segments[0].discontinuity_before);
    }

    fn parser_reads_map_segments_and_endlist() {
        let text = r#"#EXTM3U
#EXT-X-VERSION:7
#EXT-X-MAP:URI="init.mp4"
#EXT-X-PROGRAM-DATE-TIME:2024-01-01T00:00:00.000000Z
#EXTINF:1.000,
seg-0000.m4s
#EXT-X-ENDLIST
"#;
        let parsed = parse_playlist(text).unwrap();
        assert_eq!(parsed.map_uri.as_deref(), Some("init.mp4"));
        assert!(parsed.end_list);
        assert_eq!(parsed.segments.len(), 1);
        assert_eq!(parsed.segments[0].uri, "seg-0000.m4s");
        assert_eq!(
            parsed.segments[0].program_date_time.as_deref(),
            Some("2024-01-01T00:00:00.000000Z")
        );
    }

    #[test]
    fn join_and_resolve_stay_under_gateway_roots() {
        let playlist = "/media/video-live/live-1/index.m3u8";
        assert_eq!(
            resolve_playlist_asset(playlist, "seg-0001.m4s").as_deref(),
            Some("/media/video-live/live-1/seg-0001.m4s")
        );
        assert!(resolve_playlist_asset(playlist, "../secret.m3u8").is_none());
    }

    #[test]
    fn init_probe_selects_avc1_from_avcc() {
        let avcc = [1_u8, 0x42, 0x00, 0x1E, 0xFF];
        let mut init = Vec::new();
        init.extend_from_slice(&(8_u32 + avcc.len() as u32).to_be_bytes());
        init.extend_from_slice(b"avcC");
        init.extend_from_slice(&avcc);
        match probe_codec_from_init(&init) {
            CodecProbe::Supported(CodecMime::Avc1 { codecs }) => {
                assert_eq!(codecs, "avc1.42001E");
            }
            other => panic!("expected avc1 probe, got {other:?}"),
        }
        assert_eq!(probe_codec_from_init(b"ftyp"), CodecProbe::Unsupported);
    }

    #[test]
    fn init_probe_finds_avcc_in_visual_sample_entry() {
        let mut sample = vec![0_u8; 78];
        sample.extend(full_box(
            b"avcC",
            &[1, 0x42, 0x00, 0x1E, 0xFC | 3, 0xE1, 1, 0x67, 0],
        ));
        let init = full_box(b"avc1", &sample);
        match probe_codec_from_init(&init) {
            CodecProbe::Supported(CodecMime::Avc1 { codecs }) => {
                assert_eq!(codecs, "avc1.42001E");
            }
            other => panic!("expected supported avc1, got {other:?}"),
        }
    }

    fn full_box(fourcc: &[u8; 4], content: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + content.len());
        out.extend_from_slice(&(u32::try_from(8 + content.len()).unwrap()).to_be_bytes());
        out.extend_from_slice(fourcc);
        out.extend_from_slice(content);
        out
    }
}
