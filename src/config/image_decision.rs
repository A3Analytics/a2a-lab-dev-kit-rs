//! Optional `image_decision` lab-file client for one synchronous image decision.
//!
//! Downstream repositories consume this configuration from a published git revision
//! of `a2a-lab-dev-kit`, never a path dependency. HTTP, MCP, and A2A remain the
//! integration surfaces. An `agentcore` provider names a runtime for the lab agent
//! to invoke. This module does not add settings pages, whole-video indexing or
//! backfill, a Strands runtime, an AWS SDK client, or a profile 1.3.0 bump.
//!
//! Resolving the client does not register or replace the typed three-predicate
//! evaluator. Generic completion does not append a typed frame-decision record.

use std::sync::Arc;
use std::time::Duration;

use serde::de::Error as _;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::error::A2aLabError;
use crate::images::{
    ImageDecisionExecutor, ImageDecisionHttpClient, ImageDecisionHttpLimits,
    ImageDecisionModelAnswer, ImageDecisionModelInput, ImageDecisionProviderError,
    ImageDecisionRequest, ImageDecisionResult, ImageDecisionRoi, ImageTransportConfig,
    ModelPinnedImageDecision, execute_generic_image_decision,
};
use crate::{IMAGE_DECISION_DOWNSTREAM_DEPENDENCY, JEV_MAX_IMAGE_BYTES, JEV_MAX_REQUEST_BYTES};

use super::duration::ConfigDuration;
use super::object_at;

/// How downstream repositories consume this client configuration.
pub const IMAGE_DECISION_CLIENT_DOWNSTREAM_DEPENDENCY: &str = IMAGE_DECISION_DOWNSTREAM_DEPENDENCY;

/// Default synchronous client timeout when `timeout` is omitted.
pub const DEFAULT_IMAGE_DECISION_TIMEOUT_SECS: u64 = 30;

/// Largest accepted synchronous client timeout.
pub const MAX_IMAGE_DECISION_TIMEOUT_SECS: u64 = 120;

/// Largest accepted generic decision response, matching the typed JEV response cap.
pub const MAX_IMAGE_DECISION_RESPONSE_BYTES: u64 = 256 * 1024;

const FIELD_PROVIDER: &str = "image_decision.provider";
const FIELD_ENDPOINT: &str = "image_decision.endpoint";
const FIELD_MODEL: &str = "image_decision.model";
const FIELD_RUNTIME_ARN: &str = "image_decision.runtime_arn";
const FIELD_QUALIFIER: &str = "image_decision.qualifier";
const FIELD_REGION: &str = "image_decision.region";
const FIELD_TIMEOUT: &str = "image_decision.timeout";
const FIELD_IMAGE_BYTES: &str = "image_decision.max_image_bytes";
const FIELD_REQUEST_BYTES: &str = "image_decision.max_request_bytes";
const FIELD_RESPONSE_BYTES: &str = "image_decision.max_response_bytes";
const FIELD_CREDENTIALS: &str = "image_decision.credentials";
const FIELD_CREDENTIALS_ENV: &str = "image_decision.credentials.env";
const FIELD_ROI: &str = "image_decision.roi";

/// Which backend completes one configured image decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageDecisionProvider {
    /// HTTPS decision endpoint.
    Http,
    /// Amazon Bedrock `AgentCore` Runtime.
    AgentCore,
}

/// Runtime address for an `AgentCore` image-decision provider.
///
/// The model identity is pinned by the runtime deployment, not this lab file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCoreImageDecisionSettings {
    runtime_arn: String,
    qualifier: String,
    region: String,
}

impl AgentCoreImageDecisionSettings {
    /// Agent runtime `ARN` passed to `InvokeAgentRuntime`.
    #[must_use]
    pub fn runtime_arn(&self) -> &str {
        &self.runtime_arn
    }

    /// Runtime endpoint qualifier, such as `prod`.
    #[must_use]
    pub fn qualifier(&self) -> &str {
        &self.qualifier
    }

