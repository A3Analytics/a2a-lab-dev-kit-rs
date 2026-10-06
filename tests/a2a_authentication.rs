use std::sync::{Arc, Mutex};
use std::time::Duration;

use a2a_client::A2AClient;
use a2a_client::auth::AuthInterceptor;
use a2a_grpc::GrpcTransport;
use a2a_lab_dev_kit::{
    A2aClient, A2aServer, AgentCard, A2aLabService, LogSource, MemoryLogs, MemoryMetrics, MemoryTasks,
    OidcAuthenticator, OpenIdConnectSecurityScheme, SecurityScheme, SourceId, TaskDefinition,
    TaskId, bind_local,
};
use a2a_types::{Message, Part, Role, SendMessageRequest};
use base64::Engine;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::RsaPrivateKey;
use rsa::pkcs8::EncodePrivateKey;
use rsa::traits::PublicKeyParts;
use serde_json::{Value, json};

struct Keys {
    encoding: EncodingKey,
    jwk: Value,
}

fn keys(kid: &str) -> Keys {
    let mut rng = rand::thread_rng();
    let private = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let pem = private.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
    Keys {
        encoding: EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap(),
        jwk: json!({
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": kid,
            "n": b64(private.n().to_bytes_be()),
            "e": b64(private.e().to_bytes_be()),
        }),
    }
}

fn b64(bytes: Vec<u8>) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn token(keys: &Keys, kid: &str, claims: &Value) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.to_owned());
    jsonwebtoken::encode(&header, claims, &keys.encoding).unwrap()
}

fn claims(subject: &str, issuer: &str, audience: &str, exp: u64, scope: &str) -> Value {
    json!({
        "sub": subject,
        "iss": issuer,
        "aud": audience,
        "exp": exp,
        "nbf": 1_700_000_000,
        "scope": scope,
    })
}

fn exp_in(seconds: i64) -> u64 {
    u64::try_from(
        jiff::Timestamp::now()
            .checked_add(jiff::SignedDuration::from_secs(seconds))
            .unwrap()
            .as_second(),
    )
    .unwrap()
}

struct Fixture {
    base: String,
    issuer: String,
    signing: Keys,
}

async fn local_fixture() -> Fixture {
    let signing = keys("local");
    let published = Arc::new(Mutex::new(json!({ "keys": [signing.jwk.clone()] })));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let issuer_for_doc = issuer.clone();
    let keys_for_route = Arc::clone(&published);
    let app = axum::Router::new()
        .route(
            "/.well-known/openid-configuration",
            axum::routing::get(move || {
                let issuer = issuer_for_doc.clone();
                async move {
                    axum::Json(json!({
                        "issuer": issuer,
                        "jwks_uri": format!("{issuer}/jwks"),
                    }))
                }
            }),
        )
        .route(
            "/jwks",
            axum::routing::get(move || {
                let keys = Arc::clone(&keys_for_route);
                async move { axum::Json(keys.lock().unwrap().clone()) }
            }),
        );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let authenticator = OidcAuthenticator::new(&issuer, "a2a-lab").unwrap();
    let base = serve(authenticator, extended_card()).await;
    Fixture {
        base,
        issuer,
        signing,
    }
}

fn extended_card() -> AgentCard {
    serde_json::from_value(json!({
        "name": "a2a-lab-extended",
        "description": "Authenticated card",
        "version": "0.1.0",
        "supportedInterfaces": [{
            "url": "http://127.0.0.1:1",
            "protocolBinding": "HTTP+JSON",
            "protocolVersion": "1.0"
        }],
        "capabilities": {"streaming": true, "extendedAgentCard": true},
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": []
    }))
    .unwrap()
}

async fn serve(authenticator: OidcAuthenticator, card: AgentCard) -> String {
    let logs = MemoryLogs::new();
    logs.insert_source(LogSource {
        id: SourceId::new("app").unwrap(),
        name: "App".to_owned(),
        description: "Application logs".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("build").unwrap(),
            name: "Build".to_owned(),
            description: "Build the lab".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    let service = A2aLabService::new(logs, MemoryMetrics::new(), tasks).share();
    let (listener, address) = bind_local().await.unwrap();
    let discovery = authenticator.discovery_url().to_owned();
    let server = A2aServer::new(&service)
        .with_extended_card(card)
        .with_security(
            [(
                "oidc".to_owned(),
                SecurityScheme::OpenIdConnect(OpenIdConnectSecurityScheme {
                    open_id_connect_url: discovery,
                    description: None,
                }),
            )]
            .into(),
            vec![[("oidc".to_owned(), vec!["a2a.invoke".to_owned()])].into()],
        )
        .with_authenticator(Arc::new(authenticator));
    tokio::spawn(async move {
        server.listen(listener).await.unwrap();
    });
    format!("http://{address}")
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

async fn send(base: &str, token: Option<&str>) -> reqwest::Response {
    let mut request = http()
        .post(format!("{base}/message:send"))
        .header("Content-Type", "application/a2a+json")
        .header("A2A-Version", "1.0")
        .json(&json!({
            "message": {"messageId": "auth-http", "role": "ROLE_USER", "parts": [{"text": "hello"}]}
        }));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().await.unwrap()
}

async fn rpc(base: &str, token: Option<&str>) -> reqwest::Response {
    let mut request = http()
        .post(format!("{base}/"))
        .header("Content-Type", "application/json")
        .header("A2A-Version", "1.0")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "auth-rpc",
            "method": "SendMessage",
            "params": {"message": {"messageId": "auth-rpc", "role": "ROLE_USER", "parts": [{"text": "hello"}]}}
        }));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().await.unwrap()
}

