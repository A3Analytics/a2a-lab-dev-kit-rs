//! A2A-LAB providers backed by one configured SiLA server.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Map, Value};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::A2aLabError;
use crate::id::{MetricId, RunId, SourceId, TaskId};
use crate::json_object::JsonObject;
use crate::logs::{
    ListLogSourcesRequest, LogLevel, LogProvider, LogRecord, LogSource, QueryLogsRequest,
};
use crate::metrics::{
    ListMetricsRequest, MetricDescriptor, MetricPoint, MetricProvider, QueryMetricRequest,
};
use crate::page::{Page, PageRequest};
use crate::sila::cert::certificate_matches_profile;
use crate::sila::consumer::config::{
    LogBinding, MemberKind, MetricBinding, RequestBinding, SilaBinding, SilaMember,
    SilaProviderConfig, TaskBinding,
};
use crate::sila::consumer::model::{Basic, FeatureModel, SilaType};
use crate::sila::consumer::rpc::{Invoke, Poll};
use crate::sila::consumer::select::{self, command_root, property_root};
use crate::sila::consumer::session::{SilaEndpoint, SilaSession, SilaTrust};
use crate::tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskProvider,
    TaskRun, TaskState,
};

#[derive(Clone)]
struct Call {
    feature: String,
    kind: MemberKind,
    identifier: String,
    observable: bool,
    metadata: Option<JsonObject>,
    arguments: JsonObject,
}

#[derive(Clone)]
struct BoundInput {
    pointer: String,
    timestamp: bool,
    integer: bool,
}

#[derive(Clone)]
struct QueryMap {
    start: Option<BoundInput>,
    end: Option<BoundInput>,
    cursor: Option<BoundInput>,
    limit: Option<BoundInput>,
    next_cursor: Option<String>,
}

struct PreparedTask {
    definition: TaskDefinition,
    call: Call,
}

#[derive(Clone)]
struct PreparedLog {
    source: LogSource,
    call: Call,
    query: QueryMap,
    records: String,
    timestamp: String,
    level: String,
    message: String,
    attributes: Option<String>,
}

#[derive(Clone)]
struct PreparedMetric {
    descriptor: MetricDescriptor,
    call: Call,
    query: QueryMap,
    points: String,
    timestamp: String,
    value: String,
}

#[derive(Clone)]
struct StoredRun {
    id: String,
    task_id: String,
    feature: String,
    member: String,
    execution: Option<String>,
    input: JsonObject,
    state: TaskState,
    result: Option<JsonObject>,
    progress: Option<f64>,
    message: Option<String>,
    error_kind: Option<String>,
    error_identifier: Option<String>,
    canceled: bool,
}

struct Inner {
    session: SilaSession,
    metadata: Option<JsonObject>,
    tasks: Vec<PreparedTask>,
    logs: Vec<PreparedLog>,
    metrics: Vec<PreparedMetric>,
    runs: BTreeMap<String, StoredRun>,
}

/// One remote SiLA server exposed through the three lab provider traits.
#[derive(Clone)]
pub struct SilaProvider {
    inner: Arc<Mutex<Inner>>,
}

