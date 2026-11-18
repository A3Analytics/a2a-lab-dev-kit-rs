//! Semantic identity, credential-reference, and limits editors.

use dioxus::prelude::*;

use super::chrome::{SectionCard, SelectField, TextField};
use super::document::{FieldErrors, restart_required, writable};
use crate::settings_semantic::{
    COVERAGE_POLICIES, DECISIONS_COVERAGE, DECISIONS_CREDENTIALS, DECISIONS_DEADLINE,
    DECISIONS_ENDPOINT, DECISIONS_MAX_BATCH, DECISIONS_MAX_CONCURRENCY, DECISIONS_MODEL,
    DEFAULT_JEV_API_ENDPOINT, DESCRIPTION_CREDENTIALS, DESCRIPTION_MODEL, DESCRIPTION_PROVIDER,
    DESCRIPTION_PROVIDERS, EMBEDDINGS_CREDENTIALS, EMBEDDINGS_DIMENSIONS, EMBEDDINGS_GENERATION,
    EMBEDDINGS_METRIC, EMBEDDINGS_MODEL, EMBEDDINGS_NORMALIZATION, EMBEDDINGS_PROVIDER,
    METRIC_VALUES, NORMALIZATION_VALUES, ROI_MAX_EDGE, ROI_MAX_PER_FRAME, ROI_MIN_EDGE,
    SEARCH_CANDIDATES, SEARCH_NEIGHBORS, SEARCH_SCORE_FLOOR, SEARCH_TOP_K, SEARCH_WINDOW,
    STORAGE_DISK, STORAGE_INDEX, STORAGE_SQLITE, SemanticDraft, WORKER_BACKOFF, WORKER_DEADLINE,
    WORKER_MAX_BATCH, WORKER_MAX_CONCURRENCY, WORKER_MAX_QUEUE, WORKER_MAX_RETRIES, added_semantic,
    reindex_required,
};