    /// AWS region that hosts the runtime.
    #[must_use]
    pub fn region(&self) -> &str {
        &self.region
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ImageDecisionBackend {
    Http {
        endpoint: String,
        model: String,
        credentials_env: Option<String>,
    },
    AgentCore(AgentCoreImageDecisionSettings),
}

/// Validated synchronous image-decision client settings from the lab file.
///
/// An omitted `provider` selects HTTP. An omitted `roi` selects the full original image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageDecisionClientConfig {
    backend: ImageDecisionBackend,
    timeout_secs: u64,
    max_image_bytes: u64,
    max_request_bytes: u64,
    max_response_bytes: u64,
    roi_bits: Option<[u32; 4]>,
}

impl ImageDecisionClientConfig {
    /// Parses one `image_decision` object.
    pub fn parse_value(value: &Value) -> Result<Self, A2aLabError> {
        let map = object_at("image_decision", value)?;
        reject_insecure_keys(map)?;
        reject_unknown(map)?;
        let provider = parse_provider(map.get("provider"))?;
        let backend = match provider {
            ImageDecisionProvider::Http => parse_http_backend(map)?,
            ImageDecisionProvider::AgentCore => {
                ImageDecisionBackend::AgentCore(parse_agentcore(map)?)
            }
        };
        let timeout_secs = match map.get("timeout") {
            None => DEFAULT_IMAGE_DECISION_TIMEOUT_SECS,
            Some(value) => parse_timeout(value)?,
        };
        let max_image_bytes = match map.get("max_image_bytes") {
            None => u64::try_from(JEV_MAX_IMAGE_BYTES).unwrap_or(u64::MAX),
            Some(value) => parse_bounded_u64(value, FIELD_IMAGE_BYTES, jev_image_cap())?,
        };
        let max_request_bytes = match map.get("max_request_bytes") {
            None => u64::try_from(JEV_MAX_REQUEST_BYTES).unwrap_or(u64::MAX),
            Some(value) => parse_bounded_u64(value, FIELD_REQUEST_BYTES, jev_request_cap())?,
        };
        if max_request_bytes < max_image_bytes {
            return Err(A2aLabError::invalid(
                FIELD_REQUEST_BYTES,
                "must be greater than or equal to max_image_bytes",
            ));
        }
        let max_response_bytes = match map.get("max_response_bytes") {
            None => MAX_IMAGE_DECISION_RESPONSE_BYTES,
            Some(value) => parse_bounded_u64(
                value,
                FIELD_RESPONSE_BYTES,
                MAX_IMAGE_DECISION_RESPONSE_BYTES,
            )?,
        };
        let roi_bits = match map.get("roi") {
            None => None,
            Some(value) => Some(parse_roi(value)?.bounds().map(f32::to_bits)),
        };
        Ok(Self {
            backend,
            timeout_secs,
            max_image_bytes,
            max_request_bytes,
            max_response_bytes,
            roi_bits,
        })
    }

    /// Selected decision backend.
    #[must_use]
    pub const fn provider(&self) -> ImageDecisionProvider {
        match self.backend {
            ImageDecisionBackend::Http { .. } => ImageDecisionProvider::Http,
            ImageDecisionBackend::AgentCore(_) => ImageDecisionProvider::AgentCore,
        }
    }

    /// HTTPS decision endpoint when this client uses HTTP.
    #[must_use]
    pub fn endpoint(&self) -> Option<&str> {
        match &self.backend {
            ImageDecisionBackend::Http { endpoint, .. } => Some(endpoint),
            ImageDecisionBackend::AgentCore(_) => None,
        }
    }

    /// Pinned HTTP decision model when this client uses HTTP.
    #[must_use]
    pub fn model(&self) -> Option<&str> {
        match &self.backend {
            ImageDecisionBackend::Http { model, .. } => Some(model),
            ImageDecisionBackend::AgentCore(_) => None,
        }
    }

    /// `AgentCore` runtime settings when this client uses `AgentCore`.
    #[must_use]
    pub const fn agentcore(&self) -> Option<&AgentCoreImageDecisionSettings> {
        match &self.backend {
            ImageDecisionBackend::AgentCore(settings) => Some(settings),
            ImageDecisionBackend::Http { .. } => None,
        }
    }

    /// Synchronous client timeout in whole seconds.
    #[must_use]
    pub const fn timeout_secs(&self) -> u64 {
        self.timeout_secs
    }

    /// Synchronous client timeout.
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }

    /// Maximum source-image bytes accepted by the client.
    #[must_use]
    pub const fn max_image_bytes(&self) -> u64 {
        self.max_image_bytes
    }

    /// Maximum combined image and question bytes accepted by the client.
    #[must_use]
    pub const fn max_request_bytes(&self) -> u64 {
        self.max_request_bytes
    }