impl SilaProvider {
    /// Connects to `config`, checks every binding against the published features, and retains the session.
    pub async fn connect(config: SilaProviderConfig) -> Result<Self, A2aLabError> {
        config.check()?;
        if !config.plaintext {
            let pem = config.ca_pem.as_deref().unwrap_or_default();
            if !certificate_matches_profile(pem, &config.server_uuid) {
                return Err(A2aLabError::invalid(
                    "ca_pem",
                    "must use CN SiLA2, a CA constraint, and the server UUID extension",
                ));
            }
        }
        let endpoint = SilaEndpoint {
            host: config.host,
            port: config.port,
            server_uuid: config.server_uuid,
        };
        let session = if config.plaintext {
            SilaSession::connect_unencrypted(&endpoint).await?
        } else {
            let trust = SilaTrust::authority_pem(config.ca_pem.unwrap_or_default());
            SilaSession::connect(&endpoint, &trust).await?
        };
        let tasks = config
            .bindings
            .iter()
            .filter_map(|binding| match binding {
                SilaBinding::Task(binding) => Some(binding),
                _ => None,
            })
            .map(|binding| prepare_task(&session, binding))
            .collect::<Result<Vec<_>, _>>()?;
        let logs = config
            .bindings
            .iter()
            .filter_map(|binding| match binding {
                SilaBinding::Logs(binding) => Some(binding),
                _ => None,
            })
            .map(|binding| prepare_log(&session, binding))
            .collect::<Result<Vec<_>, _>>()?;
        let metrics = config
            .bindings
            .iter()
            .filter_map(|binding| match binding {
                SilaBinding::Metric(binding) => Some(binding),
                _ => None,
            })
            .map(|binding| prepare_metric(&session, binding))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Inner {
                session,
                metadata: config.metadata,
                tasks,
                logs,
                metrics,
                runs: BTreeMap::new(),
            })),
        })
    }

    async fn invoke(&self, call: &Call, input: JsonObject) -> Result<JsonObject, A2aLabError> {
        let invoked = {
            let inner = self.inner.lock().await;
            let input = merge_metadata(&input, inner.metadata.as_ref(), call.metadata.as_ref())?;
            match call.kind {
                MemberKind::Property => {
                    inner
                        .session
                        .read_property(&call.feature, &call.identifier, &input)
                        .await
                }
                MemberKind::Command => {
                    inner
                        .session
                        .call(&call.feature, &call.identifier, &input)
                        .await
                }
            }
        }?;
        match invoked {
            Invoke::Done(value) => Ok(value),
            Invoke::Failed(error) => Err(A2aLabError::protocol(format!(
                "{} {}",
                error.kind, error.message
            ))),
        }
    }
}

impl TaskProvider for SilaProvider {
    async fn list_tasks(
        &self,
        request: ListTasksRequest,
    ) -> Result<Page<TaskDefinition>, A2aLabError> {
        let inner = self.inner.lock().await;
        let tasks: Vec<_> = inner
            .tasks
            .iter()
            .map(|task| task.definition.clone())
            .collect();
        crate::page::slice_page(&tasks, &request.page)
    }

    async fn start(&self, request: StartTaskRequest) -> Result<TaskRun, A2aLabError> {
        let call = {
            let inner = self.inner.lock().await;
            inner
                .tasks
                .iter()
                .find(|task| task.definition.id == request.task_id)
                .map(|task| task.call.clone())
                .ok_or_else(|| A2aLabError::not_found("task", request.task_id.to_string()))?
        };
        let input = merge_arguments(&call.arguments, &request.input)?;
        let run = if call.observable {
            self.start_observable(&call, &request.task_id, input)
                .await?
        } else {
            self.start_immediate(&call, &request.task_id, input).await?
        };
        let task = task_run(&run)?;
        self.inner.lock().await.runs.insert(run.id.clone(), run);
        Ok(task)
    }

    async fn status(&self, request: GetTaskStatusRequest) -> Result<TaskRun, A2aLabError> {
        self.refresh(request.id.as_str()).await
    }

    async fn cancel(&self, request: GetTaskStatusRequest) -> Result<TaskRun, A2aLabError> {
        let execution = {
            let inner = self.inner.lock().await;
            inner
                .runs
                .get(request.id.as_str())
                .ok_or_else(|| A2aLabError::not_found("task run", request.id.to_string()))?
                .execution
                .clone()
        };
        let Some(execution) = execution else {
            return Err(A2aLabError::unavailable("task is not cancelable"));
        };
        self.inner.lock().await.session.cancel(&execution).await?;
        if let Some(run) = self.inner.lock().await.runs.get_mut(request.id.as_str()) {
            run.canceled = true;
        }
        self.refresh(request.id.as_str()).await
    }
}

impl LogProvider for SilaProvider {
    async fn list_sources(
        &self,
        request: ListLogSourcesRequest,
    ) -> Result<Page<LogSource>, A2aLabError> {
        let inner = self.inner.lock().await;
        let sources: Vec<_> = inner.logs.iter().map(|log| log.source.clone()).collect();
        crate::page::slice_page(&sources, &request.page)
    }

    async fn query(&self, request: QueryLogsRequest) -> Result<Page<LogRecord>, A2aLabError> {
        let prepared = {
            let inner = self.inner.lock().await;
            inner
                .logs
                .iter()
                .find(|log| log.source.id == request.source_id)
                .cloned()
                .ok_or_else(|| {
                    A2aLabError::not_found("log source", request.source_id.to_string())
                })?
        };
        let input = query_input(&prepared.call.arguments, &prepared.query, &request)?;
        let response = self.invoke(&prepared.call, input).await?;
        let records = log_records(&prepared, &request, &response)?;
        page_response(
            records,
            &request.page,
            &prepared.query,
            &Value::Object(response.as_map().clone()),
        )
    }
}

