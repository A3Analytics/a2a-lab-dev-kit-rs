//! Parser-backed semantic limit drafts and surgical PATCH fragments.

use serde_json::{Map, Value, json};

use super::{
    FieldErrors, SemanticDraft, number_or_empty, push_optional_string, push_optional_u32,
    string_or_empty, validate_choice,
};

pub const WORKER_MAX_BATCH: &str = "semantic_indexing.limits.worker.max_batch_items";
pub const WORKER_MAX_CONCURRENCY: &str = "semantic_indexing.limits.worker.max_concurrency";
pub const WORKER_MAX_QUEUE: &str = "semantic_indexing.limits.worker.max_queue_backlog";
pub const WORKER_MAX_RETRIES: &str = "semantic_indexing.limits.worker.max_retry_attempts";
pub const WORKER_DEADLINE: &str = "semantic_indexing.limits.worker.job_deadline";
pub const WORKER_BACKOFF: &str = "semantic_indexing.limits.worker.retry_backoff";
pub const SEARCH_TOP_K: &str = "semantic_indexing.limits.search.top_k";
pub const SEARCH_CANDIDATES: &str = "semantic_indexing.limits.search.candidate_limit";
pub const SEARCH_NEIGHBORS: &str = "semantic_indexing.limits.search.max_neighbor_frames";
pub const SEARCH_WINDOW: &str = "semantic_indexing.limits.search.query_window";
pub const SEARCH_SCORE_FLOOR: &str = "semantic_indexing.limits.search.score_floor";
pub const ROI_MIN_EDGE: &str = "semantic_indexing.limits.roi.min_edge_pixels";
pub const ROI_MAX_EDGE: &str = "semantic_indexing.limits.roi.max_edge_pixels";
pub const ROI_MAX_PER_FRAME: &str = "semantic_indexing.limits.roi.max_rois_per_frame";
pub const STORAGE_SQLITE: &str = "semantic_indexing.limits.storage.sqlite_path";
pub const STORAGE_INDEX: &str = "semantic_indexing.limits.storage.vector_index_path";
pub const STORAGE_DISK: &str = "semantic_indexing.limits.storage.max_disk_bytes";
pub const DECISIONS_MAX_BATCH: &str = "semantic_indexing.limits.decisions.max_batch_items";
pub const DECISIONS_MAX_CONCURRENCY: &str = "semantic_indexing.limits.decisions.max_concurrency";
pub const DECISIONS_DEADLINE: &str = "semantic_indexing.limits.decisions.job_deadline";
pub const DECISIONS_COVERAGE: &str = "semantic_indexing.limits.decisions.coverage_policy";

pub(crate) const COVERAGE_POLICIES: &[&str] = &["all_saved_frames", "on_demand", "sampled"];
const MAX_TOP_K: u32 = 100;
const MAX_CANDIDATES: u32 = 10_000;
const MAX_NEIGHBORS: u32 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticLimitsDraft {
    pub worker_max_batch_items: String,
    pub worker_max_concurrency: String,
    pub worker_max_queue_backlog: String,
    pub worker_max_retry_attempts: String,
    pub worker_job_deadline: String,
    pub worker_retry_backoff: String,
    pub search_top_k: String,
    pub search_candidate_limit: String,
    pub search_max_neighbor_frames: String,
    pub search_query_window: String,
    pub search_score_floor: String,
    pub roi_min_edge_pixels: String,
    pub roi_max_edge_pixels: String,
    pub roi_max_rois_per_frame: String,
    pub storage_sqlite_path: String,
    pub storage_vector_index_path: String,
    pub storage_max_disk_bytes: String,
    pub decisions_max_batch_items: String,
    pub decisions_max_concurrency: String,
    pub decisions_job_deadline: String,
    pub decisions_coverage_policy: String,
}

impl SemanticLimitsDraft {
    pub fn empty() -> Self {
        Self {
            worker_max_batch_items: String::new(),
            worker_max_concurrency: String::new(),
            worker_max_queue_backlog: String::new(),
            worker_max_retry_attempts: String::new(),
            worker_job_deadline: String::new(),
            worker_retry_backoff: String::new(),
            search_top_k: String::new(),
            search_candidate_limit: String::new(),
            search_max_neighbor_frames: String::new(),
            search_query_window: String::new(),
            search_score_floor: String::new(),
            roi_min_edge_pixels: String::new(),
            roi_max_edge_pixels: String::new(),
            roi_max_rois_per_frame: String::new(),
            storage_sqlite_path: String::new(),
            storage_vector_index_path: String::new(),
            storage_max_disk_bytes: String::new(),
            decisions_max_batch_items: String::new(),
            decisions_max_concurrency: String::new(),
            decisions_job_deadline: String::new(),
            decisions_coverage_policy: String::new(),
        }
    }

