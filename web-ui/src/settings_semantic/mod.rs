//! Semantic identity drafts and surgical PATCH bodies.

mod limits;
pub(crate) mod source;

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

pub(crate) use limits::COVERAGE_POLICIES;
pub use limits::{
    DECISIONS_COVERAGE, DECISIONS_DEADLINE, DECISIONS_MAX_BATCH, DECISIONS_MAX_CONCURRENCY,
    ROI_MAX_EDGE, ROI_MAX_PER_FRAME, ROI_MIN_EDGE, SEARCH_CANDIDATES, SEARCH_NEIGHBORS,
    SEARCH_SCORE_FLOOR, SEARCH_TOP_K, SEARCH_WINDOW, STORAGE_DISK, STORAGE_INDEX, STORAGE_SQLITE,
    SemanticLimitsDraft, WORKER_BACKOFF, WORKER_DEADLINE, WORKER_MAX_BATCH, WORKER_MAX_CONCURRENCY,
    WORKER_MAX_QUEUE, WORKER_MAX_RETRIES,
};

pub type FieldErrors = BTreeMap<String, String>;
pub const DESCRIPTION_PROVIDER: &str = "semantic_indexing.description.provider";
pub const DESCRIPTION_MODEL: &str = "semantic_indexing.description.model";
pub const DESCRIPTION_CREDENTIALS: &str = "semantic_indexing.description.credentials.env";
pub const EMBEDDINGS_PROVIDER: &str = "semantic_indexing.embeddings.provider";
pub const EMBEDDINGS_MODEL: &str = "semantic_indexing.embeddings.model";
pub const EMBEDDINGS_DIMENSIONS: &str = "semantic_indexing.embeddings.space.dimensions";
pub const EMBEDDINGS_NORMALIZATION: &str = "semantic_indexing.embeddings.space.normalization";
pub const EMBEDDINGS_METRIC: &str = "semantic_indexing.embeddings.space.metric";
pub const EMBEDDINGS_GENERATION: &str = "semantic_indexing.embeddings.space.generation";
pub const EMBEDDINGS_CREDENTIALS: &str = "semantic_indexing.embeddings.credentials.env";
pub const DECISIONS_PROVIDER: &str = "semantic_indexing.decisions.provider";
pub const DECISIONS_ENDPOINT: &str = "semantic_indexing.decisions.endpoint";
pub const DECISIONS_MODEL: &str = "semantic_indexing.decisions.model";
pub const DECISIONS_CREDENTIALS: &str = "semantic_indexing.decisions.credentials.env";
pub const DECISIONS_PREAMBLE: &str = "semantic_indexing.decisions.preamble";

pub(crate) const DESCRIPTION_PROVIDERS: &[&str] = &["bedrock", "openai"];
pub(crate) const DECISION_PROVIDERS: &[&str] = &["jev"];
pub(crate) const NORMALIZATION_VALUES: &[&str] = &["none", "l2_unit"];
pub(crate) const METRIC_VALUES: &[&str] = &["cosine", "dot", "euclidean"];
const JEV_MODEL: &str = "jev-1.14.0";
pub const DEFAULT_JEV_API_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
const MAX_DIMENSIONS: u32 = 16_384;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDraft {
    pub present: bool,
    pub description_provider: String,
    pub description_model: String,
    pub description_credentials: String,
    pub embeddings_present: bool,
    pub embeddings_provider: String,
    pub embeddings_model: String,
    pub space_dimensions: String,
    pub space_normalization: String,
    pub space_metric: String,
    pub space_generation: String,
    pub embeddings_credentials: String,
    pub decisions_provider: String,
    pub decisions_endpoint: String,
    pub decisions_model: String,
    pub decisions_credentials: String,
    pub decisions_preamble_set: bool,
    pub decisions_preamble: String,
    pub limits: SemanticLimitsDraft,
}

impl SemanticDraft {
    pub fn empty() -> Self {
        Self {
            present: false,
            description_provider: String::new(),
            description_model: String::new(),
            description_credentials: String::new(),
            embeddings_present: false,
            embeddings_provider: String::new(),
            embeddings_model: String::new(),
            space_dimensions: String::new(),
            space_normalization: String::new(),
            space_metric: String::new(),
            space_generation: String::new(),
            embeddings_credentials: String::new(),
            decisions_provider: "jev".to_owned(),
            decisions_endpoint: DEFAULT_JEV_API_ENDPOINT.to_owned(),
            decisions_model: String::new(),
            decisions_credentials: String::new(),
            decisions_preamble_set: false,
            decisions_preamble: String::new(),
            limits: SemanticLimitsDraft::empty(),
        }
    }
}

