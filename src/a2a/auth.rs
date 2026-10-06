//! OAuth2 and OpenID Connect bearer validation.

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use serde_json::Value;

use crate::error::A2aLabError;

const KEY_TTL: Duration = Duration::from_secs(60);

/// Caller established from a validated credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    subject: String,
    scopes: BTreeSet<String>,
}

impl Principal {
    /// Creates a principal with no scopes.
    #[must_use]
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            scopes: BTreeSet::new(),
        }
    }

    /// Replaces the granted scopes.
    #[must_use]
    pub fn with_scopes(mut self, scopes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }

    /// Stable caller id. This is not the bearer token.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Scopes granted by the credential.
    #[must_use]
    pub fn scopes(&self) -> &BTreeSet<String> {
        &self.scopes
    }

    /// Reports whether every required scope was granted.
    #[must_use]
    pub fn has_scopes(&self, required: &[String]) -> bool {
        required.iter().all(|scope| self.scopes.contains(scope))
    }
}

/// Failure while checking a bearer token.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// The credential is missing, malformed, or not trusted.
    #[error("{message}")]
    Unauthenticated {
        /// Human-readable explanation.
        message: String,
    },
    /// The credential is valid and does not grant the requested access.
    #[error("{message}")]
    Forbidden {
        /// Human-readable explanation.
        message: String,
    },
    /// The identity provider or its signing keys could not be read.
    #[error("{message}")]
    Unavailable {
        /// Human-readable explanation.
        message: String,
    },
}

impl AuthError {
    /// Creates an authentication failure.
    #[must_use]
    pub fn unauthenticated(message: impl Into<String>) -> Self {
        Self::Unauthenticated {
            message: message.into(),
        }
    }

    /// Creates an authorization failure.
    #[must_use]
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden {
            message: message.into(),
        }
    }

    /// Creates an identity-provider availability failure.
    #[must_use]
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable {
            message: message.into(),
        }
    }
}

/// Validates a bearer token into a [`Principal`].
#[async_trait]
pub trait Authenticator: Send + Sync {
    /// Validates `token` without retaining it.
    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError>;

    /// Loads provider metadata before the server accepts calls.
    async fn warm(&self) -> Result<(), AuthError> {
        Ok(())
    }
}

/// Validates RS256 access tokens from an OpenID Connect issuer.
#[derive(Clone)]
pub struct OidcAuthenticator {
    inner: std::sync::Arc<OidcInner>,
}

struct OidcInner {
    issuer: String,
    audience: String,
    discovery_url: String,
    http: reqwest::Client,
    keys: Mutex<Option<CachedKeys>>,
}

struct CachedKeys {
    fetched: Instant,
    by_kid: HashMap<String, String>,
    only: Option<String>,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    jwks_uri: String,
}

#[derive(Deserialize)]
struct Jwks {
    keys: Vec<Value>,
}

impl OidcAuthenticator {
    /// Trusts tokens for `audience` from `issuer`.
    ///
    /// Discovery is `{issuer}/.well-known/openid-configuration`.
    pub fn new(
        issuer: impl Into<String>,
        audience: impl Into<String>,
    ) -> Result<Self, A2aLabError> {
        let issuer = normalize_issuer(&issuer.into());
        let audience = audience.into();
        if issuer.is_empty() || audience.is_empty() {
            return Err(A2aLabError::invalid(
                "security",
                "issuer and audience are required",
            ));
        }
        let discovery_url = format!("{issuer}/.well-known/openid-configuration");
        Ok(Self::from_parts(issuer, audience, discovery_url))
    }

    /// Overrides the OpenID Connect discovery document URL.
    #[must_use]
    pub fn with_discovery_url(self, discovery_url: impl Into<String>) -> Self {
        let discovery_url = discovery_url.into();
        Self::from_parts(
            self.inner.issuer.clone(),
            self.inner.audience.clone(),
            discovery_url,
        )
    }

    /// Issuer configured for this authenticator.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.inner.issuer
    }

    /// Audience required on access tokens.
    #[must_use]
    pub fn audience(&self) -> &str {
        &self.inner.audience
    }

    /// OpenID Connect discovery URL advertised on the Agent Card.
    #[must_use]
    pub fn discovery_url(&self) -> &str {
        &self.inner.discovery_url
    }

    fn from_parts(issuer: String, audience: String, discovery_url: String) -> Self {
        Self {
            inner: std::sync::Arc::new(OidcInner {
                issuer,
                audience,
                discovery_url,
                http: reqwest::Client::builder()
                    .timeout(Duration::from_secs(5))
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new()),
                keys: Mutex::new(None),
            }),
        }
    }
}