    /// Maximum serialized generic result bytes accepted by the client.
    #[must_use]
    pub const fn max_response_bytes(&self) -> u64 {
        self.max_response_bytes
    }

    /// Environment variable that holds the client credential, when configured.
    #[must_use]
    pub fn credentials_env(&self) -> Option<&str> {
        match &self.backend {
            ImageDecisionBackend::Http {
                credentials_env, ..
            } => credentials_env.as_deref(),
            ImageDecisionBackend::AgentCore(_) => None,
        }
    }

    /// Configured crop. `None` means the lab file omitted `roi`.
    #[must_use]
    pub fn roi(&self) -> Option<ImageDecisionRoi> {
        self.roi_bits
            .map(|bits| ImageDecisionRoi::new(bits.map(f32::from_bits)).expect("validated roi"))
    }

    /// Whether the lab file omitted `roi`.
    #[must_use]
    pub const fn omits_roi(&self) -> bool {
        self.roi_bits.is_none()
    }

    /// Crop applied when the client builds model input. Omission selects the full image.
    #[must_use]
    pub fn effective_roi(&self) -> ImageDecisionRoi {
        self.roi().unwrap_or_else(ImageDecisionRoi::full_image)
    }

    /// Model input represented by this client. Omitted `roi` stays omitted.
    pub fn model_input(&self) -> Result<ImageDecisionModelInput, A2aLabError> {
        let Some(model) = self.model() else {
            return Err(A2aLabError::invalid(
                FIELD_PROVIDER,
                "agentcore pins the model in the runtime deployment",
            ));
        };
        ImageDecisionModelInput::new(model, self.roi())
    }
}

impl Serialize for ImageDecisionClientConfig {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut length = match &self.backend {
            ImageDecisionBackend::Http { .. } => 2,
            ImageDecisionBackend::AgentCore(_) => 4,
        };
        if self.timeout_secs != DEFAULT_IMAGE_DECISION_TIMEOUT_SECS {
            length += 1;
        }
        if self.max_image_bytes != jev_image_cap() {
            length += 1;
        }
        if self.max_request_bytes != jev_request_cap() {
            length += 1;
        }
        if self.max_response_bytes != MAX_IMAGE_DECISION_RESPONSE_BYTES {
            length += 1;
        }
        if self.credentials_env().is_some() {
            length += 1;
        }
        if self.roi_bits.is_some() {
            length += 1;
        }
        let mut map = serializer.serialize_map(Some(length))?;
        match &self.backend {
            ImageDecisionBackend::Http {
                endpoint,
                model,
                credentials_env,
            } => {
                map.serialize_entry("endpoint", endpoint)?;
                map.serialize_entry("model", model)?;
                if let Some(env_var) = credentials_env {
                    map.serialize_entry("credentials", &CredentialsEnv { env: env_var })?;
                }
            }
            ImageDecisionBackend::AgentCore(settings) => {
                map.serialize_entry("provider", "agentcore")?;
                map.serialize_entry("runtime_arn", settings.runtime_arn())?;
                map.serialize_entry("qualifier", settings.qualifier())?;
                map.serialize_entry("region", settings.region())?;
            }
        }
        if self.timeout_secs != DEFAULT_IMAGE_DECISION_TIMEOUT_SECS {
            map.serialize_entry("timeout", &format!("{}s", self.timeout_secs))?;
        }
        if self.max_image_bytes != jev_image_cap() {
            map.serialize_entry("max_image_bytes", &self.max_image_bytes)?;
        }
        if self.max_request_bytes != jev_request_cap() {
            map.serialize_entry("max_request_bytes", &self.max_request_bytes)?;
        }
        if self.max_response_bytes != MAX_IMAGE_DECISION_RESPONSE_BYTES {
            map.serialize_entry("max_response_bytes", &self.max_response_bytes)?;
        }
        if let Some(roi) = self.roi() {
            map.serialize_entry("roi", &roi)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ImageDecisionClientConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::parse_value(&value).map_err(D::Error::custom)
    }
}

#[derive(Serialize)]
struct CredentialsEnv<'a> {
    env: &'a str,
}

/// Client resolved from lab-file settings.
///
/// Completion uses the generic image-decision contract. It does not register the
/// typed three-predicate evaluator and does not append a typed ledger record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedImageDecisionClient {
    config: ImageDecisionClientConfig,
}