pub fn semantic_draft_from_document(document: &Value) -> SemanticDraft {
    let Some(semantic) = document.get("semantic_indexing") else {
        return SemanticDraft::empty();
    };
    let description = semantic.get("description");
    let embeddings = semantic.get("embeddings").filter(|value| !value.is_null());
    let space = embeddings.and_then(|value| value.get("space"));
    let decisions = semantic.get("decisions");
    let limits = semantic.get("limits");
    SemanticDraft {
        present: true,
        description_provider: string_or_empty(description.and_then(|value| value.get("provider"))),
        description_model: string_or_empty(description.and_then(|value| value.get("model"))),
        description_credentials: credential_env(description),
        embeddings_present: embeddings.is_some(),
        embeddings_provider: string_or_empty(embeddings.and_then(|value| value.get("provider"))),
        embeddings_model: string_or_empty(embeddings.and_then(|value| value.get("model"))),
        space_dimensions: number_or_empty(space.and_then(|value| value.get("dimensions"))),
        space_normalization: string_or_empty(space.and_then(|value| value.get("normalization"))),
        space_metric: string_or_empty(space.and_then(|value| value.get("metric"))),
        space_generation: number_or_empty(space.and_then(|value| value.get("generation"))),
        embeddings_credentials: credential_env(embeddings),
        decisions_provider: string_or_default(
            decisions.and_then(|value| value.get("provider")),
            "jev",
        ),
        decisions_endpoint: string_or_default(
            decisions.and_then(|value| value.get("endpoint")),
            DEFAULT_JEV_API_ENDPOINT,
        ),
        decisions_model: string_or_empty(decisions.and_then(|value| value.get("model"))),
        decisions_credentials: credential_env(decisions),
        decisions_preamble_set: decisions.and_then(|value| value.get("preamble")).is_some(),
        decisions_preamble: string_or_empty(decisions.and_then(|value| value.get("preamble"))),
        limits: limits::from_document(limits),
    }
}

pub fn surgical_semantic_changes(
    original: &Value,
    draft: &SemanticDraft,
) -> Result<Value, FieldErrors> {
    let errors = validate_semantic_draft(draft);
    if !errors.is_empty() {
        return Err(errors);
    }
    let current = original.get("semantic_indexing");
    if !draft.present {
        return Ok(json!({}));
    }
    if current.is_none() {
        return Ok(json!({ "semantic_indexing": new_semantic_object(draft) }));
    }
    let mut nested = Map::new();
    description_change(current, draft, &mut nested);
    embeddings_change(current, draft, &mut nested);
    decisions_change(current, draft, &mut nested);
    limits::change(current, draft, &mut nested);
    let mut changes = Map::new();
    if !nested.is_empty() {
        changes.insert("semantic_indexing".to_owned(), Value::Object(nested));
    }
    Ok(Value::Object(changes))
}

pub fn validate_semantic_draft(draft: &SemanticDraft) -> FieldErrors {
    let mut errors = FieldErrors::new();
    if !draft.present {
        return errors;
    }
    validate_choice(
        &mut errors,
        DESCRIPTION_PROVIDER,
        &draft.description_provider,
        DESCRIPTION_PROVIDERS,
        "semantic_indexing.description.provider is not a supported provider.",
    );
    if draft.decisions_endpoint.trim().is_empty() {
        errors.insert(
            DECISIONS_ENDPOINT.to_owned(),
            "Jev API URL is required.".to_owned(),
        );
    }
    reject_blank(
        &mut errors,
        DESCRIPTION_MODEL,
        &draft.description_model,
        "semantic_indexing.description.model must contain non-whitespace text.",
    );
    validate_env(
        &mut errors,
        DESCRIPTION_CREDENTIALS,
        &draft.description_credentials,
    );
    if draft.embeddings_present {
        reject_required(
            &mut errors,
            EMBEDDINGS_PROVIDER,
            &draft.embeddings_provider,
            "semantic_indexing.embeddings.provider must contain non-whitespace text.",
        );
        reject_required(
            &mut errors,
            EMBEDDINGS_MODEL,
            &draft.embeddings_model,
            "semantic_indexing.embeddings.model must contain non-whitespace text.",
        );
        validate_dimensions(&mut errors, &draft.space_dimensions);
        validate_choice(
            &mut errors,
            EMBEDDINGS_NORMALIZATION,
            &draft.space_normalization,
            NORMALIZATION_VALUES,
            "semantic_indexing.embeddings.space.normalization is not a supported value.",
        );
        if draft.space_normalization.trim().is_empty() {
            errors.insert(
                EMBEDDINGS_NORMALIZATION.to_owned(),
                "semantic_indexing.embeddings.space.normalization is required.".to_owned(),
            );
        }
        validate_choice(
            &mut errors,
            EMBEDDINGS_METRIC,
            &draft.space_metric,
            METRIC_VALUES,
            "semantic_indexing.embeddings.space.metric is not a supported value.",
        );
        if draft.space_metric.trim().is_empty() {
            errors.insert(
                EMBEDDINGS_METRIC.to_owned(),
                "semantic_indexing.embeddings.space.metric is required.".to_owned(),
            );
        }
        validate_generation(&mut errors, &draft.space_generation);
        validate_env(
            &mut errors,
            EMBEDDINGS_CREDENTIALS,
            &draft.embeddings_credentials,
        );
    }
    validate_choice(
        &mut errors,
        DECISIONS_PROVIDER,
        &draft.decisions_provider,
        DECISION_PROVIDERS,
        "semantic_indexing.decisions.provider is not a supported provider.",
    );
    if !draft.decisions_model.trim().is_empty() && draft.decisions_model.trim() != JEV_MODEL {
        errors.insert(
            DECISIONS_MODEL.to_owned(),
            "semantic_indexing.decisions.model is not a supported Jev model revision.".to_owned(),
        );
    }
    validate_env(
        &mut errors,
        DECISIONS_CREDENTIALS,
        &draft.decisions_credentials,
    );
    limits::validate(&mut errors, draft);
    errors
}