impl MetricProvider for SilaProvider {
    async fn list_metrics(
        &self,
        request: ListMetricsRequest,
    ) -> Result<Page<MetricDescriptor>, A2aLabError> {
        let inner = self.inner.lock().await;
        let metrics: Vec<_> = inner
            .metrics
            .iter()
            .map(|metric| metric.descriptor.clone())
            .collect();
        crate::page::slice_page(&metrics, &request.page)
    }

    async fn query(&self, request: QueryMetricRequest) -> Result<Page<MetricPoint>, A2aLabError> {
        let prepared = {
            let inner = self.inner.lock().await;
            inner
                .metrics
                .iter()
                .find(|metric| metric.descriptor.id == request.metric_id)
                .cloned()
                .ok_or_else(|| A2aLabError::not_found("metric", request.metric_id.to_string()))?
        };
        let input = metric_input(&prepared.call.arguments, &prepared.query, &request)?;
        let response = self.invoke(&prepared.call, input).await?;
        let points = metric_points(&prepared, &request, &response)?;
        page_response(
            points,
            &request.page,
            &prepared.query,
            &Value::Object(response.as_map().clone()),
        )
    }
}

impl SilaProvider {
    async fn start_immediate(
        &self,
        call: &Call,
        task_id: &TaskId,
        input: JsonObject,
    ) -> Result<StoredRun, A2aLabError> {
        let invoked = {
            let inner = self.inner.lock().await;
            let input = merge_metadata(&input, inner.metadata.as_ref(), call.metadata.as_ref())?;
            match call.kind {
                MemberKind::Property => {
                    inner
                        .session
                        .read_property(&call.feature, &call.identifier, &input)
                        .await
                }
                MemberKind::Command => {
                    inner
                        .session
                        .call(&call.feature, &call.identifier, &input)
                        .await
                }
            }
        }?;
        Ok(finish_call(task_id, call, input, None, invoked))
    }

    async fn start_observable(
        &self,
        call: &Call,
        task_id: &TaskId,
        input: JsonObject,
    ) -> Result<StoredRun, A2aLabError> {
        let execution = {
            let inner = self.inner.lock().await;
            let input = merge_metadata(&input, inner.metadata.as_ref(), call.metadata.as_ref())?;
            inner
                .session
                .start_observable(&call.feature, &call.identifier, &input)
                .await?
        };
        let mut run = stored(execution.clone(), task_id, call, input);
        run.execution = Some(execution);
        self.apply_poll(&mut run).await?;
        Ok(run)
    }

    async fn refresh(&self, run_id: &str) -> Result<TaskRun, A2aLabError> {
        let mut run = {
            let inner = self.inner.lock().await;
            inner
                .runs
                .get(run_id)
                .cloned()
                .ok_or_else(|| A2aLabError::not_found("task run", run_id))?
        };
        if run.execution.is_some() && !run.state.is_terminal() {
            self.apply_poll(&mut run).await?;
            let task = task_run(&run)?;
            self.inner.lock().await.runs.insert(run_id.to_owned(), run);
            return Ok(task);
        }
        task_run(&run)
    }

    async fn apply_poll(&self, run: &mut StoredRun) -> Result<(), A2aLabError> {
        let Some(execution) = run.execution.clone() else {
            return Ok(());
        };
        let poll = self
            .inner
            .lock()
            .await
            .session
            .poll(&run.feature, &run.member, &execution)
            .await?;
        apply_poll(run, poll);
        Ok(())
    }
}

fn prepare_task(session: &SilaSession, binding: &TaskBinding) -> Result<PreparedTask, A2aLabError> {
    let (feature, call) = prepare_call(session, &binding.member, true)?;
    let (input_schema, output_schema) = schemas(&feature, &binding.member)?;
    Ok(PreparedTask {
        definition: TaskDefinition {
            id: TaskId::new(&binding.id)?,
            name: binding.name.clone(),
            description: binding.description.clone(),
            asset_id: binding.asset_id.clone(),
            semantic_id: binding.semantic_id.clone(),
            input_schema,
            output_schema,
        },
        call,
    })
}

