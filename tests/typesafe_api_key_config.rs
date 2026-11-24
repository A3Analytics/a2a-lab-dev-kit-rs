//! Layered `TypeSafe` API key loading.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use a2a_lab_dev_kit::{A2aLabError, LabConfigStore, LoadedLabConfig};

static ENV_LOCK: Mutex<()> = Mutex::new(());

const BASE_KEY: &str = "base-typesafe-key-9f3a";
const SECRET_KEY: &str = "secret-typesafe-key-9f3a";
const ENV_KEY: &str = "env-typesafe-key-9f3a";

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "a2a-lab-typesafe-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        if let Ok(entries) = fs::read_dir(&self.0) {
            for entry in entries.flatten() {
                let _ = fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o644));
            }
        }
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn set_env(name: &str, value: Option<&str>) {
    unsafe {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}

fn with_typesafe_env<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
    let _guard = lock_env();
    let previous = std::env::var("TYPESAFE_API_KEY").ok();
    set_env("TYPESAFE_API_KEY", value);
    let result = body();
    set_env("TYPESAFE_API_KEY", previous.as_deref());
    result
}

fn write(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
}

fn base_document(api_key: &str) -> String {
    format!(
        r#"{{"schema_version":1,"chat":{{"provider":"openai","model":"base-model"}},"typesafe":{{"api_key":"{api_key}"}}}}"#
    )
}

fn invalid_field(error: &A2aLabError) -> &'static str {
    match error {
        A2aLabError::Invalid { field, .. } => field,
        other => panic!("unexpected {other}"),
    }
}

#[test]
fn loading_applies_base_then_secret_then_env() {
    let root = TempRoot::new("precedence");
    let base = root.0.join("a2a-lab.json");
    write(&base, &base_document(BASE_KEY));
    write(
        &root.0.join("a2a-lab.secret.json"),
        &format!(
            r#"{{"chat":{{"model":"secret-model"}},"typesafe":{{"api_key":"{SECRET_KEY}"}}}}"#
        ),
    );

    let secret_layer = with_typesafe_env(None, || LoadedLabConfig::load(&base).unwrap());
    assert_eq!(
        secret_layer.config.chat.as_ref().unwrap().model.as_deref(),
        Some("secret-model")
    );
    assert_eq!(
        secret_layer
            .config
            .chat
            .as_ref()
            .unwrap()
            .provider
            .unwrap()
            .as_str(),
        "openai"
    );
    assert_eq!(secret_layer.typesafe_api_key(), Some(SECRET_KEY));

    let env_layer = with_typesafe_env(Some(ENV_KEY), || LoadedLabConfig::load(&base).unwrap());
    assert_eq!(env_layer.typesafe_api_key(), Some(ENV_KEY));
    assert_eq!(
        env_layer.config.chat.as_ref().unwrap().model.as_deref(),
        Some("secret-model")
    );

    fs::remove_file(root.0.join("a2a-lab.secret.json")).unwrap();
    let base_layer = with_typesafe_env(None, || LoadedLabConfig::load(&base).unwrap());
    assert_eq!(base_layer.typesafe_api_key(), Some(BASE_KEY));
    assert_eq!(
        base_layer.config.chat.as_ref().unwrap().model.as_deref(),
        Some("base-model")
    );
}

