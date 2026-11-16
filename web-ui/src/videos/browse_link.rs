//! Videos browse deep-links: one configured source and one UTC capture instant.

use super::mjpeg::captured_us;
use super::track::{self, PLAYBACK_WINDOW_US, Window};

pub const BROWSE_HREF: &str = "/ui/videos";
pub const SOURCE_PARAM: &str = "source_id";
pub const SOURCE_PARAM_ALIAS: &str = "source";
pub const AT_PARAM: &str = "at";
pub const AUTOPLAY_PARAM: &str = "autoplay";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideosPanel {
    Browse,
    Search,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowseFrame {
    pub source_id: String,
    pub captured_at: String,
    pub captured_us: i64,
    pub autoplay: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowseApply {
    Unchanged,
    Seek(BrowseFrame),
}

pub fn route_path(value: &str) -> &str {
    let trimmed = value.trim().trim_start_matches('#');
    let trimmed = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
    let trimmed = trimmed.trim_matches('/');
    let trimmed = trimmed.strip_prefix("ui/").unwrap_or(trimmed);
    if trimmed == "ui" { "" } else { trimmed }
}

pub fn videos_panel(path: &str) -> VideosPanel {
    match route_path(path) {
        "videos/search" | "inventory/videos/search" => VideosPanel::Search,
        "videos" | "videos/browse" | "inventory/videos" | "inventory/videos/browse" => {
            VideosPanel::Browse
        }
        _ => VideosPanel::Other,
    }
}

pub fn encode_browse_query(source_id: &str, captured_at: &str, autoplay: bool) -> String {
    let mut query = String::new();
    query.push_str(SOURCE_PARAM);
    query.push('=');
    push_encoded(&mut query, source_id);
    query.push('&');
    query.push_str(AT_PARAM);
    query.push('=');
    push_encoded(&mut query, captured_at);
    query.push('&');
    query.push_str(AUTOPLAY_PARAM);
    query.push('=');
    query.push_str(if autoplay { "true" } else { "false" });
    query
}

pub fn encode_browse_href(source_id: &str, captured_at_us: i64, autoplay: bool) -> String {
    format!(
        "{BROWSE_HREF}?{}",
        encode_browse_query(source_id, &track::format_utc(captured_at_us), autoplay)
    )
}

pub fn encode_frame_href(source_id: &str, captured_at: &str, autoplay: bool) -> Option<String> {
    if source_id.is_empty() {
        return None;
    }
    Some(encode_browse_href(
        source_id,
        captured_us(captured_at)?,
        autoplay,
    ))
}

pub fn decode_browse_href(href: &str) -> Option<BrowseFrame> {
    decode_browse_query(query_of(href).unwrap_or_default())
}

pub fn decode_browse_query(query: &str) -> Option<BrowseFrame> {
    let query = query.strip_prefix('?').unwrap_or(query);
    if query.is_empty() {
        return None;
    }
    let mut source_id = None;
    let mut source_alias = None;
    let mut at = None;
    let mut autoplay = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(key)?;
        let value = percent_decode(value)?;
        match key.as_str() {
            SOURCE_PARAM => source_id = Some(value),
            SOURCE_PARAM_ALIAS => source_alias = Some(value),
            AT_PARAM => at = Some(value),
            AUTOPLAY_PARAM => {
                autoplay = match value.as_str() {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => None,
                }
            }
            _ => {}
        }
    }
    let source_id = source_id.or(source_alias).filter(|id| !id.is_empty())?;
    let captured_at = at.filter(|value| !value.is_empty())?;
    let captured_us = captured_us(&captured_at)?;
    let autoplay = autoplay?;
    Some(BrowseFrame {
        source_id,
        captured_at,
        captured_us,
        autoplay,
    })
}

pub fn apply_browse_query(catalog: &[String], query: &str) -> BrowseApply {
    let Some(frame) = decode_browse_query(query) else {
        return BrowseApply::Unchanged;
    };
    if !catalog.iter().any(|id| id == &frame.source_id) {
        return BrowseApply::Unchanged;
    }
    BrowseApply::Seek(frame)
}

pub fn session_window_for(track: &track::Track, at_us: i64) -> Option<Window> {
    track::session_at(track, at_us).filter(|window| {
        window.duration_us() <= PLAYBACK_WINDOW_US
            && at_us >= window.start_us
            && at_us < window.end_us
    })
}

pub fn query_of(href: &str) -> Option<&str> {
    let href = href.split_once('#').map(|(path, _)| path).unwrap_or(href);
    href.split_once('?').map(|(_, query)| query)
}

fn push_encoded(out: &mut String, value: &str) {
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
}

fn percent_decode(value: &str) -> Option<String> {
    let mut out = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let hi = *bytes.get(index + 1)?;
                let lo = *bytes.get(index + 2)?;
                out.push((from_hex(hi)? << 4) | from_hex(lo)?);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::videos::Timeline;
    use crate::videos::domain::Interval;
    use crate::videos::track::coverage_track;

    const AT: &str = "2024-01-01T00:00:05Z";
    const AT_US: i64 = 1_704_067_205_000_000;

    #[test]
    fn round_trips_source_and_utc_instant() {
        let href = encode_browse_href("cam-a", AT_US, true);
        assert!(href.starts_with("/ui/videos?"));
        assert!(!href.contains("/search"));
        let frame = decode_browse_href(&href).unwrap();
        assert_eq!(frame.source_id, "cam-a");
        assert_eq!(frame.captured_at, AT);
        assert_eq!(frame.captured_us, AT_US);
        let decoded = decode_browse_query(&encode_browse_query("cam-a", AT, true)).unwrap();
        assert_eq!(decoded, frame);
        let aliased =
            decode_browse_query("source=cam-a&at=2024-01-01T00:00:05Z&autoplay=false").unwrap();
        assert_eq!(aliased.source_id, "cam-a");
        assert_eq!(aliased.captured_us, AT_US);
        assert!(!aliased.autoplay);
    }

    #[test]
    fn missing_params_and_invalid_timestamps_fail_closed() {
        assert!(decode_browse_query("").is_none());
        assert!(decode_browse_query("source_id=cam").is_none());
        assert!(decode_browse_query("at=2024-01-01T00:00:00Z").is_none());
        assert!(decode_browse_query("source_id=cam&at=").is_none());
        assert!(decode_browse_query("source_id=&at=2024-01-01T00:00:00Z").is_none());
        assert!(decode_browse_query("source_id=cam&at=not-a-time").is_none());
        assert!(decode_browse_query("source_id=cam&at=2024-01-01T00:00:00").is_none());
        assert!(decode_browse_query("source_id=cam&at=2024-01-01 00:00:00Z").is_none());
        assert!(decode_browse_query("source_id=cam&at=2024-01-01T00:00:00Z").is_none());
        assert!(
            decode_browse_query("source_id=cam&at=2024-01-01T00:00:00Z&autoplay=maybe").is_none()
        );
        assert_eq!(
            apply_browse_query(&["cam".into()], "source_id=cam&at=nope"),
            BrowseApply::Unchanged
        );
    }

    #[test]
    fn unknown_source_leaves_selection_unchanged() {
        let query = encode_browse_query("missing", AT, true);
        assert_eq!(
            apply_browse_query(&["cam".into(), "door".into()], &query),
            BrowseApply::Unchanged
        );
        let applied = apply_browse_query(&["cam".into()], &query);
        assert!(matches!(applied, BrowseApply::Unchanged));
    }

    #[test]
    fn valid_location_selects_source_and_bounded_session() {
        let query = encode_browse_query("door", AT, true);
        let BrowseApply::Seek(frame) = apply_browse_query(&["cam".into(), "door".into()], &query)
        else {
            panic!("expected seek");
        };
        assert_eq!(frame.source_id, "door");
        assert_eq!(frame.captured_us, AT_US);
        let timeline = Timeline {
            source_id: "door".into(),
            intervals: vec![Interval {
                start: "2024-01-01T00:00:00Z".into(),
                end: "2024-01-01T00:03:00Z".into(),
                start_us: 1_704_067_200_000_000,
                end_us: 1_704_067_380_000_000,
            }],
            newest_at: None,
            newest_us: None,
            live_available: false,
        };
        let window = session_window_for(&coverage_track(&timeline), frame.captured_us).unwrap();
        assert!(window.duration_us() <= PLAYBACK_WINDOW_US);
        assert!(frame.captured_us >= window.start_us && frame.captured_us < window.end_us);
    }

    #[test]
    fn path_compatibility_ignores_query_and_keeps_aliases() {
        assert_eq!(route_path("/ui/videos"), "videos");
        assert_eq!(
            route_path("/ui/videos?source_id=cam&at=2024-01-01T00:00:00Z"),
            "videos"
        );
        assert_eq!(route_path("/ui/videos/browse"), "videos/browse");
        assert_eq!(route_path("/ui/inventory/videos"), "inventory/videos");
        assert_eq!(
            route_path("inventory/videos/browse"),
            "inventory/videos/browse"
        );
        assert_eq!(route_path("/ui/videos/search"), "videos/search");
        assert_eq!(
            route_path("/ui/inventory/videos/search?q=motion"),
            "inventory/videos/search"
        );
        for browse in [
            "/ui/videos",
            "/ui/videos?source_id=cam&at=2024-01-01T00:00:00Z",
            "/ui/videos/browse",
            "/ui/inventory/videos?source_id=cam&at=2024-01-01T00:00:00Z",
            "inventory/videos/browse",
            "#/ui/videos",
        ] {
            assert_eq!(videos_panel(browse), VideosPanel::Browse, "{browse}");
        }
        for search in [
            "/ui/videos/search",
            "/ui/videos/search?q=motion",
            "/ui/inventory/videos/search",
            "inventory/videos/search?source_id=cam&at=2024-01-01T00:00:00Z",
        ] {
            assert_eq!(videos_panel(search), VideosPanel::Search, "{search}");
        }
        let href = encode_browse_href("cam", AT_US, true);
        assert_eq!(videos_panel(&href), VideosPanel::Browse);
        assert!(!href.contains("/search"));
    }

    #[test]
    fn playhead_href_matches_encode_and_rejects_empty_invalid() {
        assert_eq!(
            encode_frame_href("cam-a", AT, true).as_deref(),
            Some(encode_browse_href("cam-a", AT_US, true).as_str())
        );
        assert!(encode_frame_href("", AT, true).is_none());
        assert!(encode_frame_href("cam-a", "", true).is_none());
        assert!(encode_frame_href("cam-a", "not-a-time", true).is_none());
        assert!(encode_frame_href("cam-a", "2024-01-01T00:00:00", true).is_none());
        assert!(
            encode_frame_href("cam-a", "Available from 2024-01-01 to 2024-01-02", true).is_none()
        );
    }
}
