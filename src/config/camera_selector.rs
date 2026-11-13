//! Declarative camera selector for video sources.

use serde::Serialize;
use serde_json::Value;

use crate::error::A2aLabError;

/// Non-negative index or non-empty name/UID text from the configuration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraSelector {
    /// Local capture-library index, including zero.
    Index(u32),
    /// Case-insensitive match against discovered camera metadata.
    Text(String),
}

/// JSON encoding for settings and API responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum CameraIdView {
    /// Integer `camera_id` selector.
    Index(u32),
    /// Text `camera_id` selector.
    Text(String),
}

impl CameraSelector {
    /// View field for round-trip serialization.
    #[must_use]
    pub fn as_view(&self) -> CameraIdView {
        match self {
            Self::Index(index) => CameraIdView::Index(*index),
            Self::Text(text) => CameraIdView::Text(text.clone()),
        }
    }

    /// Index used to reject duplicate index-based sources.
    #[must_use]
    pub const fn index_for_dedup(&self) -> Option<u32> {
        match self {
            Self::Index(index) => Some(*index),
            Self::Text(_) => None,
        }
    }
}

pub(crate) fn parse_source_camera(
    map: &serde_json::Map<String, Value>,
) -> Result<Option<CameraSelector>, A2aLabError> {
    let has_camera_id = map.contains_key("camera_id");
    let has_device_index = map.contains_key("device_index");
    if has_camera_id && has_device_index {
        return Err(A2aLabError::invalid(
            "videos.sources.camera_id",
            "cannot be used together with device_index",
        ));
    }
    if has_camera_id {
        return parse_camera_id(map.get("camera_id"));
    }
    if let Some(value) = map.get("device_index") {
        let index = super::parse_u32("videos.sources.device_index", value)?;
        return Ok(Some(CameraSelector::Index(index)));
    }
    Ok(None)
}

fn parse_camera_id(value: Option<&Value>) -> Result<Option<CameraSelector>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        Value::Number(number) => {
            let index = number
                .as_u64()
                .and_then(|parsed| u32::try_from(parsed).ok())
                .ok_or_else(|| {
                    A2aLabError::invalid(
                        "videos.sources.camera_id",
                        "must be a non-negative integer",
                    )
                })?;
            Ok(Some(CameraSelector::Index(index)))
        }
        Value::String(text) => {
            if text.chars().any(|character| !character.is_whitespace()) {
                Ok(Some(CameraSelector::Text(text.clone())))
            } else {
                Err(A2aLabError::invalid(
                    "videos.sources.camera_id",
                    "must be a non-empty text selector",
                ))
            }
        }
        _ => Err(A2aLabError::invalid(
            "videos.sources.camera_id",
            "must be a non-negative integer or non-empty text selector",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_accepts_index_text_and_absent() {
        let index = parse_source_camera(json!({"camera_id": 3}).as_object().unwrap()).unwrap();
        assert_eq!(index, Some(CameraSelector::Index(3)));
        let text =
            parse_source_camera(json!({"camera_id": "built-in"}).as_object().unwrap()).unwrap();
        assert_eq!(text, Some(CameraSelector::Text("built-in".to_owned())));
        let absent = parse_source_camera(json!({}).as_object().unwrap()).unwrap();
        assert_eq!(absent, None);
    }

    #[test]
    fn legacy_device_index_maps_to_index_selector() {
        let legacy = parse_source_camera(json!({"device_index": 7}).as_object().unwrap()).unwrap();
        assert_eq!(legacy, Some(CameraSelector::Index(7)));
    }

    #[test]
    fn rejects_conflict_invalid_and_whitespace() {
        let conflict = json!({"camera_id": 1, "device_index": 2});
        let map = conflict.as_object().unwrap();
        assert!(parse_source_camera(map).is_err());
        let whitespace_value = json!({"camera_id": "   "});
        let whitespace = whitespace_value.as_object().unwrap();
        assert!(parse_source_camera(whitespace).is_err());
        let bad_type_value = json!({"camera_id": true});
        let bad_type = bad_type_value.as_object().unwrap();
        assert!(parse_source_camera(bad_type).is_err());
    }

    #[test]
    fn view_round_trips_integer_and_text() {
        let index = CameraSelector::Index(0).as_view();
        assert_eq!(serde_json::to_value(index).unwrap(), json!(0));
        let text = CameraSelector::Text("emeet".to_owned()).as_view();
        assert_eq!(serde_json::to_value(text).unwrap(), json!("emeet"));
    }
}
