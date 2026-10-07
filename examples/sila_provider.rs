//! Configures a remote `SiLA` server as an A2A-LAB provider.
//!
//! This example serves a plaintext lab and reads one of its log commands through `SilaProvider`.

use a2a_lab_dev_kit::sila::{
    LogBinding, MemberKind, RequestBinding, SilaBinding, SilaIdentity, SilaMember, SilaProvider,
    SilaProviderConfig, SilaServer, TaskBinding,
};
use a2a_lab_dev_kit::{
    A2aLabError, A2aLabService, JsonObject, LogLevel, LogProvider, LogRecord, LogSource,
    MemoryLogs, MemoryMetrics, MemoryTasks, PageRequest, QueryLogsRequest, SourceId, TimeRange,
    UtcTimestamp,
};

const UUID: &str = "11111111-1111-1111-1111-111111111111";

#[tokio::main]
async fn main() -> Result<(), A2aLabError> {
    let logs = MemoryLogs::new();
    logs.insert_source(LogSource {
        id: SourceId::new("oven").unwrap(),
        name: "Oven".to_owned(),
        description: "Oven log".to_owned(),
        asset_id: None,
        semantic_id: None,
    })
    .await;
    logs.insert_record(LogRecord {
        source_id: SourceId::new("oven").unwrap(),
        timestamp: UtcTimestamp::parse("2024-01-01T00:30:00Z")?,
        level: LogLevel::Info,
        message: "ready".to_owned(),
        attributes: JsonObject::empty(),
    })
    .await?;
    let remote = A2aLabService::new(logs, MemoryMetrics::new(), MemoryTasks::new()).share();
    let server = SilaServer::new(SilaIdentity::lab_dev_kit(UUID)?, remote)
        .plaintext()
        .serve("127.0.0.1:0".parse().unwrap())
        .await?;
    let provider = SilaProvider::connect(config(server.local_addr().port())).await?;
    let page = LogProvider::query(
        &provider,
        QueryLogsRequest {
            source_id: SourceId::new("events")?,
            range: TimeRange::new(
                UtcTimestamp::parse("2024-01-01T00:00:00Z")?,
                UtcTimestamp::parse("2024-01-01T01:00:00Z")?,
            )?,
            page: PageRequest::new(None, 10)?,
        },
    )
    .await?;
    println!("log record: {}", page.items()[0].message);
    Ok(())
}

fn config(port: u16) -> SilaProviderConfig {
    SilaProviderConfig {
        host: "127.0.0.1".to_owned(),
        port,
        server_uuid: UUID.to_owned(),
        plaintext: true,
        ca_pem: None,
        metadata: None,
        bindings: vec![
            SilaBinding::Task(TaskBinding {
                id: "server-name".to_owned(),
                name: "Server name".to_owned(),
                description: "Read the SiLA server name".to_owned(),
                asset_id: None,
                semantic_id: None,
                member: SilaMember {
                    feature: "org.silastandard/core/SiLAService/v1".to_owned(),
                    kind: MemberKind::Property,
                    identifier: "ServerName".to_owned(),
                    metadata: None,
                    arguments: None,
                },
            }),
            SilaBinding::Logs(LogBinding {
                id: "events".to_owned(),
                name: "Events".to_owned(),
                description: "Remote oven log".to_owned(),
                asset_id: None,
                semantic_id: None,
                member: SilaMember {
                    feature: "com.a3analytics/lab/LabOperations/v1".to_owned(),
                    kind: MemberKind::Command,
                    identifier: "QueryLogs".to_owned(),
                    metadata: None,
                    arguments: Some(
                        JsonObject::parse(
                            r#"{"SourceId":"oven","Range":{},"Page":{"HasCursor":false,"Cursor":"","Limit":10}}"#,
                        )
                        .unwrap(),
                    ),
                },
                request: RequestBinding {
                    start: Some("/Range/Start".to_owned()),
                    end: Some("/Range/End".to_owned()),
                    cursor: None,
                    limit: None,
                },
                records: "/Records".to_owned(),
                timestamp: "/Timestamp".to_owned(),
                level: "/Level".to_owned(),
                message: "/Message".to_owned(),
                attributes: Some("/Attributes".to_owned()),
                next_cursor: None,
            }),
        ],
    }
}
