//! v2 semantic-indexing configuration fixtures and role-independence checks.

use std::path::PathBuf;

use a2a_lab_dev_kit::{
    A2A_LAB_CONFIG_SCHEMA_VERSION, A2A_LAB_CONFIG_SCHEMA_VERSION_V2, A2aLabConfig, ChatProvider,
    DEFAULT_SEMANTIC_DESCRIPTION_BACKEND, LoadedLabConfig,
};
use a2a_lab_dev_kit::{
    DEFAULT_BEDROCK_DESCRIPTION_MODEL, DEFAULT_JEV_MODEL_REVISION, DEFAULT_OPENAI_DESCRIPTION_MODEL,
};

const VALID_V2: &str = include_str!("fixtures/a2a-lab-v2-semantic-valid.json");

#[test]
fn v1_accepts_additive_semantic_indexing() {
    let config = A2aLabConfig::parse(&VALID_V2.replacen(
        "\"schema_version\": 2",
        "\"schema_version\": 1",
        1,
    ))
    .unwrap();
    assert_eq!(config.schema_version, A2A_LAB_CONFIG_SCHEMA_VERSION);
    assert!(config.semantic_indexing.is_some());
}

#[test]
fn v1_without_semantic_indexing_keeps_legacy_behavior() {
    let config = A2aLabConfig::parse(r#"{"schema_version":1,"videos":{"sources":[]}}"#).unwrap();
    assert!(config.semantic_indexing.is_none());
}

#[test]
fn fixture_v2_parses_independent_roles() {
    let config = A2aLabConfig::parse(VALID_V2).unwrap();
    assert_eq!(config.schema_version, A2A_LAB_CONFIG_SCHEMA_VERSION_V2);
    assert_eq!(
        config.chat.as_ref().unwrap().provider,
        Some(ChatProvider::Anthropic)
    );
    assert_eq!(
        config.chat.as_ref().unwrap().model.as_deref(),
        Some("claude-chat-only")
    );
    let semantic = config.semantic_indexing.as_ref().unwrap();
    assert_eq!(semantic.limits.roi.max_edge_pixels, 1568);
    assert_eq!(
        semantic.description.backend(),
        DEFAULT_SEMANTIC_DESCRIPTION_BACKEND
    );
    assert_eq!(
        semantic.description.model(),
        DEFAULT_BEDROCK_DESCRIPTION_MODEL
    );
    assert_ne!(semantic.description.model(), "claude-chat-only");
    let embeddings = semantic.embeddings.as_ref().unwrap();
    assert_eq!(embeddings.provider(), "bedrock");
    assert_eq!(embeddings.model(), "cohere.embed-english-v3");
    assert_eq!(embeddings.space().dimensions, 1024);
    assert_eq!(semantic.decisions.provider(), "jev");
    assert_eq!(semantic.decisions.model(), DEFAULT_JEV_MODEL_REVISION);
    assert_ne!(
        semantic.decisions.model(),
        DEFAULT_BEDROCK_DESCRIPTION_MODEL
    );
}

#[test]
fn fixture_v2_resolves_relative_vector_path_like_doc5() {
    let root = std::env::temp_dir().join(format!("a2a-v2-semantic-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let nested = root.join("configs");
    std::fs::create_dir_all(&nested).unwrap();
    let config_path = nested.join("a2a-lab.json");
    std::fs::write(&config_path, VALID_V2).unwrap();
    let loaded = LoadedLabConfig::load(&config_path).unwrap();
    let expected = nested.join("state/semantic-index");
    assert_eq!(loaded.semantic_vector_index_path, Some(expected.clone()));
    assert_eq!(
        loaded.semantic_sqlite_path,
        Some(nested.join("state/semantic.sqlite"))
    );
    assert!(!expected.exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rejects_description_fallback_and_judge_settings() {
    let documents = [
        (
            r#"{"schema_version":2,"semantic_indexing":{"fallback":true,"embeddings":{"provider":"p","model":"m","space":{"dimensions":4,"normalization":"l2_unit","metric":"cosine"}},"limits":LIMITS}}"#,
            "semantic_indexing.fallback",
        ),
        (
            r#"{"schema_version":2,"semantic_indexing":{"embeddings":{"provider":"p","model":"m","space":{"dimensions":4,"normalization":"l2_unit","metric":"cosine"}},"decisions":{"judge":"gpt"},"limits":LIMITS}}"#,
            "semantic_indexing.decisions.judge",
        ),
        (
            r#"{"schema_version":2,"semantic_indexing":{"embeddings":{"provider":"p","model":"m","space":{"dimensions":4,"normalization":"l2_unit","metric":"cosine"}},"decisions":{"use_description":true},"limits":LIMITS}}"#,
            "semantic_indexing.decisions.use_description",
        ),
    ];
    let limits = r#"{"worker":{"max_batch_items":1,"max_concurrency":1,"max_queue_backlog":1,"max_retry_attempts":1,"job_deadline":"30s","retry_backoff":"5s"},"search":{"top_k":1,"candidate_limit":1,"max_neighbor_frames":1,"query_window":"1h"},"roi":{"min_edge_pixels":280,"max_edge_pixels":1568,"max_rois_per_frame":1},"storage":{"vector_index_path":"./x","max_disk_bytes":1024},"decisions":{"max_batch_items":1,"max_concurrency":1,"job_deadline":"45s","coverage_policy":"on_demand"}}"#;
    for (template, field) in documents {
        let document = template.replace("LIMITS", limits);
        let error = A2aLabConfig::parse(&document).unwrap_err();
        let display = error.to_string();
        assert!(display.contains(field), "{display}");
        assert!(!display.contains("gpt"), "{display}");
    }
}

#[test]
fn rejects_non_jev_decision_provider_and_wrong_revision() {
    let limits = r#","limits":{"worker":{"max_batch_items":1,"max_concurrency":1,"max_queue_backlog":1,"max_retry_attempts":1,"job_deadline":"30s","retry_backoff":"5s"},"search":{"top_k":1,"candidate_limit":1,"max_neighbor_frames":1,"query_window":"1h"},"roi":{"min_edge_pixels":280,"max_edge_pixels":1568,"max_rois_per_frame":1},"storage":{"vector_index_path":"./x","max_disk_bytes":1024},"decisions":{"max_batch_items":1,"max_concurrency":1,"job_deadline":"45s","coverage_policy":"on_demand"}}"#;
    let openai = format!(
        r#"{{"schema_version":2,"semantic_indexing":{{"decisions":{{"provider":"openai","model":"{DEFAULT_JEV_MODEL_REVISION}"}},"embeddings":{{"provider":"p","model":"m","space":{{"dimensions":4,"normalization":"l2_unit","metric":"cosine"}}}}{limits}}}}}"#
    );
    assert!(A2aLabConfig::parse(&openai).is_err());

    let bad_revision = format!(
        r#"{{"schema_version":2,"semantic_indexing":{{"decisions":{{"provider":"jev","model":"jev-9.9.9"}},"embeddings":{{"provider":"p","model":"m","space":{{"dimensions":4,"normalization":"l2_unit","metric":"cosine"}}}}{limits}}}}}"#
    );
    let error = A2aLabConfig::parse(&bad_revision).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("semantic_indexing.decisions.model")
    );
    assert!(!error.to_string().contains("jev-9.9.9"));
}

#[test]
fn openai_description_default_does_not_change_v1_schema_constant() {
    let document = r#"{"schema_version":2,"semantic_indexing":{
            "description":{"provider":"openai"},
            "embeddings":{"provider":"e","model":"m","space":{"dimensions":8,"normalization":"l2_unit","metric":"cosine"}},
            "limits":{
                "worker":{"max_batch_items":1,"max_concurrency":1,"max_queue_backlog":1,"max_retry_attempts":1,"job_deadline":"30s","retry_backoff":"5s"},
                "search":{"top_k":1,"candidate_limit":1,"max_neighbor_frames":1,"query_window":"1h"},
                "roi":{"min_edge_pixels":280,"max_edge_pixels":1568,"max_rois_per_frame":1},
                "storage":{"vector_index_path":"./semantic","max_disk_bytes":2048},
                "decisions":{"max_batch_items":1,"max_concurrency":1,"job_deadline":"45s","coverage_policy":"sampled"}
            }
        }}"#;
    let config = A2aLabConfig::parse(document).unwrap();
    assert_eq!(A2A_LAB_CONFIG_SCHEMA_VERSION, 1);
    assert_eq!(
        config.semantic_indexing.unwrap().description.model(),
        DEFAULT_OPENAI_DESCRIPTION_MODEL
    );
}

#[test]
fn accepts_digit_containing_credential_env_references() {
    let document = r#"{"schema_version":2,"semantic_indexing":{
        "description":{"credentials":{"env":"A2ALAB_SEMANTIC_DESCRIPTION_AWS_PROFILE"}},
        "embeddings":{"provider":"p","model":"m","space":{"dimensions":4,"normalization":"l2_unit","metric":"cosine"},
            "credentials":{"env":"A2ALAB_EMBEDDING_API_KEY_V2"}},
        "decisions":{"credentials":{"env":"A2ALAB_JEV_API_KEY"}},
        "limits":{"worker":{"max_batch_items":1,"max_concurrency":1,"max_queue_backlog":1,"max_retry_attempts":1,"job_deadline":"30s","retry_backoff":"5s"},"search":{"top_k":1,"candidate_limit":1,"max_neighbor_frames":1,"query_window":"1h"},"roi":{"min_edge_pixels":280,"max_edge_pixels":1568,"max_rois_per_frame":1},"storage":{"vector_index_path":"./x","max_disk_bytes":1024},"decisions":{"max_batch_items":1,"max_concurrency":1,"job_deadline":"45s","coverage_policy":"on_demand"}}
    }}"#;
    let config = A2aLabConfig::parse(document).unwrap();
    let semantic = config.semantic_indexing.unwrap();
    assert_eq!(
        semantic.description.credentials().unwrap().env_var(),
        "A2ALAB_SEMANTIC_DESCRIPTION_AWS_PROFILE"
    );
}