pub fn reindex_required(path: &str) -> bool {
    path == DESCRIPTION_PROVIDER
        || path == DESCRIPTION_MODEL
        || path == EMBEDDINGS_PROVIDER
        || path == EMBEDDINGS_MODEL
        || path.starts_with("semantic_indexing.embeddings.space")
        || path == STORAGE_SQLITE
        || path == STORAGE_INDEX
        || path.contains(".semantic_indexing.description_model")
        || path.contains(".semantic_indexing.embedding_model")
}

pub fn added_semantic() -> SemanticDraft {
    let mut draft = SemanticDraft::empty();
    draft.present = true;
    draft.limits = SemanticLimitsDraft::bootstrap();
    draft
}

#[cfg(test)]
fn impact_label(path: &str) -> &'static str {
    if reindex_required(path) {
        "Requires semantic reindex"
    } else if restart_required(path) {
        "Requires agent restart"
    } else {
        ""
    }
}

#[cfg(test)]
fn restart_required(path: &str) -> bool {
    path == "semantic_indexing" || path.starts_with("semantic_indexing.")
}

pub(super) fn reject_blank(errors: &mut FieldErrors, field: &str, value: &str, message: &str) {
    if !value.is_empty() && !value.chars().any(|character| !character.is_whitespace()) {
        errors.insert(field.to_owned(), message.to_owned());
    }
}

fn new_semantic_object(draft: &SemanticDraft) -> Value {
    let mut nested = Map::new();
    description_change(None, draft, &mut nested);
    embeddings_change(None, draft, &mut nested);
    decisions_change(None, draft, &mut nested);
    nested.insert(
        "limits".to_owned(),
        limits::object_from_draft(&draft.limits),
    );
    Value::Object(nested)
}

fn description_change(
    current: Option<&Value>,
    draft: &SemanticDraft,
    nested: &mut Map<String, Value>,
) {
    let description = current.and_then(|value| value.get("description"));
    let mut object = Map::new();
    push_optional_string(
        &mut object,
        "provider",
        description.and_then(|value| value.get("provider")),
        &draft.description_provider,
    );
    push_optional_string(
        &mut object,
        "model",
        description.and_then(|value| value.get("model")),
        &draft.description_model,
    );
    credentials_change(
        description.and_then(|value| value.get("credentials")),
        &draft.description_credentials,
        &mut object,
    );
    if !object.is_empty() {
        nested.insert("description".to_owned(), Value::Object(object));
    }
}

fn embeddings_change(
    current: Option<&Value>,
    draft: &SemanticDraft,
    nested: &mut Map<String, Value>,
) {
    let embeddings = current
        .and_then(|value| value.get("embeddings"))
        .filter(|value| !value.is_null());
    if !draft.embeddings_present {
        if embeddings.is_some() {
            nested.insert("embeddings".to_owned(), Value::Null);
        }
        return;
    }
    let mut object = Map::new();
    if embeddings.is_none() {
        object.insert(
            "provider".to_owned(),
            json!(draft.embeddings_provider.trim()),
        );
        object.insert("model".to_owned(), json!(draft.embeddings_model.trim()));
        object.insert("space".to_owned(), new_space(draft));
        credentials_change(None, &draft.embeddings_credentials, &mut object);
        nested.insert("embeddings".to_owned(), Value::Object(object));
        return;
    }
    push_optional_string(
        &mut object,
        "provider",
        embeddings.and_then(|value| value.get("provider")),
        &draft.embeddings_provider,
    );
    push_optional_string(
        &mut object,
        "model",
        embeddings.and_then(|value| value.get("model")),
        &draft.embeddings_model,
    );
    space_change(
        embeddings.and_then(|value| value.get("space")),
        draft,
        &mut object,
    );
    credentials_change(
        embeddings.and_then(|value| value.get("credentials")),
        &draft.embeddings_credentials,
        &mut object,
    );
    if !object.is_empty() {
        nested.insert("embeddings".to_owned(), Value::Object(object));
    }
}

