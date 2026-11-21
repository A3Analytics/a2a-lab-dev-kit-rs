//! Source-backed image decision contract.
//!
//! Callers name a source, a capture time, and typed questions. The service loads
//! the image. Model selection stays in backend configuration.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::A2aLabError;
use crate::id::ImageSourceId;
use crate::images::decision::{
    ImageDecisionProviderError, MAX_IMAGE_DECISION_ANSWERS, MAX_IMAGE_DECISION_QUESTION_CHARS,
};
use crate::time::UtcTimestamp;

const MAX_QUESTION_ID_CHARS: usize = 128;
const MAX_CRITERION_CHARS: usize = 512;

/// Questions addressed to one resolved image, keyed by caller-chosen id.
pub type ImageDecisionQuestions = BTreeMap<String, ImageDecisionQuestion>;

/// One non-experimental Jev-style question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageDecisionQuestion {
    /// One option from an unordered set.
    Choice {
        /// Judgment the model should make.
        instructions: String,
        /// Option id to description.
        criteria: BTreeMap<String, String>,
    },
    /// A position on an ordered set of levels.
    Score {
        /// Judgment the model should make.
        instructions: String,
        /// Levels from low to high.
        criteria: Vec<String>,
    },
    /// A yes/no probability.
    Noul {
        /// Judgment the model should make.
        instructions: String,
        /// Optional descriptions of yes and no.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ImageDecisionQuestionRaw {
    Choice {
        instructions: String,
        criteria: BTreeMap<String, String>,
    },
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
    Noul {
        instructions: String,
        #[serde(default)]
        criteria: Option<NoulCriteria>,
    },
}

impl TryFrom<ImageDecisionQuestionRaw> for ImageDecisionQuestion {
    type Error = A2aLabError;

    fn try_from(raw: ImageDecisionQuestionRaw) -> Result<Self, Self::Error> {
        match raw {
            ImageDecisionQuestionRaw::Choice {
                instructions,
                criteria,
            } => choice_question(&instructions, criteria),
            ImageDecisionQuestionRaw::Score {
                instructions,
                criteria,
            } => score_question(&instructions, criteria),
            ImageDecisionQuestionRaw::Noul {
                instructions,
                criteria,
            } => noul_question(&instructions, criteria),
        }
    }
}

impl<'de> Deserialize<'de> for ImageDecisionQuestion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ImageDecisionQuestionRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// Descriptions of the yes and no outcomes for a noul question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoulCriteria {
    #[serde(rename = "true")]
    yes: String,
    #[serde(rename = "false")]
    no: String,
}

impl NoulCriteria {
    /// Validates both outcome descriptions.
    pub fn new(yes: impl AsRef<str>, no: impl AsRef<str>) -> Result<Self, A2aLabError> {
        Ok(Self {
            yes: bounded_text("criteria.true", yes.as_ref(), MAX_CRITERION_CHARS)?,
            no: bounded_text("criteria.false", no.as_ref(), MAX_CRITERION_CHARS)?,
        })
    }

    /// Description of the yes outcome.
    #[must_use]
    pub fn yes(&self) -> &str {
        &self.yes
    }

    /// Description of the no outcome.
    #[must_use]
    pub fn no(&self) -> &str {
        &self.no
    }
}

/// Public image-decision request. The image and model come from the backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SourceImageDecisionRequest {
    source_id: ImageSourceId,
    captured_at: UtcTimestamp,
    questions: ImageDecisionQuestions,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceImageDecisionRequestRaw {
    source_id: ImageSourceId,
    captured_at: UtcTimestamp,
    questions: ImageDecisionQuestions,
}

impl SourceImageDecisionRequest {
    /// Validates the source, capture time, and question map.
    pub fn new(
        source_id: ImageSourceId,
        captured_at: UtcTimestamp,
        questions: ImageDecisionQuestions,
    ) -> Result<Self, A2aLabError> {
        validate_questions(&questions)?;
        Ok(Self {
            source_id,
            captured_at,
            questions,
        })
    }

