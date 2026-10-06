//! Caller identity for security requirements, and task visibility for that caller.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};

use a2a_server::{InMemoryTaskStore, RequestAuthorizer, ServiceParams, TaskStore};
use a2a_types::{
    A2AError, ApiKeySecurityScheme, ListTasksRequest, ListTasksResponse, PROTOCOL_DOMAIN,
    SecurityRequirement, SecurityScheme, Task, TypedDetail,
};
use async_trait::async_trait;
use axum::http::header::{CONTENT_TYPE, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use sha2::{Digest, Sha256};

use super::auth::{AuthError, Authenticator, Principal};

const ANONYMOUS: &str = "anonymous";

tokio::task_local! {
    static CALLER: String;
}

/// Declared security requirements and the caller they identify.
#[derive(Clone)]
pub(crate) struct AccessGate {
    inner: Arc<AccessInner>,
}

struct AccessInner {
    enforced: bool,
    schemes: HashMap<String, SecurityScheme>,
    requirements: Vec<SecurityRequirement>,
    authenticator: Option<std::sync::Arc<dyn Authenticator>>,
    challenge: String,
}

#[derive(Debug)]
pub(crate) enum AccessError {
    Unauthenticated { message: String, challenge: String },
    Forbidden { message: String },
    Unavailable { message: String },
}

impl AccessGate {
    pub(crate) fn new(
        schemes: Option<HashMap<String, SecurityScheme>>,
        requirements: Option<Vec<SecurityRequirement>>,
        authenticator: Option<std::sync::Arc<dyn Authenticator>>,
    ) -> Self {
        let requirements = requirements.unwrap_or_default();
        let schemes = schemes.unwrap_or_default();
        let challenge = challenge_header(&schemes);
        Self {
            inner: Arc::new(AccessInner {
                enforced: !requirements.is_empty(),
                schemes,
                requirements,
                authenticator,
                challenge,
            }),
        }
    }

    /// Principal for this request, when security requirements are active.
    ///
    /// OAuth2 and OpenID Connect bearer tokens are validated. Other declared
    /// credentials identify a caller only after the secret is reduced to a
    /// fingerprint. Mutual TLS is not accepted here, so a card that requires
    /// only mutual TLS fails closed.
    pub(crate) async fn authenticate(
        &self,
        headers: &HeaderMap,
        query: Option<&str>,
    ) -> Result<Option<String>, AccessError> {
        if !self.inner.enforced {
            return Ok(None);
        }
        let mut anonymous = false;
        let mut forbidden = None;
        let mut unauthenticated = None;
        for requirement in &self.inner.requirements {
            if requirement.is_empty() {
                anonymous = true;
                continue;
            }
            match requirement_principal(requirement, &self.inner, headers, query).await {
                Ok(caller) => return Ok(Some(caller)),
                Err(RequirementError::Unavailable(message)) => {
                    return Err(AccessError::Unavailable { message });
                }
                Err(RequirementError::Forbidden(message)) => forbidden = Some(message),
                Err(RequirementError::Unauthenticated(message)) => {
                    unauthenticated = Some(message);
                }
                Err(RequirementError::Missing) => {}
            }
        }
        if let Some(message) = forbidden {
            return Err(AccessError::Forbidden { message });
        }
        if let Some(message) = unauthenticated {
            return Err(self.unauthenticated(message));
        }
        if anonymous {
            return Ok(Some(ANONYMOUS.to_owned()));
        }
        Err(self.unauthenticated("authentication required"))
    }

    fn unauthenticated(&self, message: impl Into<String>) -> AccessError {
        AccessError::Unauthenticated {
            message: message.into(),
            challenge: self.inner.challenge.clone(),
        }
    }

    pub(crate) fn enforced(&self) -> bool {
        self.inner.enforced
    }

    fn check_version(params: &ServiceParams) -> Result<(), A2AError> {
        let requested = params
            .get("a2a-version")
            .and_then(|values| values.first())
            .map(String::as_str);
        match requested {
            None => Ok(()),
            Some(version)
                if version
                    .trim()
                    .split('.')
                    .next()
                    .and_then(|part| part.parse::<u32>().ok())
                    == Some(1) =>
            {
                Ok(())
            }
            Some(version) => Err(A2AError::version_not_supported(version)),
        }
    }
}

impl RequestAuthorizer for AccessGate {
    fn authorize(&self, params: &ServiceParams, _task_id: Option<&str>) -> Result<(), A2AError> {
        Self::check_version(params)?;
        if self.inner.enforced && CALLER.try_with(|_| ()).is_err() {
            return Err(A2AError::invalid_request("authentication required"));
        }
        Ok(())
    }
}

pub(crate) async fn with_caller<F: Future>(caller: Option<String>, future: F) -> F::Output {
    if let Some(caller) = caller {
        CALLER.scope(caller, future).await
    } else {
        future.await
    }
}

fn current_caller() -> Option<String> {
    CALLER.try_with(Clone::clone).ok()
}

enum RequirementError {
    Missing,
    Unauthenticated(String),
    Forbidden(String),
    Unavailable(String),
}

enum SchemeOutcome {
    Missing,
    Ready(String),
    Unauthenticated(String),
    Forbidden(String),
    Unavailable(String),
}

pub(crate) fn security_configuration_error(
    schemes: &HashMap<String, SecurityScheme>,
    requirements: &[SecurityRequirement],
    has_authenticator: bool,
) -> Result<(), String> {
    for requirement in requirements {
        for (name, scopes) in requirement {
            let Some(scheme) = schemes.get(name) else {
                return Err(format!("unknown security scheme {name}"));
            };
            if matches!(
                scheme,
                SecurityScheme::OAuth2(_) | SecurityScheme::OpenIdConnect(_)
            ) && !has_authenticator
            {
                return Err(
                    "openid connect and oauth2 requirements need an authenticator".to_owned(),
                );
            }
            if !scopes.is_empty() && !has_authenticator {
                return Err("scope requirements need an authenticator".to_owned());
            }
            if !scopes.is_empty() && !bearer_scheme(scheme) {
                return Err(format!("scheme {name} cannot enforce scopes"));
            }
        }
    }
    Ok(())
}

async fn requirement_principal(
    requirement: &SecurityRequirement,
    access: &AccessInner,
    headers: &HeaderMap,
    query: Option<&str>,
) -> Result<String, RequirementError> {
    let mut names: Vec<_> = requirement.keys().cloned().collect();
    names.sort();
    let mut parts = Vec::with_capacity(names.len());
    let mut failure: Option<RequirementError> = None;
    for name in names {
        let Some(scheme) = access.schemes.get(&name) else {
            remember(&mut failure, RequirementError::Missing);
            continue;
        };
        let required = requirement.get(&name).map_or(&[][..], Vec::as_slice);
        match scheme_principal(&name, scheme, required, access, headers, query).await {
            SchemeOutcome::Ready(subject) => parts.push((name, subject)),
            SchemeOutcome::Missing => remember(&mut failure, RequirementError::Missing),
            SchemeOutcome::Unauthenticated(message) => {
                remember(&mut failure, RequirementError::Unauthenticated(message));
            }
            SchemeOutcome::Forbidden(message) => {
                remember(&mut failure, RequirementError::Forbidden(message));
            }
            SchemeOutcome::Unavailable(message) => {
                return Err(RequirementError::Unavailable(message));
            }
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(identity(parts))
}

fn remember(current: &mut Option<RequirementError>, next: RequirementError) {
    let replace = matches!(
        (&*current, &next),
        (None | Some(RequirementError::Missing), _)
            | (
                Some(RequirementError::Unauthenticated(_)),
                RequirementError::Forbidden(_)
            )
    );
    if replace {
        *current = Some(next);
    }
}

fn identity(parts: Vec<(String, String)>) -> String {
    if parts.len() == 1 {
        return parts
            .into_iter()
            .next()
            .map(|(_, subject)| subject)
            .unwrap_or_default();
    }
    parts
        .into_iter()
        .map(|(name, subject)| format!("{name}={subject}"))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn scheme_principal(
    name: &str,
    scheme: &SecurityScheme,
    required: &[String],
    access: &AccessInner,
    headers: &HeaderMap,
    query: Option<&str>,
) -> SchemeOutcome {
    let Some(secret) = scheme_credential(scheme, headers, query) else {
        return SchemeOutcome::Missing;
    };
    if bearer_scheme(scheme)
        && let Some(authenticator) = &access.authenticator
    {
        return match authenticator.authenticate(&secret).await {
            Ok(principal) => scoped(&principal, required),
            Err(AuthError::Unauthenticated { message }) => SchemeOutcome::Unauthenticated(message),
            Err(AuthError::Forbidden { message }) => SchemeOutcome::Forbidden(message),
            Err(AuthError::Unavailable { message }) => SchemeOutcome::Unavailable(message),
        };
    }
    if !required.is_empty() {
        return SchemeOutcome::Forbidden("insufficient scope".to_owned());
    }
    SchemeOutcome::Ready(credential_id(name, &secret))
}

fn scoped(principal: &Principal, required: &[String]) -> SchemeOutcome {
    if principal.has_scopes(required) {
        SchemeOutcome::Ready(principal.subject().to_owned())
    } else {
        SchemeOutcome::Forbidden("insufficient scope".to_owned())
    }
}

fn bearer_scheme(scheme: &SecurityScheme) -> bool {
    match scheme {
        SecurityScheme::OAuth2(_) | SecurityScheme::OpenIdConnect(_) => true,
        SecurityScheme::HttpAuth(scheme) => scheme.scheme.eq_ignore_ascii_case("bearer"),
        SecurityScheme::ApiKey(_) | SecurityScheme::MutualTls(_) => false,
    }
}

fn credential_id(scheme: &str, secret: &str) -> String {
    use std::fmt::Write;
    let digest = Sha256::digest(secret.as_bytes());
    let mut hex = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        let _ = write!(hex, "{byte:02x}");
    }
    format!("{scheme}:{hex}")
}

fn challenge_header(schemes: &HashMap<String, SecurityScheme>) -> String {
    if schemes.values().any(bearer_scheme) {
        return "Bearer realm=\"a2a\"".to_owned();
    }
    if schemes.values().any(|scheme| {
        matches!(scheme, SecurityScheme::HttpAuth(scheme) if scheme.scheme.eq_ignore_ascii_case("basic"))
    }) {
        return "Basic realm=\"a2a\"".to_owned();
    }
    String::new()
}

pub(crate) fn auth_response(error: &AccessError) -> Response {
    let (status, grpc_status, reason, message, challenge) = match error {
        AccessError::Unauthenticated { message, challenge } => (
            StatusCode::UNAUTHORIZED,
            "UNAUTHENTICATED",
            "INVALID_REQUEST",
            message.as_str(),
            Some(challenge.as_str()),
        ),
        AccessError::Forbidden { message } => (
            StatusCode::FORBIDDEN,
            "PERMISSION_DENIED",
            "INSUFFICIENT_SCOPE",
            message.as_str(),
            None,
        ),
        AccessError::Unavailable { message } => (
            StatusCode::SERVICE_UNAVAILABLE,
            "UNAVAILABLE",
            "INTERNAL_ERROR",
            message.as_str(),
            None,
        ),
    };
    let detail = TypedDetail::error_info(reason, PROTOCOL_DOMAIN, None);
    let body = serde_json::json!({
        "error": {
            "code": status.as_u16(),
            "status": grpc_status,
            "message": message,
            "details": [detail],
        }
    });
    let mut response = Response::new(axum::body::Body::from(body.to_string()));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(challenge) = challenge.filter(|value| !value.is_empty())
        && let Ok(value) = HeaderValue::from_str(challenge)
    {
        response.headers_mut().insert(WWW_AUTHENTICATE, value);
    }
    response
}

pub(crate) fn grpc_auth_status(error: &AccessError) -> (&'static str, String) {
    match error {
        AccessError::Unauthenticated { message, .. } => ("16", percent_encode(message)),
        AccessError::Forbidden { message } => ("7", percent_encode(message)),
        AccessError::Unavailable { message } => ("14", percent_encode(message)),
    }
}

fn percent_encode(value: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn scheme_credential(
    scheme: &SecurityScheme,
    headers: &HeaderMap,
    query: Option<&str>,
) -> Option<String> {
    match scheme {
        SecurityScheme::HttpAuth(scheme) => http_credential(&scheme.scheme, headers),
        SecurityScheme::OAuth2(_) | SecurityScheme::OpenIdConnect(_) => {
            http_credential("bearer", headers)
        }
        SecurityScheme::ApiKey(scheme) => api_key_credential(scheme, headers, query),
        SecurityScheme::MutualTls(_) => None,
    }
}

fn http_credential(scheme: &str, headers: &HeaderMap) -> Option<String> {
    let value = headers.get("authorization")?.to_str().ok()?;
    let (presented, token) = value.split_once([' ', '\t'])?;
    if !presented.eq_ignore_ascii_case(scheme) {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

fn api_key_credential(
    scheme: &ApiKeySecurityScheme,
    headers: &HeaderMap,
    query: Option<&str>,
) -> Option<String> {
    let value = match scheme.location.to_ascii_lowercase().as_str() {
        "header" => headers
            .get(scheme.name.as_str())
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned),
        "query" => query_value(query, &scheme.name),
        "cookie" => cookie_value(headers, &scheme.name),
        _ => None,
    }?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn query_value(query: Option<&str>, name: &str) -> Option<String> {
    query?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| percent_decode(value))
    })
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let header = headers.get("cookie")?.to_str().ok()?;
    header.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name).then(|| value.to_owned())
    })
}

