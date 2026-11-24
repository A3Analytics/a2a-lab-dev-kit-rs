//! Synchronous JEV-style HTTP client for one generic image decision.
//!
//! Downstream repositories construct [`ImageDecisionHttpClient`] from a published
//! git revision of `a2a-lab-dev-kit`, never a path dependency. The client posts
//! the transport-neutral request from [`super::decision`] and returns
//! [`ImageDecisionResult`]. HTTP, MCP, and A2A remain available lab transports;
//! this client does not replace them.
//!
//! An omitted ROI is left absent on the wire and is processed as the full source
//! image. Answer boxes in the response are model-input coordinates and are mapped
//! back onto the original source image. The call does not construct or append a
//! `FrameDecisionRecord`. The typed three-predicate evaluator stays on its
//! compatibility adapter.
//!
//! This module does not add settings pages, whole-video indexing or backfill,
//! `AgentCore`, Strands, an AWS SDK client, or a profile 1.3.0 bump.

use std::time::Duration;

use base64::Engine;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};

use async_trait::async_trait;

use crate::error::A2aLabError;
use crate::images::{
    Image, ImageDecisionModelAnswer, ImageDecisionModelInput, ImageDecisionPrimitiveAnswer,
    ImageDecisionProviderError, ImageDecisionQuestions, ImageDecisionRequest, ImageDecisionResult,
    ImageDecisionRoi, ImageTransportConfig, SourceImageDecisionResult,
    execute_generic_image_decision,
};

use super::decision::ImageDecisionExecutor;

/// Per-image byte cap for one JEV-style decision.
pub const IMAGE_DECISION_HTTP_MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;

/// Serialized request byte cap for one JEV-style decision.
pub const IMAGE_DECISION_HTTP_MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;

/// Response byte cap for one JEV-style decision.
pub const IMAGE_DECISION_HTTP_MAX_RESPONSE_BYTES: usize = 256 * 1024;

/// Deadline for one synchronous JEV-style decision exchange.
pub const IMAGE_DECISION_HTTP_DEADLINE: Duration = Duration::from_secs(45);

/// Bounds for one synchronous decision HTTP call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageDecisionHttpLimits {
    image: ImageTransportConfig,
    max_request_bytes: usize,
    max_response_bytes: usize,
    deadline: Duration,
}

impl ImageDecisionHttpLimits {
    /// Validates positive request, response, and deadline bounds.
    ///
    /// `image` is the existing inline-image transport limit.
    pub fn new(
        image: ImageTransportConfig,
        max_request_bytes: usize,
        max_response_bytes: usize,
        deadline: Duration,
    ) -> Result<Self, A2aLabError> {
        if max_request_bytes == 0 || max_response_bytes == 0 {
            return Err(A2aLabError::invalid("max_bytes", "must be positive"));
        }
        if deadline.is_zero() {
            return Err(A2aLabError::invalid("deadline", "must be positive"));
        }
        Ok(Self {
            image,
            max_request_bytes,
            max_response_bytes,
            deadline,
        })
    }

    /// JEV-style image, request, response, and deadline bounds.
    #[must_use]
    pub fn jev_style() -> Self {
        let image = ImageTransportConfig::new(IMAGE_DECISION_HTTP_MAX_IMAGE_BYTES)
            .expect("jev-style image limit fits");
        Self::new(
            image,
            IMAGE_DECISION_HTTP_MAX_REQUEST_BYTES,
            IMAGE_DECISION_HTTP_MAX_RESPONSE_BYTES,
            IMAGE_DECISION_HTTP_DEADLINE,
        )
        .expect("jev-style decision limits are positive")
    }

    /// Decoded image-byte limit applied before the request is sent.
    #[must_use]
    pub const fn image_transport(self) -> ImageTransportConfig {
        self.image
    }

    /// Maximum serialized JSON request body.
    #[must_use]
    pub const fn max_request_bytes(self) -> usize {
        self.max_request_bytes
    }

    /// Maximum HTTP response body retained for decoding.
    #[must_use]
    pub const fn max_response_bytes(self) -> usize {
        self.max_response_bytes
    }

    /// Deadline covering connect and the decision response.
    #[must_use]
    pub const fn deadline(self) -> Duration {
        self.deadline
    }
}