    /// Source whose retained frames are searched.
    #[must_use]
    pub const fn source_id(&self) -> &ImageSourceId {
        &self.source_id
    }

    /// Preferred capture time.
    #[must_use]
    pub const fn captured_at(&self) -> UtcTimestamp {
        self.captured_at
    }

    /// Typed questions evaluated against the resolved image.
    #[must_use]
    pub const fn questions(&self) -> &ImageDecisionQuestions {
        &self.questions
    }
}

impl TryFrom<SourceImageDecisionRequestRaw> for SourceImageDecisionRequest {
    type Error = A2aLabError;

    fn try_from(raw: SourceImageDecisionRequestRaw) -> Result<Self, Self::Error> {
        Self::new(raw.source_id, raw.captured_at, raw.questions)
    }
}

impl<'de> Deserialize<'de> for SourceImageDecisionRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        SourceImageDecisionRequestRaw::deserialize(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

/// Typed answer for one question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ImageDecisionPrimitiveAnswer {
    /// Yes probability in `0.0..=1.0`.
    Noul {
        /// Probability that the answer is yes.
        noul: f32,
    },
    /// Selected option and its distribution.
    Choice {
        /// Selected option id.
        choice: String,
        /// Probability for every supplied option.
        probabilities: BTreeMap<String, f32>,
        /// How peaked the distribution is.
        confidence: f32,
    },
    /// Position on the supplied levels.
    Score {
        /// Position along the levels.
        score: f32,
        /// Levels repeated from the question.
        legend: Vec<String>,
        /// Probability for every level.
        probabilities: Vec<f32>,
        /// How peaked the distribution is.
        confidence: f32,
    },
}

/// Answers keyed by the question id from the request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SourceImageDecisionResult {
    answers: BTreeMap<String, ImageDecisionPrimitiveAnswer>,
}

impl SourceImageDecisionResult {
    /// Validates answers against the questions that produced them.
    pub fn from_questions(
        questions: &ImageDecisionQuestions,
        answers: BTreeMap<String, ImageDecisionPrimitiveAnswer>,
    ) -> Result<Self, ImageDecisionProviderError> {
        if answers.len() != questions.len() {
            return Err(invalid_answer(
                "response answers do not match the questions",
            ));
        }
        for (id, question) in questions {
            let Some(answer) = answers.get(id) else {
                return Err(invalid_answer(
                    "response answers do not match the questions",
                ));
            };
            validate_answer(question, answer)?;
        }
        Ok(Self { answers })
    }

    /// Answers keyed by question id.
    #[must_use]
    pub const fn answers(&self) -> &BTreeMap<String, ImageDecisionPrimitiveAnswer> {
        &self.answers
    }
}

pub(crate) fn validate_answer(
    question: &ImageDecisionQuestion,
    answer: &ImageDecisionPrimitiveAnswer,
) -> Result<(), ImageDecisionProviderError> {
    match (question, answer) {
        (ImageDecisionQuestion::Noul { .. }, ImageDecisionPrimitiveAnswer::Noul { noul }) => {
            check_unit(*noul, "noul")
        }
        (
            ImageDecisionQuestion::Choice { criteria, .. },
            ImageDecisionPrimitiveAnswer::Choice {
                choice,
                probabilities,
                confidence,
            },
        ) => {
            if !criteria.contains_key(choice) || probabilities.len() != criteria.len() {
                return Err(invalid_answer("choice answer does not match its criteria"));
            }
            for key in criteria.keys() {
                let Some(probability) = probabilities.get(key) else {
                    return Err(invalid_answer("choice answer does not match its criteria"));
                };
                check_unit(*probability, "probability")?;
            }
            check_unit(*confidence, "confidence")
        }
        (
            ImageDecisionQuestion::Score { criteria, .. },
            ImageDecisionPrimitiveAnswer::Score {
                score,
                legend,
                probabilities,
                confidence,
            },
        ) => {
            if !score.is_finite() || legend != criteria || probabilities.len() != criteria.len() {
                return Err(invalid_answer("score answer does not match its criteria"));
            }
            for probability in probabilities {
                check_unit(*probability, "probability")?;
            }
            check_unit(*confidence, "confidence")
        }
        _ => Err(invalid_answer(
            "response answer type does not match the question",
        )),
    }
}