impl ResolvedImageDecisionClient {
    /// Resolves one already validated lab-file client.
    #[must_use]
    pub fn from_config(config: ImageDecisionClientConfig) -> Self {
        Self { config }
    }

    /// Lab-file settings used to resolve this client.
    #[must_use]
    pub const fn config(&self) -> &ImageDecisionClientConfig {
        &self.config
    }

    /// Configured crop, or the full original image when `roi` was omitted.
    #[must_use]
    pub fn effective_roi(&self) -> ImageDecisionRoi {
        self.config.effective_roi()
    }

    /// Whether configuration omitted the ROI.
    #[must_use]
    pub const fn omits_roi(&self) -> bool {
        self.config.omits_roi()
    }

    /// Model input for this client. Omitted configuration ROI stays omitted.
    pub fn model_input(&self) -> Result<ImageDecisionModelInput, A2aLabError> {
        self.config.model_input()
    }

    /// HTTP executor pinned to this client's model and bounds.
    pub fn executor(&self) -> Result<Arc<dyn ImageDecisionExecutor>, A2aLabError> {
        let (Some(endpoint), Some(model)) = (self.config.endpoint(), self.config.model()) else {
            return Err(A2aLabError::unavailable(
                "agentcore image decisions are executed by the lab agent",
            ));
        };
        let limits = ImageDecisionHttpLimits::new(
            ImageTransportConfig::new(self.config.max_image_bytes())?,
            usize::try_from(self.config.max_request_bytes()).unwrap_or(usize::MAX),
            usize::try_from(self.config.max_response_bytes()).unwrap_or(usize::MAX),
            self.config.timeout(),
        )?;
        let http = ImageDecisionHttpClient::new(endpoint, limits)?;
        Ok(Arc::new(ModelPinnedImageDecision::with_roi(
            http,
            model,
            self.config.roi(),
        )))
    }

    /// Completes one generic decision and maps boxes onto the original image.
    ///
    /// The returned value is an [`ImageDecisionResult`]. This method does not
    /// construct or append a typed frame-decision record.
    pub fn complete(
        &self,
        request: &ImageDecisionRequest,
        model_answers: &[ImageDecisionModelAnswer],
    ) -> Result<ImageDecisionResult, ImageDecisionProviderError> {
        let Some(model) = self.config.model() else {
            return Err(bounded_invalid_answer(
                "agentcore does not complete generic box decisions",
            ));
        };
        if request.model_input().model() != model {
            return Err(bounded_invalid_answer(
                "model does not match the configured client",
            ));
        }
        let image_bytes = u64::try_from(request.image().data().len()).unwrap_or(u64::MAX);
        if image_bytes > self.config.max_image_bytes() {
            return Err(bounded_limit(
                "max_image_bytes",
                format!("image exceeds {} bytes", self.config.max_image_bytes()),
            ));
        }
        let question_bytes = u64::try_from(request.question().len()).unwrap_or(u64::MAX);
        if image_bytes.saturating_add(question_bytes) > self.config.max_request_bytes() {
            return Err(bounded_limit(
                "max_request_bytes",
                format!("request exceeds {} bytes", self.config.max_request_bytes()),
            ));
        }
        let result = execute_generic_image_decision(request, model_answers)?;
        let encoded = serde_json::to_vec(&result)
            .map_err(|_| bounded_invalid_answer("result could not be serialized"))?;
        if u64::try_from(encoded.len()).unwrap_or(u64::MAX) > self.config.max_response_bytes() {
            return Err(bounded_limit(
                "max_response_bytes",
                format!(
                    "response exceeds {} bytes",
                    self.config.max_response_bytes()
                ),
            ));
        }
        Ok(result)
    }
}

fn jev_image_cap() -> u64 {
    u64::try_from(JEV_MAX_IMAGE_BYTES).unwrap_or(u64::MAX)
}

fn jev_request_cap() -> u64 {
    u64::try_from(JEV_MAX_REQUEST_BYTES).unwrap_or(u64::MAX)
}

fn bounded_limit(limit: &str, message: String) -> ImageDecisionProviderError {
    ImageDecisionProviderError::limit_exceeded(limit, message)
        .expect("limit text is within the provider-error bounds")
}

fn bounded_invalid_answer(message: &str) -> ImageDecisionProviderError {
    ImageDecisionProviderError::invalid_answer(message)
        .expect("answer text is within the provider-error bounds")
}