    pub(super) fn bootstrap() -> Self {
        Self {
            worker_max_batch_items: "1".to_owned(),
            worker_max_concurrency: "1".to_owned(),
            worker_max_queue_backlog: "1".to_owned(),
            worker_max_retry_attempts: "1".to_owned(),
            worker_job_deadline: "30s".to_owned(),
            worker_retry_backoff: "5s".to_owned(),
            search_top_k: "1".to_owned(),
            search_candidate_limit: "1".to_owned(),
            search_max_neighbor_frames: "1".to_owned(),
            search_query_window: "1h".to_owned(),
            search_score_floor: String::new(),
            roi_min_edge_pixels: "280".to_owned(),
            roi_max_edge_pixels: "1568".to_owned(),
            roi_max_rois_per_frame: "1".to_owned(),
            storage_sqlite_path: String::new(),
            storage_vector_index_path: "./semantic".to_owned(),
            storage_max_disk_bytes: "1024".to_owned(),
            decisions_max_batch_items: "1".to_owned(),
            decisions_max_concurrency: "1".to_owned(),
            decisions_job_deadline: "45s".to_owned(),
            decisions_coverage_policy: "on_demand".to_owned(),
        }
    }
}

pub(super) fn from_document(limits: Option<&Value>) -> SemanticLimitsDraft {
    let worker = limits.and_then(|value| value.get("worker"));
    let search = limits.and_then(|value| value.get("search"));
    let roi = limits.and_then(|value| value.get("roi"));
    let storage = limits.and_then(|value| value.get("storage"));
    let decisions = limits.and_then(|value| value.get("decisions"));
    SemanticLimitsDraft {
        worker_max_batch_items: number_or_empty(
            worker.and_then(|value| value.get("max_batch_items")),
        ),
        worker_max_concurrency: number_or_empty(
            worker.and_then(|value| value.get("max_concurrency")),
        ),
        worker_max_queue_backlog: number_or_empty(
            worker.and_then(|value| value.get("max_queue_backlog")),
        ),
        worker_max_retry_attempts: number_or_empty(
            worker.and_then(|value| value.get("max_retry_attempts")),
        ),
        worker_job_deadline: string_or_empty(worker.and_then(|value| value.get("job_deadline"))),
        worker_retry_backoff: string_or_empty(worker.and_then(|value| value.get("retry_backoff"))),
        search_top_k: number_or_empty(search.and_then(|value| value.get("top_k"))),
        search_candidate_limit: number_or_empty(
            search.and_then(|value| value.get("candidate_limit")),
        ),
        search_max_neighbor_frames: number_or_empty(
            search.and_then(|value| value.get("max_neighbor_frames")),
        ),
        search_query_window: string_or_empty(search.and_then(|value| value.get("query_window"))),
        search_score_floor: number_or_empty(search.and_then(|value| value.get("score_floor"))),
        roi_min_edge_pixels: number_or_empty(roi.and_then(|value| value.get("min_edge_pixels"))),
        roi_max_edge_pixels: number_or_empty(roi.and_then(|value| value.get("max_edge_pixels"))),
        roi_max_rois_per_frame: number_or_empty(
            roi.and_then(|value| value.get("max_rois_per_frame")),
        ),
        storage_sqlite_path: string_or_empty(storage.and_then(|value| value.get("sqlite_path"))),
        storage_vector_index_path: string_or_empty(
            storage.and_then(|value| value.get("vector_index_path")),
        ),
        storage_max_disk_bytes: number_or_empty(
            storage.and_then(|value| value.get("max_disk_bytes")),
        ),
        decisions_max_batch_items: number_or_empty(
            decisions.and_then(|value| value.get("max_batch_items")),
        ),
        decisions_max_concurrency: number_or_empty(
            decisions.and_then(|value| value.get("max_concurrency")),
        ),
        decisions_job_deadline: string_or_empty(
            decisions.and_then(|value| value.get("job_deadline")),
        ),
        decisions_coverage_policy: string_or_empty(
            decisions.and_then(|value| value.get("coverage_policy")),
        ),
    }
}