fn prepare_log(session: &SilaSession, binding: &LogBinding) -> Result<PreparedLog, A2aLabError> {
    let (feature, call) = prepare_call(session, &binding.member, false)?;
    let root = response_root(&feature, &binding.member)?;
    let parameters = parameter_root(&feature, &binding.member)?;
    let records = select::require_type(&feature, &root, &binding.records, &[], true)?;
    let item = select::list_item(&feature, &records)?;
    select::require_type(
        &feature,
        &item,
        &binding.timestamp,
        &[Basic::Timestamp, Basic::String],
        false,
    )?;
    select::require_type(&feature, &item, &binding.level, &[Basic::String], false)?;
    select::require_type(&feature, &item, &binding.message, &[Basic::String], false)?;
    if let Some(pointer) = &binding.attributes {
        let selected = select::pointer_type(&feature, &item, pointer)?;
        if !select::is_structure(&feature, &selected)
            && !matches!(
                normalize_basic(&feature, &selected),
                Some(Basic::String | Basic::Any)
            )
        {
            return Err(A2aLabError::invalid(
                "attributes",
                "must select a structure or string",
            ));
        }
    }
    Ok(PreparedLog {
        source: LogSource {
            id: SourceId::new(&binding.id)?,
            name: binding.name.clone(),
            description: binding.description.clone(),
            asset_id: binding.asset_id.clone(),
            semantic_id: binding.semantic_id.clone(),
        },
        query: query_map(
            &feature,
            &parameters,
            &root,
            &binding.request,
            binding.next_cursor.clone(),
        )?,
        records: binding.records.clone(),
        timestamp: binding.timestamp.clone(),
        level: binding.level.clone(),
        message: binding.message.clone(),
        attributes: binding.attributes.clone(),
        call,
    })
}

fn prepare_metric(
    session: &SilaSession,
    binding: &MetricBinding,
) -> Result<PreparedMetric, A2aLabError> {
    let (feature, call) = prepare_call(session, &binding.member, false)?;
    let root = response_root(&feature, &binding.member)?;
    let parameters = parameter_root(&feature, &binding.member)?;
    let points = select::require_type(&feature, &root, &binding.points, &[], true)?;
    let item = select::list_item(&feature, &points)?;
    select::require_type(
        &feature,
        &item,
        &binding.timestamp,
        &[Basic::Timestamp, Basic::String],
        false,
    )?;
    select::require_type(
        &feature,
        &item,
        &binding.value,
        &[Basic::Real, Basic::Integer],
        false,
    )?;
    Ok(PreparedMetric {
        descriptor: MetricDescriptor {
            id: MetricId::new(&binding.id)?,
            name: binding.name.clone(),
            description: binding.description.clone(),
            unit: binding.unit.clone(),
            asset_id: binding.asset_id.clone(),
            semantic_id: binding.semantic_id.clone(),
        },
        query: query_map(
            &feature,
            &parameters,
            &root,
            &binding.request,
            binding.next_cursor.clone(),
        )?,
        points: binding.points.clone(),
        timestamp: binding.timestamp.clone(),
        value: binding.value.clone(),
        call,
    })
}

fn prepare_call(
    session: &SilaSession,
    member: &SilaMember,
    task: bool,
) -> Result<(FeatureModel, Call), A2aLabError> {
    let feature = session
        .features()
        .get(&member.feature)
        .cloned()
        .ok_or_else(|| A2aLabError::not_found("feature", member.feature.clone()))?;
    let observable = match member.kind {
        MemberKind::Command => feature
            .command(&member.identifier)
            .map(|command| command.observable)
            .ok_or_else(|| A2aLabError::not_found("command", member.identifier.clone()))?,
        MemberKind::Property => {
            let property = feature
                .property(&member.identifier)
                .ok_or_else(|| A2aLabError::not_found("property", member.identifier.clone()))?;
            if property.observable {
                return Err(A2aLabError::invalid(
                    "member",
                    "observable properties are not lab tasks or queries",
                ));
            }
            false
        }
    };
    if observable && !task {
        return Err(A2aLabError::invalid(
            "member",
            "log and metric queries use an unobservable command or property",
        ));
    }
    Ok((
        feature,
        Call {
            feature: member.feature.clone(),
            kind: member.kind,
            identifier: member.identifier.clone(),
            observable,
            metadata: member.metadata.clone(),
            arguments: member.arguments.clone().unwrap_or_else(JsonObject::empty),
        },
    ))
}

