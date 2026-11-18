//! Hardware-free tests for the surgical a2a-lab.json file store.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use a2a_lab_dev_kit::{ConfigPersistence, LabConfigStore};
use serde_json::{Value, json};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

const ORIGINAL: &str = r#"{"schema_version":1,"chat":{"provider":"openai","model":"keep-me"},"videos":{"sources":[{"id":"camera","format":"yuyv","device_index":6}],"fps":5,"dvr":{"storage_path":"./dvr"}}}"#;

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "a2a-lab-config-store-{}-{}-{}",
            std::process::id(),
            unique,
            nanos
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn write_config(&self, body: &str) -> PathBuf {
        let path = self.0.join("a2a-lab.json");
        fs::write(&path, body).unwrap();
        path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn bytes(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
}

#[test]
fn open_exposes_original_document_and_deterministic_revision() {
    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let store = LabConfigStore::open(&path).unwrap();
    let first = store.snapshot().unwrap();
    let second = store.snapshot().unwrap();
    assert_eq!(first.persistence, ConfigPersistence::Writable);
    assert_eq!(first.persistence.as_str(), "writable");
    assert_eq!(
        first.document,
        serde_json::from_str::<Value>(ORIGINAL).unwrap()
    );
    assert_eq!(
        first.document["videos"]["sources"][0]["format"],
        json!("yuyv")
    );
    assert!(first.document.get("logging").is_none());
    let revision = first.revision.expect("revision");
    assert!(revision.starts_with("sha256:"));
    assert_eq!(revision.len(), "sha256:".len() + 64);
    assert_eq!(second.revision.as_deref(), Some(revision.as_str()));
    assert_eq!(first.loaded.unwrap().config.schema_version, 1);
}

#[test]
fn open_or_create_writes_minimal_valid_config_and_preserves_existing_file() {
    let root = TempRoot::new();
    let path = root.0.join("created.json");
    let store = LabConfigStore::open_or_create(&path).unwrap();
    assert_eq!(
        store.snapshot().unwrap().document,
        json!({"schema_version": 1})
    );

    fs::write(&path, ORIGINAL).unwrap();
    let existing = LabConfigStore::open_or_create(&path).unwrap();
    assert_eq!(
        existing.snapshot().unwrap().document,
        serde_json::from_str::<Value>(ORIGINAL).unwrap()
    );
}

#[test]
fn surgical_patch_preserves_aliases_and_omitted_keys() {
    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let store = LabConfigStore::open(&path).unwrap();
    let revision = store.snapshot().unwrap().revision.unwrap();
    let updated = store
        .patch(&revision, &json!({ "videos": { "fps": 12 } }))
        .unwrap();
    let on_disk: Value = serde_json::from_slice(&bytes(&path)).unwrap();
    assert_eq!(updated.document, on_disk);
    assert_eq!(on_disk["videos"]["fps"], json!(12));
    assert_eq!(on_disk["videos"]["sources"][0]["format"], json!("yuyv"));
    assert_eq!(on_disk["videos"]["sources"][0]["device_index"], json!(6));
    assert_eq!(on_disk["chat"]["model"], json!("keep-me"));
    assert!(on_disk.get("logging").is_none());
    assert!(on_disk["videos"]["dvr"].get("retention").is_none());
    assert!(on_disk["videos"]["sources"][0].get("name").is_none());
}

#[test]
fn surgical_source_array_patch_preserves_identity_and_aliases() {
    let root = TempRoot::new();
    let path = root.write_config(
        r#"{"schema_version":1,"videos":{"sources":[{"id":"camera","format":"yuyv","device_index":6},{"id":"detail","name":"Detail","camera_id":"emeet"}],"fps":5,"dvr":{"storage_path":"./dvr"}}}"#,
    );
    let store = LabConfigStore::open(&path).unwrap();
    let revision = store.snapshot().unwrap().revision.unwrap();
    store
        .patch(
            &revision,
            &json!({ "videos": { "sources": [{}, { "name": "Closer" }] } }),
        )
        .unwrap();
    let on_disk: Value = serde_json::from_slice(&bytes(&path)).unwrap();
    assert_eq!(on_disk["videos"]["sources"][0]["id"], json!("camera"));
    assert_eq!(on_disk["videos"]["sources"][0]["format"], json!("yuyv"));
    assert_eq!(on_disk["videos"]["sources"][0]["device_index"], json!(6));
    assert_eq!(on_disk["videos"]["sources"][1]["id"], json!("detail"));
    assert_eq!(on_disk["videos"]["sources"][1]["name"], json!("Closer"));
    assert_eq!(on_disk["videos"]["sources"][1]["camera_id"], json!("emeet"));
    assert_eq!(on_disk["videos"]["dvr"]["storage_path"], json!("./dvr"));
}

#[test]
fn invalid_candidate_leaves_original_bytes_unchanged() {
    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let before = bytes(&path);
    let store = LabConfigStore::open(&path).unwrap();
    let revision = store.snapshot().unwrap().revision.unwrap();
    let error = store
        .patch(
            &revision,
            &json!({ "chat": { "provider": "not-a-provider" } }),
        )
        .unwrap_err();
    assert_eq!(error.kind(), "invalid");
    assert_eq!(bytes(&path), before);
}

#[test]
fn json_null_removes_optional_key_without_default() {
    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let store = LabConfigStore::open(&path).unwrap();
    let revision = store.snapshot().unwrap().revision.unwrap();
    let updated = store
        .patch(&revision, &json!({ "chat": Value::Null }))
        .unwrap();
    let on_disk: Value = serde_json::from_slice(&bytes(&path)).unwrap();
    assert!(updated.document.get("chat").is_none());
    assert!(on_disk.get("chat").is_none());
    assert_eq!(on_disk["schema_version"], json!(1));
    assert_eq!(on_disk["videos"]["fps"], json!(5));
}

#[test]
fn nested_null_removes_optional_field_only() {
    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let store = LabConfigStore::open(&path).unwrap();
    let revision = store.snapshot().unwrap().revision.unwrap();
    store
        .patch(&revision, &json!({ "chat": { "model": Value::Null } }))
        .unwrap();
    let on_disk: Value = serde_json::from_slice(&bytes(&path)).unwrap();
    assert_eq!(on_disk["chat"]["provider"], json!("openai"));
    assert!(on_disk["chat"].get("model").is_none());
}

#[test]
fn stale_revision_is_distinct_and_does_not_mutate() {
    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let store = LabConfigStore::open(&path).unwrap();
    let revision = store.snapshot().unwrap().revision.unwrap();
    store
        .patch(&revision, &json!({ "videos": { "fps": 8 } }))
        .unwrap();
    let before = bytes(&path);
    let error = store
        .patch(&revision, &json!({ "videos": { "fps": 9 } }))
        .unwrap_err();
    assert_eq!(error.kind(), "stale_revision");
    assert_ne!(error.kind(), "invalid");
    assert_ne!(error.kind(), "path");
    assert_ne!(error.kind(), "permission");
    assert_eq!(bytes(&path), before);
}

#[test]
fn unconfigured_store_rejects_patch_and_exposes_state() {
    let store = LabConfigStore::unconfigured();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.persistence, ConfigPersistence::Unconfigured);
    assert_eq!(snapshot.persistence.as_str(), "unconfigured");
    assert!(snapshot.revision.is_none());
    assert!(snapshot.loaded.is_none());
    let error = store
        .patch("sha256:never", &json!({ "videos": { "fps": 3 } }))
        .unwrap_err();
    assert_eq!(error.kind(), "unconfigured");
}