#[test]
fn strict_parser_rejects_duplicate_and_unknown_structure_before_merge() {
    let root = TempRoot::new("strict");
    let base = root.0.join("a2a-lab.json");
    write(&base, r#"{"schema_version":1}"#);
    write(
        &root.0.join("a2a-lab.secret.json"),
        r#"{"typesafe":{"api_key":"secret-dup-alpha","api_key":"secret-dup-beta"}}"#,
    );
    let duplicate = with_typesafe_env(None, || LoadedLabConfig::load(&base).unwrap_err());
    assert_eq!(invalid_field(&duplicate), "config.secret");
    assert!(duplicate.to_string().contains("duplicate JSON object key"));
    assert!(!duplicate.to_string().contains("secret-dup-alpha"));
    assert!(!duplicate.to_string().contains("secret-dup-beta"));

    write(
        &root.0.join("a2a-lab.secret.json"),
        r#"{"retired_envelope_field":true,"typesafe":{"api_key":"secret-unknown-field"}}"#,
    );
    let unknown = with_typesafe_env(None, || LoadedLabConfig::load(&base).unwrap_err());
    assert_eq!(invalid_field(&unknown), "config.secret");
    assert!(unknown.to_string().contains("unknown field"));
    assert!(!unknown.to_string().contains("secret-unknown-field"));

    write(
        &root.0.join("a2a-lab.secret.json"),
        r#"{"chat":{"provider":"not-a-provider"}}"#,
    );
    let merged = with_typesafe_env(None, || LoadedLabConfig::load(&base).unwrap_err());
    assert_eq!(invalid_field(&merged), "config.secret");
    assert!(!merged.to_string().contains("not-a-provider"));
}

#[test]
fn resolved_key_stays_out_of_debug_serialized_config_and_settings() {
    let root = TempRoot::new("redact");
    let base = root.0.join("a2a-lab.json");
    let secret = "sk-redacted-typesafe-key";
    write(
        &base,
        &format!(
            r#"{{"schema_version":1,"chat":{{"provider":"openai","model":"visible-model"}},"image_decision":{{"endpoint":"https://jev.example/v1/decide","model":"jev-1.14.0","credentials":{{"env":"IMAGE_DECISION_VISIBLE"}}}},"typesafe":{{"api_key":"{secret}"}}}}"#
        ),
    );
    let loaded = with_typesafe_env(None, || LoadedLabConfig::load(&base).unwrap());
    assert_eq!(loaded.typesafe_api_key(), Some(secret));
    let debug = format!("{loaded:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains(secret));
    let config_debug = format!("{:?}", loaded.config);
    assert!(!config_debug.contains(secret));
    let serialized = serde_json::to_string(loaded.config.image_decision.as_ref().unwrap()).unwrap();
    assert!(!serialized.contains(secret));
    assert!(serialized.contains("IMAGE_DECISION_VISIBLE"));

    let store = with_typesafe_env(None, || LabConfigStore::open(&base).unwrap());
    let snapshot = store.snapshot().unwrap();
    let document = snapshot.document.to_string();
    assert!(!document.contains(secret));
    assert!(snapshot.document.get("typesafe").is_none());
    let snapshot_debug = format!("{snapshot:?}");
    assert!(!snapshot_debug.contains(secret));
}

#[test]
fn missing_sibling_loads_and_present_failures_name_their_source() {
    let root = TempRoot::new("sibling");
    let base = root.0.join("lab.config.json");
    write(&base, r#"{"schema_version":1,"chat":{"model":"kept"}}"#);
    let loaded = with_typesafe_env(None, || LoadedLabConfig::load(&base).unwrap());
    assert_eq!(loaded.typesafe_api_key(), None);
    assert_eq!(
        loaded.config.chat.as_ref().unwrap().model.as_deref(),
        Some("kept")
    );

    let secret = root.0.join("lab.config.secret.json");
    write(&secret, "{");
    let malformed = with_typesafe_env(None, || LoadedLabConfig::load(&base).unwrap_err());
    assert_eq!(invalid_field(&malformed), "config.secret");
    assert!(malformed.to_string().contains("must be valid JSON"));
    assert!(!malformed.to_string().contains('{'));

    write(
        &secret,
        &format!(r#"{{"typesafe":{{"api_key":"{SECRET_KEY}"}}}}"#),
    );
    let mut permissions = fs::metadata(&secret).unwrap().permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(&secret, permissions).unwrap();
    let unreadable = with_typesafe_env(None, || LoadedLabConfig::load(&base).unwrap_err());
    assert_eq!(invalid_field(&unreadable), "config.secret");
    assert!(unreadable.to_string().contains("unreadable"));
    assert!(!unreadable.to_string().contains(SECRET_KEY));
    let mut restore = fs::metadata(&secret).unwrap().permissions();
    restore.set_mode(0o644);
    fs::set_permissions(&secret, restore).unwrap();
}

#[test]
fn empty_resolved_key_names_the_winning_source() {
    let root = TempRoot::new("empty");
    let base = root.0.join("a2a-lab.json");
    write(
        &base,
        r#"{"schema_version":1,"typesafe":{"api_key":"   "}}"#,
    );
    let empty_base = with_typesafe_env(None, || LoadedLabConfig::load(&base).unwrap_err());
    assert_eq!(invalid_field(&empty_base), "typesafe.api_key");
    assert!(empty_base.to_string().contains("must not be empty"));

    write(&base, &base_document(BASE_KEY));
    write(
        &root.0.join("a2a-lab.secret.json"),
        r#"{"typesafe":{"api_key":""}}"#,
    );
    let empty_secret = with_typesafe_env(None, || LoadedLabConfig::load(&base).unwrap_err());
    assert_eq!(invalid_field(&empty_secret), "config.secret");
    assert!(!empty_secret.to_string().contains(BASE_KEY));

    let recovered = with_typesafe_env(Some(ENV_KEY), || LoadedLabConfig::load(&base).unwrap());
    assert_eq!(recovered.typesafe_api_key(), Some(ENV_KEY));

    let empty_env = with_typesafe_env(Some(" \n"), || LoadedLabConfig::load(&base).unwrap_err());
    assert_eq!(invalid_field(&empty_env), "TYPESAFE_API_KEY");
    assert!(!empty_env.to_string().contains(BASE_KEY));
    assert!(!empty_env.to_string().contains(ENV_KEY));
}

#[test]
fn credentials_env_is_fallback_only_without_layered_key() {
    let root = TempRoot::new("fallback");
    let base = root.0.join("a2a-lab.json");
    write(
        &base,
        r#"{"schema_version":2,"image_decision":{"endpoint":"https://jev.example/v1/decide","model":"jev-1.14.0","credentials":{"env":"IMAGE_DECISION_FALLBACK_KEY"}},"semantic_indexing":{"decisions":{"credentials":{"env":"SEMANTIC_DECISION_FALLBACK_KEY"}},"embeddings":{"provider":"p","model":"m","space":{"dimensions":8,"normalization":"l2_unit","metric":"cosine","generation":1}},"limits":{"worker":{"max_batch_items":1,"max_concurrency":1,"max_queue_backlog":1,"max_retry_attempts":1,"job_deadline":"30s","retry_backoff":"5s"},"search":{"top_k":1,"candidate_limit":1,"max_neighbor_frames":1,"query_window":"1h"},"roi":{"min_edge_pixels":280,"max_edge_pixels":1568,"max_rois_per_frame":1},"storage":{"vector_index_path":"./semantic","max_disk_bytes":1024},"decisions":{"max_batch_items":1,"max_concurrency":1,"job_deadline":"45s","coverage_policy":"on_demand"}}}}"#,
    );
    let _guard = lock_env();
    set_env("TYPESAFE_API_KEY", None);
    set_env("IMAGE_DECISION_FALLBACK_KEY", Some("image-fallback-secret"));
    set_env(
        "SEMANTIC_DECISION_FALLBACK_KEY",
        Some("semantic-fallback-secret"),
    );
    let loaded = LoadedLabConfig::load(&base).unwrap();
    assert_eq!(loaded.typesafe_api_key(), None);
    assert_eq!(
        loaded
            .config
            .image_decision
            .as_ref()
            .unwrap()
            .credentials_env(),
        Some("IMAGE_DECISION_FALLBACK_KEY")
    );
    assert_eq!(
        loaded
            .config
            .semantic_indexing
            .as_ref()
            .unwrap()
            .decisions
            .credentials()
            .unwrap()
            .env_var(),
        "SEMANTIC_DECISION_FALLBACK_KEY"
    );
    assert_eq!(
        loaded.image_decision_api_key().unwrap().as_deref(),
        Some("image-fallback-secret")
    );
    assert_eq!(
        loaded.semantic_decision_api_key().unwrap().as_deref(),
        Some("semantic-fallback-secret")
    );

    set_env("TYPESAFE_API_KEY", Some(ENV_KEY));
    let layered = LoadedLabConfig::load(&base).unwrap();
    assert_eq!(
        layered.image_decision_api_key().unwrap().as_deref(),
        Some(ENV_KEY)
    );
    assert_eq!(
        layered.semantic_decision_api_key().unwrap().as_deref(),
        Some(ENV_KEY)
    );
    set_env("TYPESAFE_API_KEY", None);
    set_env("IMAGE_DECISION_FALLBACK_KEY", None);
    set_env("SEMANTIC_DECISION_FALLBACK_KEY", None);
}

#[test]
fn secret_json_files_are_gitignored() {
    let status = std::process::Command::new("git")
        .args(["check-ignore", "-q", "nested/a2a-lab.secret.json"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .unwrap();
    assert!(status.success());
    let ignored =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(".gitignore")).unwrap();
    assert!(ignored.contains("*.secret.json"));
}
