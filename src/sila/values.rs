//! SiLA basic-type and lab-structure conversions.

use jiff::civil::{Date, Time};

use crate::error::A2aLabError;
use crate::id::{MetricId, RunId, SourceId, TaskId};
use crate::json_object::JsonObject;
use crate::logs::{LogLevel, LogRecord, LogSource};
use crate::metrics::{MetricDescriptor, MetricPoint};
use crate::page::PageRequest;
use crate::sila::constraints::bounded_characters;
use crate::sila::wire::sila2::com::a3analytics::lab::laboperations::v1::{
    DataTypeLogRecord, DataTypeLogSource, DataTypeMetricDescriptor, DataTypeMetricPoint,
    DataTypePageRequest, DataTypeTaskDefinition, DataTypeTaskRun, DataTypeTimeRange,
    data_type_log_record::LogRecordStruct, data_type_log_source::LogSourceStruct,
    data_type_metric_descriptor::MetricDescriptorStruct, data_type_metric_point::MetricPointStruct,
    data_type_task_definition::TaskDefinitionStruct, data_type_task_run::TaskRunStruct,
};
use crate::sila::wire::sila2::org::silastandard::{
    Boolean, Real, String as SilaString, Timestamp, Timezone,
};
use crate::tasks::{TaskDefinition, TaskRun, TaskState};
use crate::time::{TimeRange, UtcTimestamp};

pub(crate) const JSON_LIMIT: usize = 262_144;
const LAB_FEATURE: &str = "com.a3analytics/lab/LabOperations/v1";

pub(crate) fn parameter(command: &str, name: &str) -> String {
    format!("{LAB_FEATURE}/Command/{command}/Parameter/{name}")
}

pub(crate) fn sila_string(value: impl Into<std::string::String>) -> SilaString {
    SilaString {
        value: value.into(),
    }
}

pub(crate) fn sila_bool(value: bool) -> Boolean {
    Boolean { value }
}

fn sila_real(value: f64) -> Real {
    Real { value }
}

fn text_of(value: Option<&SilaString>) -> String {
    value.map(|item| item.value.clone()).unwrap_or_default()
}

fn flag_of(value: Option<&Boolean>) -> bool {
    value.is_some_and(|item| item.value)
}

pub(crate) fn timestamp_of(value: UtcTimestamp) -> Result<Timestamp, A2aLabError> {
    let parsed = value
        .to_rfc3339()
        .parse::<jiff::Timestamp>()
        .map_err(|_| A2aLabError::invalid("timestamp", "is not a UTC timestamp"))?;
    let zoned = parsed.to_zoned(jiff::tz::TimeZone::UTC);
    Ok(Timestamp {
        second: u32::try_from(zoned.second()).unwrap_or(0),
        minute: u32::try_from(zoned.minute()).unwrap_or(0),
        hour: u32::try_from(zoned.hour()).unwrap_or(0),
        day: u32::try_from(zoned.day()).unwrap_or(1),
        month: u32::try_from(zoned.month()).unwrap_or(1),
        year: u32::try_from(zoned.year()).unwrap_or(0),
        timezone: Some(Timezone {
            hours: 0,
            minutes: 0,
        }),
        millisecond: u32::try_from(zoned.millisecond()).unwrap_or(0),
    })
}

pub(crate) fn read_timestamp(value: &Timestamp) -> Result<UtcTimestamp, A2aLabError> {
    let timezone = value.timezone.unwrap_or(Timezone {
        hours: 0,
        minutes: 0,
    });
    if timezone.hours != 0 || timezone.minutes != 0 {
        return Err(A2aLabError::invalid("timestamp", "must use a UTC offset"));
    }
    let date = Date::new(
        i16::try_from(value.year)
            .map_err(|_| A2aLabError::invalid("timestamp", "year is out of range"))?,
        i8::try_from(value.month)
            .map_err(|_| A2aLabError::invalid("timestamp", "month is out of range"))?,
        i8::try_from(value.day)
            .map_err(|_| A2aLabError::invalid("timestamp", "day is out of range"))?,
    )
    .map_err(|_| A2aLabError::invalid("timestamp", "is not a calendar date"))?;
    let time = Time::new(
        i8::try_from(value.hour).unwrap_or(i8::MAX),
        i8::try_from(value.minute).unwrap_or(i8::MAX),
        i8::try_from(value.second).unwrap_or(i8::MAX),
        i32::try_from(value.millisecond).unwrap_or(0) * 1_000_000,
    )
    .map_err(|_| A2aLabError::invalid("timestamp", "is not a time of day"))?;
    let zoned = date
        .to_datetime(time)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .map_err(|_| A2aLabError::invalid("timestamp", "is not a UTC timestamp"))?;
    UtcTimestamp::parse(&zoned.timestamp().to_string())
}

