//! Per-source semantic schedule drafts nested under `videos.sources[]`.

use serde_json::{Map, Value};

use super::{FieldErrors, SemanticDraft, push_optional_string, string_or_empty};

pub const EVERY_SAVED_FRAME: &str = "every_saved_frame";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSemanticDraft {
    pub present: bool,
    pub description: String,
    pub embeddings_present: bool,
    pub embeddings: String,
    pub description_model: String,
    pub embedding_model: String,
}

impl SourceSemanticDraft {
    pub fn empty() -> Self {
        Self {
            present: false,
            description: String::new(),
            embeddings_present: false,
            embeddings: String::new(),
            description_model: String::new(),
            embedding_model: String::new(),
        }
    }

    pub fn enabled() -> Self {
        Self {
            present: true,
            ..Self::empty()
        }
    }
}

pub fn from_value(value: &Value) -> SourceSemanticDraft {
    let Some(semantic) = value.get("semantic_indexing") else {
        return SourceSemanticDraft::empty();
    };
    SourceSemanticDraft {
        present: true,
        description: string_or_empty(semantic.get("description")),
        embeddings_present: semantic.get("embeddings").is_some()
            || semantic.get("embedding_model").is_some(),
        embeddings: string_or_empty(semantic.get("embeddings")),
        description_model: string_or_empty(semantic.get("description_model")),
        embedding_model: string_or_empty(semantic.get("embedding_model")),
    }
}

pub fn patch(original: Option<&Value>, draft: &SourceSemanticDraft) -> Map<String, Value> {
    let mut nested = Map::new();
    let current = original.and_then(|value| value.get("semantic_indexing"));
    if !draft.present {
        if current.is_some() {
            nested.insert("semantic_indexing".to_owned(), Value::Null);
        }
        return nested;
    }
    let mut object = Map::new();
    push_optional_string(
        &mut object,
        "description",
        current.and_then(|value| value.get("description")),
        &draft.description,
    );
    push_optional_string(
        &mut object,
        "description_model",
        current.and_then(|value| value.get("description_model")),
        &draft.description_model,
    );
    if draft.embeddings_present {
        let embeddings = if draft.embeddings.trim().is_empty()
            && current.and_then(|value| value.get("embeddings")).is_none()
        {
            EVERY_SAVED_FRAME
        } else {
            draft.embeddings.trim()
        };
        push_optional_string(
            &mut object,
            "embeddings",
            current.and_then(|value| value.get("embeddings")),
            embeddings,
        );
        push_optional_string(
            &mut object,
            "embedding_model",
            current.and_then(|value| value.get("embedding_model")),
            &draft.embedding_model,
        );
    } else {
        if current.and_then(|value| value.get("embeddings")).is_some() {
            object.insert("embeddings".to_owned(), Value::Null);
        }
        if current
            .and_then(|value| value.get("embedding_model"))
            .is_some()
        {
            object.insert("embedding_model".to_owned(), Value::Null);
        }
    }
    if current.is_none() || !object.is_empty() {
        nested.insert("semantic_indexing".to_owned(), Value::Object(object));
    }
    nested
}

pub fn validate(
    errors: &mut FieldErrors,
    index: usize,
    draft: &SourceSemanticDraft,
    semantic: &SemanticDraft,
) {
    if !draft.present {
        return;
    }
    validate_schedule(errors, index, "description", &draft.description);
    reject_blank_model(errors, index, "description_model", &draft.description_model);
    if draft.embeddings_present || !draft.embedding_model.is_empty() {
        if !semantic.embeddings_present {
            errors.insert(
                source_path(index, "semantic_indexing.embeddings"),
                "videos.sources.semantic_indexing.embeddings: embedding settings are not supported in description_only mode".to_owned(),
            );
        }
        validate_schedule(errors, index, "embeddings", &draft.embeddings);
        reject_blank_model(errors, index, "embedding_model", &draft.embedding_model);
        let model = draft.embedding_model.trim();
        if !model.is_empty()
            && semantic.embeddings_present
            && model != semantic.embeddings_model.trim()
        {
            errors.insert(
                source_path(index, "semantic_indexing.embedding_model"),
                "videos.sources.semantic_indexing.embedding_model: must match semantic_indexing.embeddings.model".to_owned(),
            );
        }
    }
}

pub fn is_parser_schedule(text: &str) -> bool {
    text == EVERY_SAVED_FRAME || is_positive_finite_duration(text)
}

pub fn source_path(index: usize, field: &str) -> String {
    format!("videos.sources.{index}.{field}")
}

