//! Focused configuration tests for declarative camera selectors.

use a2a_lab_dev_kit::{
    A2aLabConfig, CameraIdView, CameraSelector, VideoCaptureSettings, VideoInputFormat,
};

#[test]
fn camera_id_integer_parses_and_round_trips_through_settings_view() {
    let config = A2aLabConfig::parse(
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","camera_id":3,"format":"mjpeg"}]}}"#,
    )
    .unwrap();
    let videos = config.videos.as_ref().unwrap();
    let source = &videos.sources[0];
    assert_eq!(source.camera_id, Some(CameraSelector::Index(3)));
    let view = VideoCaptureSettings::from_video_config(config.videos.as_ref()).view();
    let view = &view.sources[0];
    assert_eq!(view.camera_id, Some(CameraIdView::Index(3)));
}

#[test]
fn camera_id_text_parses_and_round_trips_through_settings_view() {
    let config = A2aLabConfig::parse(
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","camera_id":"built-in","format":"yuy2"}]}}"#,
    )
    .unwrap();
    let videos = config.videos.as_ref().unwrap();
    let source = &videos.sources[0];
    assert_eq!(
        source.camera_id,
        Some(CameraSelector::Text("built-in".to_owned()))
    );
    let view = VideoCaptureSettings::from_video_config(config.videos.as_ref())
        .view()
        .sources[0]
        .clone();
    assert_eq!(
        view.camera_id,
        Some(CameraIdView::Text("built-in".to_owned()))
    );
}

#[test]
fn omitted_camera_id_selects_default_and_omits_view_field() {
    let config = A2aLabConfig::parse(
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","width":1280,"height":720}]}}"#,
    )
    .unwrap();
    let videos = config.videos.as_ref().unwrap();
    let source = &videos.sources[0];
    assert_eq!(source.camera_id, None);
    assert_eq!(source.format, VideoInputFormat::Auto);
    let view = VideoCaptureSettings::from_video_config(config.videos.as_ref()).view();
    assert!(view.sources[0].camera_id.is_none());
    assert_eq!(view.sources[0].format, "auto");
}

#[test]
fn legacy_device_index_alias_maps_to_index_selector() {
    let config = A2aLabConfig::parse(
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","device_index":9}]}}"#,
    )
    .unwrap();
    let videos = config.videos.as_ref().unwrap();
    let source = &videos.sources[0];
    assert_eq!(source.camera_id, Some(CameraSelector::Index(9)));
    assert_eq!(source.device_index(), Some(9));
    assert_eq!(source.format, VideoInputFormat::Auto);
    let view = VideoCaptureSettings::from_video_config(config.videos.as_ref()).view();
    assert_eq!(view.sources[0].camera_id, Some(CameraIdView::Index(9)));
}

#[test]
fn rejected_cases_keep_valid_configuration_parseable() {
    let valid = A2aLabConfig::parse(
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","camera_id":1,"format":"mjpeg"}]}}"#,
    )
    .unwrap();
    let rejected = [
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","camera_id":1,"device_index":2}]}}"#,
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","camera_id":"   "}]}}"#,
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","camera_id":true}]}}"#,
        r#"{"schema_version":1,"videos":{"sources":[{"id":"a","camera_id":1},{"id":"b","camera_id":1}]}}"#,
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","device_index":-1}]}}"#,
    ];
    for document in rejected {
        let error = A2aLabConfig::parse(document).unwrap_err();
        assert_eq!(error.code(), "invalid", "{document}");
    }
    let again = A2aLabConfig::parse(
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","camera_id":1,"format":"mjpeg"}]}}"#,
    )
    .unwrap();
    assert_eq!(valid, again);
}