fn parameter_root(feature: &FeatureModel, member: &SilaMember) -> Result<SilaType, A2aLabError> {
    match member.kind {
        MemberKind::Command => {
            let command = feature
                .command(&member.identifier)
                .ok_or_else(|| A2aLabError::not_found("command", member.identifier.clone()))?;
            Ok(command_root(&command.parameters))
        }
        MemberKind::Property => Ok(command_root(&[])),
    }
}

fn response_root(feature: &FeatureModel, member: &SilaMember) -> Result<SilaType, A2aLabError> {
    match member.kind {
        MemberKind::Command => {
            let command = feature
                .command(&member.identifier)
                .ok_or_else(|| A2aLabError::not_found("command", member.identifier.clone()))?;
            Ok(command_root(&command.responses))
        }
        MemberKind::Property => {
            let property = feature
                .property(&member.identifier)
                .ok_or_else(|| A2aLabError::not_found("property", member.identifier.clone()))?;
            Ok(property_root(&property.data_type))
        }
    }
}

fn schemas(
    feature: &FeatureModel,
    member: &SilaMember,
) -> Result<(Option<String>, Option<String>), A2aLabError> {
    match member.kind {
        MemberKind::Command => {
            let command = feature
                .command(&member.identifier)
                .ok_or_else(|| A2aLabError::not_found("command", member.identifier.clone()))?;
            let (input, output) = SilaSession::command_schemas(feature, command);
            Ok((Some(input), Some(output)))
        }
        MemberKind::Property => {
            let property = feature
                .property(&member.identifier)
                .ok_or_else(|| A2aLabError::not_found("property", member.identifier.clone()))?;
            let (input, output) = SilaSession::property_schemas(feature, property);
            Ok((Some(input), Some(output)))
        }
    }
}

fn query_map(
    feature: &FeatureModel,
    parameters: &SilaType,
    response: &SilaType,
    request: &RequestBinding,
    next_cursor: Option<String>,
) -> Result<QueryMap, A2aLabError> {
    if let Some(pointer) = &next_cursor {
        select::require_type(feature, response, pointer, &[Basic::String], false)?;
    }
    Ok(QueryMap {
        start: bound_input(feature, parameters, request.start.as_deref(), true)?,
        end: bound_input(feature, parameters, request.end.as_deref(), true)?,
        cursor: bound_input(feature, parameters, request.cursor.as_deref(), false)?,
        limit: bound_input(feature, parameters, request.limit.as_deref(), false)?,
        next_cursor,
    })
}

fn bound_input(
    feature: &FeatureModel,
    root: &SilaType,
    pointer: Option<&str>,
    time: bool,
) -> Result<Option<BoundInput>, A2aLabError> {
    let Some(pointer) = pointer else {
        return Ok(None);
    };
    let selected = if time {
        select::require_type(
            feature,
            root,
            pointer,
            &[Basic::Timestamp, Basic::String],
            false,
        )?
    } else if pointer.ends_with("Limit") || pointer.ends_with("limit") {
        select::require_type(
            feature,
            root,
            pointer,
            &[Basic::Integer, Basic::String],
            false,
        )?
    } else {
        select::require_type(
            feature,
            root,
            pointer,
            &[Basic::String, Basic::Integer],
            false,
        )?
    };
    Ok(Some(BoundInput {
        pointer: pointer.to_owned(),
        timestamp: select::is_timestamp(feature, &selected),
        integer: select::is_integer(feature, &selected),
    }))
}

fn query_input(
    arguments: &JsonObject,
    query: &QueryMap,
    request: &QueryLogsRequest,
) -> Result<JsonObject, A2aLabError> {
    let mut input = arguments.clone();
    input = assign_time(input, query.start.as_ref(), request.range.start())?;
    input = assign_time(input, query.end.as_ref(), request.range.end())?;
    input = assign_cursor(input, query.cursor.as_ref(), request.page.cursor())?;
    assign_limit(input, query.limit.as_ref(), request.page.limit())
}

fn metric_input(
    arguments: &JsonObject,
    query: &QueryMap,
    request: &QueryMetricRequest,
) -> Result<JsonObject, A2aLabError> {
    let mut input = arguments.clone();
    input = assign_time(input, query.start.as_ref(), request.range.start())?;
    input = assign_time(input, query.end.as_ref(), request.range.end())?;
    input = assign_cursor(input, query.cursor.as_ref(), request.page.cursor())?;
    assign_limit(input, query.limit.as_ref(), request.page.limit())
}