/// Posts one generic image decision to a JEV-compatible HTTP endpoint.
///
/// The endpoint must answer in the same HTTP response. The client does not poll,
/// and it does not append a typed frame-decision ledger record.
#[derive(Debug, Clone)]
pub struct ImageDecisionHttpClient {
    endpoint: Url,
    limits: ImageDecisionHttpLimits,
    preamble: Option<String>,
    http: reqwest::Client,
}

impl ImageDecisionHttpClient {
    /// Builds a client for an absolute `http` or `https` endpoint.
    pub fn new(
        endpoint: impl AsRef<str>,
        limits: ImageDecisionHttpLimits,
    ) -> Result<Self, A2aLabError> {
        let endpoint = parse_endpoint(endpoint.as_ref())?;
        let http = reqwest::Client::builder()
            .timeout(limits.deadline())
            .connect_timeout(limits.deadline())
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| A2aLabError::unavailable("decision http client could not be built"))?;
        Ok(Self {
            endpoint,
            limits,
            preamble: None,
            http,
        })
    }

    /// Stores the exact preamble. Blank text is kept and does not change questions.
    #[must_use]
    pub fn with_preamble(mut self, preamble: Option<String>) -> Self {
        self.preamble = preamble;
        self
    }

    /// Absolute decision endpoint.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }

    /// Active image, body, and deadline bounds.
    #[must_use]
    pub const fn limits(&self) -> ImageDecisionHttpLimits {
        self.limits
    }

    /// Sends one validated generic request and returns source-normalized answers.
    ///
    /// Non-success statuses, malformed JSON, unexpected content types, oversized
    /// bodies, timeouts, and schema-invalid answers become
    /// [`ImageDecisionProviderError`]. This function does not append a typed
    /// frame-decision ledger record.
    pub async fn decide(
        &self,
        request: &ImageDecisionRequest,
    ) -> Result<ImageDecisionResult, ImageDecisionProviderError> {
        let body = prepare_body(request, self.preamble.as_deref(), self.limits)?;
        let response = self
            .http
            .post(self.endpoint.clone())
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json")
            .body(body)
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        let json = content_type_is_json(response.headers());
        let payload = read_capped(response, self.limits.max_response_bytes()).await?;
        interpret(status, json, &payload, request)
    }

    /// Posts typed questions and a resolved image to the configured provider.
    ///
    /// `model` and `roi` come from backend configuration. The public request does
    /// not carry either value.
    pub async fn decide_questions(
        &self,
        model: &str,
        roi: Option<ImageDecisionRoi>,
        image: &Image,
        questions: &ImageDecisionQuestions,
    ) -> Result<SourceImageDecisionResult, ImageDecisionProviderError> {
        self.limits
            .image_transport()
            .check_payload(image.data())
            .map_err(|_| limit_exceeded("image", "image exceeds the configured maximum"))?;
        let outbound = super::preamble::outbound_questions(self.preamble.as_deref(), questions)?;
        let data = base64::engine::general_purpose::STANDARD.encode(image.data());
        let body = ProviderBody {
            model,
            state: ProviderState {
                image: ProviderImage {
                    media_type: image.descriptor().media_type(),
                    data: &data,
                },
                roi,
            },
            questions: outbound.as_ref(),
        };
        let body =
            serde_json::to_vec(&body).map_err(|_| invalid_answer("request json is invalid"))?;
        if body.len() > self.limits.max_request_bytes() {
            return Err(limit_exceeded(
                "request",
                "request exceeds the configured maximum",
            ));
        }
        let response = self
            .http
            .post(self.endpoint.clone())
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json")
            .body(body)
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        let json = content_type_is_json(response.headers());
        let payload = read_capped(response, self.limits.max_response_bytes()).await?;
        interpret_questions(status, json, &payload, questions)
    }
}

#[derive(Serialize)]
struct ProviderBody<'a> {
    model: &'a str,
    state: ProviderState<'a>,
    questions: &'a ImageDecisionQuestions,
}

#[derive(Serialize)]
struct ProviderState<'a> {
    image: ProviderImage<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    roi: Option<ImageDecisionRoi>,
}

#[derive(Serialize)]
struct ProviderImage<'a> {
    media_type: &'a str,
    data: &'a str,
}

#[derive(Deserialize)]
struct ProviderAnswers {
    answers: std::collections::BTreeMap<String, ImageDecisionPrimitiveAnswer>,
}