pub(super) fn change(
    current: Option<&Value>,
    draft: &SemanticDraft,
    nested: &mut Map<String, Value>,
) {
    let limits = current.and_then(|value| value.get("limits"));
    let mut object = Map::new();
    worker_change(
        limits.and_then(|value| value.get("worker")),
        &draft.limits,
        &mut object,
    );
    search_change(
        limits.and_then(|value| value.get("search")),
        &draft.limits,
        &mut object,
    );
    roi_change(
        limits.and_then(|value| value.get("roi")),
        &draft.limits,
        &mut object,
    );
    storage_change(
        limits.and_then(|value| value.get("storage")),
        &draft.limits,
        &mut object,
    );
    decision_change(
        limits.and_then(|value| value.get("decisions")),
        &draft.limits,
        &mut object,
    );
    if !object.is_empty() {
        nested.insert("limits".to_owned(), Value::Object(object));
    }
}

pub(super) fn object_from_draft(draft: &SemanticLimitsDraft) -> Value {
    let mut object = Map::new();
    worker_change(None, draft, &mut object);
    search_change(None, draft, &mut object);
    roi_change(None, draft, &mut object);
    storage_change(None, draft, &mut object);
    decision_change(None, draft, &mut object);
    Value::Object(object)
}

pub(super) fn validate(errors: &mut FieldErrors, draft: &SemanticDraft) {
    let limits = &draft.limits;
    required_positive(
        errors,
        WORKER_MAX_BATCH,
        &limits.worker_max_batch_items,
        None,
    );
    required_positive(
        errors,
        WORKER_MAX_CONCURRENCY,
        &limits.worker_max_concurrency,
        None,
    );
    required_positive(
        errors,
        WORKER_MAX_QUEUE,
        &limits.worker_max_queue_backlog,
        None,
    );
    required_positive(
        errors,
        WORKER_MAX_RETRIES,
        &limits.worker_max_retry_attempts,
        None,
    );
    required_whole_duration(errors, WORKER_DEADLINE, &limits.worker_job_deadline);
    required_whole_duration(errors, WORKER_BACKOFF, &limits.worker_retry_backoff);
    required_positive(errors, SEARCH_TOP_K, &limits.search_top_k, Some(MAX_TOP_K));
    required_positive(
        errors,
        SEARCH_CANDIDATES,
        &limits.search_candidate_limit,
        Some(MAX_CANDIDATES),
    );
    required_positive(
        errors,
        SEARCH_NEIGHBORS,
        &limits.search_max_neighbor_frames,
        Some(MAX_NEIGHBORS),
    );
    required_whole_duration(errors, SEARCH_WINDOW, &limits.search_query_window);
    optional_score_floor(errors, &limits.search_score_floor);
    required_positive(errors, ROI_MIN_EDGE, &limits.roi_min_edge_pixels, None);
    required_positive(errors, ROI_MAX_EDGE, &limits.roi_max_edge_pixels, None);
    required_positive(
        errors,
        ROI_MAX_PER_FRAME,
        &limits.roi_max_rois_per_frame,
        None,
    );
    if let (Ok(min), Ok(max)) = (
        limits.roi_min_edge_pixels.trim().parse::<u32>(),
        limits.roi_max_edge_pixels.trim().parse::<u32>(),
    ) && max < min
    {
        errors.insert(
            ROI_MAX_EDGE.to_owned(),
            "semantic_indexing.limits.roi.max_edge_pixels must be greater than or equal to min_edge_pixels.".to_owned(),
        );
    }
    optional_path(errors, STORAGE_SQLITE, &limits.storage_sqlite_path);
    optional_path(errors, STORAGE_INDEX, &limits.storage_vector_index_path);
    optional_positive_u64(errors, STORAGE_DISK, &limits.storage_max_disk_bytes);
    required_positive(
        errors,
        DECISIONS_MAX_BATCH,
        &limits.decisions_max_batch_items,
        None,
    );
    required_positive(
        errors,
        DECISIONS_MAX_CONCURRENCY,
        &limits.decisions_max_concurrency,
        None,
    );
    required_whole_duration(errors, DECISIONS_DEADLINE, &limits.decisions_job_deadline);
    if !limits.decisions_coverage_policy.trim().is_empty() {
        validate_choice(
            errors,
            DECISIONS_COVERAGE,
            &limits.decisions_coverage_policy,
            COVERAGE_POLICIES,
            "semantic_indexing.limits.decisions.coverage_policy is not a supported coverage policy.",
        );
    }
}

