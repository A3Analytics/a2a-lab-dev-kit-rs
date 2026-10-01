//! Serves a memory lab over A2A HTTP+JSON for the official TCK.

use std::io::{Write, stderr};

use a2a_lab_dev_kit::{
    A2aServer, LabService, LogSource, MemoryLogs, MemoryMetrics, MemoryTasks, SourceId,
    TaskDefinition, TaskId, bind_local,
};

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
    let (listener, address) = bind_local().await?;
    writeln!(stderr(), "A2A_TCK_SUT=http://{address}")?;
    stderr().flush()?;
    A2aServer::new(&service).listen(listener).await?;
    Ok(())
}
