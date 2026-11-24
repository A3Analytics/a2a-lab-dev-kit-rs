//! Restart and reindex paths relative to the running configuration baseline.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

const LIVE_CADENCE: &[&str] = &["videos.fps", "videos.dvr.save_interval"];

const REINDEX: &[&str] = &[
    "semantic_indexing.description.model",
    "semantic_indexing.description.provider",
    "semantic_indexing.embeddings.model",
    "semantic_indexing.embeddings.provider",
    "semantic_indexing.embeddings.space",
    "semantic_indexing.limits.storage.sqlite_path",
    "semantic_indexing.limits.storage.vector_index_path",
];

/// Changed persisted paths that need a process restart and/or semantic reindex.
#[must_use]
pub(crate) fn pending_restart_and_reindex(
    baseline: &Value,
    current: &Value,
) -> (Vec<String>, Vec<String>) {
    let restart: Vec<String> = changed_paths(baseline, current)
        .into_iter()
        .filter(|path| !is_live_cadence(path))
        .collect();
    let reindex = restart
        .iter()
        .filter(|path| is_reindex(path))
        .cloned()
        .collect();
    (restart, reindex)
}

fn changed_paths(baseline: &Value, current: &Value) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    collect("", baseline, current, &mut paths);
    paths
}

fn collect(path: &str, left: &Value, right: &Value, out: &mut BTreeSet<String>) {
    if left == right {
        return;
    }
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => walk_objects(path, left, right, out),
        (Value::Null, Value::Object(right)) => walk_objects(path, &Map::new(), right, out),
        (Value::Object(left), Value::Null) => walk_objects(path, left, &Map::new(), out),
        (Value::Array(left), Value::Array(right)) => walk_arrays(path, left, right, out),
        (Value::Null, Value::Array(right)) => walk_arrays(path, &[], right, out),
        (Value::Array(left), Value::Null) => walk_arrays(path, left, &[], out),
        _ if !path.is_empty() => {
            out.insert(path.to_owned());
        }
        _ => {}
    }
}

fn walk_objects(
    path: &str,
    left: &Map<String, Value>,
    right: &Map<String, Value>,
    out: &mut BTreeSet<String>,
) {
    let mut keys: Vec<&String> = left.keys().chain(right.keys()).collect();
    keys.sort();
    keys.dedup();
    for key in keys {
        collect(
            &join(path, key),
            left.get(key).unwrap_or(&Value::Null),
            right.get(key).unwrap_or(&Value::Null),
            out,
        );
    }
}

fn walk_arrays(path: &str, left: &[Value], right: &[Value], out: &mut BTreeSet<String>) {
    let len = left.len().max(right.len());
    for index in 0..len {
        collect(
            &format!("{path}.{index}"),
            left.get(index).unwrap_or(&Value::Null),
            right.get(index).unwrap_or(&Value::Null),
            out,
        );
    }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

fn is_live_cadence(path: &str) -> bool {
    LIVE_CADENCE.contains(&path)
}

fn is_reindex(path: &str) -> bool {
    REINDEX.iter().any(|prefix| matches_prefix(path, prefix))
        || path.contains(".semantic_indexing.description_model")
        || path.contains(".semantic_indexing.embedding_model")
}

fn matches_prefix(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::pending_restart_and_reindex;
    use serde_json::json;

    #[test]
    fn live_cadence_changes_are_not_pending() {
        let baseline = json!({"videos":{"fps":5,"dvr":{"save_interval":"1s","retention":"1h"}}});
        let current = json!({"videos":{"fps":12,"dvr":{"save_interval":"2s","retention":"1h"}}});
        let (restart, reindex) = pending_restart_and_reindex(&baseline, &current);
        assert!(restart.is_empty());
        assert!(reindex.is_empty());
    }

    #[test]
    fn preamble_changes_are_restart_required() {
        let baseline = json!({"chat":{"model":"keep-me"}});
        let current = json!({"chat":{"model":"keep-me","preamble":""}});
        let (restart, reindex) = pending_restart_and_reindex(&baseline, &current);
        assert_eq!(restart, vec!["chat.preamble"]);
        assert!(reindex.is_empty());
    }

    #[test]
    fn other_fields_are_restart_required() {
        let baseline = json!({"chat":{"model":"keep-me"},"videos":{"live_retention":"30s"}});
        let current = json!({"chat":{"model":"other"},"videos":{"live_retention":"45s"}});
        let (restart, reindex) = pending_restart_and_reindex(&baseline, &current);
        assert_eq!(restart, vec!["chat.model", "videos.live_retention"]);
        assert!(reindex.is_empty());
    }

    #[test]
    fn semantic_identity_is_restart_and_reindex() {
        let baseline = json!({
            "semantic_indexing": {
                "description": {"model":"luna","provider":"bedrock"},
                "embeddings": {
                    "provider":"p",
                    "model":"m",
                    "space":{"dimensions":4,"normalization":"l2_unit","metric":"cosine"}
                },
                "limits":{"storage":{"sqlite_path":"./db","vector_index_path":"./idx"}}
            }
        });
        let current = json!({
            "semantic_indexing": {
                "description": {"model":"luna-2","provider":"bedrock"},
                "embeddings": {
                    "provider":"p",
                    "model":"m2",
                    "space":{"dimensions":8,"normalization":"l2_unit","metric":"cosine"}
                },
                "limits":{"storage":{"sqlite_path":"./db2","vector_index_path":"./idx2"}}
            }
        });
        let (restart, reindex) = pending_restart_and_reindex(&baseline, &current);
        assert_eq!(
            restart,
            vec![
                "semantic_indexing.description.model",
                "semantic_indexing.embeddings.model",
                "semantic_indexing.embeddings.space.dimensions",
                "semantic_indexing.limits.storage.sqlite_path",
                "semantic_indexing.limits.storage.vector_index_path",
            ]
        );
        assert_eq!(reindex, restart);
    }

    #[test]
    fn source_schedule_is_restart_and_models_are_reindex() {
        let baseline = json!({
            "videos": {
                "sources": [{
                    "id": "camera",
                    "semantic_indexing": {
                        "description": "every_saved_frame",
                        "description_model": "luna",
                        "embeddings": "1s",
                        "embedding_model": "m"
                    }
                }]
            }
        });
        let current = json!({
            "videos": {
                "sources": [{
                    "id": "camera",
                    "semantic_indexing": {
                        "description": "2s",
                        "description_model": "luna-2",
                        "embeddings": "5s",
                        "embedding_model": "m"
                    }
                }]
            }
        });
        let (restart, reindex) = pending_restart_and_reindex(&baseline, &current);
        assert_eq!(
            restart,
            vec![
                "videos.sources.0.semantic_indexing.description",
                "videos.sources.0.semantic_indexing.description_model",
                "videos.sources.0.semantic_indexing.embeddings",
            ]
        );
        assert_eq!(
            reindex,
            vec!["videos.sources.0.semantic_indexing.description_model"]
        );
    }
}