fn worker_change(
    current: Option<&Value>,
    draft: &SemanticLimitsDraft,
    limits: &mut Map<String, Value>,
) {
    let mut object = Map::new();
    push_optional_u32(
        &mut object,
        "max_batch_items",
        current.and_then(|value| value.get("max_batch_items")),
        &draft.worker_max_batch_items,
    );
    push_optional_u32(
        &mut object,
        "max_concurrency",
        current.and_then(|value| value.get("max_concurrency")),
        &draft.worker_max_concurrency,
    );
    push_optional_u32(
        &mut object,
        "max_queue_backlog",
        current.and_then(|value| value.get("max_queue_backlog")),
        &draft.worker_max_queue_backlog,
    );
    push_optional_u32(
        &mut object,
        "max_retry_attempts",
        current.and_then(|value| value.get("max_retry_attempts")),
        &draft.worker_max_retry_attempts,
    );
    push_optional_string(
        &mut object,
        "job_deadline",
        current.and_then(|value| value.get("job_deadline")),
        &draft.worker_job_deadline,
    );
    push_optional_string(
        &mut object,
        "retry_backoff",
        current.and_then(|value| value.get("retry_backoff")),
        &draft.worker_retry_backoff,
    );
    if !object.is_empty() {
        limits.insert("worker".to_owned(), Value::Object(object));
    }
}

fn search_change(
    current: Option<&Value>,
    draft: &SemanticLimitsDraft,
    limits: &mut Map<String, Value>,
) {
    let mut object = Map::new();
    push_optional_u32(
        &mut object,
        "top_k",
        current.and_then(|value| value.get("top_k")),
        &draft.search_top_k,
    );
    push_optional_u32(
        &mut object,
        "candidate_limit",
        current.and_then(|value| value.get("candidate_limit")),
        &draft.search_candidate_limit,
    );
    push_optional_u32(
        &mut object,
        "max_neighbor_frames",
        current.and_then(|value| value.get("max_neighbor_frames")),
        &draft.search_max_neighbor_frames,
    );
    push_optional_string(
        &mut object,
        "query_window",
        current.and_then(|value| value.get("query_window")),
        &draft.search_query_window,
    );
    push_optional_f64(
        &mut object,
        "score_floor",
        current.and_then(|value| value.get("score_floor")),
        &draft.search_score_floor,
    );
    if !object.is_empty() {
        limits.insert("search".to_owned(), Value::Object(object));
    }
}

fn roi_change(
    current: Option<&Value>,
    draft: &SemanticLimitsDraft,
    limits: &mut Map<String, Value>,
) {
    let mut object = Map::new();
    push_optional_u32(
        &mut object,
        "min_edge_pixels",
        current.and_then(|value| value.get("min_edge_pixels")),
        &draft.roi_min_edge_pixels,
    );
    push_optional_u32(
        &mut object,
        "max_edge_pixels",
        current.and_then(|value| value.get("max_edge_pixels")),
        &draft.roi_max_edge_pixels,
    );
    push_optional_u32(
        &mut object,
        "max_rois_per_frame",
        current.and_then(|value| value.get("max_rois_per_frame")),
        &draft.roi_max_rois_per_frame,
    );
    if !object.is_empty() {
        limits.insert("roi".to_owned(), Value::Object(object));
    }
}

fn storage_change(
    current: Option<&Value>,
    draft: &SemanticLimitsDraft,
    limits: &mut Map<String, Value>,
) {
    let mut object = Map::new();
    push_path(
        &mut object,
        "sqlite_path",
        current.and_then(|value| value.get("sqlite_path")),
        &draft.storage_sqlite_path,
    );
    push_path(
        &mut object,
        "vector_index_path",
        current.and_then(|value| value.get("vector_index_path")),
        &draft.storage_vector_index_path,
    );
    push_optional_u64(
        &mut object,
        "max_disk_bytes",
        current.and_then(|value| value.get("max_disk_bytes")),
        &draft.storage_max_disk_bytes,
    );
    if !object.is_empty() {
        limits.insert("storage".to_owned(), Value::Object(object));
    }
}

fn decision_change(
    current: Option<&Value>,
    draft: &SemanticLimitsDraft,
    limits: &mut Map<String, Value>,
) {
    let mut object = Map::new();
    push_optional_u32(
        &mut object,
        "max_batch_items",
        current.and_then(|value| value.get("max_batch_items")),
        &draft.decisions_max_batch_items,
    );
    push_optional_u32(
        &mut object,
        "max_concurrency",
        current.and_then(|value| value.get("max_concurrency")),
        &draft.decisions_max_concurrency,
    );
    push_optional_string(
        &mut object,
        "job_deadline",
        current.and_then(|value| value.get("job_deadline")),
        &draft.decisions_job_deadline,
    );
    push_optional_string(
        &mut object,
        "coverage_policy",
        current.and_then(|value| value.get("coverage_policy")),
        &draft.decisions_coverage_policy,
    );
    if !object.is_empty() {
        limits.insert("decisions".to_owned(), Value::Object(object));
    }
}