fn assign_time(
    input: JsonObject,
    field: Option<&BoundInput>,
    value: crate::time::UtcTimestamp,
) -> Result<JsonObject, A2aLabError> {
    let Some(field) = field else {
        return Ok(input);
    };
    let text = if field.timestamp {
        select::sila_timestamp(value)
    } else {
        value.to_rfc3339()
    };
    select::set_pointer(&input, &field.pointer, Value::String(text))
}

fn assign_cursor(
    input: JsonObject,
    field: Option<&BoundInput>,
    cursor: Option<&str>,
) -> Result<JsonObject, A2aLabError> {
    let (Some(field), Some(cursor)) = (field, cursor) else {
        return Ok(input);
    };
    let value = if field.integer {
        let number = cursor
            .parse::<i64>()
            .map_err(|_| A2aLabError::invalid("cursor", "is not a valid page cursor"))?;
        Value::Number(number.into())
    } else {
        Value::String(cursor.to_owned())
    };
    select::set_pointer(&input, &field.pointer, value)
}

fn assign_limit(
    input: JsonObject,
    field: Option<&BoundInput>,
    limit: u32,
) -> Result<JsonObject, A2aLabError> {
    let Some(field) = field else {
        return Ok(input);
    };
    let value = if field.integer {
        Value::Number(i64::from(limit).into())
    } else {
        Value::String(limit.to_string())
    };
    select::set_pointer(&input, &field.pointer, value)
}

fn log_records(
    prepared: &PreparedLog,
    request: &QueryLogsRequest,
    response: &JsonObject,
) -> Result<Vec<LogRecord>, A2aLabError> {
    let value = Value::Object(response.as_map().clone());
    let items = select::select(&value, &prepared.records)?
        .as_array()
        .ok_or_else(|| A2aLabError::protocol("log records must be an array"))?;
    let mut records = Vec::new();
    for item in items {
        let timestamp = select::timestamp_text(select::select(item, &prepared.timestamp)?)?;
        if !request.range.contains(timestamp) {
            continue;
        }
        records.push(LogRecord {
            source_id: request.source_id.clone(),
            timestamp,
            level: log_level(select::select(item, &prepared.level)?)?,
            message: json_string(select::select(item, &prepared.message)?)?,
            attributes: log_attributes(item, prepared.attributes.as_deref())?,
        });
    }
    Ok(records)
}

fn metric_points(
    prepared: &PreparedMetric,
    request: &QueryMetricRequest,
    response: &JsonObject,
) -> Result<Vec<MetricPoint>, A2aLabError> {
    let value = Value::Object(response.as_map().clone());
    let items = select::select(&value, &prepared.points)?
        .as_array()
        .ok_or_else(|| A2aLabError::protocol("metric points must be an array"))?;
    let mut points = Vec::new();
    for item in items {
        let timestamp = select::timestamp_text(select::select(item, &prepared.timestamp)?)?;
        if !request.range.contains(timestamp) {
            continue;
        }
        let number = select::select(item, &prepared.value)?
            .as_f64()
            .ok_or_else(|| A2aLabError::protocol("metric value must be a number"))?;
        points.push(MetricPoint::new(timestamp, number)?);
    }
    Ok(points)
}

fn page_response<T: Clone>(
    items: Vec<T>,
    page: &PageRequest,
    query: &QueryMap,
    response: &Value,
) -> Result<Page<T>, A2aLabError> {
    if let Some(pointer) = &query.next_cursor {
        let cursor = select::select(response, pointer)?
            .as_str()
            .unwrap_or_default();
        let next_cursor = (!cursor.is_empty()).then(|| cursor.to_owned());
        return Ok(Page::new(items, next_cursor));
    }
    crate::page::slice_page(&items, page)
}

fn log_level(value: &Value) -> Result<LogLevel, A2aLabError> {
    match json_string(value)?.to_ascii_lowercase().as_str() {
        "trace" => Ok(LogLevel::Trace),
        "debug" => Ok(LogLevel::Debug),
        "info" | "information" => Ok(LogLevel::Info),
        "warn" | "warning" => Ok(LogLevel::Warn),
        "error" | "critical" | "fatal" => Ok(LogLevel::Error),
        _ => Err(A2aLabError::protocol("unknown log level")),
    }
}