pub(crate) fn read_page(
    page: &DataTypePageRequest,
    command: &str,
) -> Result<PageRequest, A2aLabError> {
    let inner = page
        .page_request
        .as_ref()
        .ok_or_else(|| A2aLabError::invalid("page", "is required"))?;
    let _ = command;
    let cursor = if flag_of(inner.has_cursor.as_ref()) {
        let cursor = text_of(inner.cursor.as_ref());
        if cursor.is_empty() {
            return Err(A2aLabError::invalid("cursor", "is not a valid page cursor"));
        }
        Some(cursor)
    } else {
        None
    };
    let limit = inner.limit.as_ref().map_or(0, |item| item.value);
    let limit = u32::try_from(limit).unwrap_or(0);
    PageRequest::new(cursor, limit)
}

pub(crate) fn read_range(range: &DataTypeTimeRange) -> Result<TimeRange, A2aLabError> {
    let inner = range
        .time_range
        .as_ref()
        .ok_or_else(|| A2aLabError::invalid("range", "is required"))?;
    let start = inner
        .start
        .as_ref()
        .ok_or_else(|| A2aLabError::invalid("start", "is required"))
        .and_then(read_timestamp)?;
    let end = inner
        .end
        .as_ref()
        .ok_or_else(|| A2aLabError::invalid("end", "is required"))
        .and_then(read_timestamp)?;
    TimeRange::new(start, end)
}

pub(crate) fn read_json(
    value: &SilaString,
    field: &'static str,
) -> Result<JsonObject, A2aLabError> {
    if !bounded_characters(&value.value, JSON_LIMIT) {
        return Err(A2aLabError::invalid(field, "exceeds 262144 characters"));
    }
    JsonObject::parse(&value.value)
        .map_err(|_| A2aLabError::invalid(field, "must be a JSON object"))
}

fn optional_text(has: bool, value: Option<&str>) -> (Boolean, SilaString) {
    (
        sila_bool(has),
        sila_string(if has { value.unwrap_or_default() } else { "" }),
    )
}

pub(crate) fn source_message(source: &LogSource) -> DataTypeLogSource {
    let (has_asset, asset) = optional_text(source.asset_id.is_some(), source.asset_id.as_deref());
    let (has_semantic, semantic) =
        optional_text(source.semantic_id.is_some(), source.semantic_id.as_deref());
    DataTypeLogSource {
        log_source: Some(LogSourceStruct {
            id: Some(sila_string(source.id.as_str())),
            name: Some(sila_string(&source.name)),
            description: Some(sila_string(&source.description)),
            has_asset_id: Some(has_asset),
            asset_id: Some(asset),
            has_semantic_id: Some(has_semantic),
            semantic_id: Some(semantic),
        }),
    }
}

pub(crate) fn record_message(record: &LogRecord) -> Result<DataTypeLogRecord, A2aLabError> {
    Ok(DataTypeLogRecord {
        log_record: Some(LogRecordStruct {
            source_id: Some(sila_string(record.source_id.as_str())),
            timestamp: Some(timestamp_of(record.timestamp)?),
            level: Some(sila_string(level_name(record.level))),
            message: Some(sila_string(&record.message)),
            attributes: Some(bounded_json(record.attributes.to_string())?),
        }),
    })
}

pub(crate) fn metric_message(metric: &MetricDescriptor) -> DataTypeMetricDescriptor {
    let (has_asset, asset) = optional_text(metric.asset_id.is_some(), metric.asset_id.as_deref());
    let (has_semantic, semantic) =
        optional_text(metric.semantic_id.is_some(), metric.semantic_id.as_deref());
    DataTypeMetricDescriptor {
        metric_descriptor: Some(MetricDescriptorStruct {
            id: Some(sila_string(metric.id.as_str())),
            name: Some(sila_string(&metric.name)),
            description: Some(sila_string(&metric.description)),
            unit: Some(sila_string(&metric.unit)),
            has_asset_id: Some(has_asset),
            asset_id: Some(asset),
            has_semantic_id: Some(has_semantic),
            semantic_id: Some(semantic),
        }),
    }
}

