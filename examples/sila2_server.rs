//! One lab service exposed over A2A, MCP, and `SiLA` 2.

use std::env;
use std::fs;
use std::sync::Arc;

use a2a_lab_dev_kit::sila::{SilaCertificate, SilaIdentity, SilaServer};
use a2a_lab_dev_kit::{
    A2aLabError, A2aServer, GetTaskStatusRequest, JsonObject, A2aLabService, ListTasksRequest,
    LogLevel, LogRecord, LogSource, McpServer, MemoryLogs, MemoryMetrics, MemoryTasks,
    MetricDescriptor, MetricId, MetricPoint, Page, SourceId, StartTaskRequest, TaskDefinition,
    TaskId, TaskProvider, TaskRun, TaskState, UtcTimestamp,
};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), A2aLabError> {
    let tasks = DemoTasks::new();
    tasks
        .insert(TaskDefinition {
            id: TaskId::new("mix")?,
            name: "Mix".to_owned(),
            description: "Mix the represented plate.".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    let logs = MemoryLogs::new();
    let metrics = MemoryMetrics::new();
    seed(&logs, &metrics).await?;
    let lab = A2aLabService::new(logs, metrics, tasks).share();
    let a2a = TcpListener::bind("127.0.0.1:0").await.map_err(transport)?;
    let mcp = TcpListener::bind("127.0.0.1:0").await.map_err(transport)?;
    println!("a2a {}", a2a.local_addr().map_err(transport)?);
    println!("mcp {}", mcp.local_addr().map_err(transport)?);
    let a2a_lab = Arc::clone(&lab);
    let mcp_lab = Arc::clone(&lab);
    tokio::spawn(async move { A2aServer::new(&a2a_lab).listen(a2a).await });
    tokio::spawn(async move { McpServer::new(&mcp_lab).serve_http(mcp).await });

    let uuid =
        env::var("SILA_UUID").unwrap_or_else(|_| "11111111-1111-1111-1111-111111111111".to_owned());
    let port = env::var("SILA_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(50052);
    let host = env::var("SILA_HOST").unwrap_or_else(|_| "0.0.0.0".to_owned());
    let store = env::var("SILA_CONNECTION_STORE")
        .unwrap_or_else(|_| "/tmp/sila-connections.json".to_owned());
    let mut server = SilaServer::new(SilaIdentity::lab_dev_kit(&uuid)?, lab)
        .connection_store(store.into())
        .announce();
    if env::var("SILA_PLAIN").ok().as_deref() == Some("1") {
        server = server.plaintext().cloud_plaintext();
    } else if let Ok(directory) = env::var("SILA_WRITE_CERTS") {
        let certificate = SilaCertificate::self_signed(
            &env::var("SILA_UUID")
                .unwrap_or_else(|_| "11111111-1111-1111-1111-111111111111".to_owned()),
        )?;
        fs::create_dir_all(&directory).map_err(transport)?;
        fs::write(format!("{directory}/server.crt"), &certificate.cert_pem).map_err(transport)?;
        fs::write(format!("{directory}/server.key"), &certificate.key_pem).map_err(transport)?;
        fs::write(format!("{directory}/ca.crt"), &certificate.ca_pem).map_err(transport)?;
        server = server.certificate(certificate);
    } else {
        let certificate = match (
            env::var("SILA_CERT"),
            env::var("SILA_KEY"),
            env::var("SILA_CA"),
        ) {
            (Ok(cert), Ok(key), Ok(ca)) => SilaCertificate::from_pem(
                fs::read_to_string(cert).map_err(transport)?,
                fs::read_to_string(key).map_err(transport)?,
                fs::read_to_string(ca).map_err(transport)?,
                &uuid,
            )?,
            _ => SilaCertificate::self_signed(
                &env::var("SILA_UUID")
                    .unwrap_or_else(|_| "11111111-1111-1111-1111-111111111111".to_owned()),
            )?,
        };
        server = server.certificate(certificate);
    }
    let address = format!("{host}:{port}").parse().map_err(|_| {
        A2aLabError::invalid("sila_address", "host and port are not a socket address")
    })?;
    let handle = server.serve(address).await?;
    println!("sila {}", handle.local_addr());
    std::future::pending::<()>().await;
    Ok(())
}

async fn seed(logs: &MemoryLogs, metrics: &MemoryMetrics) -> Result<(), A2aLabError> {
    let source = SourceId::new("journal")?;
    logs.insert_source(LogSource {
        id: source.clone(),
        name: "Journal".to_owned(),
        description: "Demo log".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
    logs.insert_record(LogRecord {
        source_id: source,
        timestamp: UtcTimestamp::parse("2024-01-01T00:30:00Z")?,
        level: LogLevel::Info,
        message: "ready".to_owned(),
        attributes: JsonObject::empty(),
    })
    .await?;
    let metric = MetricId::new("temp")?;
    metrics
        .insert_metric(MetricDescriptor {
            id: metric.clone(),
            name: "Temperature".to_owned(),
            description: "Demo metric".to_owned(),
            unit: "C".to_owned(),
            asset_id: None,
            semantic_id: None,
        })
        .await;
    metrics
        .insert_point(
            metric,
            MetricPoint::new(UtcTimestamp::parse("2024-01-01T00:30:00Z")?, 21.5)?,
        )
        .await
}

#[allow(clippy::needless_pass_by_value)]
fn transport(error: std::io::Error) -> A2aLabError {
    A2aLabError::transport(error.to_string())
}

#[derive(Clone)]
struct DemoTasks {
    inner: MemoryTasks,
}

impl DemoTasks {
    fn new() -> Self {
        Self {
            inner: MemoryTasks::new(),
        }
    }

    async fn insert(&self, definition: TaskDefinition) {
        self.inner.insert(definition).await;
    }
}

impl TaskProvider for DemoTasks {
    async fn list_tasks(
        &self,
        request: ListTasksRequest,
    ) -> Result<Page<TaskDefinition>, A2aLabError> {
        self.inner.list_tasks(request).await
    }

    async fn start(&self, request: StartTaskRequest) -> Result<TaskRun, A2aLabError> {
        let complete = request.input.to_string().contains("\"complete\":true");
        let run = self.inner.start(request).await?;
        if complete {
            let tasks = self.inner.clone();
            let id = run.id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                advance(&tasks, &id, TaskState::Working, None).await;
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                advance(&tasks, &id, TaskState::Completed, Some("done".to_owned())).await;
            });
        }
        Ok(run)
    }

    async fn status(&self, request: GetTaskStatusRequest) -> Result<TaskRun, A2aLabError> {
        self.inner.status(request).await
    }

    async fn cancel(&self, request: GetTaskStatusRequest) -> Result<TaskRun, A2aLabError> {
        self.inner.cancel(request).await
    }
}

async fn advance(
    tasks: &MemoryTasks,
    id: &a2a_lab_dev_kit::RunId,
    state: TaskState,
    message: Option<String>,
) {
    let Ok(current) = tasks.status(GetTaskStatusRequest { id: id.clone() }).await else {
        return;
    };
    if current.state.is_terminal() {
        return;
    }
    let _ = tasks.transition(id, state, message).await;
}