#[test]
fn credential_reference_is_separate_from_chat_and_redacted_on_failure() {
    let document = r#"{"schema_version":2,"chat":{"provider":"openai"},"semantic_indexing":{
        "description":{"credentials":{"env":"not-valid"}},"embeddings":{"provider":"p","model":"m","space":{"dimensions":4,"normalization":"l2_unit","metric":"cosine"}},
        "limits":{"worker":{"max_batch_items":1,"max_concurrency":1,"max_queue_backlog":1,"max_retry_attempts":1,"job_deadline":"30s","retry_backoff":"5s"},"search":{"top_k":1,"candidate_limit":1,"max_neighbor_frames":1,"query_window":"1h"},"roi":{"min_edge_pixels":280,"max_edge_pixels":1568,"max_rois_per_frame":1},"storage":{"vector_index_path":"./x","max_disk_bytes":1024},"decisions":{"max_batch_items":1,"max_concurrency":1,"job_deadline":"45s","coverage_policy":"on_demand"}}
    }}"#;
    let error = A2aLabConfig::parse(document).unwrap_err();
    let display = error.to_string();
    assert!(display.contains("semantic_indexing.description.credentials.env"));
    assert!(!display.contains("not-valid"));
}

#[test]
fn rejects_duplicate_semantic_limits_key() {
    let document = r#"{"schema_version":2,"semantic_indexing":{
        "embeddings":{"provider":"p","model":"m","space":{"dimensions":4,"normalization":"l2_unit","metric":"cosine"}},
        "limits":{"worker":{"max_batch_items":1,"max_concurrency":1,"max_queue_backlog":1,"max_retry_attempts":1,"job_deadline":"30s","retry_backoff":"5s"},
        "search":{"top_k":1,"candidate_limit":1,"max_neighbor_frames":1,"query_window":"1h"},
        "roi":{"min_edge_pixels":280,"max_edge_pixels":1568,"max_rois_per_frame":1},
        "storage":{"vector_index_path":"./x","max_disk_bytes":1024},
        "decisions":{"max_batch_items":1,"max_concurrency":1,"job_deadline":"45s","coverage_policy":"on_demand",
        "max_batch_items":2}}
    }}"#;
    let error = A2aLabConfig::parse(document).unwrap_err();
    assert_eq!(error.code(), "invalid");
}

#[test]
fn load_does_not_require_vector_directory() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/a2a-lab-v2-semantic-valid.json");
    let loaded = LoadedLabConfig::load(path).unwrap();
    assert!(loaded.semantic_vector_index_path.is_some());
}