fn new_space(draft: &SemanticDraft) -> Value {
    let mut space = Map::new();
    if let Ok(dimensions) = draft.space_dimensions.trim().parse::<u32>() {
        space.insert("dimensions".to_owned(), json!(dimensions));
    }
    space.insert(
        "normalization".to_owned(),
        json!(draft.space_normalization.trim()),
    );
    space.insert("metric".to_owned(), json!(draft.space_metric.trim()));
    if let Ok(generation) = draft.space_generation.trim().parse::<u32>() {
        space.insert("generation".to_owned(), json!(generation));
    }
    Value::Object(space)
}

fn space_change(space: Option<&Value>, draft: &SemanticDraft, embeddings: &mut Map<String, Value>) {
    let mut nested = Map::new();
    push_optional_u32(
        &mut nested,
        "dimensions",
        space.and_then(|value| value.get("dimensions")),
        &draft.space_dimensions,
    );
    push_optional_string(
        &mut nested,
        "normalization",
        space.and_then(|value| value.get("normalization")),
        &draft.space_normalization,
    );
    push_optional_string(
        &mut nested,
        "metric",
        space.and_then(|value| value.get("metric")),
        &draft.space_metric,
    );
    push_optional_u32(
        &mut nested,
        "generation",
        space.and_then(|value| value.get("generation")),
        &draft.space_generation,
    );
    if !nested.is_empty() {
        embeddings.insert("space".to_owned(), Value::Object(nested));
    }
}

fn decisions_change(
    current: Option<&Value>,
    draft: &SemanticDraft,
    nested: &mut Map<String, Value>,
) {
    let decisions = current.and_then(|value| value.get("decisions"));
    let mut object = Map::new();
    if decisions.and_then(|value| value.get("provider")).is_some()
        || draft.decisions_provider != "jev"
    {
        push_optional_string(
            &mut object,
            "provider",
            decisions.and_then(|value| value.get("provider")),
            &draft.decisions_provider,
        );
    }
    if decisions.and_then(|value| value.get("endpoint")).is_some()
        || draft.decisions_endpoint != DEFAULT_JEV_API_ENDPOINT
    {
        push_optional_string(
            &mut object,
            "endpoint",
            decisions.and_then(|value| value.get("endpoint")),
            &draft.decisions_endpoint,
        );
    }
    push_optional_string(
        &mut object,
        "model",
        decisions.and_then(|value| value.get("model")),
        &draft.decisions_model,
    );
    credentials_change(
        decisions.and_then(|value| value.get("credentials")),
        &draft.decisions_credentials,
        &mut object,
    );
    push_exact_string(
        &mut object,
        "preamble",
        decisions
            .and_then(|value| value.get("preamble"))
            .and_then(Value::as_str),
        draft.decisions_preamble_set,
        &draft.decisions_preamble,
    );
    if !object.is_empty() {
        nested.insert("decisions".to_owned(), Value::Object(object));
    }
}

fn credentials_change(current: Option<&Value>, next: &str, parent: &mut Map<String, Value>) {
    let next = next.trim();
    let existing = current
        .and_then(|value| value.get("env"))
        .and_then(Value::as_str);
    match (existing, next) {
        (None, "") => {}
        (Some(existing), next) if existing == next => {}
        (Some(_), "") => {
            parent.insert("credentials".to_owned(), Value::Null);
        }
        (_, next) => {
            parent.insert("credentials".to_owned(), json!({ "env": next }));
        }
    }
}

pub(super) fn validate_choice(
    errors: &mut FieldErrors,
    field: &str,
    value: &str,
    allowed: &[&str],
    message: &str,
) {
    let trimmed = value.trim();
    if !trimmed.is_empty() && !allowed.contains(&trimmed) {
        errors.insert(field.to_owned(), message.to_owned());
    }
}

fn reject_required(errors: &mut FieldErrors, field: &str, value: &str, message: &str) {
    if value.trim().is_empty() {
        errors.insert(field.to_owned(), message.to_owned());
    }
}

fn validate_env(errors: &mut FieldErrors, field: &str, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    if !is_env_name(trimmed) {
        errors.insert(
            field.to_owned(),
            format!("{field} must be an uppercase identifier."),
        );
    }
}

fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_uppercase()
        && chars.all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
}

fn validate_dimensions(errors: &mut FieldErrors, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        errors.insert(
            EMBEDDINGS_DIMENSIONS.to_owned(),
            "semantic_indexing.embeddings.space.dimensions is required.".to_owned(),
        );
        return;
    }
    match trimmed.parse::<u32>() {
        Ok(dimensions) if (1..=MAX_DIMENSIONS).contains(&dimensions) => {}
        _ => {
            errors.insert(
                EMBEDDINGS_DIMENSIONS.to_owned(),
                "semantic_indexing.embeddings.space.dimensions must be 1..=16384.".to_owned(),
            );
        }
    }
}

