//! Hardware-free tests for the v1 configuration envelope, chat, and video settings.

use std::path::PathBuf;
use std::sync::Mutex;

use a2a_lab_dev_kit::videos::{CaptureFormat, CaptureFormatPreference};
use a2a_lab_dev_kit::{
    A2A_LAB_CONFIG_SCHEMA_VERSION, A2aLabConfig, A2aLabError, ChatProvider, LoadedLabConfig,
    VideoInputFormat,
};

static WORKING_DIRECTORY: Mutex<()> = Mutex::new(());

#[test]
fn accepts_each_supported_provider() {
    for (document, provider) in [
        (
            r#"{"schema_version":1,"chat":{"provider":"bedrock"}}"#,
            ChatProvider::Bedrock,
        ),
        (
            r#"{"schema_version":1,"chat":{"provider":"openai"}}"#,
            ChatProvider::Openai,
        ),
        (
            r#"{"schema_version":1,"chat":{"provider":"anthropic"}}"#,
            ChatProvider::Anthropic,
        ),
    ] {
        let config = A2aLabConfig::parse(document).unwrap();
        assert_eq!(config.schema_version, A2A_LAB_CONFIG_SCHEMA_VERSION);
        assert_eq!(config.chat.as_ref().unwrap().provider, Some(provider));
    }
}

#[test]
fn rejects_non_object_document_root() {
    let error = A2aLabConfig::parse("[]").unwrap_err();
    assert_eq!(error.code(), "invalid");
}

#[test]
fn parse_is_side_effect_free() {
    let document = r#"{"schema_version":1,"chat":{"provider":"openai","model":"test-model"}}"#;
    let first = A2aLabConfig::parse(document).unwrap();
    let second = A2aLabConfig::parse(document).unwrap();
    assert_eq!(first, second);
}

#[test]
fn omitted_videos_stays_absent_and_empty_sources_parse() {
    let omitted = A2aLabConfig::parse(r#"{"schema_version":1}"#).unwrap();
    assert!(omitted.videos.is_none());
    let empty = A2aLabConfig::parse(r#"{"schema_version":1,"videos":{"sources":[]}}"#).unwrap();
    assert!(empty.videos.unwrap().sources.is_empty());
}

#[test]
#[allow(clippy::cognitive_complexity)]
fn three_sources_keep_ids_formats_and_optional_presence() {
    let config = A2aLabConfig::parse(
        r#"{"schema_version":1,"videos":{"sources":[
            {"id":"external-camera","name":"Bench camera","description":"Overview","format":"mjpeg","device_index":0},
            {"id":"camera","format":"yuyv","device_index":6},
            {"id":"detail","name":"Detail","format":"yuy2","device_index":4}
        ],"fps":5,"live_retention":"5s","dvr":{"retention":"3d","storage_path":"./state/video-dvr"}}}"#,
    )
    .unwrap();
    let videos = config.videos.unwrap();
    assert_eq!(videos.fps, Some(5));
    assert_eq!(videos.live_retention.unwrap().as_secs(), 5);
    let dvr = videos.dvr.unwrap();
    assert_eq!(dvr.retention.unwrap().as_secs(), 3 * 24 * 60 * 60);
    assert_eq!(dvr.storage_path.as_deref(), Some("./state/video-dvr"));
    assert_eq!(videos.sources.len(), 3);
    assert_eq!(
        videos.sources[0].format.capture_format(),
        CaptureFormat::Mjpeg
    );
    assert_eq!(
        videos.sources[0].format.capture_preference(),
        CaptureFormatPreference::Mjpeg
    );
    assert_eq!(videos.sources[1].format, VideoInputFormat::Yuy2);
    assert_eq!(
        videos.sources[1].format.capture_format(),
        CaptureFormat::Yuy2
    );
    assert_ne!(
        videos.sources[1].format.capture_format(),
        CaptureFormat::Mjpeg
    );
    assert!(videos.sources[1].name.is_none());
    assert_eq!(videos.sources[1].display_name(), "camera");
    assert_eq!(videos.sources[1].display_description(), "camera");
    assert_eq!(videos.sources[0].display_description(), "Overview");
    assert_eq!(videos.sources[2].display_description(), "Detail");
}

