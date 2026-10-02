//! Serves a memory lab over A2A HTTP+JSON, JSON-RPC, and gRPC for the official TCK.

use std::io::{Write, stderr};

use a2a_lab_dev_kit::{
    A2aServer, LabService, LogSource, MemoryLogs, MemoryMetrics, MemoryTasks, SourceId,
    TaskDefinition, TaskId,
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
    let service = LabService::new(logs, MemoryMetrics::new(), tasks).share();
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
    server.listen(listener).await?;
    Ok(())
}
