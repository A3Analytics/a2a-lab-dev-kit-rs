//! TypeSafe System One client for one image and named questions.
//!
//! The caller supplies the absolute endpoint and API key. This module does not
//! read lab configuration or append a typed frame-decision record.

use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;
use base64::Engine;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use reqwest::{StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;
use crate::images::{
    Image, ImageDecisionAnswer, ImageDecisionHttpLimits, ImageDecisionModelInput,
    ImageDecisionPrimitiveAnswer, ImageDecisionProviderError, ImageDecisionQuestion,
    ImageDecisionQuestions, ImageDecisionRoi, MAX_IMAGE_DECISION_QUESTION_CHARS,
    SourceImageDecisionResult,
};

use super::decision::{ImageDecisionExecutor, ImageDecisionRequest, ImageDecisionResult};
use super::decision_http::{
    content_type_is_json, deadline_exceeded, invalid_answer, limit_exceeded, parse_endpoint,
    read_capped, transport_error, unavailable,
};

const IMAGE_REFERENCE: &str = "3f6c2a90-7b14-4e2d-9a55-c0ffee12a21b";
const DANGER_ZONE: &str = "typesafe-danger-zone-images";
const MAX_QUESTION_NAME_CHARS: usize = 128;

/// Posts named image questions to a TypeSafe System One endpoint.
///
/// Choice, score, and noul answers become [`SourceImageDecisionResult`]. Box
/// answers become [`ImageDecisionAnswer`] values whose source box uses
/// [`ImageDecisionRoi`] bounds.
#[derive(Clone)]
pub struct TypeSafeSystemOneClient {
    endpoint: Url,
    authorization: HeaderValue,
    limits: ImageDecisionHttpLimits,
    http: reqwest::Client,
}

impl fmt::Debug for TypeSafeSystemOneClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TypeSafeSystemOneClient")
            .field("endpoint", &self.endpoint.as_str())
            .field("api_key", &"<redacted>")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl TypeSafeSystemOneClient {
    /// Builds a client for an absolute `http` or `https` endpoint and a bearer key.
    pub fn new(
        endpoint: impl AsRef<str>,
        api_key: impl AsRef<str>,
        limits: ImageDecisionHttpLimits,
    ) -> Result<Self, A2aLabError> {
        let endpoint = parse_endpoint(endpoint.as_ref())?;
        let authorization = bearer(api_key.as_ref())?;
        let http = reqwest::Client::builder()
            .timeout(limits.deadline())
            .connect_timeout(limits.deadline())
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| A2aLabError::unavailable("decision http client could not be built"))?;
        Ok(Self {
            endpoint,
            authorization,
            limits,
            http,
        })
    }

    /// Absolute System One endpoint.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }

    /// Active image, body, and deadline bounds.
    #[must_use]
    pub const fn limits(&self) -> ImageDecisionHttpLimits {
        self.limits
    }

    /// Sends choice, score, and noul questions for one image.
    pub async fn decide_questions(
        &self,
        model: &str,
        image: &Image,
        questions: &ImageDecisionQuestions,
    ) -> Result<SourceImageDecisionResult, ImageDecisionProviderError> {
        if questions.is_empty() {
            return Err(invalid_answer("questions are invalid"));
        }
        let model = validated_model(model)?;
        let answers = self.exchange(&model, image, questions, false).await?;
        let mut primitives = BTreeMap::new();
        for (name, question) in questions {
            let Some(answer) = answers.get(name) else {
                return Err(invalid_answer(
                    "response answers do not match the questions",
                ));
            };
            primitives.insert(name.clone(), primitive_answer(question, answer)?);
        }
        if primitives.len() != answers.len() {
            return Err(invalid_answer(
                "response answers do not match the questions",
            ));
        }
        SourceImageDecisionResult::from_questions(questions, primitives)
    }

    /// Sends named box questions and returns source-image bounds.
    ///
    /// `questions` maps a question name to its locate instructions.
    pub async fn decide_boxes(
        &self,
        model: &str,
        image: &Image,
        questions: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, ImageDecisionAnswer>, ImageDecisionProviderError> {
        let wire = box_questions(questions)?;
        let model = validated_model(model)?;
        let answers = self.exchange(&model, image, &wire, true).await?;
        boxes_from_answers(questions, &answers)
    }

    async fn exchange<Q: Serialize>(
        &self,
        model: &str,
        image: &Image,
        questions: &Q,
        has_box: bool,
    ) -> Result<BTreeMap<String, WireAnswer>, ImageDecisionProviderError> {
        let body = encode_body(model, image, questions, self.limits)?;
        let response = authorized(
            self.http.post(self.endpoint.clone()),
            &self.authorization,
            has_box,
        )
        .body(body)
        .send()
        .await
        .map_err(transport_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_error(status));
        }
        let json = content_type_is_json(response.headers());
        let payload = read_capped(response, self.limits.max_response_bytes()).await?;
        if !json {
            return Err(invalid_answer(
                "response content type is not application/json",
            ));
        }
        decode_answers(&payload)
    }
}