fn log_attributes(item: &Value, pointer: Option<&str>) -> Result<JsonObject, A2aLabError> {
    let Some(pointer) = pointer else {
        return Ok(JsonObject::empty());
    };
    match select::select(item, pointer)? {
        Value::Object(map) => JsonObject::try_from_value(Value::Object(map.clone())),
        Value::String(text) if text.is_empty() => Ok(JsonObject::empty()),
        Value::String(text) => JsonObject::parse(text),
        Value::Null => Ok(JsonObject::empty()),
        _ => Err(A2aLabError::protocol("log attributes must be an object")),
    }
}

fn json_string(value: &Value) -> Result<String, A2aLabError> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| A2aLabError::protocol("expected a string"))
}

fn merge_arguments(arguments: &JsonObject, input: &JsonObject) -> Result<JsonObject, A2aLabError> {
    let mut merged = arguments.as_map().clone();
    for (key, value) in input.as_map() {
        merged.insert(key.clone(), value.clone());
    }
    JsonObject::try_from_value(Value::Object(merged))
}

fn normalize_basic(feature: &FeatureModel, data_type: &SilaType) -> Option<Basic> {
    select::basic_kind(feature, data_type)
}

fn merge_metadata(
    input: &JsonObject,
    provider: Option<&JsonObject>,
    binding: Option<&JsonObject>,
) -> Result<JsonObject, A2aLabError> {
    if provider.is_none() && binding.is_none() && !input.as_map().contains_key("metadata") {
        return Ok(input.clone());
    }
    let mut metadata = Map::new();
    if let Some(provider) = provider {
        metadata.extend(provider.as_map().clone());
    }
    if let Some(binding) = binding {
        metadata.extend(binding.as_map().clone());
    }
    if let Some(Value::Object(existing)) = input.as_map().get("metadata") {
        metadata.extend(existing.clone());
    }
    let mut object = input.as_map().clone();
    object.insert("metadata".to_owned(), Value::Object(metadata));
    JsonObject::try_from_value(Value::Object(object))
}

fn finish_call(
    task_id: &TaskId,
    call: &Call,
    input: JsonObject,
    execution: Option<String>,
    result: Invoke,
) -> StoredRun {
    let id = execution.unwrap_or_else(|| format!("run-{}", Uuid::new_v4()));
    let mut run = stored(id, task_id, call, input);
    match result {
        Invoke::Done(value) => {
            run.state = TaskState::Completed;
            run.result = Some(value);
        }
        Invoke::Failed(error) => {
            run.state = TaskState::Failed;
            run.error_kind = Some(error.kind);
            run.error_identifier = error.identifier;
            run.message = Some(error.message);
        }
    }
    run
}

fn apply_poll(run: &mut StoredRun, poll: Poll) {
    run.progress = poll.progress.filter(|value| value.is_finite());
    run.state = match poll.status {
        0 => TaskState::Submitted,
        1 => TaskState::Working,
        2 => TaskState::Completed,
        _ if run.canceled => TaskState::Canceled,
        _ => TaskState::Failed,
    };
    if let Some(mut result) = poll.result {
        if !poll.intermediates.is_empty() {
            let mut object = result.as_map().clone();
            object.insert("intermediate".to_owned(), Value::Array(poll.intermediates));
            if let Ok(merged) = JsonObject::try_from_value(Value::Object(object)) {
                result = merged;
            }
        }
        run.result = Some(result);
        run.state = TaskState::Completed;
    }
    if let Some(error) = poll.failure {
        run.error_kind = Some(error.kind);
        run.error_identifier = error.identifier;
        run.message = Some(error.message);
        if run.canceled {
            run.state = TaskState::Canceled;
        } else {
            run.state = TaskState::Failed;
        }
    }
}

fn stored(id: String, task_id: &TaskId, call: &Call, input: JsonObject) -> StoredRun {
    StoredRun {
        id,
        task_id: task_id.as_str().to_owned(),
        feature: call.feature.clone(),
        member: call.identifier.clone(),
        execution: None,
        input,
        state: TaskState::Working,
        result: None,
        progress: None,
        message: None,
        error_kind: None,
        error_identifier: None,
        canceled: false,
    }
}

fn task_run(run: &StoredRun) -> Result<TaskRun, A2aLabError> {
    Ok(TaskRun {
        id: RunId::new(&run.id)?,
        task_id: TaskId::new(&run.task_id)?,
        state: run.state,
        input: run.input.clone(),
        message: run.message.clone(),
        result: run.result.clone(),
        progress: run.progress,
        error_kind: run.error_kind.clone(),
        error_identifier: run.error_identifier.clone(),
    })
}