#[test]
fn auto_format_maps_to_capture_preference() {
    let config = A2aLabConfig::parse(
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","format":"auto","width":1280,"height":720,"device_index":0}]}}"#,
    )
    .unwrap();
    let source = &config.videos.unwrap().sources[0];
    assert_eq!(source.format, VideoInputFormat::Auto);
    assert_eq!(
        source.format.capture_preference(),
        CaptureFormatPreference::Auto
    );
}

#[test]
fn duration_strings_share_one_elapsed_second_interpretation() {
    for (text, seconds) in [
        ("5s", 5),
        ("5m", 300),
        ("1h", 3_600),
        ("60m", 3_600),
        ("1h30m", 5_400),
        ("3d", 259_200),
        ("1w", 604_800),
        ("1.5m", 90),
        ("1000ms", 1),
    ] {
        let document = format!(
            r#"{{"schema_version":1,"videos":{{"sources":[],"live_retention":"{text}"}}}}"#
        );
        let config = A2aLabConfig::parse(&document).unwrap();
        assert_eq!(
            config.videos.unwrap().live_retention.unwrap().as_secs(),
            seconds,
            "{text}"
        );
    }
}

#[test]
fn rejects_invalid_video_contract_without_echoing_values() {
    let rejected = [
        r#"{"schema_version":1,"videos":{}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"disk_retention_secs":3}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"dvr_root":"./state"}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"live_retention_secs":5}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"backend":"nokhwa"}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"output_format":"h264"}}"#,
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","format":"h264","device_index":1}]}}"#,
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","format":"h265","device_index":1}]}}"#,
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","format":"mjpeg","device_index":1},{"id":"cam","format":"yuy2","device_index":2}]}}"#,
        r#"{"schema_version":1,"videos":{"sources":[{"id":"a","format":"mjpeg","device_index":1},{"id":"b","format":"yuy2","device_index":1}]}}"#,
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","format":"mjpeg","device_index":-1}]}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"fps":0}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"fps":1.5}}"#,
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","name":" ","format":"mjpeg","device_index":0}]}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"dvr":{"storage_path":"   "}}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"live_retention":"1.5s"}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"live_retention":0}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"live_retention":"5"}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"live_retention":""}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"live_retention":"-5s"}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"dvr":{"retention":"1y"}}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"dvr":{"retention":"1mo"}}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"live_retention":"badspanzz"}}"#,
        r#"{"schema_version":1,"videos":{"sources":[],"live_retention":"999999999999w"}}"#,
        r#"{"schema_version":1,"videos":{"sources":[{"id":"cam","format":"mjpeg","device_index":0,"device_index":1}]}}"#,
    ];
    for document in rejected {
        let error = A2aLabConfig::parse(document).expect_err(document);
        let display = error.to_string();
        assert!(!display.contains("badspanzz"), "{display}");
        assert!(!display.contains("h265"), "{display}");
        assert_eq!(error.code(), "invalid");
    }
}