fn validate_schedule(errors: &mut FieldErrors, index: usize, field: &str, value: &str) {
    let path = source_path(index, &format!("semantic_indexing.{field}"));
    let trimmed = value.trim();
    if trimmed.is_empty() || is_parser_schedule(trimmed) {
        return;
    }
    errors.insert(
        path,
        format!(
            "videos.sources.semantic_indexing.{field} must be every_saved_frame or a positive finite duration."
        ),
    );
}

fn reject_blank_model(errors: &mut FieldErrors, index: usize, field: &str, value: &str) {
    if value.is_empty() || value.chars().any(|character| !character.is_whitespace()) {
        return;
    }
    errors.insert(
        source_path(index, &format!("semantic_indexing.{field}")),
        format!("videos.sources.semantic_indexing.{field} must be a non-empty string."),
    );
}

fn is_positive_finite_duration(text: &str) -> bool {
    if text.starts_with('-') {
        return false;
    }
    let mut rest = text;
    let mut nanos: u128 = 0;
    let mut saw = false;
    while !rest.is_empty() {
        let Some((amount, _unit, scale, next)) = parse_amount(rest) else {
            return false;
        };
        rest = next;
        let Some(contribution) = fractional_nanos(amount, scale) else {
            return false;
        };
        let Some(total) = nanos.checked_add(contribution) else {
            return false;
        };
        nanos = total;
        saw = true;
    }
    saw && nanos > 0
}

fn parse_amount(text: &str) -> Option<(f64, &'static str, u128, &str)> {
    let mut end = 0;
    let mut dot = false;
    for (index, character) in text.char_indices() {
        if character.is_ascii_digit() {
            end = index + 1;
            continue;
        }
        if character == '.' && !dot {
            dot = true;
            end = index + 1;
            continue;
        }
        break;
    }
    if end == 0 || text[..end].ends_with('.') {
        return None;
    }
    let amount: f64 = text[..end].parse().ok()?;
    let rest = &text[end..];
    let (unit, scale) = duration_unit(rest)?;
    Some((amount, unit, scale, &rest[unit.len()..]))
}

fn duration_unit(text: &str) -> Option<(&'static str, u128)> {
    const SECOND: u128 = 1_000_000_000;
    [
        ("ms", 1_000_000u128),
        ("us", 1_000u128),
        ("ns", 1u128),
        ("s", SECOND),
        ("m", 60 * SECOND),
        ("h", 60 * 60 * SECOND),
        ("d", 24 * 60 * 60 * SECOND),
        ("w", 7 * 24 * 60 * 60 * SECOND),
    ]
    .into_iter()
    .find(|(unit, _)| text.starts_with(unit))
}