/// HTTP decision client that also checks the configured model name.
pub struct ModelPinnedImageDecision {
    http: ImageDecisionHttpClient,
    model: String,
    roi: Option<ImageDecisionRoi>,
}

impl ModelPinnedImageDecision {
    /// Pins `http` to one model identifier.
    #[must_use]
    pub fn new(http: ImageDecisionHttpClient, model: impl Into<String>) -> Self {
        Self::with_roi(http, model, None)
    }

    /// Pins `http` to one model and an optional configured crop.
    #[must_use]
    pub fn with_roi(
        http: ImageDecisionHttpClient,
        model: impl Into<String>,
        roi: Option<ImageDecisionRoi>,
    ) -> Self {
        Self {
            http,
            model: model.into(),
            roi,
        }
    }
}

#[async_trait]
impl ImageDecisionExecutor for ModelPinnedImageDecision {
    async fn decide(
        &self,
        request: &ImageDecisionRequest,
    ) -> Result<ImageDecisionResult, ImageDecisionProviderError> {
        if request.model_input().model() != self.model {
            return Err(invalid_answer("model does not match the configured client"));
        }
        ImageDecisionHttpClient::decide(&self.http, request).await
    }

    async fn decide_questions(
        &self,
        image: &Image,
        questions: &ImageDecisionQuestions,
    ) -> Result<SourceImageDecisionResult, ImageDecisionProviderError> {
        self.http
            .decide_questions(&self.model, self.roi, image, questions)
            .await
    }
}