struct RestoreDir(PathBuf);
impl Drop for RestoreDir {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

#[test]
fn load_resolves_file_relative_storage_without_creating_it() {
    let _guard = WORKING_DIRECTORY.lock().unwrap();
    let original = std::env::current_dir().unwrap();
    let _restore = RestoreDir(original.clone());
    let root = std::env::temp_dir().join(format!("a2a lab config {}", std::process::id()));
    let config_dir = root.join("nested");
    std::fs::create_dir_all(&config_dir).unwrap();
    let config_path = config_dir.join("a2a-lab.json");
    std::fs::write(
        &config_path,
        r#"{"schema_version":1,"videos":{"sources":[],"dvr":{"storage_path":"state/video-dvr","retention":"1h30m"}}}"#,
    )
    .unwrap();
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::env::set_current_dir(&elsewhere).unwrap();
    let loaded = LoadedLabConfig::load(&config_path).unwrap();
    std::env::set_current_dir(&original).unwrap();
    let expected = config_dir.join("state/video-dvr");
    assert_eq!(loaded.dvr_storage_path, Some(expected.clone()));
    assert!(!expected.exists());
    assert_eq!(
        loaded
            .config
            .videos
            .unwrap()
            .dvr
            .unwrap()
            .retention
            .unwrap()
            .as_secs(),
        5_400
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn load_keeps_absolute_storage_and_does_not_expand_tilde() {
    let root = std::env::temp_dir().join(format!("a2a-lab-abs-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let absolute = root.join("absolute-dvr");
    let config_path = root.join("spaced name.json");
    let body = format!(
        r#"{{"schema_version":1,"videos":{{"sources":[],"dvr":{{"storage_path":"{}"}}}}}}"#,
        absolute.display().to_string().replace('\\', "\\\\")
    );
    std::fs::write(&config_path, &body).unwrap();
    let loaded = LoadedLabConfig::load(&config_path).unwrap();
    assert_eq!(loaded.dvr_storage_path, Some(absolute.clone()));
    assert!(!absolute.exists());

    let tilde_path = root.join("tilde.json");
    std::fs::write(
        &tilde_path,
        r#"{"schema_version":1,"videos":{"sources":[],"dvr":{"storage_path":"~/not-expanded"}}}"#,
    )
    .unwrap();
    let tilde = LoadedLabConfig::load(&tilde_path).unwrap();
    let resolved = tilde.dvr_storage_path.unwrap();
    assert_eq!(resolved, root.join("~/not-expanded"));
    assert!(
        resolved
            .components()
            .any(|component| component.as_os_str() == "~")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn load_rejects_missing_invalid_utf8_and_invalid_documents() {
    let missing = LoadedLabConfig::load(PathBuf::from("/no/such/a2a-lab-config.json")).unwrap_err();
    assert_eq!(missing.code(), "invalid");
    assert!(!missing.to_string().contains("secret-value"));

    let root = std::env::temp_dir().join(format!("a2a-lab-bad-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let utf8 = root.join("bad utf8.json");
    std::fs::write(&utf8, [0xFF, 0xFE, b'{']).unwrap();
    assert!(LoadedLabConfig::load(&utf8).is_err());
    let malformed = root.join("malformed.json");
    std::fs::write(&malformed, b"{").unwrap();
    let error = LoadedLabConfig::load(&malformed).unwrap_err();
    assert!(!error.to_string().contains('{'));
    let invalid = root.join("invalid.json");
    std::fs::write(
        &invalid,
        br#"{"schema_version":1,"videos":{"sources":[],"live_retention":"badspanzz"}}"#,
    )
    .unwrap();
    let error = LoadedLabConfig::load(&invalid).unwrap_err();
    assert!(!error.to_string().contains("badspanzz"));
    assert!(LoadedLabConfig::load("").is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn partial_dvr_block_keeps_omitted_fields_absent() {
    let config = A2aLabConfig::parse(
        r#"{"schema_version":1,"videos":{"sources":[],"dvr":{"storage_path":"/var/dvr"}}}"#,
    )
    .unwrap();
    let dvr = config.videos.unwrap().dvr.unwrap();
    assert!(dvr.retention.is_none());
    assert!(dvr.save_interval.is_none());
    assert_eq!(dvr.storage_path.as_deref(), Some("/var/dvr"));
}

#[test]
fn dvr_save_interval_accepts_subsecond_durations() {
    for (text, nanos) in [
        ("500ms", 500_000_000_u64),
        ("1s", 1_000_000_000),
        ("5s", 5_000_000_000),
        ("30s", 30_000_000_000),
    ] {
        let document = format!(
            r#"{{"schema_version":1,"videos":{{"sources":[],"dvr":{{"save_interval":"{text}"}}}}}}"#
        );
        let config = A2aLabConfig::parse(&document).unwrap();
        let interval = config.videos.unwrap().dvr.unwrap().save_interval.unwrap();
        assert_eq!(interval.as_nanos(), nanos, "{text}");
    }
}

#[test]
fn load_preserves_exact_preamble_and_omitted_preamble() {
    let root = std::env::temp_dir().join(format!(
        "a2a-lab-preamble-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let omitted = root.join("omitted.json");
    std::fs::write(
        &omitted,
        r#"{"schema_version":1,"chat":{"provider":"openai","model":"kept"}}"#,
    )
    .unwrap();
    let omitted = LoadedLabConfig::load(&omitted).unwrap();
    let chat = omitted.config.chat.unwrap();
    assert!(chat.preamble.is_none());
    assert_eq!(chat.provider, Some(ChatProvider::Openai));
    assert_eq!(chat.model.as_deref(), Some("kept"));

    let blank = root.join("blank.json");
    std::fs::write(
        &blank,
        r#"{"schema_version":1,"chat":{"provider":"anthropic","model":"kept","preamble":""}}"#,
    )
    .unwrap();
    let blank = LoadedLabConfig::load(&blank).unwrap();
    let chat = blank.config.chat.unwrap();
    assert_eq!(chat.preamble.as_deref(), Some(""));
    assert_eq!(chat.provider, Some(ChatProvider::Anthropic));
    assert_eq!(chat.model.as_deref(), Some("kept"));

    let base = root.join("a2a-lab.json");
    std::fs::write(
        &base,
        r#"{"schema_version":1,"chat":{"provider":"openai","model":"base-model","preamble":" \t\n"}}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("a2a-lab.secret.json"),
        r#"{"chat":{"model":"secret-model"}}"#,
    )
    .unwrap();
    let loaded = LoadedLabConfig::load(&base).unwrap();
    let chat = loaded.config.chat.unwrap();
    assert_eq!(chat.preamble.as_deref(), Some(" \t\n"));
    assert_eq!(chat.provider, Some(ChatProvider::Openai));
    assert_eq!(chat.model.as_deref(), Some("secret-model"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rejects_invalid_dvr_save_interval_without_echoing_values() {
    for text in ["", "0ms", "-1s", "badspanzz"] {
        let document = format!(
            r#"{{"schema_version":1,"videos":{{"sources":[],"dvr":{{"save_interval":"{text}"}}}}}}"#
        );
        let error = A2aLabConfig::parse(&document).unwrap_err();
        assert!(!error.to_string().contains("badspanzz"));
        assert_eq!(error.code(), "invalid");
    }
}

#[test]
fn semantic_decision_preamble_keeps_any_string_and_rejects_non_strings() {
    let limits = r#""worker":{"max_batch_items":1,"max_concurrency":1,"max_queue_backlog":1,"max_retry_attempts":1,"job_deadline":"30s","retry_backoff":"5s"},"search":{"top_k":1,"candidate_limit":1,"max_neighbor_frames":1,"query_window":"1h"},"roi":{"min_edge_pixels":280,"max_edge_pixels":1568,"max_rois_per_frame":1},"storage":{"vector_index_path":"./semantic","max_disk_bytes":1024},"decisions":{"max_batch_items":1,"max_concurrency":1,"job_deadline":"45s","coverage_policy":"on_demand"}"#;
    let document = |decisions: &str| {
        format!(
            r#"{{"schema_version":2,"semantic_indexing":{{"embeddings":{{"provider":"p","model":"m","space":{{"dimensions":8,"normalization":"l2_unit","metric":"cosine","generation":1}}}},"decisions":{decisions},"limits":{{{limits}}}}}}}"#
        )
    };
    let omitted = A2aLabConfig::parse(&document("{}")).unwrap();
    let omitted = omitted.semantic_indexing.unwrap().decisions;
    assert!(omitted.preamble().is_none());
    assert!(omitted.clone().preamble().is_none());
    for text in ["", " ", " \t\n", "  keep\n"] {
        let decisions = format!(r#"{{"preamble":{}}}"#, serde_json::to_string(text).unwrap());
        let parsed = A2aLabConfig::parse(&document(&decisions)).unwrap();
        let decisions = parsed.semantic_indexing.unwrap().decisions;
        assert_eq!(decisions.preamble(), Some(text));
        assert_eq!(decisions.clone().preamble(), Some(text));
    }
    for decisions in [
        r#"{"preamble":1}"#,
        r#"{"preamble":null}"#,
        r#"{"preamble":true}"#,
        r#"{"preamble":["a"]}"#,
        r#"{"preamble":{"text":"a"}}"#,
    ] {
        match A2aLabConfig::parse(&document(decisions)).unwrap_err() {
            A2aLabError::Invalid { field, message } => {
                assert_eq!(field, "semantic_indexing.decisions.preamble");
                assert_eq!(message, "must be a string");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