fn fractional_nanos(amount: f64, scale: u128) -> Option<u128> {
    if !amount.is_finite() || amount <= 0.0 {
        return None;
    }
    let nanos = amount * scale as f64;
    if !nanos.is_finite() || nanos <= 0.0 {
        return None;
    }
    Some(nanos.round() as u128)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_semantic::{reindex_required, semantic_draft_from_document};
    use serde_json::json;

    fn original() -> Value {
        json!({
            "schema_version": 2,
            "semantic_indexing": {
                "description": { "model": "luna" },
                "embeddings": {
                    "provider": "p",
                    "model": "m",
                    "space": { "dimensions": 4, "normalization": "l2_unit", "metric": "cosine" }
                }
            },
            "videos": {
                "sources": [
                    { "id": "camera", "format": "yuyv", "device_index": 6 },
                    {
                        "id": "detail",
                        "camera_id": "/dev/video1",
                        "semantic_indexing": {
                            "description": "every_saved_frame",
                            "embeddings": "1.5s",
                            "embedding_model": "m"
                        }
                    }
                ],
                "dvr": { "storage_path": "./relative/dvr" }
            }
        })
    }

    fn sources() -> Vec<Value> {
        original()["videos"]["sources"].as_array().unwrap().clone()
    }

    fn semantic() -> SemanticDraft {
        semantic_draft_from_document(&original())
    }

    fn source_changes(drafts: &[SourceSemanticDraft]) -> Value {
        let items = sources();
        let mut array = Vec::new();
        let mut last = None;
        for (index, draft) in drafts.iter().enumerate() {
            let nested = super::patch(items.get(index), draft);
            if nested.is_empty() {
                array.push(json!({}));
            } else {
                last = Some(index);
                array.push(Value::Object(nested));
            }
        }
        let Some(last) = last else {
            return json!({});
        };
        array.truncate(last + 1);
        json!({ "videos": { "sources": array } })
    }

    #[test]
    fn omitted_source_block_stays_omitted() {
        let items = sources();
        let drafts: Vec<_> = items.iter().map(from_value).collect();
        assert!(!drafts[0].present);
        assert!(drafts[1].present);
        assert_eq!(source_changes(&drafts), json!({}));
        assert!(drafts[1].description_model.is_empty());
    }

    #[test]
    fn schedule_forms_keep_source_identity_inside_sources() {
        let items = sources();
        let mut drafts: Vec<_> = items.iter().map(from_value).collect();
        drafts[0].present = true;
        drafts[0].description = EVERY_SAVED_FRAME.to_owned();
        drafts[1].description = "2s".to_owned();
        let changes = source_changes(&drafts);
        assert_eq!(
            changes,
            json!({
                "videos": {
                    "sources": [
                        { "semantic_indexing": { "description": "every_saved_frame" } },
                        { "semantic_indexing": { "description": "2s" } }
                    ]
                }
            })
        );
        assert!(changes["videos"]["sources"][0].get("id").is_none());
        assert!(changes["videos"]["sources"][1].get("camera_id").is_none());
        assert!(changes.get("semantic_indexing").is_none());
    }

    #[test]
    fn omitted_model_overrides_inherit_and_are_not_written() {
        let mut draft = SourceSemanticDraft::enabled();
        draft.description = "5s".to_owned();
        let nested = patch(sources().first(), &draft);
        let semantic = &nested["semantic_indexing"];
        assert_eq!(semantic["description"], "5s");
        assert!(semantic.get("description_model").is_none());
        assert!(semantic.get("embedding_model").is_none());
        assert!(semantic.get("embeddings").is_none());
    }

    #[test]
    fn typed_paths_stay_literal_on_identity_change() {
        let items = sources();
        let mut drafts: Vec<_> = items.iter().map(from_value).collect();
        drafts[1].description_model = "luna-2".to_owned();
        let changes = source_changes(&drafts);
        assert_eq!(
            changes["videos"]["sources"][1]["semantic_indexing"]["description_model"],
            "luna-2"
        );
        assert_eq!(
            original()["videos"]["dvr"]["storage_path"],
            "./relative/dvr"
        );
        assert_eq!(
            original()["videos"]["sources"][1]["camera_id"],
            "/dev/video1"
        );
        assert!(changes["videos"].get("dvr").is_none());
    }

    #[test]
    fn mismatched_embedding_model_is_rejected() {
        let items = sources();
        let mut drafts: Vec<_> = items.iter().map(from_value).collect();
        drafts[1].embedding_model = "other".to_owned();
        let mut errors = FieldErrors::new();
        validate(&mut errors, 1, &drafts[1], &semantic());
        assert!(errors.contains_key("videos.sources.1.semantic_indexing.embedding_model"));
        assert!(patch(items.get(1), &drafts[1]).contains_key("semantic_indexing"));
    }

    #[test]
    fn description_only_rejects_source_embeddings() {
        let semantic = semantic_draft_from_document(&json!({
            "semantic_indexing": { "description": { "model": "luna" } }
        }));
        let mut draft = SourceSemanticDraft::enabled();
        draft.embeddings_present = true;
        draft.embeddings = EVERY_SAVED_FRAME.to_owned();
        let mut errors = FieldErrors::new();
        validate(&mut errors, 0, &draft, &semantic);
        assert!(errors.contains_key("videos.sources.0.semantic_indexing.embeddings"));
    }

    #[test]
    fn permitted_identity_change_is_reindex_not_live() {
        assert!(reindex_required(
            "videos.sources.0.semantic_indexing.description_model"
        ));
        assert!(reindex_required(
            "videos.sources.1.semantic_indexing.embedding_model"
        ));
        assert!(!reindex_required(
            "videos.sources.0.semantic_indexing.description"
        ));
        assert!(!reindex_required(
            "videos.sources.0.semantic_indexing.embeddings"
        ));
    }

    #[test]
    fn invalid_schedule_is_rejected() {
        let mut draft = from_value(&sources()[1]);
        draft.description = "0s".to_owned();
        let mut errors = FieldErrors::new();
        validate(&mut errors, 1, &draft, &semantic());
        assert!(errors.contains_key("videos.sources.1.semantic_indexing.description"));
        assert!(is_parser_schedule("1.5s"));
        assert!(is_parser_schedule("every_saved_frame"));
        assert!(!is_parser_schedule("0s"));
    }
}
