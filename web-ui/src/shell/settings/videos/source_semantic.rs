//! Per-source semantic schedule and permitted model-override editors.

use dioxus::prelude::*;

use super::{SourceDraft, error_text, field_id, source_path};
use crate::settings_semantic::reindex_required;
use crate::settings_semantic::source::{EVERY_SAVED_FRAME, SourceSemanticDraft};
use crate::shell::settings::chrome::TextField;
use crate::shell::settings::document::{FieldErrors, restart_required};

#[component]
pub fn SourceSemanticFields(
    index: usize,
    source: SourceDraft,
    errors: FieldErrors,
    locked: bool,
) -> Element {
    if !source.semantic.present {
        return rsx! {
            p { class: "settings-help", "videos.sources.semantic_indexing is omitted. Global models stay inherited until this source enables a schedule." }
            button {
                id: field_id(index, "semantic-add"),
                class: "settings-button-outline",
                r#type: "button",
                disabled: locked,
                onclick: move |_| super::super::edit_source(index, |source| source.semantic = SourceSemanticDraft::enabled()),
                "Add semantic indexing"
            }
        };
    }
    rsx! {
        div { class: "settings-source-semantic",
            TextField {
                id: field_id(index, "semantic-description"),
                label: format!("videos.sources.{index}.semantic_indexing.description"),
                help: "every_saved_frame or a positive finite duration such as 1.5s. Leave blank to omit the key.".to_string(),
                error: error_text(&errors, &source_path(index, "semantic_indexing.description")),
                restart: restart_required(&source_path(index, "semantic_indexing.description")),
                reindex: reindex_required(&source_path(index, "semantic_indexing.description")),
                locked,
                value: source.semantic.description.clone(),
                oninput: move |value| super::super::edit_source(index, move |source| source.semantic.description = value),
            }
            TextField {
                id: field_id(index, "semantic-description-model"),
                label: format!("videos.sources.{index}.semantic_indexing.description_model"),
                help: "Optional source description model. Leave blank to inherit the global description model.".to_string(),
                error: error_text(&errors, &source_path(index, "semantic_indexing.description_model")),
                restart: restart_required(&source_path(index, "semantic_indexing.description_model")),
                reindex: reindex_required(&source_path(index, "semantic_indexing.description_model")),
                locked,
                value: source.semantic.description_model.clone(),
                oninput: move |value| super::super::edit_source(index, move |source| source.semantic.description_model = value),
            }
            if source.semantic.embeddings_present {
                TextField {
                    id: field_id(index, "semantic-embeddings"),
                    label: format!("videos.sources.{index}.semantic_indexing.embeddings"),
                    help: "every_saved_frame or a positive finite duration. Leave blank to omit the key.".to_string(),
                    error: error_text(&errors, &source_path(index, "semantic_indexing.embeddings")),
                    restart: restart_required(&source_path(index, "semantic_indexing.embeddings")),
                    reindex: reindex_required(&source_path(index, "semantic_indexing.embeddings")),
                    locked,
                    value: source.semantic.embeddings.clone(),
                    oninput: move |value| super::super::edit_source(index, move |source| source.semantic.embeddings = value),
                }
                TextField {
                    id: field_id(index, "semantic-embedding-model"),
                    label: format!("videos.sources.{index}.semantic_indexing.embedding_model"),
                    help: "Optional. When set, must match semantic_indexing.embeddings.model. Leave blank to inherit.".to_string(),
                    error: error_text(&errors, &source_path(index, "semantic_indexing.embedding_model")),
                    restart: restart_required(&source_path(index, "semantic_indexing.embedding_model")),
                    reindex: reindex_required(&source_path(index, "semantic_indexing.embedding_model")),
                    locked,
                    value: source.semantic.embedding_model.clone(),
                    oninput: move |value| super::super::edit_source(index, move |source| source.semantic.embedding_model = value),
                }
                button {
                    id: field_id(index, "semantic-remove-embeddings"),
                    class: "settings-button-outline",
                    r#type: "button",
                    disabled: locked,
                    onclick: move |_| super::super::edit_source(index, |source| {
                        source.semantic.embeddings_present = false;
                        source.semantic.embeddings.clear();
                        source.semantic.embedding_model.clear();
                    }),
                    "Remove embeddings"
                }
            } else {
                p { class: "settings-help", "Source embeddings are omitted. Description-only sources inherit the global embedding policy when one exists." }
                button {
                    id: field_id(index, "semantic-add-embeddings"),
                    class: "settings-button-outline",
                    r#type: "button",
                    disabled: locked,
                    onclick: move |_| super::super::edit_source(index, |source| {
                        source.semantic.embeddings_present = true;
                        if source.semantic.embeddings.is_empty() {
                            source.semantic.embeddings = EVERY_SAVED_FRAME.to_owned();
                        }
                    }),
                    "Add embeddings"
                }
            }
            button {
                id: field_id(index, "semantic-remove"),
                class: "settings-button-outline",
                r#type: "button",
                disabled: locked,
                onclick: move |_| super::super::edit_source(index, |source| source.semantic = SourceSemanticDraft::empty()),
                "Remove semantic indexing"
            }
        }
    }
}