#[component]
pub fn SemanticPanel(
    draft: SemanticDraft,
    errors: FieldErrors,
    persistence: String,
    saving: bool,
) -> Element {
    let locked = !writable(&persistence) || saving;
    if !draft.present {
        return rsx! {
            SectionCard {
                title: "Semantic indexing",
                description: "Optional identity for descriptions, embeddings, and Jev decisions. Credential fields store environment-variable names only.",
                p { class: "settings-help", "semantic_indexing is omitted. Description-only operation stays available after add without creating embeddings." }
                button {
                    id: "settings-semantic-add",
                    class: "settings-button-outline",
                    r#type: "button",
                    disabled: locked,
                    onclick: move |_| super::edit_semantic(|semantic| *semantic = added_semantic()),
                    "Add semantic indexing"
                }
            }
        };
    }
    rsx! {
        SectionCard {
            title: "Semantic indexing",
            description: "Identity and credential-reference editors. Saved identity changes are not applied live.",
            SelectField {
                id: "settings-semantic-description-provider".to_string(),
                label: "semantic_indexing.description.provider".to_string(),
                help: "Optional. bedrock or openai. Leave unset to omit the key.".to_string(),
                error: error_text(&errors, DESCRIPTION_PROVIDER),
                restart: restart_required(DESCRIPTION_PROVIDER),
                reindex: reindex_required(DESCRIPTION_PROVIDER),
                locked,
                value: draft.description_provider.clone(),
                options: optional_options(DESCRIPTION_PROVIDERS, &draft.description_provider),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.description_provider = value),
            }
            TextField {
                id: "settings-semantic-description-model".to_string(),
                label: "semantic_indexing.description.model".to_string(),
                help: "Optional Luna model. Leave blank to omit the key.".to_string(),
                error: error_text(&errors, DESCRIPTION_MODEL),
                restart: restart_required(DESCRIPTION_MODEL),
                reindex: reindex_required(DESCRIPTION_MODEL),
                locked,
                value: draft.description_model.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.description_model = value),
            }
            TextField {
                id: "settings-semantic-description-credentials".to_string(),
                label: "semantic_indexing.description.credentials.env".to_string(),
                help: "Environment-variable name only. Values are never resolved or shown.".to_string(),
                error: error_text(&errors, DESCRIPTION_CREDENTIALS),
                restart: restart_required(DESCRIPTION_CREDENTIALS),
                locked,
                value: draft.description_credentials.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.description_credentials = value),
            }
        }
        if draft.embeddings_present {
            SectionCard {
                title: "Embeddings",
                description: "Shared embedding identity and space. Omitting embeddings keeps description-only mode.",
                TextField {
                    id: "settings-semantic-embeddings-provider".to_string(),
                    label: "semantic_indexing.embeddings.provider".to_string(),
                    help: "Operator-selected embedding provider id.".to_string(),
                    error: error_text(&errors, EMBEDDINGS_PROVIDER),
                    restart: restart_required(EMBEDDINGS_PROVIDER),
                    reindex: reindex_required(EMBEDDINGS_PROVIDER),
                    locked,
                    value: draft.embeddings_provider.clone(),
                    oninput: move |value| super::edit_semantic(move |semantic| semantic.embeddings_provider = value),
                }
                TextField {
                    id: "settings-semantic-embeddings-model".to_string(),
                    label: "semantic_indexing.embeddings.model".to_string(),
                    help: "Embedding model identity. Changing this requires semantic reindex.".to_string(),
                    error: error_text(&errors, EMBEDDINGS_MODEL),
                    restart: restart_required(EMBEDDINGS_MODEL),
                    reindex: reindex_required(EMBEDDINGS_MODEL),
                    locked,
                    value: draft.embeddings_model.clone(),
                    oninput: move |value| super::edit_semantic(move |semantic| semantic.embeddings_model = value),
                }
                TextField {
                    id: "settings-semantic-embeddings-dimensions".to_string(),
                    label: "semantic_indexing.embeddings.space.dimensions".to_string(),
                    help: "Vector width from 1 to 16384.".to_string(),
                    error: error_text(&errors, EMBEDDINGS_DIMENSIONS),
                    restart: restart_required(EMBEDDINGS_DIMENSIONS),
                    reindex: reindex_required(EMBEDDINGS_DIMENSIONS),
                    locked,
                    value: draft.space_dimensions.clone(),
                    oninput: move |value| super::edit_semantic(move |semantic| semantic.space_dimensions = value),
                }
                SelectField {
                    id: "settings-semantic-embeddings-normalization".to_string(),
                    label: "semantic_indexing.embeddings.space.normalization".to_string(),
                    help: "none or l2_unit.".to_string(),
                    error: error_text(&errors, EMBEDDINGS_NORMALIZATION),
                    restart: restart_required(EMBEDDINGS_NORMALIZATION),
                    reindex: reindex_required(EMBEDDINGS_NORMALIZATION),
                    locked,
                    value: draft.space_normalization.clone(),
                    options: required_options(NORMALIZATION_VALUES, &draft.space_normalization),
                    oninput: move |value| super::edit_semantic(move |semantic| semantic.space_normalization = value),
                }
                SelectField {
                    id: "settings-semantic-embeddings-metric".to_string(),
                    label: "semantic_indexing.embeddings.space.metric".to_string(),
                    help: "cosine, dot, or euclidean.".to_string(),
                    error: error_text(&errors, EMBEDDINGS_METRIC),
                    restart: restart_required(EMBEDDINGS_METRIC),
                    reindex: reindex_required(EMBEDDINGS_METRIC),
                    locked,
                    value: draft.space_metric.clone(),
                    options: required_options(METRIC_VALUES, &draft.space_metric),
                    oninput: move |value| super::edit_semantic(move |semantic| semantic.space_metric = value),
                }
                TextField {
                    id: "settings-semantic-embeddings-generation".to_string(),
                    label: "semantic_indexing.embeddings.space.generation".to_string(),
                    help: "Optional provenance generation. Leave blank to omit the key.".to_string(),
                    error: error_text(&errors, EMBEDDINGS_GENERATION),
                    restart: restart_required(EMBEDDINGS_GENERATION),
                    reindex: reindex_required(EMBEDDINGS_GENERATION),
                    locked,
                    value: draft.space_generation.clone(),
                    oninput: move |value| super::edit_semantic(move |semantic| semantic.space_generation = value),
                }
                TextField {
                    id: "settings-semantic-embeddings-credentials".to_string(),
                    label: "semantic_indexing.embeddings.credentials.env".to_string(),
                    help: "Environment-variable name only. Values are never resolved or shown.".to_string(),
                    error: error_text(&errors, EMBEDDINGS_CREDENTIALS),
                    restart: restart_required(EMBEDDINGS_CREDENTIALS),
                    locked,
                    value: draft.embeddings_credentials.clone(),
                    oninput: move |value| super::edit_semantic(move |semantic| semantic.embeddings_credentials = value),
                }
                button {
                    id: "settings-semantic-remove-embeddings",
                    class: "settings-button-outline",
                    r#type: "button",
                    disabled: locked,
                    onclick: move |_| super::edit_semantic(|semantic| semantic.embeddings_present = false),
                    "Remove embeddings"
                }
            }
        } else {
            SectionCard {
                title: "Embeddings",
                description: "Embeddings are omitted. Description-only operation does not materialize embedding defaults.",
                p { class: "settings-help", "Add embeddings to set provider, model, and space identity." }
                button {
                    id: "settings-semantic-add-embeddings",
                    class: "settings-button-outline",
                    r#type: "button",
                    disabled: locked,
                    onclick: move |_| super::edit_semantic(|semantic| semantic.embeddings_present = true),
                    "Add embeddings"
                }
            }
        }
        SectionCard {
            title: "Decisions",
            description: "Jev-compatible decision API settings.",
            TextField {
                id: "settings-semantic-decisions-endpoint".to_string(),
                label: "Jev API URL".to_string(),
                help: format!("Jev-compatible System One endpoint. Default: {DEFAULT_JEV_API_ENDPOINT}."),
                error: error_text(&errors, DECISIONS_ENDPOINT),
                restart: restart_required(DECISIONS_ENDPOINT),
                locked,
                value: draft.decisions_endpoint.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.decisions_endpoint = value),
            }
            TextField {
                id: "settings-semantic-decisions-model".to_string(),
                label: "semantic_indexing.decisions.model".to_string(),
                help: "Pinned Jev revision. Leave blank to omit the key.".to_string(),
                error: error_text(&errors, DECISIONS_MODEL),
                restart: restart_required(DECISIONS_MODEL),
                locked,
                value: draft.decisions_model.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.decisions_model = value),
            }
            TextField {
                id: "settings-semantic-decisions-credentials".to_string(),
                label: "semantic_indexing.decisions.credentials.env".to_string(),
                help: "Environment-variable name only. Values are never resolved or shown.".to_string(),
                error: error_text(&errors, DECISIONS_CREDENTIALS),
                restart: restart_required(DECISIONS_CREDENTIALS),
                locked,
                value: draft.decisions_credentials.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.decisions_credentials = value),
            }
        }
        LimitsPanel { draft: draft.clone(), errors: errors.clone(), locked }
    }
}

