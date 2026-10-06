//! Serves a memory lab over A2A HTTP+JSON, JSON-RPC, and gRPC for the official TCK.

use std::io::{Write, stderr};
use std::sync::Arc;

use a2a_lab_dev_kit::{
    A2aLabService, A2aServer, AgentCard, LogSource, MemoryLogs, MemoryMetrics, MemoryTasks,
    OidcAuthenticator, OpenIdConnectSecurityScheme, SecurityScheme, SourceId, TaskDefinition,
    TaskId,
};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let logs = MemoryLogs::new();
    logs.insert_source(LogSource {
        id: SourceId::new("app")?,
        name: "App".to_owned(),
        description: "Application logs".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
    let tasks = MemoryTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("build")?,
            name: "Build".to_owned(),
            description: "Build the lab".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    let service = A2aLabService::new(logs, MemoryMetrics::new(), tasks).share();
    let advertise = std::env::var("A2A_TCK_ADVERTISE").ok();
    let bind = if advertise.is_some() {
        "0.0.0.0:0"
    } else {
        "127.0.0.1:0"
    };
    let listener = TcpListener::bind(bind).await?;
    let port = listener.local_addr()?.port();
    writeln!(stderr(), "A2A_TCK_SUT=http://127.0.0.1:{port}")?;
    if let Some(host) = advertise.as_deref() {
        writeln!(stderr(), "A2A_TCK_DOCKER_SUT=http://{host}:{port}")?;
    }
    stderr().flush()?;
    let mut server = A2aServer::new(&service);
    if let Some(host) = advertise {
        server = server
            .with_public_url(format!("http://{host}:{port}"))
            .with_grpc_host(host);
    }
    server = with_optional_oidc(server)?;
    server.listen(listener).await?;
    Ok(())
}

fn with_optional_oidc(
    server: A2aServer,
) -> Result<A2aServer, Box<dyn std::error::Error + Send + Sync>> {
    let Ok(issuer) = std::env::var("A2A_TCK_OIDC_ISSUER") else {
        return Ok(server);
    };
    let audience = std::env::var("A2A_TCK_OIDC_AUDIENCE").unwrap_or_else(|_| "a2a-lab".to_owned());
    let scope = std::env::var("A2A_TCK_OIDC_SCOPE").unwrap_or_else(|_| "a2a.invoke".to_owned());
    let mut authenticator = OidcAuthenticator::new(issuer, audience)?;
    if let Ok(discovery) = std::env::var("A2A_TCK_OIDC_DISCOVERY") {
        authenticator = authenticator.with_discovery_url(discovery);
    }
    let mut server = server.with_security(
        [(
            "oidc".to_owned(),
            SecurityScheme::OpenIdConnect(OpenIdConnectSecurityScheme {
                open_id_connect_url: authenticator.discovery_url().to_owned(),
                description: Some("OpenID Connect client credentials".to_owned()),
            }),
        )]
        .into(),
        vec![[("oidc".to_owned(), vec![scope])].into()],
    );
    if std::env::var("A2A_TCK_EXTENDED_CARD").is_ok_and(|value| value == "1") {
        server = server.with_extended_card(extended_card());
    }
    Ok(server.with_authenticator(Arc::new(authenticator)))
}

fn extended_card() -> AgentCard {
    serde_json::from_value(serde_json::json!({
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
    .expect("extended card")
}
