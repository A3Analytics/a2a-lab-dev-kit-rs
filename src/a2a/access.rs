//! Caller identity for security requirements, and task visibility for that caller.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};

use a2a_server::{InMemoryTaskStore, RequestAuthorizer, ServiceParams, TaskStore};
use a2a_types::{
    A2AError, ApiKeySecurityScheme, ListTasksRequest, ListTasksResponse, SecurityRequirement,
    SecurityScheme, Task,
};
use async_trait::async_trait;
use axum::http::HeaderMap;

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
}

impl AccessGate {
    pub(crate) fn new(
        schemes: Option<HashMap<String, SecurityScheme>>,
        requirements: Option<Vec<SecurityRequirement>>,
    ) -> Self {
        let requirements = requirements.unwrap_or_default();
        Self {
            inner: Arc::new(AccessInner {
                enforced: !requirements.is_empty(),
                schemes: schemes.unwrap_or_default(),
                requirements,
            }),
        }
    }

    /// Credential identity for this request, when security requirements are active.
    ///
    /// HTTP bearer, HTTP basic, API key, OAuth2, and OpenID Connect credentials are
    /// the caller. This server does not contact an identity provider. Mutual TLS is
    /// not accepted here, so a card that requires only mutual TLS fails closed.
    pub(crate) fn caller(&self, headers: &HeaderMap, query: Option<&str>) -> Option<String> {
        if !self.inner.enforced {
            return None;
        }
        let mut anonymous = false;
        for requirement in &self.inner.requirements {
            if requirement.is_empty() {
                anonymous = true;
                continue;
            }
            if let Some(caller) =
                requirement_caller(requirement, &self.inner.schemes, headers, query)
            {
                return Some(caller);
            }
        }
        anonymous.then(|| ANONYMOUS.to_owned())
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

fn requirement_caller(
    requirement: &SecurityRequirement,
    schemes: &HashMap<String, SecurityScheme>,
    headers: &HeaderMap,
    query: Option<&str>,
) -> Option<String> {
    let mut names: Vec<_> = requirement.keys().cloned().collect();
    names.sort();
    let mut parts = Vec::with_capacity(names.len());
    for name in names {
        let scheme = schemes.get(&name)?;
        let credential = scheme_credential(scheme, headers, query)?;
        parts.push(format!("{name}:{credential}"));
    }
    Some(parts.join("\n"))
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
    use a2a_types::HttpAuthSecurityScheme;
    use axum::http::HeaderValue;

    fn bearer_gate() -> AccessGate {
        AccessGate::new(
            Some(
                [(
                    "bearerAuth".to_owned(),
                    SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                        scheme: "bearer".to_owned(),
                        description: None,
                        bearer_format: None,
                    }),
                )]
                .into(),
            ),
            Some(vec![[("bearerAuth".to_owned(), Vec::new())].into()]),
        )
    }

    #[test]
    fn bearer_token_identifies_the_caller() {
        let gate = bearer_gate();
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer alice-token"),
        );
        assert_eq!(
            gate.caller(&headers, None).as_deref(),
            Some("bearerAuth:alice-token")
        );
    }

    #[test]
    fn missing_credential_has_no_caller() {
        let gate = bearer_gate();
        assert!(gate.caller(&HeaderMap::new(), None).is_none());
    }
}
