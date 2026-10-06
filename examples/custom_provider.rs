//! Custom log provider passed to `A2aLabService`.
//!
//! `OvenLogs` implements `LogProvider`. Metrics and tasks use the in-memory providers.

use tokio::sync::Mutex;

use a2a_lab_dev_kit::{
    A2aLabCommand, A2aLabError, A2aLabResult, A2aLabService, JsonObject, ListLogSourcesRequest,
    LogLevel, LogProvider, LogRecord, LogSource, MemoryMetrics, MemoryTasks, Page, PageRequest,
    QueryLogsRequest, SourceId, UtcTimestamp,
};

struct OvenLogs {
    records: Mutex<Vec<LogRecord>>,
}

impl OvenLogs {
    fn new() -> Result<Self, A2aLabError> {
        Ok(Self {
            records: Mutex::new(vec![LogRecord {
                source_id: SourceId::new("oven")?,
                timestamp: UtcTimestamp::parse("2024-01-01T00:30:00Z")?,
                level: LogLevel::Info,
                message: "ready".to_owned(),
                attributes: JsonObject::empty(),
            }]),
        })
    }
}

impl LogProvider for OvenLogs {
    async fn list_sources(
        &self,
        request: ListLogSourcesRequest,
    ) -> Result<Page<LogSource>, A2aLabError> {
        let records = self.records.lock().await;
        let source = LogSource {
            id: records[0].source_id.clone(),
            name: "Oven".to_owned(),
            description: "Oven controller log".to_owned(),
            asset_id: None,
            semantic_id: None,
        };
        page_of(vec![source], &request.page)
    }

    async fn query(&self, request: QueryLogsRequest) -> Result<Page<LogRecord>, A2aLabError> {
        let records = self.records.lock().await;
        if !records
            .iter()
            .any(|record| record.source_id == request.source_id)
        {
            return Err(A2aLabError::not_found(
                "log source",
                request.source_id.to_string(),
            ));
        }
        let matched = records
            .iter()
            .filter(|record| {
                record.source_id == request.source_id && request.range.contains(record.timestamp)
            })
            .cloned()
            .collect();
        page_of(matched, &request.page)
    }
}

fn page_of<T>(items: Vec<T>, request: &PageRequest) -> Result<Page<T>, A2aLabError> {
    let start = match request.cursor() {
        None => 0,
        Some(cursor) => cursor
            .parse::<usize>()
            .map_err(|_| A2aLabError::invalid("cursor", "is not a valid page cursor"))?,
    };
    if start > items.len() {
        return Err(A2aLabError::invalid("cursor", "is past the end"));
    }
    let end = start
        .saturating_add(usize::try_from(request.limit()).unwrap_or(usize::MAX))
        .min(items.len());
    let next_cursor = (end < items.len()).then(|| end.to_string());
    Ok(Page::new(
        items.into_iter().skip(start).take(end - start).collect(),
        next_cursor,
    ))
}

#[tokio::main]
async fn main() -> Result<(), A2aLabError> {
    let service = A2aLabService::new(OvenLogs::new()?, MemoryMetrics::new(), MemoryTasks::new());
    let outcome = service
        .execute(A2aLabCommand::ListLogSources(ListLogSourcesRequest {
            page: PageRequest::new(None, 100)?,
        }))
        .await?;
    let A2aLabResult::ListLogSources(page) = outcome.task.result else {
        return Err(A2aLabError::protocol("expected log sources"));
    };
    println!("log source: {}", page.items()[0].id.as_str());
    Ok(())
}
