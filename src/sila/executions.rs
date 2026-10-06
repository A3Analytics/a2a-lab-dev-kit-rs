//! Observable `StartTask` executions.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{RwLock, watch};
use uuid::Uuid;

use crate::error::A2aLabError;
use crate::service::{LabApi, LabResult};
use crate::tasks::{GetTaskStatusRequest, TaskRun, TaskState};

const KEEP_AFTER_TERMINAL: Duration = Duration::from_secs(60);

#[derive(Clone, PartialEq)]
pub(crate) struct View {
    pub state: TaskState,
    pub progress: Option<f64>,
    pub run: Option<TaskRun>,
    pub failure: Option<String>,
}

impl View {
    fn from_run(run: TaskRun) -> Self {
        let progress = match run.state {
            TaskState::Submitted => Some(0.0),
            TaskState::Working => None,
            TaskState::Completed | TaskState::Failed | TaskState::Canceled => Some(1.0),
        };
        let terminal = run.state.is_terminal();
        Self {
            state: run.state,
            progress,
            run: terminal.then_some(run),
            failure: None,
        }
    }
}

struct Record {
    run_id: String,
    updates: watch::Sender<View>,
}

#[derive(Clone, Default)]
pub(crate) struct Executions {
    inner: Arc<RwLock<BTreeMap<String, Record>>>,
}

impl Executions {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn start(&self, lab: Arc<dyn LabApi>, run: TaskRun) -> String {
        let execution = Uuid::new_v4().to_string();
        let run_id = run.id.as_str().to_owned();
        let (sender, _) = watch::channel(View::from_run(run));
        self.inner.write().await.insert(
            execution.clone(),
            Record {
                run_id: run_id.clone(),
                updates: sender.clone(),
            },
        );
        let executions = self.clone();
        let execution_id = execution.clone();
        tokio::spawn(async move {
            poll_until_terminal(lab, &run_id, sender).await;
            tokio::time::sleep(KEEP_AFTER_TERMINAL).await;
            executions.inner.write().await.remove(&execution_id);
        });
        execution
    }

    pub(crate) async fn subscribe(
        &self,
        execution: &str,
    ) -> Result<watch::Receiver<View>, A2aLabError> {
        self.inner
            .read()
            .await
            .get(execution)
            .map(|record| record.updates.subscribe())
            .ok_or_else(|| A2aLabError::not_found("command execution", execution))
    }

    pub(crate) async fn run_id(&self, execution: &str) -> Result<String, A2aLabError> {
        self.inner
            .read()
            .await
            .get(execution)
            .map(|record| record.run_id.clone())
            .ok_or_else(|| A2aLabError::not_found("command execution", execution))
    }

    pub(crate) async fn active_run_ids(&self) -> Vec<(String, String)> {
        let records = self.inner.read().await;
        records
            .iter()
            .filter(|(_, record)| !record.updates.borrow().state.is_terminal())
            .map(|(execution, record)| (execution.clone(), record.run_id.clone()))
            .collect()
    }

    pub(crate) async fn mark_canceled(&self, execution: &str, run: TaskRun) {
        let Some(record) = self.inner.read().await.get(execution).cloned_sender() else {
            return;
        };
        record.send_modify(|view| {
            view.state = TaskState::Canceled;
            view.progress = Some(1.0);
            view.run = Some(run.clone());
            view.failure = Some("canceled".to_owned());
        });
    }
}

trait SenderLookup {
    fn cloned_sender(&self) -> Option<watch::Sender<View>>;
}

impl SenderLookup for Option<&Record> {
    fn cloned_sender(&self) -> Option<watch::Sender<View>> {
        self.map(|record| record.updates.clone())
    }
}

fn publish(sender: &watch::Sender<View>, view: &View) {
    sender.send_modify(|current| {
        if current.state == TaskState::Canceled {
            return;
        }
        *current = view.clone();
    });
}

async fn poll_until_terminal(lab: Arc<dyn LabApi>, run_id: &str, sender: watch::Sender<View>) {
    loop {
        if sender.borrow().state.is_terminal() {
            return;
        }
        match lab.task(run_id).await {
            Ok(snapshot) => {
                let LabResult::StartTask(run) = snapshot.result else {
                    publish(
                        &sender,
                        &View {
                            state: TaskState::Failed,
                            progress: Some(1.0),
                            run: None,
                            failure: Some("task snapshot was not a run".to_owned()),
                        },
                    );
                    return;
                };
                let terminal = run.state.is_terminal();
                publish(&sender, &View::from_run(run));
                if terminal || sender.borrow().state == TaskState::Canceled {
                    return;
                }
            }
            Err(error) => {
                publish(
                    &sender,
                    &View {
                        state: TaskState::Failed,
                        progress: Some(1.0),
                        run: None,
                        failure: Some(error.to_string()),
                    },
                );
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

pub(crate) fn status_request(run_id: &str) -> Result<GetTaskStatusRequest, A2aLabError> {
    Ok(GetTaskStatusRequest {
        id: crate::id::RunId::new(run_id)?,
    })
}