/// `decide_image` executor that pins one model on a TypeSafe System One client.
pub(crate) struct PinnedTypeSafeDecision {
    client: TypeSafeSystemOneClient,
    model: String,
}

impl PinnedTypeSafeDecision {
    pub(crate) fn new(
        endpoint: &str,
        api_key: &str,
        model: &str,
        limits: ImageDecisionHttpLimits,
    ) -> Result<Self, A2aLabError> {
        Ok(Self {
            client: TypeSafeSystemOneClient::new(endpoint, api_key, limits)?,
            model: model.to_owned(),
        })
    }
}

#[async_trait]
impl ImageDecisionExecutor for PinnedTypeSafeDecision {
    async fn decide(
        &self,
        _request: &ImageDecisionRequest,
    ) -> Result<ImageDecisionResult, ImageDecisionProviderError> {
        Err(unavailable(
            "image decision client does not accept generic requests",
        ))
    }

    async fn decide_questions(
        &self,
        image: &Image,
        questions: &ImageDecisionQuestions,
    ) -> Result<SourceImageDecisionResult, ImageDecisionProviderError> {
        self.client
            .decide_questions(&self.model, image, questions)
            .await
    }
}

fn bearer(api_key: &str) -> Result<HeaderValue, A2aLabError> {
    if api_key.is_empty() || api_key.chars().any(char::is_whitespace) {
        return Err(A2aLabError::invalid(
            "api_key",
            "must be a non-empty bearer token",
        ));
    }
    let mut value = HeaderValue::try_from(format!("Bearer {api_key}"))
        .map_err(|_| A2aLabError::invalid("api_key", "must be a non-empty bearer token"))?;
    value.set_sensitive(true);
    Ok(value)
}

fn validated_model(model: &str) -> Result<String, ImageDecisionProviderError> {
    ImageDecisionModelInput::new(model, None)
        .map(|input| input.model().to_owned())
        .map_err(|_| invalid_answer("model is invalid"))
}

const fn sends_danger_zone(has_image: bool, has_box: bool) -> bool {
    has_image || has_box
}

fn authorized(
    request: reqwest::RequestBuilder,
    authorization: &HeaderValue,
    has_box: bool,
) -> reqwest::RequestBuilder {
    let request = request
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json")
        .header(AUTHORIZATION, authorization);
    if sends_danger_zone(true, has_box) {
        request.header(DANGER_ZONE, "true")
    } else {
        request
    }
}

fn encode_body<Q: Serialize>(
    model: &str,
    image: &Image,
    questions: &Q,
    limits: ImageDecisionHttpLimits,
) -> Result<Vec<u8>, ImageDecisionProviderError> {
    let format = image_format(image.descriptor().media_type())?;
    limits
        .image_transport()
        .check_payload(image.data())
        .map_err(|_| limit_exceeded("image", "image exceeds the configured maximum"))?;
    let data_base64 = base64::engine::general_purpose::STANDARD.encode(image.data());
    let mut assets = BTreeMap::new();
    assets.insert(
        IMAGE_REFERENCE,
        Asset {
            kind: "image",
            format,
            data_base64: &data_base64,
        },
    );
    let body = SystemOneBody {
        model,
        state: ImageReference {
            image: IMAGE_REFERENCE,
        },
        assets,
        questions,
    };
    let bytes = serde_json::to_vec(&body).map_err(|_| invalid_answer("request json is invalid"))?;
    if bytes.len() > limits.max_request_bytes() {
        return Err(limit_exceeded(
            "request",
            "request exceeds the configured maximum",
        ));
    }
    Ok(bytes)
}

