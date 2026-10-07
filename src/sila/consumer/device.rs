//! Lab tasks backed by one connected SiLA server.

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::A2aLabError;
use crate::id::{RunId, TaskId};
use crate::json_object::JsonObject;
use crate::page::Page;
use crate::sila::consumer::model::FeatureModel;
use crate::sila::consumer::rpc::{Invoke, Poll};
use crate::sila::consumer::session::SilaSession;
use crate::tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskProvider,
    TaskRun, TaskState,
};

#[derive(Clone)]
struct Member {
    lab_id: String,
    feature: String,
    name: String,
    observable: bool,
    definition: TaskDefinition,
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
    members: Vec<Member>,
    runs: BTreeMap<String, StoredRun>,
}

/// A connected SiLA server whose commands and readable properties are lab tasks.
#[derive(Clone)]
pub struct SilaDevice {
    inner: Arc<Mutex<Inner>>,
}

/// Lists and runs the commands and readable properties of one SiLA server.
#[derive(Clone)]
pub struct SilaTasks {
    device: SilaDevice,
}

impl SilaDevice {
    /// Indexes the commands and readable properties published by `session`.
    pub fn new(session: SilaSession) -> Result<Self, A2aLabError> {
        let members = members(&session)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Inner {
                session,
                members,
                runs: BTreeMap::new(),
            })),
        })
    }

    /// Task provider for this server.
    #[must_use]
    pub fn tasks(&self) -> SilaTasks {
        SilaTasks {
            device: self.clone(),
        }
    }

    async fn start_lab(&self, lab_id: &str, input: JsonObject) -> Result<TaskRun, A2aLabError> {
        let (feature, member, observable) = {
            let inner = self.inner.lock().await;
            let member = inner
                .members
                .iter()
                .find(|member| member.lab_id == lab_id)
                .cloned()
                .ok_or_else(|| A2aLabError::not_found("task", lab_id))?;
            (member.feature, member.name, member.observable)
        };
        let run = if observable {
            self.start_observable(&feature, &member, lab_id, input)
                .await?
        } else {
            self.start_immediate(&feature, &member, lab_id, input)
                .await?
        };
        let task = task_run(&run)?;
        self.inner.lock().await.runs.insert(run.id.clone(), run);
        Ok(task)
    }

    async fn start_immediate(
        &self,
        feature: &str,
        member: &str,
        task_id: &str,
        input: JsonObject,
    ) -> Result<StoredRun, A2aLabError> {
        let kind = {
            let inner = self.inner.lock().await;
            inner.session.features().get(feature).map(|model| {
                if model
                    .property(member)
                    .is_some_and(|property| property.observable)
                {
                    "observable-property"
                } else if model.property(member).is_some() {
                    "property"
                } else {
                    "command"
                }
            })
        };
        let invoked = {
            let inner = self.inner.lock().await;
            match kind {
                Some("observable-property") => {
                    let count = usize::try_from(
                        input
                            .as_map()
                            .get("count")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(1),
                    )
                    .unwrap_or(1);
                    inner
                        .session
                        .subscribe_property(feature, member, count, &input)
                        .await
                }
                Some("property") => inner.session.read_property(feature, member, &input).await,
                _ => inner.session.call(feature, member, &input).await,
            }
        }?;
        Ok(finish_call(task_id, feature, member, input, None, invoked))
    }

    async fn start_observable(
        &self,
        feature: &str,
        member: &str,
        task_id: &str,
        input: JsonObject,
    ) -> Result<StoredRun, A2aLabError> {
        let execution = {
            let inner = self.inner.lock().await;
            inner
                .session
                .start_observable(feature, member, &input)
                .await?
        };
        let mut run = stored(execution.clone(), task_id, feature, member, input);
        run.execution = Some(execution);
        self.apply_poll(&mut run).await?;
        Ok(run)
    }

    async fn status(&self, run_id: &str) -> Result<TaskRun, A2aLabError> {
        let inner = self.inner.lock().await;
        if !inner.runs.contains_key(run_id) {
            return Err(A2aLabError::not_found("task run", run_id));
        }
        if inner
            .runs
            .get(run_id)
            .is_some_and(|run| run.execution.is_some() && !run.state.is_terminal())
        {
            let mut run = inner.runs.get(run_id).cloned().expect("run exists");
            drop(inner);
            self.apply_poll(&mut run).await?;
            let task = task_run(&run)?;
            self.inner.lock().await.runs.insert(run_id.to_owned(), run);
            return Ok(task);
        }
        let stored = inner
            .runs
            .get(run_id)
            .ok_or_else(|| A2aLabError::not_found("task run", run_id))?;
        task_run(stored)
    }

    async fn cancel(&self, run_id: &str) -> Result<TaskRun, A2aLabError> {
        let execution = {
            let inner = self.inner.lock().await;
            inner
                .runs
                .get(run_id)
                .ok_or_else(|| A2aLabError::not_found("task run", run_id))?
                .execution
                .clone()
        };
        let Some(execution) = execution else {
            return Err(A2aLabError::unavailable("task is not cancelable"));
        };
        self.inner.lock().await.session.cancel(&execution).await?;
        let mut inner = self.inner.lock().await;
        if let Some(run) = inner.runs.get_mut(run_id) {
            run.canceled = true;
        }
        drop(inner);
        self.status(run_id).await
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

impl TaskProvider for SilaTasks {
    async fn list_tasks(
        &self,
        request: ListTasksRequest,
    ) -> Result<Page<TaskDefinition>, A2aLabError> {
        let inner = self.device.inner.lock().await;
        let tasks: Vec<_> = inner
            .members
            .iter()
            .map(|member| member.definition.clone())
            .collect();
        crate::page::slice_page(&tasks, &request.page)
    }

    async fn start(&self, request: StartTaskRequest) -> Result<TaskRun, A2aLabError> {
        self.device
            .start_lab(request.task_id.as_str(), request.input)
            .await
    }

    async fn status(&self, request: GetTaskStatusRequest) -> Result<TaskRun, A2aLabError> {
        self.device.status(request.id.as_str()).await
    }

    async fn cancel(&self, request: GetTaskStatusRequest) -> Result<TaskRun, A2aLabError> {
        self.device.cancel(request.id.as_str()).await
    }
}

fn members(session: &SilaSession) -> Result<Vec<Member>, A2aLabError> {
    let mut members = Vec::new();
    let features: Vec<FeatureModel> = session.features().values().cloned().collect();
    for feature in features {
        let feature_id = feature.fqi();
        for command in &feature.commands {
            let schemas = SilaSession::command_schemas(&feature, command);
            members.push(member(
                &feature_id,
                &command.identifier,
                &command.display_name,
                &command.description,
                "command",
                command.observable,
                schemas,
                session.server_uuid(),
            )?);
        }
        for property in &feature.properties {
            let schemas = SilaSession::property_schemas(&feature, property);
            members.push(member(
                &feature_id,
                &property.identifier,
                &property.display_name,
                &property.description,
                "property",
                false,
                schemas,
                session.server_uuid(),
            )?);
        }
    }
    Ok(members)
}

fn member(
    feature: &str,
    name: &str,
    display: &str,
    description: &str,
    kind_name: &str,
    observable: bool,
    schemas: (String, String),
    server_uuid: &str,
) -> Result<Member, A2aLabError> {
    let lab_id = format!("{}:{kind_name}:{name}", feature.replace('/', ":"));
    Ok(Member {
        lab_id: lab_id.clone(),
        feature: feature.to_owned(),
        name: name.to_owned(),
        observable,
        definition: TaskDefinition {
            id: TaskId::new(lab_id)?,
            name: display.to_owned(),
            description: description.to_owned(),
            asset_id: Some(server_uuid.to_owned()),
            semantic_id: Some(feature.to_owned()),
            input_schema: Some(schemas.0),
            output_schema: Some(schemas.1),
        },
    })
}

fn finish_call(
    task_id: &str,
    feature: &str,
    member: &str,
    input: JsonObject,
    execution: Option<String>,
    result: Invoke,
) -> StoredRun {
    let id = execution.unwrap_or_else(|| format!("run-{}", Uuid::new_v4()));
    let mut run = stored(id, task_id, feature, member, input);
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
    run.progress = poll.progress;
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
            object.insert(
                "intermediate".to_owned(),
                serde_json::Value::Array(poll.intermediates),
            );
            if let Ok(merged) = JsonObject::try_from_value(serde_json::Value::Object(object)) {
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

fn stored(id: String, task_id: &str, feature: &str, member: &str, input: JsonObject) -> StoredRun {
    StoredRun {
        id,
        task_id: task_id.to_owned(),
        feature: feature.to_owned(),
        member: member.to_owned(),
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
        id: RunId::new(run.id.as_str())?,
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