#[test]
fn read_only_store_rejects_patch_without_mutation() {
    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let store = LabConfigStore::open(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.persistence, ConfigPersistence::ReadOnly);
    assert_eq!(snapshot.persistence.as_str(), "read_only");
    let before = bytes(&path);
    let error = store
        .patch(
            snapshot.revision.as_deref().unwrap(),
            &json!({ "videos": { "fps": 30 } }),
        )
        .unwrap_err();
    assert_eq!(error.kind(), "read_only");
    assert_eq!(bytes(&path), before);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn path_and_permission_errors_are_distinct() {
    let missing = LabConfigStore::open("/no/such/a2a-lab-config-store.json").unwrap_err();
    assert_eq!(missing.kind(), "path");
    let empty = LabConfigStore::open("").unwrap_err();
    assert_eq!(empty.kind(), "path");

    let root = TempRoot::new();
    let path = root.write_config(ORIGINAL);
    let store = LabConfigStore::open(&path).unwrap();
    let revision = store.snapshot().unwrap().revision.unwrap();
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o555)).unwrap();
    let before = bytes(&path);
    let error = store
        .patch(&revision, &json!({ "videos": { "fps": 11 } }))
        .unwrap_err();
    assert_eq!(error.kind(), "permission");
    assert_ne!(error.kind(), "path");
    assert_ne!(error.kind(), "stale_revision");
    assert_eq!(bytes(&path), before);
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn utf8_and_missing_file_are_path_errors() {
    let root = TempRoot::new();
    let path = root.0.join("bad utf8.json");
    fs::write(&path, [0xFF, 0xFE, b'{']).unwrap();
    let error = LabConfigStore::open(&path).unwrap_err();
    assert_eq!(error.kind(), "path");
}