fn image_format(media_type: &str) -> Result<&'static str, ImageDecisionProviderError> {
    let Some((kind, rest)) = media_type.split_once('/') else {
        return Err(invalid_answer("image format is not supported"));
    };
    if !kind.eq_ignore_ascii_case("image") {
        return Err(invalid_answer("image format is not supported"));
    }
    let subtype = rest
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match subtype.as_str() {
        "png" => Ok("png"),
        "jpeg" => Ok("jpeg"),
        "jpg" => Ok("jpg"),
        _ => Err(invalid_answer("image format is not supported")),
    }
}

fn box_questions(
    questions: &BTreeMap<String, String>,
) -> Result<BTreeMap<&str, BoxQuestion<'_>>, ImageDecisionProviderError> {
    if questions.is_empty() {
        return Err(invalid_answer("questions are invalid"));
    }
    let mut wire = BTreeMap::new();
    for (name, instructions) in questions {
        if !valid_question_name(name) || !valid_instructions(instructions) {
            return Err(invalid_answer("questions are invalid"));
        }
        wire.insert(
            name.as_str(),
            BoxQuestion {
                kind: "box",
                instructions: instructions.as_str(),
            },
        );
    }
    Ok(wire)
}

fn valid_question_name(name: &str) -> bool {
    let count = name.chars().count();
    (1..=MAX_QUESTION_NAME_CHARS).contains(&count) && !name.chars().any(char::is_whitespace)
}

fn valid_instructions(text: &str) -> bool {
    let count = text.chars().count();
    (1..=MAX_IMAGE_DECISION_QUESTION_CHARS).contains(&count)
        && !text.is_empty()
        && text.trim() == text
}

fn status_error(status: StatusCode) -> ImageDecisionProviderError {
    match status.as_u16() {
        400 => invalid_answer("http status 400"),
        401 => unavailable("http status 401"),
        408 | 504 => deadline_exceeded("decision deadline elapsed"),
        413 => limit_exceeded("request", "request exceeds the configured maximum"),
        415 => invalid_answer("http status 415"),
        422 => invalid_answer("http status 422"),
        429 => unavailable("http status 429"),
        529 => unavailable("http status 529"),
        code => unavailable(&format!("http status {code}")),
    }
}

fn decode_answers(
    payload: &[u8],
) -> Result<BTreeMap<String, WireAnswer>, ImageDecisionProviderError> {
    let response: WireResponse = decode_json(payload)?;
    Ok(response.answers)
}

fn decode_json<T: DeserializeOwned>(payload: &[u8]) -> Result<T, ImageDecisionProviderError> {
    let value: serde_json::Value = serde_json::from_slice(payload)
        .map_err(|_| invalid_answer("response json is malformed"))?;
    serde_json::from_value(value).map_err(|_| invalid_answer("response answers are invalid"))
}

fn primitive_answer(
    question: &ImageDecisionQuestion,
    answer: &WireAnswer,
) -> Result<ImageDecisionPrimitiveAnswer, ImageDecisionProviderError> {
    match (question, answer) {
        (ImageDecisionQuestion::Noul { .. }, WireAnswer::Noul { noul }) => {
            Ok(ImageDecisionPrimitiveAnswer::Noul { noul: *noul })
        }
        (
            ImageDecisionQuestion::Choice { .. },
            WireAnswer::Choice {
                choice,
                confidence,
                probabilities,
            },
        ) => Ok(ImageDecisionPrimitiveAnswer::Choice {
            choice: choice.clone(),
            probabilities: probabilities.clone(),
            confidence: *confidence,
        }),
        (
            ImageDecisionQuestion::Score { criteria, .. },
            WireAnswer::Score {
                score,
                confidence,
                legend,
                probabilities,
            },
        ) => Ok(ImageDecisionPrimitiveAnswer::Score {
            score: *score,
            legend: ordered(legend, criteria.len())?,
            probabilities: ordered(probabilities, criteria.len())?,
            confidence: *confidence,
        }),
        _ => Err(invalid_answer(
            "response answer type does not match the question",
        )),
    }
}

fn ordered<T: Clone>(
    values: &BTreeMap<String, T>,
    len: usize,
) -> Result<Vec<T>, ImageDecisionProviderError> {
    if values.len() != len {
        return Err(invalid_answer("score answer does not match its criteria"));
    }
    let mut ordered_values = Vec::with_capacity(len);
    for index in 0..len {
        let Some(value) = values.get(&index.to_string()) else {
            return Err(invalid_answer("score answer does not match its criteria"));
        };
        ordered_values.push(value.clone());
    }
    Ok(ordered_values)
}