fn push_optional_u64(
    target: &mut Map<String, Value>,
    key: &str,
    current: Option<&Value>,
    next: &str,
) {
    let next = next.trim();
    if next.is_empty() {
        if current.is_some() {
            target.insert(key.to_owned(), Value::Null);
        }
        return;
    }
    let Ok(parsed) = next.parse::<u64>() else {
        return;
    };
    if current.and_then(Value::as_u64) == Some(parsed) {
        return;
    }
    target.insert(key.to_owned(), json!(parsed));
}

fn push_optional_f64(
    target: &mut Map<String, Value>,
    key: &str,
    current: Option<&Value>,
    next: &str,
) {
    let next = next.trim();
    if next.is_empty() {
        if current.is_some() {
            target.insert(key.to_owned(), Value::Null);
        }
        return;
    }
    let Ok(parsed) = next.parse::<f64>() else {
        return;
    };
    if current
        .and_then(Value::as_f64)
        .is_some_and(|value| value == parsed)
    {
        return;
    }
    target.insert(key.to_owned(), json!(parsed));
}

fn push_path(target: &mut Map<String, Value>, key: &str, current: Option<&Value>, next: &str) {
    let current = current.and_then(Value::as_str);
    match (current, next) {
        (None, "") => {}
        (Some(current), next) if current == next => {}
        (Some(_), "") => {
            target.insert(key.to_owned(), Value::Null);
        }
        (_, next) => {
            target.insert(key.to_owned(), json!(next));
        }
    }
}

fn required_positive(errors: &mut FieldErrors, field: &str, value: &str, max: Option<u32>) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    match trimmed.parse::<u32>() {
        Ok(parsed) if parsed > 0 && max.is_none_or(|max| parsed <= max) => {}
        _ => {
            let message = match max {
                Some(max) => format!("{field} must be 1..={max}."),
                None => format!("{field} must be greater than zero."),
            };
            errors.insert(field.to_owned(), message);
        }
    }
}

fn required_whole_duration(errors: &mut FieldErrors, field: &str, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    if !is_whole_second_duration(trimmed) {
        errors.insert(
            field.to_owned(),
            format!("{field} must be a positive whole-second duration."),
        );
    }
}

fn optional_score_floor(errors: &mut FieldErrors, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    match trimmed.parse::<f64>() {
        Ok(score) if score.is_finite() && (-1.0..=1.0).contains(&score) => {}
        _ => {
            errors.insert(
                SEARCH_SCORE_FLOOR.to_owned(),
                "semantic_indexing.limits.search.score_floor must be a finite number between -1 and 1.".to_owned(),
            );
        }
    }
}

fn optional_path(errors: &mut FieldErrors, field: &str, value: &str) {
    if !value.is_empty() && !value.chars().any(|character| !character.is_whitespace()) {
        errors.insert(
            field.to_owned(),
            format!("{field} must contain non-whitespace text."),
        );
    }
}

fn optional_positive_u64(errors: &mut FieldErrors, field: &str, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    match trimmed.parse::<u64>() {
        Ok(parsed) if parsed > 0 => {}
        _ => {
            errors.insert(
                field.to_owned(),
                format!("{field} must be greater than zero."),
            );
        }
    }
}

fn is_whole_second_duration(text: &str) -> bool {
    duration_nanos(text).is_some_and(|nanos| nanos.is_multiple_of(1_000_000_000))
}

fn duration_nanos(text: &str) -> Option<u128> {
    if text.starts_with('-') {
        return None;
    }
    let mut rest = text;
    let mut nanos: u128 = 0;
    let mut saw = false;
    while !rest.is_empty() {
        let digits = rest
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .count();
        if digits == 0 {
            return None;
        }
        let amount: u128 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let unit = [
            ("ms", 1_000_000u128),
            ("us", 1_000u128),
            ("ns", 1u128),
            ("s", 1_000_000_000u128),
            ("m", 60 * 1_000_000_000),
            ("h", 60 * 60 * 1_000_000_000),
            ("d", 24 * 60 * 60 * 1_000_000_000),
            ("w", 7 * 24 * 60 * 60 * 1_000_000_000),
        ]
        .into_iter()
        .find(|(name, _)| rest.starts_with(name))?;
        if matches!(unit.0, "ms" | "us" | "ns") {
            return None;
        }
        rest = &rest[unit.0.len()..];
        nanos = nanos.checked_add(amount.checked_mul(unit.1)?)?;
        saw = true;
    }
    if saw && nanos > 0 { Some(nanos) } else { None }
}