fn reject_insecure_keys(map: &Map<String, Value>) -> Result<(), A2aLabError> {
    const INSECURE: &[&str] = &["api_key", "token", "secret", "password", "authorization"];
    for key in map.keys() {
        if INSECURE.contains(&key.as_str()) {
            return Err(A2aLabError::invalid(
                FIELD_CREDENTIALS,
                "inline secrets are not accepted",
            ));
        }
    }
    Ok(())
}

fn reject_unknown(map: &Map<String, Value>) -> Result<(), A2aLabError> {
    const ALLOWED: &[&str] = &[
        "provider",
        "endpoint",
        "model",
        "runtime_arn",
        "qualifier",
        "region",
        "timeout",
        "max_image_bytes",
        "max_request_bytes",
        "max_response_bytes",
        "credentials",
        "roi",
    ];
    for key in map.keys() {
        if !ALLOWED.contains(&key.as_str()) {
            return Err(A2aLabError::invalid(unknown_field(key), "unknown field"));
        }
    }
    Ok(())
}

fn unknown_field(key: &str) -> &'static str {
    match key {
        "fallback" => "image_decision.fallback",
        "timeout_ms" => "image_decision.timeout_ms",
        _ => "image_decision",
    }
}

fn required_string(value: Option<&Value>, field: &'static str) -> Result<String, A2aLabError> {
    let Some(value) = value else {
        return Err(A2aLabError::invalid(field, "required field is missing"));
    };
    let Value::String(text) = value else {
        return Err(A2aLabError::invalid(field, "must be a string"));
    };
    if text.chars().all(char::is_whitespace) {
        return Err(A2aLabError::invalid(
            field,
            "must contain non-whitespace text",
        ));
    }
    Ok(text.clone())
}

fn parse_provider(value: Option<&Value>) -> Result<ImageDecisionProvider, A2aLabError> {
    let Some(value) = value else {
        return Ok(ImageDecisionProvider::Http);
    };
    let Value::String(text) = value else {
        return Err(A2aLabError::invalid(FIELD_PROVIDER, "must be a string"));
    };
    match text.as_str() {
        "http" => Ok(ImageDecisionProvider::Http),
        "agentcore" => Ok(ImageDecisionProvider::AgentCore),
        _ => Err(A2aLabError::invalid(
            FIELD_PROVIDER,
            "must be http or agentcore",
        )),
    }
}

fn parse_http_backend(map: &Map<String, Value>) -> Result<ImageDecisionBackend, A2aLabError> {
    reject_agentcore_fields(map)?;
    let endpoint = required_string(map.get("endpoint"), FIELD_ENDPOINT)?;
    let endpoint = parse_endpoint(&endpoint)?;
    let model = required_string(map.get("model"), FIELD_MODEL)?;
    let model_input = ImageDecisionModelInput::new(&model, None).map_err(|_| {
        A2aLabError::invalid(
            FIELD_MODEL,
            "must be a single token within the model length limit",
        )
    })?;
    Ok(ImageDecisionBackend::Http {
        endpoint,
        model: model_input.model().to_owned(),
        credentials_env: parse_credentials(map.get("credentials"))?,
    })
}

fn parse_agentcore(
    map: &Map<String, Value>,
) -> Result<AgentCoreImageDecisionSettings, A2aLabError> {
    reject_http_fields(map)?;
    Ok(AgentCoreImageDecisionSettings {
        runtime_arn: parse_runtime_arn(&required_string(
            map.get("runtime_arn"),
            FIELD_RUNTIME_ARN,
        )?)?,
        qualifier: parse_qualifier(&required_string(map.get("qualifier"), FIELD_QUALIFIER)?)?,
        region: parse_region(&required_string(map.get("region"), FIELD_REGION)?)?,
    })
}

fn reject_agentcore_fields(map: &Map<String, Value>) -> Result<(), A2aLabError> {
    for (key, field) in [
        ("runtime_arn", FIELD_RUNTIME_ARN),
        ("qualifier", FIELD_QUALIFIER),
        ("region", FIELD_REGION),
    ] {
        if map.contains_key(key) {
            return Err(A2aLabError::invalid(
                field,
                "is only valid for provider agentcore",
            ));
        }
    }
    Ok(())
}