pub(crate) fn point_message(point: &MetricPoint) -> Result<DataTypeMetricPoint, A2aLabError> {
    Ok(DataTypeMetricPoint {
        metric_point: Some(MetricPointStruct {
            timestamp: Some(timestamp_of(point.timestamp)?),
            value: Some(sila_real(point.value)),
        }),
    })
}

pub(crate) fn task_message(task: &TaskDefinition) -> DataTypeTaskDefinition {
    let (has_asset, asset) = optional_text(task.asset_id.is_some(), task.asset_id.as_deref());
    let (has_semantic, semantic) =
        optional_text(task.semantic_id.is_some(), task.semantic_id.as_deref());
    let (has_input, input) =
        optional_text(task.input_schema.is_some(), task.input_schema.as_deref());
    let (has_output, output) =
        optional_text(task.output_schema.is_some(), task.output_schema.as_deref());
    DataTypeTaskDefinition {
        task_definition: Some(TaskDefinitionStruct {
            id: Some(sila_string(task.id.as_str())),
            name: Some(sila_string(&task.name)),
            description: Some(sila_string(&task.description)),
            has_asset_id: Some(has_asset),
            asset_id: Some(asset),
            has_semantic_id: Some(has_semantic),
            semantic_id: Some(semantic),
            has_input_schema: Some(has_input),
            input_schema: Some(sila_string(&input.value)),
            has_output_schema: Some(has_output),
            output_schema: Some(sila_string(&output.value)),
        }),
    }
}

pub(crate) fn run_message(run: &TaskRun) -> Result<DataTypeTaskRun, A2aLabError> {
    let (has_message, message) = optional_text(run.message.is_some(), run.message.as_deref());
    let result = run.result.as_ref().map(ToString::to_string);
    let (has_result, result) = optional_text(result.is_some(), result.as_deref());
    let (has_error_kind, error_kind) =
        optional_text(run.error_kind.is_some(), run.error_kind.as_deref());
    let (has_error_identifier, error_identifier) = optional_text(
        run.error_identifier.is_some(),
        run.error_identifier.as_deref(),
    );
    Ok(DataTypeTaskRun {
        task_run: Some(TaskRunStruct {
            id: Some(sila_string(run.id.as_str())),
            task_id: Some(sila_string(run.task_id.as_str())),
            state: Some(sila_string(state_name(run.state))),
            input: Some(bounded_json(run.input.to_string())?),
            has_message: Some(has_message),
            message: Some(message),
            has_result: Some(has_result),
            result: Some(bounded_json(result.value)?),
            has_progress: Some(sila_bool(run.progress.is_some())),
            progress: Some(sila_real(run.progress.unwrap_or(0.0))),
            has_error_kind: Some(has_error_kind),
            error_kind: Some(error_kind),
            has_error_identifier: Some(has_error_identifier),
            error_identifier: Some(error_identifier),
        }),
    })
}

fn bounded_json(value: String) -> Result<SilaString, A2aLabError> {
    if !bounded_characters(&value, JSON_LIMIT) {
        return Err(A2aLabError::invalid("json", "exceeds 262144 characters"));
    }
    Ok(sila_string(value))
}

pub(crate) fn read_source_id(value: &SilaString) -> Result<SourceId, A2aLabError> {
    SourceId::new(&value.value)
}

pub(crate) fn read_metric_id(value: &SilaString) -> Result<MetricId, A2aLabError> {
    MetricId::new(&value.value)
}

pub(crate) fn read_task_id(value: &SilaString) -> Result<TaskId, A2aLabError> {
    TaskId::new(&value.value)
}

pub(crate) fn read_run_id(value: &SilaString) -> Result<RunId, A2aLabError> {
    RunId::new(&value.value)
}

pub(crate) fn state_name(state: TaskState) -> &'static str {
    match state {
        TaskState::Submitted => "submitted",
        TaskState::Working => "working",
        TaskState::Completed => "completed",
        TaskState::Failed => "failed",
        TaskState::Canceled => "canceled",
    }
}

fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Trace => "trace",
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}
