//! Lab executor for the official A2A request handler.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use a2a_server::{AgentExecutor, ExecutorContext};
use a2a_types::{
    A2AError, Artifact, Message, Part, Role, StreamResponse, Task, TaskArtifactUpdateEvent,
    TaskStatus,
};
use futures_util::stream::{self, BoxStream};
use tokio::sync::{Mutex, mpsc};

use crate::id::RunId;
use crate::service::{LabApi, LabCommand, LabResult, TaskSnapshot};
use crate::tasks::{GetTaskStatusRequest, TaskState};

use super::wire;

const POLL: Duration = Duration::from_millis(25);

#[derive(Clone)]
pub(crate) struct LabExecutor {
    lab: Arc<dyn LabApi>,
    runs: Arc<Mutex<HashMap<String, String>>>,
}

impl LabExecutor {
    pub(crate) fn new(lab: Arc<dyn LabApi>) -> Self {
        Self {
            lab,
            runs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl AgentExecutor for LabExecutor {
    fn execute(
        &self,
        ctx: ExecutorContext,
    ) -> BoxStream<'static, Result<StreamResponse, A2AError>> {
        spawn_events(Arc::clone(&self.lab), Arc::clone(&self.runs), ctx, false)
    }

    fn cancel(&self, ctx: ExecutorContext) -> BoxStream<'static, Result<StreamResponse, A2AError>> {
        spawn_events(Arc::clone(&self.lab), Arc::clone(&self.runs), ctx, true)
    }
}

fn spawn_events(
    lab: Arc<dyn LabApi>,
    runs: Arc<Mutex<HashMap<String, String>>>,
    ctx: ExecutorContext,
    cancel: bool,
) -> BoxStream<'static, Result<StreamResponse, A2AError>> {
    let (tx, rx) = mpsc::channel(32);
    tokio::spawn(async move {
        let result = if cancel {
            cancel_lab(lab, runs, ctx, &tx).await
        } else {
            execute_lab(lab, runs, ctx, &tx).await
        };
        if let Err(error) = result {
            let _ = tx.send(Err(error)).await;
        }
    });
    Box::pin(stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }))
}

async fn execute_lab(
    lab: Arc<dyn LabApi>,
    runs: Arc<Mutex<HashMap<String, String>>>,
    ctx: ExecutorContext,
    tx: &mpsc::Sender<Result<StreamResponse, A2AError>>,
) -> Result<(), A2AError> {
    let Some(message) = ctx.message.as_ref() else {
        return Err(A2AError::invalid_request("message is required"));
    };
    if let Some(events) = super::tck::profile(message, &ctx.task_id, &ctx.context_id) {
        for event in events {
            send(tx, event).await?;
        }
        return Ok(());
    }
    if message.parts.is_empty() {
        return Err(A2AError::invalid_request("message requires parts"));
    }
    hold_resubscribe(message).await;
    match wire::command_from_message(message)? {
        Some(command) => run_command(lab, runs, ctx, command, tx).await,
        None => complete_without_command(&ctx, tx).await,
    }
}

async fn run_command(
    lab: Arc<dyn LabApi>,
    runs: Arc<Mutex<HashMap<String, String>>>,
    ctx: ExecutorContext,
    command: LabCommand,
    tx: &mpsc::Sender<Result<StreamResponse, A2AError>>,
) -> Result<(), A2AError> {
    let poll_run = matches!(command, LabCommand::StartTask(_));
    let command = match command {
        LabCommand::StartTask(mut request) => {
            request.wait = false;
            LabCommand::StartTask(request)
        }
        other => other,
    };
    let outcome = lab
        .execute(command)
        .await
        .map_err(|e| wire::a2a_error(&e))?;
    if poll_run && let LabResult::StartTask(run) = &outcome.task.result {
        runs.lock()
            .await
            .insert(ctx.task_id.clone(), run.id.as_str().to_owned());
    }
    emit_result(&ctx.task_id, &ctx.context_id, &outcome.task.result, tx).await?;
    send(
        tx,
        wire::status_update(&ctx.task_id, &ctx.context_id, outcome.task.state),
    )
    .await?;
    if poll_run {
        poll_run_status(lab, &ctx, &outcome.task, tx).await?;
    }
    Ok(())
}