fn validate_generation(errors: &mut FieldErrors, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    match trimmed.parse::<u32>() {
        Ok(generation) if generation > 0 => {}
        _ => {
            errors.insert(
                EMBEDDINGS_GENERATION.to_owned(),
                "semantic_indexing.embeddings.space.generation must be greater than zero."
                    .to_owned(),
            );
        }
    }
}

fn push_exact_string(
    target: &mut Map<String, Value>,
    key: &str,
    current: Option<&str>,
    set: bool,
    next: &str,
) {
    match (current, set) {
        (None, false) => {}
        (Some(current), true) if current == next => {}
        (Some(_), false) => {
            target.insert(key.to_owned(), Value::Null);
        }
        (_, true) => {
            target.insert(key.to_owned(), Value::String(next.to_owned()));
        }
    }
}

pub(super) fn push_optional_string(
    target: &mut Map<String, Value>,
    key: &str,
    current: Option<&Value>,
    next: &str,
) {
    let next = next.trim();
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

pub(super) fn push_optional_u32(
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
    let Ok(parsed) = next.parse::<u32>() else {
        return;
    };
    if current
        .and_then(Value::as_u64)
        .is_some_and(|value| value == u64::from(parsed))
    {
        return;
    }
    target.insert(key.to_owned(), json!(parsed));
}

fn credential_env(parent: Option<&Value>) -> String {
    string_or_empty(
        parent
            .and_then(|value| value.get("credentials"))
            .and_then(|value| value.get("env")),
    )
}

pub(super) fn string_or_empty(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn string_or_default(value: Option<&Value>, default: &str) -> String {
    let value = string_or_empty(value);
    if value.is_empty() {
        default.to_owned()
    } else {
        value
    }
}

pub(super) fn number_or_empty(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::String(text)) => text.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn original() -> Value {
        json!({
            "schema_version": 2,
            "semantic_indexing": {
                "description": {
                    "provider": "bedrock",
                    "model": "luna",
                    "credentials": { "env": "A2ALAB_DESC" }
                },
                "embeddings": {
                    "provider": "p",
                    "model": "m",
                    "space": {
                        "dimensions": 8,
                        "normalization": "l2_unit",
                        "metric": "cosine"
                    }
                },
                "decisions": { "provider": "jev", "model": "jev-1.14.0" },
                "limits": { "storage": { "vector_index_path": "./x" } }
            }
        })
    }

    #[test]
    fn add_action_enables_description_only_draft() {
        let draft = added_semantic();
        assert!(draft.present);
        assert!(!draft.embeddings_present);
        assert!(draft.space_dimensions.is_empty());
        assert_eq!(draft.limits.worker_max_batch_items, "1");
        assert_eq!(draft.limits.storage_vector_index_path, "./semantic");
        assert!(draft.limits.storage_sqlite_path.is_empty());
        assert!(draft.limits.search_score_floor.is_empty());
    }

    #[test]
    fn omitted_semantic_indexing_emits_no_patch() {
        let document = json!({ "schema_version": 2 });
        let draft = semantic_draft_from_document(&document);
        assert!(!draft.present);
        assert!(!draft.embeddings_present);
        assert_eq!(
            surgical_semantic_changes(&document, &draft).unwrap(),
            json!({})
        );
    }

    #[test]
    fn omitted_embeddings_are_not_materialized() {
        let document = json!({
            "schema_version": 2,
            "semantic_indexing": {
                "description": { "model": "luna" },
                "limits": { "storage": { "vector_index_path": "./x" } }
            }
        });
        let draft = semantic_draft_from_document(&document);
        assert!(draft.present);
        assert!(!draft.embeddings_present);
        assert!(draft.space_dimensions.is_empty());
        let mut next = draft.clone();
        next.description_model = "luna-2".to_owned();
        let changes = surgical_semantic_changes(&document, &next).unwrap();
        assert_eq!(
            changes,
            json!({ "semantic_indexing": { "description": { "model": "luna-2" } } })
        );
        assert!(changes["semantic_indexing"].get("embeddings").is_none());
        assert!(changes["semantic_indexing"].get("limits").is_none());
    }

    #[test]
    fn decision_preamble_draft_keeps_exact_text() {
        let document = json!({
            "schema_version": 2,
            "semantic_indexing": {
                "description": { "model": "luna" },
                "decisions": { "model": "jev-1.14.0", "preamble": "  keep\n" },
                "limits": { "storage": { "vector_index_path": "./x" } }
            }
        });
        let draft = semantic_draft_from_document(&document);
        assert_eq!(DECISIONS_PREAMBLE, "semantic_indexing.decisions.preamble");
        assert!(draft.decisions_preamble_set);
        assert_eq!(draft.decisions_preamble, "  keep\n");
        let mut unchanged = draft.clone();
        unchanged.description_model = "luna-2".to_owned();
        let changes = surgical_semantic_changes(&document, &unchanged).unwrap();
        assert!(changes["semantic_indexing"].get("decisions").is_none());

        let mut blank = semantic_draft_from_document(&json!({
            "schema_version": 2,
            "semantic_indexing": {
                "description": { "model": "luna" },
                "limits": { "storage": { "vector_index_path": "./x" } }
            }
        }));
        assert!(!blank.decisions_preamble_set);
        blank.decisions_preamble_set = true;
        blank.decisions_preamble = String::new();
        let changes = surgical_semantic_changes(
            &json!({
                "schema_version": 2,
                "semantic_indexing": {
                    "description": { "model": "luna" },
                    "limits": { "storage": { "vector_index_path": "./x" } }
                }
            }),
            &blank,
        )
        .unwrap();
        assert_eq!(
            changes["semantic_indexing"]["decisions"]["preamble"],
            json!("")
        );

        let mut clear = draft;
        clear.decisions_preamble_set = false;
        let changes = surgical_semantic_changes(&document, &clear).unwrap();
        assert_eq!(
            changes["semantic_indexing"]["decisions"]["preamble"],
            Value::Null
        );
    }

    #[test]
    fn adding_embeddings_writes_identity_only() {
        let document = json!({
            "schema_version": 2,
            "semantic_indexing": {
                "description": { "model": "luna" },
                "limits": { "storage": { "vector_index_path": "./x" } }
            }
        });
        let mut draft = semantic_draft_from_document(&document);
        draft.embeddings_present = true;
        draft.embeddings_provider = "p".to_owned();
        draft.embeddings_model = "m".to_owned();
        draft.space_dimensions = "8".to_owned();
        draft.space_normalization = "l2_unit".to_owned();
        draft.space_metric = "cosine".to_owned();
        let changes = surgical_semantic_changes(&document, &draft).unwrap();
        assert_eq!(
            changes["semantic_indexing"]["embeddings"],
            json!({
                "provider": "p",
                "model": "m",
                "space": { "dimensions": 8, "normalization": "l2_unit", "metric": "cosine" }
            })
        );
        assert!(changes["semantic_indexing"].get("description").is_none());
    }

    #[test]
    fn removing_embeddings_emits_null() {
        let document = original();
        let mut draft = semantic_draft_from_document(&document);
        draft.embeddings_present = false;
        let changes = surgical_semantic_changes(&document, &draft).unwrap();
        assert_eq!(changes["semantic_indexing"]["embeddings"], Value::Null);
        assert!(changes["semantic_indexing"].get("description").is_none());
    }

    #[test]
    fn adding_absent_block_omits_embeddings_and_keeps_env_names() {
        let document = json!({ "schema_version": 2 });
        let mut draft = added_semantic();
        draft.description_credentials = "A2ALAB_DESC_KEY".to_owned();
        let changes = surgical_semantic_changes(&document, &draft).unwrap();
        let semantic = &changes["semantic_indexing"];
        assert!(semantic.get("embeddings").is_none());
        assert_eq!(
            semantic["description"]["credentials"]["env"],
            json!("A2ALAB_DESC_KEY")
        );
        assert_eq!(
            semantic["limits"]["storage"]["vector_index_path"],
            "./semantic"
        );
        assert!(semantic["limits"]["storage"].get("sqlite_path").is_none());
        assert!(semantic["limits"]["search"].get("score_floor").is_none());
        let serialized = changes.to_string();
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("sk-"));
    }

    #[test]
    fn credential_and_space_validation_blocks_patch() {
        let document = original();
        let mut draft = semantic_draft_from_document(&document);
        draft.description_credentials = "lowercase".to_owned();
        draft.space_metric = "manhattan".to_owned();
        draft.decisions_provider = "openai".to_owned();
        let errors = surgical_semantic_changes(&document, &draft).unwrap_err();
        assert!(errors.contains_key(DESCRIPTION_CREDENTIALS));
        assert!(errors.contains_key(EMBEDDINGS_METRIC));
        assert!(errors.contains_key(DECISIONS_PROVIDER));
    }

    #[test]
    fn impact_labels_split_reindex_and_restart() {
        assert_eq!(
            impact_label("videos.sources.0.semantic_indexing.description_model"),
            "Requires semantic reindex"
        );
        assert_eq!(
            impact_label("videos.sources.0.semantic_indexing.embedding_model"),
            "Requires semantic reindex"
        );
        assert_eq!(impact_label(DESCRIPTION_MODEL), "Requires semantic reindex");
        assert_eq!(
            impact_label(EMBEDDINGS_DIMENSIONS),
            "Requires semantic reindex"
        );
        assert_eq!(
            impact_label(EMBEDDINGS_PROVIDER),
            "Requires semantic reindex"
        );
        assert_eq!(impact_label(DECISIONS_PROVIDER), "Requires agent restart");
        assert_eq!(
            impact_label(DESCRIPTION_CREDENTIALS),
            "Requires agent restart"
        );
        assert_eq!(
            impact_label(EMBEDDINGS_CREDENTIALS),
            "Requires agent restart"
        );
        assert_eq!(impact_label(STORAGE_INDEX), "Requires semantic reindex");
        assert_eq!(impact_label(STORAGE_SQLITE), "Requires semantic reindex");
        assert_eq!(impact_label(WORKER_MAX_BATCH), "Requires agent restart");
        assert_eq!(
            impact_label(WORKER_MAX_CONCURRENCY),
            "Requires agent restart"
        );
        assert_eq!(impact_label(WORKER_MAX_QUEUE), "Requires agent restart");
        assert_eq!(impact_label(WORKER_MAX_RETRIES), "Requires agent restart");
        assert_eq!(impact_label(WORKER_BACKOFF), "Requires agent restart");
        assert_eq!(impact_label(SEARCH_TOP_K), "Requires agent restart");
        assert_eq!(impact_label(SEARCH_CANDIDATES), "Requires agent restart");
        assert_eq!(impact_label(SEARCH_NEIGHBORS), "Requires agent restart");
        assert_eq!(impact_label(SEARCH_WINDOW), "Requires agent restart");
        assert_eq!(impact_label(SEARCH_SCORE_FLOOR), "Requires agent restart");
        assert_eq!(impact_label(ROI_MIN_EDGE), "Requires agent restart");
        assert_eq!(impact_label(ROI_MAX_PER_FRAME), "Requires agent restart");
        assert_eq!(impact_label(STORAGE_DISK), "Requires agent restart");
        assert_eq!(impact_label(DECISIONS_MAX_BATCH), "Requires agent restart");
        assert_eq!(
            impact_label(DECISIONS_MAX_CONCURRENCY),
            "Requires agent restart"
        );
        assert_eq!(impact_label(DECISIONS_DEADLINE), "Requires agent restart");
        assert_eq!(impact_label(DECISIONS_COVERAGE), "Requires agent restart");
        assert_eq!(
            COVERAGE_POLICIES,
            &["all_saved_frames", "on_demand", "sampled"]
        );
        assert!(restart_required(DESCRIPTION_MODEL));
        assert!(restart_required(DECISIONS_MODEL));
        assert!(restart_required(STORAGE_INDEX));
        assert!(!reindex_required(DECISIONS_MODEL));
        assert!(!reindex_required(DESCRIPTION_CREDENTIALS));
        assert!(!reindex_required(WORKER_DEADLINE));
    }

    fn complete_limits() -> Value {
        json!({
            "schema_version": 2,
            "semantic_indexing": {
                "description": { "model": "luna" },
                "limits": {
                    "worker": {
                        "max_batch_items": 1,
                        "max_concurrency": 1,
                        "max_queue_backlog": 1,
                        "max_retry_attempts": 1,
                        "job_deadline": "30s",
                        "retry_backoff": "5s"
                    },
                    "search": {
                        "top_k": 1,
                        "candidate_limit": 1,
                        "max_neighbor_frames": 1,
                        "query_window": "1h"
                    },
                    "roi": {
                        "min_edge_pixels": 280,
                        "max_edge_pixels": 1568,
                        "max_rois_per_frame": 1
                    },
                    "storage": {
                        "vector_index_path": "./semantic",
                        "max_disk_bytes": 1024
                    },
                    "decisions": {
                        "max_batch_items": 1,
                        "max_concurrency": 1,
                        "job_deadline": "45s",
                        "coverage_policy": "on_demand"
                    }
                }
            }
        })
    }

    #[test]
    fn surgical_limit_patches_cover_every_group() {
        let document = complete_limits();
        let mut draft = semantic_draft_from_document(&document);
        draft.limits.worker_max_batch_items = "2".to_owned();
        draft.limits.worker_max_concurrency = "3".to_owned();
        draft.limits.worker_max_queue_backlog = "4".to_owned();
        draft.limits.worker_max_retry_attempts = "5".to_owned();
        draft.limits.worker_job_deadline = "1m".to_owned();
        draft.limits.worker_retry_backoff = "10s".to_owned();
        draft.limits.search_top_k = "8".to_owned();
        draft.limits.search_candidate_limit = "20".to_owned();
        draft.limits.search_max_neighbor_frames = "6".to_owned();
        draft.limits.search_query_window = "2h".to_owned();
        draft.limits.search_score_floor = "0.25".to_owned();
        draft.limits.roi_min_edge_pixels = "300".to_owned();
        draft.limits.roi_max_edge_pixels = "1600".to_owned();
        draft.limits.roi_max_rois_per_frame = "4".to_owned();
        draft.limits.storage_sqlite_path = "./db".to_owned();
        draft.limits.storage_vector_index_path = "./idx".to_owned();
        draft.limits.storage_max_disk_bytes = "2048".to_owned();
        draft.limits.decisions_max_batch_items = "2".to_owned();
        draft.limits.decisions_max_concurrency = "2".to_owned();
        draft.limits.decisions_job_deadline = "1m".to_owned();
        draft.limits.decisions_coverage_policy = "sampled".to_owned();
        let changes = surgical_semantic_changes(&document, &draft).unwrap();
        assert_eq!(
            changes["semantic_indexing"]["limits"]["worker"],
            json!({
                "max_batch_items": 2,
                "max_concurrency": 3,
                "max_queue_backlog": 4,
                "max_retry_attempts": 5,
                "job_deadline": "1m",
                "retry_backoff": "10s"
            })
        );
        assert_eq!(
            changes["semantic_indexing"]["limits"]["search"],
            json!({
                "top_k": 8,
                "candidate_limit": 20,
                "max_neighbor_frames": 6,
                "query_window": "2h",
                "score_floor": 0.25
            })
        );
        assert_eq!(
            changes["semantic_indexing"]["limits"]["roi"],
            json!({
                "min_edge_pixels": 300,
                "max_edge_pixels": 1600,
                "max_rois_per_frame": 4
            })
        );
        assert_eq!(
            changes["semantic_indexing"]["limits"]["storage"],
            json!({
                "sqlite_path": "./db",
                "vector_index_path": "./idx",
                "max_disk_bytes": 2048
            })
        );
        assert_eq!(
            changes["semantic_indexing"]["limits"]["decisions"],
            json!({
                "max_batch_items": 2,
                "max_concurrency": 2,
                "job_deadline": "1m",
                "coverage_policy": "sampled"
            })
        );
        assert!(changes["semantic_indexing"].get("description").is_none());
    }

    #[test]
    fn cross_field_limit_validation_is_atomic() {
        let document = complete_limits();
        let mut draft = semantic_draft_from_document(&document);
        draft.limits.roi_min_edge_pixels = "400".to_owned();
        draft.limits.roi_max_edge_pixels = "200".to_owned();
        draft.limits.search_top_k = "0".to_owned();
        draft.limits.decisions_coverage_policy = "always".to_owned();
        draft.description_model = "luna-2".to_owned();
        let errors = surgical_semantic_changes(&document, &draft).unwrap_err();
        assert!(errors.contains_key(ROI_MAX_EDGE));
        assert!(errors.contains_key(SEARCH_TOP_K));
        assert!(errors.contains_key(DECISIONS_COVERAGE));
        assert!(!errors.contains_key(DESCRIPTION_MODEL));
    }

    #[test]
    fn storage_paths_persist_exactly_as_entered() {
        let document = complete_limits();
        let mut draft = semantic_draft_from_document(&document);
        draft.limits.storage_vector_index_path = "./relative/index".to_owned();
        draft.limits.storage_sqlite_path = "/var/lib/a2a/semantic.sqlite".to_owned();
        let changes = surgical_semantic_changes(&document, &draft).unwrap();
        assert_eq!(
            changes["semantic_indexing"]["limits"]["storage"]["vector_index_path"],
            "./relative/index"
        );
        assert_eq!(
            changes["semantic_indexing"]["limits"]["storage"]["sqlite_path"],
            "/var/lib/a2a/semantic.sqlite"
        );
        assert!(
            changes["semantic_indexing"]["limits"]["storage"]
                .get("max_disk_bytes")
                .is_none()
        );
    }

    #[test]
    fn omitted_optional_limit_keys_stay_omitted() {
        let document = complete_limits();
        let mut draft = semantic_draft_from_document(&document);
        draft.limits.worker_max_concurrency = "4".to_owned();
        let changes = surgical_semantic_changes(&document, &draft).unwrap();
        assert_eq!(
            changes["semantic_indexing"]["limits"]["worker"]["max_concurrency"],
            4
        );
        assert!(
            changes["semantic_indexing"]["limits"]["worker"]
                .get("max_batch_items")
                .is_none()
        );
        assert!(
            changes["semantic_indexing"]["limits"]
                .get("search")
                .is_none()
        );
        assert!(
            changes["semantic_indexing"]["limits"]
                .get("storage")
                .is_none()
        );
        assert!(draft.limits.storage_sqlite_path.is_empty());
        assert!(draft.limits.search_score_floor.is_empty());
    }

    #[test]
    fn env_names_accept_digits_after_first() {
        assert!(is_env_name("A2ALAB_JEV_API_KEY"));
        assert!(!is_env_name("2FAST"));
        assert!(!is_env_name("HAS-DASH"));
    }
}