fn validate_questions(questions: &ImageDecisionQuestions) -> Result<(), A2aLabError> {
    if questions.is_empty() || questions.len() > MAX_IMAGE_DECISION_ANSWERS {
        return Err(A2aLabError::invalid(
            "questions",
            format!("must contain 1..={MAX_IMAGE_DECISION_ANSWERS} questions"),
        ));
    }
    for id in questions.keys() {
        bounded_token("questions", id, MAX_QUESTION_ID_CHARS)?;
    }
    Ok(())
}

fn choice_question(
    instructions: &str,
    criteria: BTreeMap<String, String>,
) -> Result<ImageDecisionQuestion, A2aLabError> {
    if criteria.len() < 2 {
        return Err(A2aLabError::invalid(
            "criteria",
            "choice requires at least two options",
        ));
    }
    let mut validated = BTreeMap::new();
    for (key, value) in criteria {
        let key = bounded_token("criteria", &key, MAX_QUESTION_ID_CHARS)?;
        let value = bounded_text("criteria", &value, MAX_CRITERION_CHARS)?;
        if validated.insert(key, value).is_some() {
            return Err(A2aLabError::invalid(
                "criteria",
                "option ids must be unique",
            ));
        }
    }
    Ok(ImageDecisionQuestion::Choice {
        instructions: bounded_text(
            "instructions",
            instructions,
            MAX_IMAGE_DECISION_QUESTION_CHARS,
        )?,
        criteria: validated,
    })
}

fn score_question(
    instructions: &str,
    criteria: Vec<String>,
) -> Result<ImageDecisionQuestion, A2aLabError> {
    if criteria.len() < 2 {
        return Err(A2aLabError::invalid(
            "criteria",
            "score requires at least two levels",
        ));
    }
    let criteria = criteria
        .into_iter()
        .map(|level| bounded_text("criteria", &level, MAX_CRITERION_CHARS))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ImageDecisionQuestion::Score {
        instructions: bounded_text(
            "instructions",
            instructions,
            MAX_IMAGE_DECISION_QUESTION_CHARS,
        )?,
        criteria,
    })
}

fn noul_question(
    instructions: &str,
    criteria: Option<NoulCriteria>,
) -> Result<ImageDecisionQuestion, A2aLabError> {
    let criteria = criteria
        .map(|criteria| NoulCriteria::new(criteria.yes(), criteria.no()))
        .transpose()?;
    Ok(ImageDecisionQuestion::Noul {
        instructions: bounded_text(
            "instructions",
            instructions,
            MAX_IMAGE_DECISION_QUESTION_CHARS,
        )?,
        criteria,
    })
}

fn check_unit(value: f32, field: &str) -> Result<(), ImageDecisionProviderError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        return Ok(());
    }
    Err(invalid_answer(&format!(
        "{field} must be a finite value in 0.0..=1.0"
    )))
}

fn invalid_answer(message: &str) -> ImageDecisionProviderError {
    ImageDecisionProviderError::invalid_answer(message).unwrap_or_else(|_| {
        ImageDecisionProviderError::InvalidAnswer {
            message: "response answers are invalid".to_owned(),
        }
    })
}

fn bounded_text(field: &'static str, value: &str, max_chars: usize) -> Result<String, A2aLabError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > max_chars {
        return Err(A2aLabError::invalid(
            field,
            format!("must be 1..={max_chars} visible characters"),
        ));
    }
    Ok(trimmed.to_owned())
}

fn bounded_token(
    field: &'static str,
    value: &str,
    max_chars: usize,
) -> Result<String, A2aLabError> {
    let token = bounded_text(field, value, max_chars)?;
    if token.chars().any(char::is_whitespace) {
        return Err(A2aLabError::invalid(field, "must not contain whitespace"));
    }
    Ok(token)
}