fn boxes_from_answers(
    questions: &BTreeMap<String, String>,
    answers: &BTreeMap<String, WireAnswer>,
) -> Result<BTreeMap<String, ImageDecisionAnswer>, ImageDecisionProviderError> {
    if answers.len() != questions.len() {
        return Err(invalid_answer(
            "response answers do not match the questions",
        ));
    }
    let mut boxes = BTreeMap::new();
    for name in questions.keys() {
        let Some(answer) = answers.get(name) else {
            return Err(invalid_answer(
                "response answers do not match the questions",
            ));
        };
        let WireAnswer::Box { bounds, confidence } = answer else {
            return Err(invalid_answer(
                "response answer type does not match the question",
            ));
        };
        let roi = roi_from_edges(bounds.top, bounds.bottom, bounds.left, bounds.right)?;
        let decision = ImageDecisionAnswer::new(None, *confidence, Some(roi))
            .map_err(|_| invalid_answer("response answers are invalid"))?;
        boxes.insert(name.clone(), decision);
    }
    Ok(boxes)
}

fn roi_from_edges(
    top: f32,
    bottom: f32,
    left: f32,
    right: f32,
) -> Result<ImageDecisionRoi, ImageDecisionProviderError> {
    // `ImageDecisionRoi` stores `[x_min, y_min, x_max, y_max]`. TypeSafe measures
    // `left`/`right` from the left edge and `top`/`bottom` from the top edge.
    ImageDecisionRoi::new([left, top, right, bottom])
        .map_err(|_| invalid_answer("response answers are invalid"))
}

#[derive(Serialize)]
struct SystemOneBody<'a, Q> {
    model: &'a str,
    state: ImageReference<'a>,
    assets: BTreeMap<&'a str, Asset<'a>>,
    questions: &'a Q,
}

#[derive(Serialize)]
struct ImageReference<'a> {
    image: &'a str,
}

#[derive(Serialize)]
struct Asset<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    format: &'a str,
    data_base64: &'a str,
}

#[derive(Serialize)]
struct BoxQuestion<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    instructions: &'a str,
}

#[derive(Deserialize)]
struct WireResponse {
    answers: BTreeMap<String, WireAnswer>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WireAnswer {
    Noul {
        noul: f32,
    },
    Choice {
        choice: String,
        confidence: f32,
        probabilities: BTreeMap<String, f32>,
    },
    Score {
        score: f32,
        confidence: f32,
        legend: BTreeMap<String, String>,
        probabilities: BTreeMap<String, f32>,
    },
    Box {
        #[serde(rename = "box")]
        bounds: WireEdges,
        confidence: Option<f32>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEdges {
    top: f32,
    bottom: f32,
    left: f32,
    right: f32,
}

#[cfg(test)]
mod tests {
    use super::{TypeSafeSystemOneClient, roi_from_edges, sends_danger_zone};
    use crate::images::{ImageDecisionHttpLimits, ImageTransportConfig};

    #[test]
    fn typesafe_box_edges_follow_roi_axis_order() {
        let roi = roi_from_edges(0.25, 0.75, 0.125, 0.5).unwrap();
        assert_eq!(
            roi.bounds().map(f32::to_bits),
            [0.125, 0.25, 0.5, 0.75].map(f32::to_bits)
        );
        assert!(roi_from_edges(0.8, 0.2, 0.1, 0.4).is_err());
        assert!(roi_from_edges(0.1, 0.4, 0.8, 0.2).is_err());
    }

    #[test]
    fn typesafe_danger_zone_header_follows_image_or_box() {
        assert!(sends_danger_zone(true, false));
        assert!(sends_danger_zone(false, true));
        assert!(sends_danger_zone(true, true));
        assert!(!sends_danger_zone(false, false));
    }

    #[test]
    fn typesafe_api_key_must_be_a_bearer_token() {
        let limits = ImageDecisionHttpLimits::new(
            ImageTransportConfig::new(64).unwrap(),
            1024,
            1024,
            std::time::Duration::from_secs(1),
        )
        .unwrap();
        for key in ["", " ", "secret key", "secret\nkey"] {
            let error = TypeSafeSystemOneClient::new("http://127.0.0.1/v1/systemone", key, limits)
                .unwrap_err();
            assert_eq!(error.code(), "invalid");
            assert!(!error.to_string().contains("secret"));
        }
        assert_eq!(
            TypeSafeSystemOneClient::new("ftp://example.com/v1/systemone", "secret", limits)
                .unwrap_err()
                .code(),
            "invalid"
        );
    }
}