fn reject_http_fields(map: &Map<String, Value>) -> Result<(), A2aLabError> {
    for (key, field) in [
        ("endpoint", FIELD_ENDPOINT),
        ("model", FIELD_MODEL),
        ("credentials", FIELD_CREDENTIALS),
    ] {
        if map.contains_key(key) {
            return Err(A2aLabError::invalid(
                field,
                "is only valid for provider http",
            ));
        }
    }
    Ok(())
}

fn parse_runtime_arn(text: &str) -> Result<String, A2aLabError> {
    if text.len() > 2048
        || text.chars().any(char::is_whitespace)
        || !text.starts_with("arn:")
        || !text.contains(":bedrock-agentcore:")
        || !text.contains(":runtime/")
    {
        return Err(A2aLabError::invalid(
            FIELD_RUNTIME_ARN,
            "must be an AgentCore runtime ARN",
        ));
    }
    Ok(text.to_owned())
}

fn parse_qualifier(text: &str) -> Result<String, A2aLabError> {
    if (1..=256).contains(&text.len())
        && text.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        Ok(text.to_owned())
    } else {
        Err(A2aLabError::invalid(
            FIELD_QUALIFIER,
            "must be a runtime endpoint name",
        ))
    }
}

fn parse_region(text: &str) -> Result<String, A2aLabError> {
    let mut parts = text.split('-');
    let Some(first) = parts.next() else {
        return Err(invalid_region());
    };
    if first.len() < 2
        || !first
            .chars()
            .all(|character| character.is_ascii_lowercase())
    {
        return Err(invalid_region());
    }
    let mut saw_name = false;
    let mut saw_number = false;
    for part in parts {
        if part.is_empty() {
            return Err(invalid_region());
        }
        if part.chars().all(|character| character.is_ascii_digit()) {
            saw_number = true;
        } else if part.chars().all(|character| character.is_ascii_lowercase()) {
            saw_name = true;
        } else {
            return Err(invalid_region());
        }
    }
    if saw_name && saw_number {
        Ok(text.to_owned())
    } else {
        Err(invalid_region())
    }
}

fn invalid_region() -> A2aLabError {
    A2aLabError::invalid(FIELD_REGION, "must be an AWS region")
}

fn parse_endpoint(text: &str) -> Result<String, A2aLabError> {
    if text.chars().any(char::is_whitespace) {
        return Err(malformed_endpoint());
    }
    if text.starts_with("http://") || text.contains('@') || embeds_credential_query(text) {
        return Err(A2aLabError::invalid(
            FIELD_ENDPOINT,
            "endpoint must use https and must not embed credentials",
        ));
    }
    let Some(rest) = text.strip_prefix("https://") else {
        return Err(malformed_endpoint());
    };
    let host_port = rest.split_once('/').map_or(rest, |(host, _)| host);
    if host_port.is_empty() {
        return Err(malformed_endpoint());
    }
    validate_host_port(host_port)?;
    Ok(text.to_owned())
}

fn embeds_credential_query(text: &str) -> bool {
    let Some((_, query)) = text.split_once('?') else {
        return false;
    };
    let query = query.to_ascii_lowercase();
    [
        "api_key=",
        "token=",
        "secret=",
        "password=",
        "authorization=",
    ]
    .iter()
    .any(|needle| query.contains(needle))
}

fn malformed_endpoint() -> A2aLabError {
    A2aLabError::invalid(FIELD_ENDPOINT, "must be an https URL")
}

fn validate_host_port(host_port: &str) -> Result<(), A2aLabError> {
    if let Some(stripped) = host_port.strip_prefix('[') {
        let Some((host, rest)) = stripped.split_once(']') else {
            return Err(malformed_endpoint());
        };
        if host.is_empty()
            || !host
                .chars()
                .all(|character| character.is_ascii_hexdigit() || character == ':')
        {
            return Err(malformed_endpoint());
        }
        if rest.is_empty() {
            return Ok(());
        }
        let Some(port) = rest.strip_prefix(':') else {
            return Err(malformed_endpoint());
        };
        return validate_port(port);
    }
    let (host, port) = match host_port.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (host_port, None),
    };
    if host.is_empty()
        || host.starts_with('.')
        || host.ends_with('.')
        || host.contains("..")
        || !host.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '.' || character == '-'
        })
    {
        return Err(malformed_endpoint());
    }
    if let Some(port) = port {
        validate_port(port)?;
    }
    Ok(())
}