#[component]
fn LimitsPanel(draft: SemanticDraft, errors: FieldErrors, locked: bool) -> Element {
    rsx! {
        SectionCard {
            title: "Worker limits",
            description: "Indexing worker budgets. Saved changes require an agent restart and are not applied live.",
            TextField {
                id: "settings-semantic-worker-max-batch".to_string(),
                label: WORKER_MAX_BATCH.to_string(),
                help: "Positive batch size.".to_string(),
                error: error_text(&errors, WORKER_MAX_BATCH),
                restart: restart_required(WORKER_MAX_BATCH),
                locked,
                value: draft.limits.worker_max_batch_items.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.worker_max_batch_items = value),
            }
            TextField {
                id: "settings-semantic-worker-max-concurrency".to_string(),
                label: WORKER_MAX_CONCURRENCY.to_string(),
                help: "Positive worker concurrency.".to_string(),
                error: error_text(&errors, WORKER_MAX_CONCURRENCY),
                restart: restart_required(WORKER_MAX_CONCURRENCY),
                locked,
                value: draft.limits.worker_max_concurrency.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.worker_max_concurrency = value),
            }
            TextField {
                id: "settings-semantic-worker-max-queue".to_string(),
                label: WORKER_MAX_QUEUE.to_string(),
                help: "Positive queue backlog.".to_string(),
                error: error_text(&errors, WORKER_MAX_QUEUE),
                restart: restart_required(WORKER_MAX_QUEUE),
                locked,
                value: draft.limits.worker_max_queue_backlog.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.worker_max_queue_backlog = value),
            }
            TextField {
                id: "settings-semantic-worker-max-retries".to_string(),
                label: WORKER_MAX_RETRIES.to_string(),
                help: "Positive retry budget.".to_string(),
                error: error_text(&errors, WORKER_MAX_RETRIES),
                restart: restart_required(WORKER_MAX_RETRIES),
                locked,
                value: draft.limits.worker_max_retry_attempts.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.worker_max_retry_attempts = value),
            }
            TextField {
                id: "settings-semantic-worker-deadline".to_string(),
                label: WORKER_DEADLINE.to_string(),
                help: "Positive whole-second duration such as 30s.".to_string(),
                error: error_text(&errors, WORKER_DEADLINE),
                restart: restart_required(WORKER_DEADLINE),
                locked,
                value: draft.limits.worker_job_deadline.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.worker_job_deadline = value),
            }
            TextField {
                id: "settings-semantic-worker-backoff".to_string(),
                label: WORKER_BACKOFF.to_string(),
                help: "Positive whole-second retry backoff such as 5s.".to_string(),
                error: error_text(&errors, WORKER_BACKOFF),
                restart: restart_required(WORKER_BACKOFF),
                locked,
                value: draft.limits.worker_retry_backoff.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.worker_retry_backoff = value),
            }
        }
        SectionCard {
            title: "Search limits",
            description: "Retrieval budgets. Saved changes require an agent restart and are not applied live.",
            TextField {
                id: "settings-semantic-search-top-k".to_string(),
                label: SEARCH_TOP_K.to_string(),
                help: "Positive 1..=100.".to_string(),
                error: error_text(&errors, SEARCH_TOP_K),
                restart: restart_required(SEARCH_TOP_K),
                locked,
                value: draft.limits.search_top_k.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.search_top_k = value),
            }
            TextField {
                id: "settings-semantic-search-candidates".to_string(),
                label: SEARCH_CANDIDATES.to_string(),
                help: "Positive 1..=10000.".to_string(),
                error: error_text(&errors, SEARCH_CANDIDATES),
                restart: restart_required(SEARCH_CANDIDATES),
                locked,
                value: draft.limits.search_candidate_limit.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.search_candidate_limit = value),
            }
            TextField {
                id: "settings-semantic-search-neighbors".to_string(),
                label: SEARCH_NEIGHBORS.to_string(),
                help: "Positive 1..=30.".to_string(),
                error: error_text(&errors, SEARCH_NEIGHBORS),
                restart: restart_required(SEARCH_NEIGHBORS),
                locked,
                value: draft.limits.search_max_neighbor_frames.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.search_max_neighbor_frames = value),
            }
            TextField {
                id: "settings-semantic-search-window".to_string(),
                label: SEARCH_WINDOW.to_string(),
                help: "Positive whole-second window such as 1h.".to_string(),
                error: error_text(&errors, SEARCH_WINDOW),
                restart: restart_required(SEARCH_WINDOW),
                locked,
                value: draft.limits.search_query_window.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.search_query_window = value),
            }
            TextField {
                id: "settings-semantic-search-score-floor".to_string(),
                label: SEARCH_SCORE_FLOOR.to_string(),
                help: "Optional finite score between -1 and 1. Leave blank to omit the key.".to_string(),
                error: error_text(&errors, SEARCH_SCORE_FLOOR),
                restart: restart_required(SEARCH_SCORE_FLOOR),
                locked,
                value: draft.limits.search_score_floor.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.search_score_floor = value),
            }
        }
        SectionCard {
            title: "ROI limits",
            description: "Saved-frame ROI extraction bounds. max_edge_pixels must be at least min_edge_pixels.",
            TextField {
                id: "settings-semantic-roi-min-edge".to_string(),
                label: ROI_MIN_EDGE.to_string(),
                help: "Positive minimum edge in pixels.".to_string(),
                error: error_text(&errors, ROI_MIN_EDGE),
                restart: restart_required(ROI_MIN_EDGE),
                locked,
                value: draft.limits.roi_min_edge_pixels.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.roi_min_edge_pixels = value),
            }
            TextField {
                id: "settings-semantic-roi-max-edge".to_string(),
                label: ROI_MAX_EDGE.to_string(),
                help: "Positive maximum edge; must be >= min_edge_pixels.".to_string(),
                error: error_text(&errors, ROI_MAX_EDGE),
                restart: restart_required(ROI_MAX_EDGE),
                locked,
                value: draft.limits.roi_max_edge_pixels.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.roi_max_edge_pixels = value),
            }
            TextField {
                id: "settings-semantic-roi-max-per-frame".to_string(),
                label: ROI_MAX_PER_FRAME.to_string(),
                help: "Positive ROI count per frame.".to_string(),
                error: error_text(&errors, ROI_MAX_PER_FRAME),
                restart: restart_required(ROI_MAX_PER_FRAME),
                locked,
                value: draft.limits.roi_max_rois_per_frame.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.roi_max_rois_per_frame = value),
            }
        }
        SectionCard {
            title: "Storage limits",
            description: "Index paths stay as typed. Path and index-storage identity changes require semantic reindex and remain pending restart.",
            TextField {
                id: "settings-semantic-storage-sqlite".to_string(),
                label: STORAGE_SQLITE.to_string(),
                help: "Optional sqlite path exactly as entered. Leave blank to omit the key.".to_string(),
                error: error_text(&errors, STORAGE_SQLITE),
                restart: restart_required(STORAGE_SQLITE),
                reindex: reindex_required(STORAGE_SQLITE),
                locked,
                value: draft.limits.storage_sqlite_path.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.storage_sqlite_path = value),
            }
            TextField {
                id: "settings-semantic-storage-index".to_string(),
                label: STORAGE_INDEX.to_string(),
                help: "Vector index path exactly as entered. Relative and absolute paths are not resolved.".to_string(),
                error: error_text(&errors, STORAGE_INDEX),
                restart: restart_required(STORAGE_INDEX),
                reindex: reindex_required(STORAGE_INDEX),
                locked,
                value: draft.limits.storage_vector_index_path.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.storage_vector_index_path = value),
            }
            TextField {
                id: "settings-semantic-storage-disk".to_string(),
                label: STORAGE_DISK.to_string(),
                help: "Positive on-disk budget in bytes.".to_string(),
                error: error_text(&errors, STORAGE_DISK),
                restart: restart_required(STORAGE_DISK),
                locked,
                value: draft.limits.storage_max_disk_bytes.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.storage_max_disk_bytes = value),
            }
        }
        SectionCard {
            title: "Decision limits",
            description: "Jev evaluation budgets. Saved changes require an agent restart and are not applied live.",
            TextField {
                id: "settings-semantic-decision-limits-max-batch".to_string(),
                label: DECISIONS_MAX_BATCH.to_string(),
                help: "Positive decision batch size.".to_string(),
                error: error_text(&errors, DECISIONS_MAX_BATCH),
                restart: restart_required(DECISIONS_MAX_BATCH),
                locked,
                value: draft.limits.decisions_max_batch_items.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.decisions_max_batch_items = value),
            }
            TextField {
                id: "settings-semantic-decision-limits-max-concurrency".to_string(),
                label: DECISIONS_MAX_CONCURRENCY.to_string(),
                help: "Positive decision concurrency.".to_string(),
                error: error_text(&errors, DECISIONS_MAX_CONCURRENCY),
                restart: restart_required(DECISIONS_MAX_CONCURRENCY),
                locked,
                value: draft.limits.decisions_max_concurrency.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.decisions_max_concurrency = value),
            }
            TextField {
                id: "settings-semantic-decision-limits-deadline".to_string(),
                label: DECISIONS_DEADLINE.to_string(),
                help: "Positive whole-second duration such as 45s.".to_string(),
                error: error_text(&errors, DECISIONS_DEADLINE),
                restart: restart_required(DECISIONS_DEADLINE),
                locked,
                value: draft.limits.decisions_job_deadline.clone(),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.decisions_job_deadline = value),
            }
            SelectField {
                id: "settings-semantic-decision-limits-coverage".to_string(),
                label: DECISIONS_COVERAGE.to_string(),
                help: "all_saved_frames, on_demand, or sampled.".to_string(),
                error: error_text(&errors, DECISIONS_COVERAGE),
                restart: restart_required(DECISIONS_COVERAGE),
                locked,
                value: draft.limits.decisions_coverage_policy.clone(),
                options: required_options(COVERAGE_POLICIES, &draft.limits.decisions_coverage_policy),
                oninput: move |value| super::edit_semantic(move |semantic| semantic.limits.decisions_coverage_policy = value),
            }
        }
    }
}

fn error_text(errors: &FieldErrors, field: &str) -> String {
    errors.get(field).cloned().unwrap_or_default()
}

fn optional_options(values: &[&str], current: &str) -> Vec<(String, String)> {
    let mut options = vec![(String::new(), "Not set (optional)".to_owned())];
    for value in values {
        options.push(((*value).to_owned(), (*value).to_owned()));
    }
    if !current.is_empty() && !values.contains(&current) {
        options.push((current.to_owned(), current.to_owned()));
    }
    options
}

fn required_options(values: &[&str], current: &str) -> Vec<(String, String)> {
    let mut options: Vec<(String, String)> = values
        .iter()
        .map(|value| ((*value).to_owned(), (*value).to_owned()))
        .collect();
    if !current.is_empty() && !values.contains(&current) {
        options.push((current.to_owned(), current.to_owned()));
    }
    options
}