#[async_trait]
impl Authenticator for OidcAuthenticator {
    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        let header =
            decode_header(token).map_err(|_| AuthError::unauthenticated("token malformed"))?;
        if header.alg != Algorithm::RS256 {
            return Err(AuthError::unauthenticated("token algorithm rejected"));
        }
        let jwk = self.inner.signing_key(header.kid.as_deref()).await?;
        let key = DecodingKey::from_jwk(&jwk)
            .map_err(|_| AuthError::unavailable("signing key rejected"))?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.inner.issuer]);
        validation.set_audience(&[&self.inner.audience]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.validate_nbf = true;
        validation.leeway = 5;
        let data = decode::<Value>(token, &key, &validation)
            .map_err(|error| AuthError::unauthenticated(jwt_message(error.kind())))?;
        principal_from_claims(&data.claims)
    }

    async fn warm(&self) -> Result<(), AuthError> {
        self.inner.refresh().await.map(|_| ())
    }
}

impl OidcInner {
    async fn signing_key(&self, kid: Option<&str>) -> Result<Jwk, AuthError> {
        if let Some(jwk) = self.lookup(kid) {
            return json_jwk(&jwk);
        }
        self.refresh().await?;
        self.lookup(kid)
            .ok_or_else(|| AuthError::unauthenticated("token key rejected"))
            .and_then(|jwk| json_jwk(&jwk))
    }

    fn lookup(&self, kid: Option<&str>) -> Option<String> {
        let guard = self
            .keys
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cached = guard.as_ref()?;
        if cached.fetched.elapsed() > KEY_TTL {
            return None;
        }
        match kid {
            Some(kid) => cached.by_kid.get(kid).cloned(),
            None => cached.only.clone(),
        }
    }

    async fn refresh(&self) -> Result<CachedKeys, AuthError> {
        let discovery = self
            .http
            .get(&self.discovery_url)
            .send()
            .await
            .map_err(|error| AuthError::unavailable(error.to_string()))?
            .error_for_status()
            .map_err(|error| AuthError::unavailable(error.to_string()))?
            .json::<Discovery>()
            .await
            .map_err(|error| AuthError::unavailable(error.to_string()))?;
        if normalize_issuer(&discovery.issuer) != self.issuer {
            return Err(AuthError::unavailable("discovery issuer mismatch"));
        }
        let jwks = self
            .http
            .get(&discovery.jwks_uri)
            .send()
            .await
            .map_err(|error| AuthError::unavailable(error.to_string()))?
            .error_for_status()
            .map_err(|error| AuthError::unavailable(error.to_string()))?
            .json::<Jwks>()
            .await
            .map_err(|error| AuthError::unavailable(error.to_string()))?;
        let cached = cache_jwks(jwks)?;
        *self
            .keys
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(CachedKeys {
            fetched: cached.fetched,
            by_kid: cached.by_kid.clone(),
            only: cached.only.clone(),
        });
        Ok(cached)
    }
}

fn cache_jwks(jwks: Jwks) -> Result<CachedKeys, AuthError> {
    let mut by_kid = HashMap::new();
    let mut signing = Vec::new();
    for key in jwks.keys {
        if key.get("kty").and_then(Value::as_str) != Some("RSA") {
            continue;
        }
        if key.get("use").and_then(Value::as_str) == Some("enc") {
            continue;
        }
        let encoded = serde_json::to_string(&key)
            .map_err(|error| AuthError::unavailable(error.to_string()))?;
        signing.push(encoded.clone());
        if let Some(kid) = key.get("kid").and_then(Value::as_str) {
            by_kid.insert(kid.to_owned(), encoded);
        }
    }
    if signing.is_empty() {
        return Err(AuthError::unavailable(
            "discovery document has no signing keys",
        ));
    }
    let only = (signing.len() == 1).then(|| signing.remove(0));
    Ok(CachedKeys {
        fetched: Instant::now(),
        by_kid,
        only,
    })
}

fn json_jwk(value: &str) -> Result<Jwk, AuthError> {
    serde_json::from_str(value).map_err(|error| AuthError::unavailable(error.to_string()))
}

fn principal_from_claims(claims: &Value) -> Result<Principal, AuthError> {
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|subject| !subject.is_empty())
        .ok_or_else(|| AuthError::unauthenticated("token subject missing"))?;
    Ok(Principal::new(subject).with_scopes(scopes_from_claims(claims)))
}

fn scopes_from_claims(claims: &Value) -> BTreeSet<String> {
    let mut scopes = BTreeSet::new();
    if let Some(scope) = claims.get("scope").and_then(Value::as_str) {
        scopes.extend(scope.split_whitespace().map(str::to_owned));
    }
    match claims.get("scp") {
        Some(Value::String(scope)) => {
            scopes.extend(scope.split_whitespace().map(str::to_owned));
        }
        Some(Value::Array(items)) => {
            scopes.extend(
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned)),
            );
        }
        _ => {}
    }
    scopes
}

fn jwt_message(kind: &jsonwebtoken::errors::ErrorKind) -> &'static str {
    use jsonwebtoken::errors::ErrorKind;
    match kind {
        ErrorKind::ExpiredSignature => "token expired",
        ErrorKind::ImmatureSignature => "token not yet valid",
        ErrorKind::InvalidAudience => "token audience rejected",
        ErrorKind::InvalidIssuer => "token issuer rejected",
        ErrorKind::InvalidSignature => "token signature rejected",
        _ => "token rejected",
    }
}

