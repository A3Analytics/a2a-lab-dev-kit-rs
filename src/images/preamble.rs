//! Composes an optional decision preamble onto outbound question instructions.
//!
//! An omitted or blank preamble leaves the caller instructions unchanged. A
//! nonblank preamble is placed before those instructions, separated by one
//! newline. The exact configured characters are kept.

use std::borrow::Cow;

use super::decision::{ImageDecisionProviderError, MAX_IMAGE_DECISION_QUESTION_CHARS};
use super::source_decision::ImageDecisionQuestions;

pub(crate) fn has_preamble(preamble: Option<&str>) -> bool {
    preamble.is_some_and(|text| text.chars().any(|character| !character.is_whitespace()))
}

pub(crate) fn outbound_instructions<'a>(
    preamble: Option<&str>,
    instructions: &'a str,
) -> Result<Cow<'a, str>, ImageDecisionProviderError> {
    let Some(preamble) = preamble.filter(|_| has_preamble(preamble)) else {
        return Ok(Cow::Borrowed(instructions));
    };
    let composed = format!("{preamble}\n{instructions}");
    ensure_question_len(&composed)?;
    Ok(Cow::Owned(composed))
}

pub(crate) fn outbound_questions<'a>(
    preamble: Option<&str>,
    questions: &'a ImageDecisionQuestions,
) -> Result<Cow<'a, ImageDecisionQuestions>, ImageDecisionProviderError> {
    if !has_preamble(preamble) {
        return Ok(Cow::Borrowed(questions));
    }
    let mut composed = ImageDecisionQuestions::new();
    for (name, question) in questions {
        let instructions = outbound_instructions(preamble, question.instructions())?;
        composed.insert(
            name.clone(),
            question.with_instructions(instructions.into_owned()),
        );
    }
    Ok(Cow::Owned(composed))
}

fn ensure_question_len(text: &str) -> Result<(), ImageDecisionProviderError> {
    let count = text.chars().count();
    if (1..=MAX_IMAGE_DECISION_QUESTION_CHARS).contains(&count) {
        return Ok(());
    }
    Err(ImageDecisionProviderError::limit_exceeded(
        "question",
        "question exceeds the configured maximum",
    )
    .expect("question limit text is bounded"))
}