#[async_trait]
impl ImageDecisionExecutor for ImageDecisionHttpClient {
    async fn decide(
        &self,
        request: &ImageDecisionRequest,
    ) -> Result<ImageDecisionResult, ImageDecisionProviderError> {
        ImageDecisionHttpClient::decide(self, request).await
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResponse {
    answers: Vec<ImageDecisionModelAnswer>,
}

fn prepare_body(
    request: &ImageDecisionRequest,
    preamble: Option<&str>,
    limits: ImageDecisionHttpLimits,
) -> Result<Vec<u8>, ImageDecisionProviderError> {
    limits
        .image_transport()
        .check_payload(request.image().data())
        .map_err(|_| limit_exceeded("image", "image exceeds the configured maximum"))?;
    let question = super::preamble::outbound_instructions(preamble, request.question())?;
    let body = match &question {
        std::borrow::Cow::Borrowed(_) => serde_json::to_vec(request),
        std::borrow::Cow::Owned(question) => serde_json::to_vec(&OutboundGeneric {
            image: request.image(),
            question,
            model_input: request.model_input(),
        }),
    }
    .map_err(|_| invalid_answer("request json is invalid"))?;
    if body.len() > limits.max_request_bytes() {
        return Err(limit_exceeded(
            "request",
            "request exceeds the configured maximum",
        ));
    }
    Ok(body)
}

#[derive(Serialize)]
struct OutboundGeneric<'a> {
    image: &'a Image,
    question: &'a str,
    model_input: &'a ImageDecisionModelInput,
}

fn interpret(
    status: StatusCode,
    json: bool,
    payload: &[u8],
    request: &ImageDecisionRequest,
) -> Result<ImageDecisionResult, ImageDecisionProviderError> {
    if !status.is_success() {
        return Err(failure_status(status, json, payload));
    }
    if !json {
        return Err(invalid_answer(
            "response content type is not application/json",
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(payload)
        .map_err(|_| invalid_answer("response json is malformed"))?;
    let response: WireResponse = serde_json::from_value(value)
        .map_err(|_| invalid_answer("response answers are invalid"))?;
    execute_generic_image_decision(request, &response.answers)
}

fn interpret_questions(
    status: StatusCode,
    json: bool,
    payload: &[u8],
    questions: &ImageDecisionQuestions,
) -> Result<SourceImageDecisionResult, ImageDecisionProviderError> {
    if !status.is_success() {
        return Err(failure_status(status, json, payload));
    }
    if !json {
        return Err(invalid_answer(
            "response content type is not application/json",
        ));
    }
    let response: ProviderAnswers = serde_json::from_slice(payload)
        .map_err(|_| invalid_answer("response answers are invalid"))?;
    SourceImageDecisionResult::from_questions(questions, response.answers)
}

fn failure_status(status: StatusCode, json: bool, payload: &[u8]) -> ImageDecisionProviderError {
    if status == StatusCode::REQUEST_TIMEOUT || status == StatusCode::GATEWAY_TIMEOUT {
        return deadline_exceeded("decision deadline elapsed");
    }
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        return limit_exceeded("request", "request exceeds the configured maximum");
    }
    if json && let Ok(error) = serde_json::from_slice::<ImageDecisionProviderError>(payload) {
        return error;
    }
    let code = status.as_u16();
    unavailable(&format!("http status {code}"))
}

pub(crate) async fn read_capped(
    mut response: reqwest::Response,
    max: usize,
) -> Result<Vec<u8>, ImageDecisionProviderError> {
    if response
        .content_length()
        .is_some_and(|length| length > u64::try_from(max).unwrap_or(u64::MAX))
    {
        return Err(limit_exceeded(
            "response",
            "response exceeds the configured maximum",
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        push_capped(&mut body, &chunk, max)?;
    }
    Ok(body)
}

fn push_capped(
    body: &mut Vec<u8>,
    chunk: &[u8],
    max: usize,
) -> Result<(), ImageDecisionProviderError> {
    let next = body.len().saturating_add(chunk.len());
    if next < body.len() || next > max {
        return Err(limit_exceeded(
            "response",
            "response exceeds the configured maximum",
        ));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

pub(crate) fn parse_endpoint(endpoint: &str) -> Result<Url, A2aLabError> {
    let url = Url::parse(endpoint.trim()).map_err(|_| endpoint_error())?;
    let http = matches!(url.scheme(), "http" | "https");
    if !http || url.host_str().is_none() {
        return Err(endpoint_error());
    }
    Ok(url)
}

fn endpoint_error() -> A2aLabError {
    A2aLabError::invalid("endpoint", "must be an absolute http(s) url")
}

pub(crate) fn content_type_is_json(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json")
        })
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn transport_error(error: reqwest::Error) -> ImageDecisionProviderError {
    if error.is_timeout() {
        deadline_exceeded("decision deadline elapsed")
    } else {
        unavailable("decision endpoint is unavailable")
    }
}

pub(crate) fn unavailable(message: &str) -> ImageDecisionProviderError {
    provider_or(ImageDecisionProviderError::unavailable(message), || {
        ImageDecisionProviderError::Unavailable {
            message: "decision endpoint is unavailable".to_owned(),
        }
    })
}

pub(crate) fn invalid_answer(message: &str) -> ImageDecisionProviderError {
    provider_or(ImageDecisionProviderError::invalid_answer(message), || {
        ImageDecisionProviderError::InvalidAnswer {
            message: "response answers are invalid".to_owned(),
        }
    })
}

pub(crate) fn limit_exceeded(limit: &str, message: &str) -> ImageDecisionProviderError {
    provider_or(
        ImageDecisionProviderError::limit_exceeded(limit, message),
        || ImageDecisionProviderError::LimitExceeded {
            limit: "response".to_owned(),
            message: "response exceeds the configured maximum".to_owned(),
        },
    )
}

pub(crate) fn deadline_exceeded(message: &str) -> ImageDecisionProviderError {
    provider_or(
        ImageDecisionProviderError::deadline_exceeded(message),
        || ImageDecisionProviderError::DeadlineExceeded {
            message: "decision deadline elapsed".to_owned(),
        },
    )
}

fn provider_or<F>(
    result: Result<ImageDecisionProviderError, A2aLabError>,
    fallback: F,
) -> ImageDecisionProviderError
where
    F: FnOnce() -> ImageDecisionProviderError,
{
    result.unwrap_or_else(|_| fallback())
}

#[cfg(test)]
mod tests {
    use super::{ImageDecisionProviderError, push_capped};

    #[test]
    fn push_capped_rejects_the_chunk_that_crosses_the_maximum() {
        let mut body = Vec::new();
        push_capped(&mut body, &[1, 2, 3, 4], 6).unwrap();
        let error = push_capped(&mut body, &[5, 6, 7], 6).unwrap_err();
        assert_eq!(error.code(), "limit_exceeded");
        assert!(matches!(
            error,
            ImageDecisionProviderError::LimitExceeded { ref limit, .. } if limit == "response"
        ));
        assert_eq!(body, vec![1, 2, 3, 4]);
    }
}