fn normalize_issuer(issuer: &str) -> String {
    issuer.trim().trim_end_matches('/').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header};
    use rsa::RsaPrivateKey;
    use rsa::pkcs8::EncodePrivateKey;
    use rsa::traits::PublicKeyParts;

    struct Keys {
        encoding: EncodingKey,
        jwk: Value,
    }

    fn keys(kid: &str) -> Keys {
        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pem = private.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let n = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            private.n().to_bytes_be(),
        );
        let e = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            private.e().to_bytes_be(),
        );
        Keys {
            encoding: EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap(),
            jwk: serde_json::json!({
                "kty": "RSA",
                "use": "sig",
                "alg": "RS256",
                "kid": kid,
                "n": n,
                "e": e,
            }),
        }
    }

    fn token(keys: &Keys, kid: &str, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_owned());
        jsonwebtoken::encode(&header, claims, &keys.encoding).unwrap()
    }

    fn claims(issuer: &str, audience: &str, exp: u64, scope: &str) -> Value {
        serde_json::json!({
            "sub": "client-a",
            "iss": issuer,
            "aud": audience,
            "exp": exp,
            "nbf": 1_700_000_000,
            "scope": scope,
        })
    }

    async fn provider(published: std::sync::Arc<Mutex<Value>>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let issuer_for_doc = issuer.clone();
        let keys_for_route = std::sync::Arc::clone(&published);
        let app = axum::Router::new()
            .route(
                "/.well-known/openid-configuration",
                axum::routing::get(move || {
                    let issuer = issuer_for_doc.clone();
                    async move {
                        axum::Json(serde_json::json!({
                            "issuer": issuer,
                            "jwks_uri": format!("{issuer}/jwks"),
                        }))
                    }
                }),
            )
            .route(
                "/jwks",
                axum::routing::get(move || {
                    let keys = std::sync::Arc::clone(&keys_for_route);
                    async move {
                        axum::Json(
                            keys.lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .clone(),
                        )
                    }
                }),
            );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        issuer
    }

    #[test]
    fn scopes_combine_string_and_array_claims() {
        let claims = serde_json::json!({
            "scope": "a2a.invoke profile",
            "scp": ["extra", "a2a.invoke"],
        });
        let scopes = scopes_from_claims(&claims);
        assert!(scopes.contains("a2a.invoke"));
        assert!(scopes.contains("profile"));
        assert!(scopes.contains("extra"));
    }

    #[tokio::test]
    async fn validates_issuer_audience_expiry_and_refreshes_unknown_keys() {
        let current = keys("current");
        let rotated = keys("rotated");
        let published = std::sync::Arc::new(Mutex::new(serde_json::json!({
            "keys": [current.jwk.clone()],
        })));
        let issuer = provider(std::sync::Arc::clone(&published)).await;
        let authenticator = OidcAuthenticator::new(&issuer, "a2a-lab").unwrap();
        authenticator.warm().await.unwrap();
        let exp = u64::try_from(
            jiff::Timestamp::now()
                .checked_add(jiff::SignedDuration::from_secs(600))
                .unwrap()
                .as_second(),
        )
        .unwrap();
        let principal = authenticator
            .authenticate(&token(
                &current,
                "current",
                &claims(&issuer, "a2a-lab", exp, "a2a.invoke"),
            ))
            .await
            .unwrap();
        assert_eq!(principal.subject(), "client-a");
        assert!(principal.has_scopes(&["a2a.invoke".to_owned()]));

        let expired = authenticator
            .authenticate(&token(
                &current,
                "current",
                &claims(&issuer, "a2a-lab", 1_700_000_100, "a2a.invoke"),
            ))
            .await
            .unwrap_err();
        assert_eq!(expired, AuthError::unauthenticated("token expired"));
        let audience = authenticator
            .authenticate(&token(
                &current,
                "current",
                &claims(&issuer, "other", exp, "a2a.invoke"),
            ))
            .await
            .unwrap_err();
        assert_eq!(
            audience,
            AuthError::unauthenticated("token audience rejected")
        );
        let wrong_issuer = authenticator
            .authenticate(&token(
                &current,
                "current",
                &claims("http://other.example", "a2a-lab", exp, "a2a.invoke"),
            ))
            .await
            .unwrap_err();
        assert_eq!(
            wrong_issuer,
            AuthError::unauthenticated("token issuer rejected")
        );
        let signature = authenticator
            .authenticate(&token(
                &rotated,
                "current",
                &claims(&issuer, "a2a-lab", exp, "a2a.invoke"),
            ))
            .await
            .unwrap_err();
        assert_eq!(
            signature,
            AuthError::unauthenticated("token signature rejected")
        );

        *published.lock().unwrap() = serde_json::json!({ "keys": [rotated.jwk.clone()] });
        let refreshed = authenticator
            .authenticate(&token(
                &rotated,
                "rotated",
                &claims(&issuer, "a2a-lab", exp, "a2a.read"),
            ))
            .await
            .unwrap();
        assert!(refreshed.has_scopes(&["a2a.read".to_owned()]));
    }
}