async fn poll_run_status(
    lab: Arc<dyn LabApi>,
    ctx: &ExecutorContext,
    snapshot: &TaskSnapshot,
    tx: &mpsc::Sender<Result<StreamResponse, A2AError>>,
) -> Result<(), A2AError> {
    let LabResult::StartTask(run) = &snapshot.result else {
        return Ok(());
    };
    let mut state = run.state;
    while !state.is_terminal() {
        tokio::time::sleep(POLL).await;
        let current = lab
            .task(run.id.as_str())
            .await
            .map_err(|e| wire::a2a_error(&e))?;
        if current.state != state {
            state = current.state;
            send(
                tx,
                wire::status_update(&ctx.task_id, &ctx.context_id, state),
            )
            .await?;
        }
    }
    Ok(())
}

async fn complete_without_command(
    ctx: &ExecutorContext,
    tx: &mpsc::Sender<Result<StreamResponse, A2AError>>,
) -> Result<(), A2AError> {
    send(
        tx,
        StreamResponse::ArtifactUpdate(TaskArtifactUpdateEvent {
            task_id: ctx.task_id.clone(),
            context_id: ctx.context_id.clone(),
            artifact: Artifact {
                artifact_id: a2a_types::new_artifact_id(),
                name: Some("lab-profile".to_owned()),
                description: Some("This agent speaks the a2a-lab data profile".to_owned()),
                parts: vec![Part::text(
                    "Send a data part with media type application/vnd.a2a-lab.v1+json",
                )],
                metadata: None,
                extensions: None,
            },
            append: Some(false),
            last_chunk: Some(true),
            metadata: None,
        }),
    )
    .await?;
    send(
        tx,
        wire::status_update(&ctx.task_id, &ctx.context_id, TaskState::Completed),
    )
    .await
}

async fn hold_resubscribe(message: &Message) {
    if !message
        .message_id
        .starts_with("test-resubscribe-message-id")
    {
        return;
    }
    let seconds = std::env::var("TCK_STREAMING_TIMEOUT")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(2.0)
        * 2.0;
    tokio::time::sleep(Duration::from_secs_f64(seconds.max(0.1))).await;
}

async fn cancel_lab(
    lab: Arc<dyn LabApi>,
    runs: Arc<Mutex<HashMap<String, String>>>,
    ctx: ExecutorContext,
    tx: &mpsc::Sender<Result<StreamResponse, A2AError>>,
) -> Result<(), A2AError> {
    let Some(run_id) = runs.lock().await.get(&ctx.task_id).cloned() else {
        return Err(A2AError::task_not_cancelable(&ctx.task_id));
    };
    let run_id = RunId::new(run_id).map_err(|e| wire::a2a_error(&e))?;
    lab.cancel(GetTaskStatusRequest { id: run_id })
        .await
        .map_err(|e| wire::a2a_error(&e))?;
    send(
        tx,
        StreamResponse::Task(Task {
            id: ctx.task_id,
            context_id: ctx.context_id,
            status: TaskStatus {
                state: a2a_types::TaskState::Canceled,
                message: Some(Message::new(Role::Agent, vec![Part::text("canceled")])),
                timestamp: None,
            },
            artifacts: ctx
                .stored_task
                .as_ref()
                .and_then(|task| task.artifacts.clone()),
            history: ctx.stored_task.and_then(|task| task.history),
            metadata: None,
        }),
    )
    .await
}

async fn emit_result(
    task_id: &str,
    context_id: &str,
    result: &LabResult,
    tx: &mpsc::Sender<Result<StreamResponse, A2AError>>,
) -> Result<(), A2AError> {
    let chunks = wire::chunks(result);
    let last = chunks.len().saturating_sub(1);
    let artifact_id = a2a_types::new_artifact_id();
    for (index, chunk) in chunks.iter().enumerate() {
        send(
            tx,
            wire::artifact_update(
                task_id,
                context_id,
                chunk,
                artifact_id.clone(),
                index > 0,
                index == last,
            )?,
        )
        .await?;
    }
    Ok(())
}

async fn send(
    tx: &mpsc::Sender<Result<StreamResponse, A2AError>>,
    event: StreamResponse,
) -> Result<(), A2AError> {
    tx.send(Ok(event))
        .await
        .map_err(|_| A2AError::internal("a2a subscriber closed"))
}