async fn grpc_url(base: &str) -> String {
    let card: Value = http()
        .get(format!("{base}/.well-known/agent-card.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    card["supportedInterfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|interface| interface["protocolBinding"] == "GRPC")
        .unwrap()["url"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn grpc_send(url: &str, token: Option<&str>) -> Result<(), String> {
    let transport = GrpcTransport::connect(url)
        .await
        .map_err(|error| error.to_string())?;
    let mut client = A2AClient::new(transport);
    if let Some(token) = token {
        client = client.with_interceptors(vec![Arc::new(AuthInterceptor::bearer(token))]);
    }
    let mut message = Message::new(Role::User, vec![Part::text("hello")]);
    "auth-grpc".clone_into(&mut message.message_id);
    client
        .send_message(&SendMessageRequest {
            message,
            configuration: None,
            metadata: None,
            tenant: None,
        })
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

async fn assert_local_rejections(fixture: &Fixture) {
    let rejected = [
        ("malformed", "not-a-token".to_owned()),
        (
            "expired",
            token(
                &fixture.signing,
                "local",
                &claims(
                    "alice",
                    &fixture.issuer,
                    "a2a-lab",
                    1_700_000_100,
                    "a2a.invoke",
                ),
            ),
        ),
        (
            "audience",
            token(
                &fixture.signing,
                "local",
                &claims("alice", &fixture.issuer, "other", exp_in(600), "a2a.invoke"),
            ),
        ),
        (
            "issuer",
            token(
                &fixture.signing,
                "local",
                &claims(
                    "alice",
                    "http://other.example",
                    "a2a-lab",
                    exp_in(600),
                    "a2a.invoke",
                ),
            ),
        ),
        (
            "signature",
            token(
                &keys("other"),
                "local",
                &claims(
                    "alice",
                    &fixture.issuer,
                    "a2a-lab",
                    exp_in(600),
                    "a2a.invoke",
                ),
            ),
        ),
    ];
    for (name, presented) in rejected {
        let response = send(&fixture.base, Some(&presented)).await;
        assert_eq!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED,
            "{name}"
        );
    }
    let limited = token(
        &fixture.signing,
        "local",
        &claims("alice", &fixture.issuer, "a2a-lab", exp_in(600), "a2a.read"),
    );
    assert_eq!(
        send(&fixture.base, Some(&limited)).await.status(),
        reqwest::StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn local_oidc_tokens_are_enforced_on_every_transport() {
    let fixture = local_fixture().await;
    let valid = token(
        &fixture.signing,
        "local",
        &claims(
            "alice",
            &fixture.issuer,
            "a2a-lab",
            exp_in(600),
            "a2a.invoke",
        ),
    );
    let card = http()
        .get(format!("{}/.well-known/agent-card.json", fixture.base))
        .send()
        .await
        .unwrap();
    assert!(card.status().is_success());
    let body: Value = card.json().await.unwrap();
    assert!(body["securitySchemes"]["oidc"]["openIdConnectSecurityScheme"].is_object());

    let missing = send(&fixture.base, None).await;
    assert_eq!(missing.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(missing.headers().get("www-authenticate").is_some());
    assert_eq!(
        rpc(&fixture.base, None).await.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let grpc = grpc_url(&fixture.base).await;
    let unauthenticated = grpc_send(&grpc, None).await.unwrap_err();
    assert!(
        unauthenticated.contains("authentication required"),
        "{unauthenticated}"
    );

    assert_local_rejections(&fixture).await;

    assert!(
        send(&fixture.base, Some(&valid))
            .await
            .status()
            .is_success()
    );
    assert!(rpc(&fixture.base, Some(&valid)).await.status().is_success());
    grpc_send(&grpc, Some(&valid)).await.unwrap();

    let extended = http()
        .get(format!("{}/extendedAgentCard", fixture.base))
        .send()
        .await
        .unwrap();
    assert_eq!(extended.status(), reqwest::StatusCode::UNAUTHORIZED);
    let opened = http()
        .get(format!("{}/extendedAgentCard", fixture.base))
        .bearer_auth(&valid)
        .send()
        .await
        .unwrap();
    assert!(opened.status().is_success());

    let alice = A2aClient::new(&fixture.base)
        .unwrap()
        .with_bearer_token(&valid);
    let bob_token = token(
        &fixture.signing,
        "local",
        &claims("bob", &fixture.issuer, "a2a-lab", exp_in(600), "a2a.invoke"),
    );
    let bob = A2aClient::new(&fixture.base)
        .unwrap()
        .with_bearer_token(bob_token);
    let created = alice.agent_message("hello", None).await.unwrap();
    let hidden = bob
        .get_a2a_task(created.task_id.as_deref().unwrap())
        .await
        .unwrap_err();
    assert_eq!(hidden.code(), "not_found");
}

#[tokio::test]
async fn oauth_security_without_an_authenticator_fails_startup() {
    let logs = MemoryLogs::new();
    let service = A2aLabService::new(logs, MemoryMetrics::new(), MemoryTasks::new()).share();
    let (listener, _) = bind_local().await.unwrap();
    let error = A2aServer::new(&service)
        .with_security(
            [(
                "oidc".to_owned(),
                SecurityScheme::OpenIdConnect(OpenIdConnectSecurityScheme {
                    open_id_connect_url: "http://127.0.0.1/discovery".to_owned(),
                    description: None,
                }),
            )]
            .into(),
            vec![[("oidc".to_owned(), Vec::new())].into()],
        )
        .listen(listener)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "invalid");
}

#[tokio::test]
async fn keycloak_client_credentials_are_enforced_on_every_transport() {
    let issuer = std::env::var("A2A_AUTH_ISSUER").expect("A2A_AUTH_ISSUER");
    let authenticator = OidcAuthenticator::new(&issuer, "a2a-lab").unwrap();
    let base = serve(authenticator, extended_card()).await;
    let alice = keycloak_token(&issuer, "a2a-valid", "a2a-valid-secret").await;
    let bob = keycloak_token(&issuer, "a2a-bob", "a2a-bob-secret").await;
    let card = http()
        .get(format!("{base}/.well-known/agent-card.json"))
        .send()
        .await
        .unwrap();
    assert!(card.status().is_success());
    assert_eq!(
        send(&base, None).await.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert!(send(&base, Some(&alice)).await.status().is_success());
    assert!(rpc(&base, Some(&alice)).await.status().is_success());
    let grpc = grpc_url(&base).await;
    grpc_send(&grpc, Some(&alice)).await.unwrap();
    let extended = http()
        .get(format!("{base}/extendedAgentCard"))
        .bearer_auth(&alice)
        .send()
        .await
        .unwrap();
    assert!(extended.status().is_success());
    let alice_client = A2aClient::new(&base).unwrap().with_bearer_token(&alice);
    let bob_client = A2aClient::new(&base).unwrap().with_bearer_token(&bob);
    let created = alice_client.agent_message("hello", None).await.unwrap();
    let hidden = bob_client
        .get_a2a_task(created.task_id.as_deref().unwrap())
        .await
        .unwrap_err();
    assert_eq!(hidden.code(), "not_found");
}

#[tokio::test]
async fn keycloak_rejects_missing_invalid_and_insufficient_tokens() {
    let issuer = std::env::var("A2A_AUTH_ISSUER").expect("A2A_AUTH_ISSUER");
    let authenticator = OidcAuthenticator::new(&issuer, "a2a-lab").unwrap();
    let base = serve(authenticator, extended_card()).await;
    assert_eq!(
        send(&base, Some("not-a-token")).await.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let mut tampered = keycloak_token(&issuer, "a2a-valid", "a2a-valid-secret").await;
    tampered.push('x');
    assert_eq!(
        send(&base, Some(&tampered)).await.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let foreign = token(
        &keys("foreign"),
        "foreign",
        &claims(
            "alice",
            "http://other.example",
            "a2a-lab",
            exp_in(600),
            "a2a.invoke",
        ),
    );
    assert_eq!(
        send(&base, Some(&foreign)).await.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let limited = keycloak_token(&issuer, "a2a-limited", "a2a-limited-secret").await;
    assert_eq!(
        send(&base, Some(&limited)).await.status(),
        reqwest::StatusCode::FORBIDDEN
    );
    let other_audience = keycloak_token(&issuer, "a2a-audience", "a2a-audience-secret").await;
    assert_eq!(
        send(&base, Some(&other_audience)).await.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let expired = keycloak_token(&issuer, "a2a-expired", "a2a-expired-secret").await;
    tokio::time::sleep(Duration::from_secs(8)).await;
    assert_eq!(
        send(&base, Some(&expired)).await.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
}

async fn keycloak_token(issuer: &str, client_id: &str, secret: &str) -> String {
    let response = http()
        .post(format!("{issuer}/protocol/openid-connect/token"))
        .basic_auth(client_id, Some(secret))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("grant_type=client_credentials")
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert!(status.is_success(), "{client_id}: {status} {body}");
    serde_json::from_str::<Value>(&body).unwrap()["access_token"]
        .as_str()
        .unwrap()
        .to_owned()
}