pub(crate) fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &value[index + 1..index + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                decoded.push(byte);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

struct Owners {
    tasks: HashMap<String, String>,
    contexts: HashMap<String, String>,
}

/// Task store that keeps each context with the caller that created it.
pub(crate) struct OwnedTaskStore {
    inner: InMemoryTaskStore,
    enforced: bool,
    owners: Mutex<Owners>,
}

impl OwnedTaskStore {
    pub(crate) fn new(enforced: bool) -> Self {
        Self {
            inner: InMemoryTaskStore::new(),
            enforced,
            owners: Mutex::new(Owners {
                tasks: HashMap::new(),
                contexts: HashMap::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Owners> {
        self.owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn visible(&self, task_id: &str) -> bool {
        if !self.enforced {
            return true;
        }
        let Some(caller) = current_caller() else {
            return true;
        };
        self.lock().tasks.get(task_id) == Some(&caller)
    }

    fn claim(&self, task: &Task) -> Result<(), A2AError> {
        let Some(caller) = current_caller() else {
            return Err(A2AError::invalid_request("authentication required"));
        };
        let mut owners = self.lock();
        if let Some(existing) = owners.contexts.get(&task.context_id)
            && existing != &caller
        {
            return Err(A2AError::invalid_params(
                "contextId is not available to this caller",
            ));
        }
        owners
            .contexts
            .insert(task.context_id.clone(), caller.clone());
        owners.tasks.insert(task.id.clone(), caller);
        Ok(())
    }
}

#[async_trait]
impl TaskStore for OwnedTaskStore {
    async fn create(&self, task: Task) -> Result<u64, A2AError> {
        if self.enforced {
            self.claim(&task)?;
        }
        match self.inner.create(task.clone()).await {
            Ok(version) => Ok(version),
            Err(error) => {
                if self.enforced {
                    self.lock().tasks.remove(&task.id);
                }
                Err(error)
            }
        }
    }

    async fn update(&self, task: Task) -> Result<u64, A2AError> {
        if self.enforced && !self.visible(&task.id) {
            return Err(A2AError::task_not_found(&task.id));
        }
        self.inner.update(task).await
    }

    async fn get(&self, task_id: &str) -> Result<Option<Task>, A2AError> {
        let task = self.inner.get(task_id).await?;
        if task.is_some() && !self.visible(task_id) {
            return Ok(None);
        }
        Ok(task)
    }

    async fn list(&self, req: &ListTasksRequest) -> Result<ListTasksResponse, A2AError> {
        if !self.enforced {
            return self.inner.list(req).await;
        }
        let Some(caller) = current_caller() else {
            return Ok(empty_list(req.page_size.unwrap_or(0)));
        };
        let owned = self.owned_ids(&caller);
        let tasks = self.matching_tasks(req, &owned).await?;
        page_tasks(&tasks, req)
    }

    async fn begin_cancel(&self, task_id: &str) -> Result<Task, A2AError> {
        if !self.visible(task_id) {
            return Err(A2AError::task_not_found(task_id));
        }
        self.inner.begin_cancel(task_id).await
    }
}

impl OwnedTaskStore {
    fn owned_ids(&self, caller: &str) -> HashSet<String> {
        self.lock()
            .tasks
            .iter()
            .filter(|(_, owner)| owner.as_str() == caller)
            .map(|(task_id, _)| task_id.clone())
            .collect()
    }

    async fn matching_tasks(
        &self,
        req: &ListTasksRequest,
        owned: &HashSet<String>,
    ) -> Result<Vec<Task>, A2AError> {
        let mut tasks = Vec::new();
        let mut token = None;
        for _ in 0..10_000 {
            let mut page_req = req.clone();
            page_req.page_size = Some(100);
            page_req.page_token = token.clone();
            let page = self.inner.list(&page_req).await?;
            let next = page.next_page_token;
            tasks.extend(
                page.tasks
                    .into_iter()
                    .filter(|task| owned.contains(&task.id)),
            );
            if next.is_empty() || token.as_deref() == Some(next.as_str()) {
                break;
            }
            token = Some(next);
        }
        Ok(tasks)
    }
}

fn page_tasks(tasks: &[Task], req: &ListTasksRequest) -> Result<ListTasksResponse, A2AError> {
    let page_size = a2a_server::pagination::resolve_page_size(req.page_size);
    let start = match req.page_token.as_deref() {
        None | Some("") => 0,
        Some(token) => token
            .parse::<usize>()
            .map_err(|_| A2AError::invalid_params("invalid page token"))?,
    };
    let start = start.min(tasks.len());
    let end = start.saturating_add(page_size).min(tasks.len());
    Ok(ListTasksResponse {
        tasks: tasks[start..end].to_vec(),
        next_page_token: if end < tasks.len() {
            end.to_string()
        } else {
            String::new()
        },
        page_size: i32::try_from(page_size).unwrap_or(i32::MAX),
        total_size: i32::try_from(tasks.len()).unwrap_or(i32::MAX),
    })
}

fn empty_list(page_size: i32) -> ListTasksResponse {
    ListTasksResponse {
        tasks: Vec::new(),
        next_page_token: String::new(),
        page_size,
        total_size: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a2a_types::{ApiKeySecurityScheme, HttpAuthSecurityScheme};
    use axum::http::HeaderValue;
    use std::collections::HashMap as Map;
    use std::sync::Arc;

    struct Tokens(Map<String, Result<Principal, AuthError>>);

    #[async_trait]
    impl Authenticator for Tokens {
        async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
            self.0
                .get(token)
                .cloned()
                .unwrap_or_else(|| Err(AuthError::unauthenticated("token rejected")))
        }
    }

    fn bearer_scheme() -> SecurityScheme {
        SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
            scheme: "bearer".to_owned(),
            description: None,
            bearer_format: None,
        })
    }

    fn api_key(name: &str) -> SecurityScheme {
        SecurityScheme::ApiKey(ApiKeySecurityScheme {
            location: "header".to_owned(),
            name: name.to_owned(),
            description: None,
        })
    }

    fn bearer_gate() -> AccessGate {
        AccessGate::new(
            Some([("bearerAuth".to_owned(), bearer_scheme())].into()),
            Some(vec![[("bearerAuth".to_owned(), Vec::new())].into()]),
            None,
        )
    }

    fn bearer(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {value}")).unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn bearer_token_does_not_keep_the_secret_as_the_caller() {
        let gate = bearer_gate();
        let headers = bearer("alice-token");
        let first = gate.authenticate(&headers, None).await.unwrap();
        let second = gate.authenticate(&headers, None).await.unwrap();
        assert_eq!(first, second);
        let caller = first.unwrap();
        assert!(!caller.contains("alice-token"));
        assert!(caller.starts_with("bearerAuth:"));
    }

    #[tokio::test]
    async fn missing_credential_has_no_caller() {
        let gate = bearer_gate();
        let error = gate
            .authenticate(&HeaderMap::new(), None)
            .await
            .unwrap_err();
        assert!(matches!(error, AccessError::Unauthenticated { .. }));
    }

    #[tokio::test]
    async fn requirements_are_alternatives_and_scopes_are_enforced() {
        let authenticator = Arc::new(Tokens(
            [(
                "writer".to_owned(),
                Ok(Principal::new("writer-client").with_scopes(["write"])),
            )]
            .into(),
        ));
        let gate = AccessGate::new(
            Some(
                [
                    ("api".to_owned(), api_key("x-api-key")),
                    ("bearerAuth".to_owned(), bearer_scheme()),
                ]
                .into(),
            ),
            Some(vec![
                [("api".to_owned(), Vec::new())].into(),
                [("bearerAuth".to_owned(), vec!["write".to_owned()])].into(),
            ]),
            Some(authenticator),
        );
        let mut api_headers = HeaderMap::new();
        api_headers.insert("x-api-key", HeaderValue::from_static("secret-key"));
        let api_caller = gate
            .authenticate(&api_headers, None)
            .await
            .unwrap()
            .unwrap();
        assert!(api_caller.starts_with("api:"));
        assert!(!api_caller.contains("secret-key"));
        assert_eq!(
            gate.authenticate(&bearer("writer"), None)
                .await
                .unwrap()
                .as_deref(),
            Some("writer-client")
        );
        let reader = AccessGate::new(
            Some([("bearerAuth".to_owned(), bearer_scheme())].into()),
            Some(vec![
                [("bearerAuth".to_owned(), vec!["admin".to_owned()])].into(),
            ]),
            Some(Arc::new(Tokens(
                [(
                    "writer".to_owned(),
                    Ok(Principal::new("writer-client").with_scopes(["write"])),
                )]
                .into(),
            ))),
        );
        let forbidden = reader
            .authenticate(&bearer("writer"), None)
            .await
            .unwrap_err();
        assert!(matches!(forbidden, AccessError::Forbidden { .. }));
    }

    #[tokio::test]
    async fn and_requirement_needs_every_scheme() {
        let gate = AccessGate::new(
            Some(
                [
                    ("left".to_owned(), api_key("x-left")),
                    ("right".to_owned(), api_key("x-right")),
                ]
                .into(),
            ),
            Some(vec![
                [
                    ("left".to_owned(), Vec::new()),
                    ("right".to_owned(), Vec::new()),
                ]
                .into(),
            ]),
            None,
        );
        let mut one = HeaderMap::new();
        one.insert("x-left", HeaderValue::from_static("left-secret"));
        assert!(matches!(
            gate.authenticate(&one, None).await.unwrap_err(),
            AccessError::Unauthenticated { .. }
        ));
        let mut both = one;
        both.insert("x-right", HeaderValue::from_static("right-secret"));
        let caller = gate.authenticate(&both, None).await.unwrap().unwrap();
        assert!(caller.contains("left=") && caller.contains("right="));
        assert!(!caller.contains("left-secret"));
        assert!(!caller.contains("right-secret"));
    }

    #[test]
    fn oauth_and_scopes_require_an_authenticator() {
        let oauth = SecurityScheme::OpenIdConnect(a2a_types::OpenIdConnectSecurityScheme {
            open_id_connect_url: "http://issuer/.well-known/openid-configuration".to_owned(),
            description: None,
        });
        let schemes = [("oidc".to_owned(), oauth)].into();
        let requirements = vec![[("oidc".to_owned(), vec!["a2a.invoke".to_owned()])].into()];
        assert!(security_configuration_error(&schemes, &requirements, false).is_err());
        assert!(security_configuration_error(&schemes, &requirements, true).is_ok());
    }
}
