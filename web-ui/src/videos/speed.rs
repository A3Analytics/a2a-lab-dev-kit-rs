//! Shared DVR playback rates for MJPEG and HLS gateway sessions.

pub const HLS_PLAYLIST: &str = "index.m3u8";
pub const HLS_TRICK_8X_PLAYLIST: &str = "index-8x.m3u8";

const OPTIONS: [PlaybackSpeed; 4] = [
    PlaybackSpeed::One,
    PlaybackSpeed::Two,
    PlaybackSpeed::Four,
    PlaybackSpeed::Eight,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlaybackSpeed {
    #[default]
    One,
    Two,
    Four,
    Eight,
}

impl PlaybackSpeed {
    pub const fn all() -> &'static [PlaybackSpeed; 4] {
        &OPTIONS
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::One => "1x",
            Self::Two => "2x",
            Self::Four => "4x",
            Self::Eight => "8x",
        }
    }

    pub const fn control_value(self) -> &'static str {
        self.label()
    }

    pub const fn mjpeg_pace(self) -> u32 {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Four => 4,
            Self::Eight => 8,
        }
    }

    pub const fn ordinary_hls_rate(self) -> f64 {
        match self {
            Self::One => 1.0,
            Self::Two => 2.0,
            Self::Four => 4.0,
            Self::Eight => 1.0,
        }
    }

    pub fn parse_control(raw: &str) -> Self {
        match raw.trim() {
            "1x" => Self::One,
            "2x" => Self::Two,
            "4x" => Self::Four,
            "8x" => Self::Eight,
            _ => Self::One,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Playback8xMetadata {
    pub rendition_path: Option<String>,
    pub fallback_path: String,
    pub unavailable_reason: Option<String>,
}

impl Playback8xMetadata {
    pub fn parse(value: &serde_json::Value) -> Option<Self> {
        let object = value.as_object()?;
        let fallback = object
            .get("fallback_path")
            .and_then(serde_json::Value::as_str)
            .filter(|path| safe_relative_asset(path))?;
        Some(Self {
            rendition_path: object
                .get("rendition_path")
                .and_then(serde_json::Value::as_str)
                .filter(|path| safe_relative_asset(path))
                .map(str::to_owned),
            fallback_path: fallback.to_owned(),
            unavailable_reason: object
                .get("unavailable_reason")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        })
    }

    pub fn absent() -> Self {
        Self {
            fallback_path: HLS_PLAYLIST.to_owned(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StreamPlan {
    pub fetch_path: String,
    pub hls_playback_rate: f64,
    pub mjpeg_pace: u32,
    pub trick_fallback: bool,
}

pub fn is_hls_fetch_path(path: &str) -> bool {
    path.ends_with(".m3u8")
}

pub fn is_hls_session(media_type: &str) -> bool {
    matches!(
        media_type,
        "application/vnd.apple.mpegurl" | "application/x-mpegURL"
    )
}

pub fn plan_stream(
    speed: PlaybackSpeed,
    gateway_root: &str,
    media_type: &str,
    playback_8x: &Playback8xMetadata,
) -> StreamPlan {
    if is_hls_session(media_type) {
        plan_hls(speed, gateway_root, playback_8x)
    } else {
        StreamPlan {
            fetch_path: gateway_root.to_owned(),
            hls_playback_rate: 1.0,
            mjpeg_pace: speed.mjpeg_pace(),
            trick_fallback: false,
        }
    }
}

fn plan_hls(
    speed: PlaybackSpeed,
    gateway_root: &str,
    playback_8x: &Playback8xMetadata,
) -> StreamPlan {
    let fallback = if playback_8x.fallback_path.is_empty() {
        HLS_PLAYLIST
    } else {
        playback_8x.fallback_path.as_str()
    };
    match speed {
        PlaybackSpeed::Eight => {
            if let Some(rendition) = playback_8x.rendition_path.as_deref() {
                StreamPlan {
                    fetch_path: join_gateway(gateway_root, rendition),
                    hls_playback_rate: 1.0,
                    mjpeg_pace: 8,
                    trick_fallback: false,
                }
            } else {
                StreamPlan {
                    fetch_path: join_gateway(gateway_root, fallback),
                    hls_playback_rate: 1.0,
                    mjpeg_pace: 8,
                    trick_fallback: true,
                }
            }
        }
        rate => StreamPlan {
            fetch_path: join_gateway(gateway_root, fallback),
            hls_playback_rate: rate.ordinary_hls_rate(),
            mjpeg_pace: rate.mjpeg_pace(),
            trick_fallback: false,
        },
    }
}

pub fn join_gateway(root: &str, asset: &str) -> String {
    debug_assert!(safe_relative_asset(asset));
    if asset.is_empty() {
        return root.to_owned();
    }
    let trimmed = root.trim_end_matches('/');
    format!("{trimmed}/{asset}")
}

pub fn safe_relative_asset(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 128
        && !path.contains("..")
        && !path.contains(':')
        && !path.contains('\\')
        && !path.starts_with('/')
        && !path.contains('?')
        && !path.contains('#')
        && path
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn control_offers_four_rates_with_one_x_default() {
        assert_eq!(PlaybackSpeed::all().len(), 4);
        assert_eq!(PlaybackSpeed::default(), PlaybackSpeed::One);
        assert_eq!(
            PlaybackSpeed::all()
                .iter()
                .map(|rate| rate.label())
                .collect::<Vec<_>>(),
            ["1x", "2x", "4x", "8x"]
        );
    }

    #[test]
    fn stale_speed_values_resolve_to_one_x() {
        assert_eq!(PlaybackSpeed::parse_control(""), PlaybackSpeed::One);
        assert_eq!(
            PlaybackSpeed::parse_control("index-8x.m3u8"),
            PlaybackSpeed::One
        );
        assert_eq!(PlaybackSpeed::parse_control("16x"), PlaybackSpeed::One);
    }

    #[test]
    fn hls_ordinary_rates_use_fallback_playlist_and_playback_rate() {
        let root = "/media/video-clips/clip-1/";
        let meta = Playback8xMetadata {
            rendition_path: Some(HLS_TRICK_8X_PLAYLIST.to_owned()),
            fallback_path: HLS_PLAYLIST.to_owned(),
            unavailable_reason: None,
        };
        let plan = plan_stream(
            PlaybackSpeed::Four,
            root,
            "application/vnd.apple.mpegurl",
            &meta,
        );
        assert_eq!(plan.fetch_path, "/media/video-clips/clip-1/index.m3u8");
        assert_eq!(plan.hls_playback_rate, 4.0);
        assert!(!plan.trick_fallback);
    }

    #[test]
    fn hls_eight_x_uses_trick_rendition_when_present() {
        let root = "/media/video-live/live-1/";
        let meta = Playback8xMetadata {
            rendition_path: Some(HLS_TRICK_8X_PLAYLIST.to_owned()),
            fallback_path: HLS_PLAYLIST.to_owned(),
            unavailable_reason: None,
        };
        let plan = plan_stream(PlaybackSpeed::Eight, root, "application/x-mpegURL", &meta);
        assert_eq!(plan.fetch_path, "/media/video-live/live-1/index-8x.m3u8");
        assert_eq!(plan.hls_playback_rate, 1.0);
        assert!(!plan.trick_fallback);
    }

    #[test]
    fn hls_eight_x_falls_back_when_trick_missing() {
        let root = "/media/video-clips/clip-9/";
        let meta = Playback8xMetadata {
            rendition_path: None,
            fallback_path: HLS_PLAYLIST.to_owned(),
            unavailable_reason: Some("sparse_keyframes".to_owned()),
        };
        let plan = plan_stream(
            PlaybackSpeed::Eight,
            root,
            "application/vnd.apple.mpegurl",
            &meta,
        );
        assert_eq!(plan.fetch_path, "/media/video-clips/clip-9/index.m3u8");
        assert!(plan.trick_fallback);
    }

    #[test]
    fn mjpeg_plans_never_embed_speed_in_paths() {
        for speed in PlaybackSpeed::all() {
            let plan = plan_stream(
                *speed,
                "/media/video-clips/clip-1/",
                crate::videos::MJPEG_MEDIA_TYPE,
                &Playback8xMetadata::absent(),
            );
            assert_eq!(plan.fetch_path, "/media/video-clips/clip-1/");
            assert!(!plan.fetch_path.contains(speed.label()));
        }
    }

    #[test]
    fn playback_8x_parser_rejects_unsafe_paths() {
        let bad = json!({
            "rendition_path": "../secret.m3u8",
            "fallback_path": "index.m3u8"
        });
        assert!(
            Playback8xMetadata::parse(&bad)
                .unwrap()
                .rendition_path
                .is_none()
        );
        let ok = json!({
            "rendition_path": "index-8x.m3u8",
            "fallback_path": "index.m3u8",
            "unavailable_reason": "sparse_keyframes"
        });
        let meta = Playback8xMetadata::parse(&ok).unwrap();
        assert_eq!(meta.rendition_path.as_deref(), Some(HLS_TRICK_8X_PLAYLIST));
    }
}