fn validate_port(port: &str) -> Result<(), A2aLabError> {
    let parsed: u16 = port.parse().map_err(|_| {
        if port.chars().all(|character| character.is_ascii_digit()) {
            A2aLabError::invalid(FIELD_ENDPOINT, "port must be 1..=65535")
        } else {
            malformed_endpoint()
        }
    })?;
    if parsed == 0 {
        return Err(A2aLabError::invalid(
            FIELD_ENDPOINT,
            "port must be 1..=65535",
        ));
    }
    Ok(())
}

fn parse_timeout(value: &Value) -> Result<u64, A2aLabError> {
    let Value::String(text) = value else {
        return Err(A2aLabError::invalid(
            FIELD_TIMEOUT,
            "must be a positive whole-second duration",
        ));
    };
    let duration = ConfigDuration::parse(FIELD_TIMEOUT, text)?;
    if duration.as_secs() > MAX_IMAGE_DECISION_TIMEOUT_SECS {
        return Err(A2aLabError::invalid(
            FIELD_TIMEOUT,
            format!("must be 1..={MAX_IMAGE_DECISION_TIMEOUT_SECS} seconds"),
        ));
    }
    Ok(duration.as_secs())
}

fn parse_bounded_u64(value: &Value, field: &'static str, max: u64) -> Result<u64, A2aLabError> {
    let Value::Number(number) = value else {
        return Err(A2aLabError::invalid(field, format!("must be 1..={max}")));
    };
    let Some(parsed) = number.as_u64() else {
        return Err(A2aLabError::invalid(field, format!("must be 1..={max}")));
    };
    if parsed == 0 || parsed > max {
        return Err(A2aLabError::invalid(field, format!("must be 1..={max}")));
    }
    Ok(parsed)
}

fn parse_credentials(value: Option<&Value>) -> Result<Option<String>, A2aLabError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let map = object_at(FIELD_CREDENTIALS, value)?;
    for key in map.keys() {
        if key != "env" {
            return Err(A2aLabError::invalid(
                FIELD_CREDENTIALS,
                "credentials must name an environment variable",
            ));
        }
    }
    let Some(env_value) = map.get("env") else {
        return Err(A2aLabError::invalid(
            FIELD_CREDENTIALS_ENV,
            "required field is missing",
        ));
    };
    let Value::String(name) = env_value else {
        return Err(A2aLabError::invalid(
            FIELD_CREDENTIALS_ENV,
            "must be a string",
        ));
    };
    validate_env_reference(name)?;
    Ok(Some(name.clone()))
}

fn validate_env_reference(name: &str) -> Result<(), A2aLabError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(invalid_env());
    };
    if !first.is_ascii_uppercase() {
        return Err(invalid_env());
    }
    if chars.all(|character| {
        character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
    }) {
        Ok(())
    } else {
        Err(invalid_env())
    }
}

fn invalid_env() -> A2aLabError {
    A2aLabError::invalid(
        FIELD_CREDENTIALS_ENV,
        "must be an uppercase environment variable name",
    )
}

fn parse_roi(value: &Value) -> Result<ImageDecisionRoi, A2aLabError> {
    let map = object_at(FIELD_ROI, value)?;
    for key in map.keys() {
        if key != "bounds" {
            return Err(A2aLabError::invalid(FIELD_ROI, "unknown field"));
        }
    }
    let Some(bounds) = map.get("bounds").and_then(Value::as_array) else {
        return Err(A2aLabError::invalid(
            FIELD_ROI,
            "bounds must be four normalized numbers",
        ));
    };
    if bounds.len() != 4 {
        return Err(A2aLabError::invalid(
            FIELD_ROI,
            "bounds must be four normalized numbers",
        ));
    }
    let mut parsed = [0.0_f32; 4];
    for (index, item) in bounds.iter().enumerate() {
        let Some(number) = item.as_f64() else {
            return Err(A2aLabError::invalid(
                FIELD_ROI,
                "bounds must be four normalized numbers",
            ));
        };
        if !number.is_finite() {
            return Err(A2aLabError::invalid(
                FIELD_ROI,
                "bounds must be normalized with min strictly before max",
            ));
        }
        #[allow(clippy::cast_possible_truncation)]
        {
            parsed[index] = number as f32;
        }
    }
    ImageDecisionRoi::new(parsed).map_err(|_| {
        A2aLabError::invalid(
            FIELD_ROI,
            "bounds must be normalized with min strictly before max",
        )
    })
}
