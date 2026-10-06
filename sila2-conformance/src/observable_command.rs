//! ObservableCommandTest feature.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures_util::stream;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::error;
use crate::support::{self, confirmation, execution, guard, integer, RpcStream};
use crate::wire::sila2::org::silastandard::test::observablecommandtest::v1::observable_command_test_server::ObservableCommandTest;
use crate::wire::sila2::org::silastandard::test::observablecommandtest::v1::{
    CountIntermediateResponses, CountParameters, CountResponses, EchoValueAfterDelayParameters, EchoValueAfterDelayResponses,
};
use crate::wire::sila2::org::silastandard::{self as fw, execution_info::CommandStatus};

pub struct Feature {
    counts: Mutex<Vec<(Uuid, Arc<CountExec>)>>,
    echoes: Mutex<Vec<(Uuid, EchoExec)>>,
}

struct CountExec {
    target: i64,
    delay: f64,
    start: Instant,
    values: Mutex<Vec<i64>>,
}

#[derive(Clone, Copy)]
struct EchoExec {
    value: i64,
    delay: f64,
    start: Instant,
}

impl Default for Feature {
    fn default() -> Self {
        Self {
            counts: Mutex::new(Vec::new()),
            echoes: Mutex::new(Vec::new()),
        }
    }
}

#[tonic::async_trait]
impl ObservableCommandTest for Feature {
    type Count_InfoStream = RpcStream<fw::ExecutionInfo>;
    type Count_IntermediateStream = RpcStream<CountIntermediateResponses>;
    type EchoValueAfterDelay_InfoStream = RpcStream<fw::ExecutionInfo>;

    async fn count(
        &self,
        request: Request<CountParameters>,
    ) -> Result<Response<fw::CommandConfirmation>, Status> {
        let parameters = request.into_inner();
        let Some(n_value) = parameters.n else {
            return Err(missing(
                "org.silastandard/test/ObservableCommandTest/v1/Command/Count/Parameter/N",
                "Missing parameter 'N'",
            ));
        };
        let Some(delay) = parameters.delay else {
            return Err(missing(
                "org.silastandard/test/ObservableCommandTest/v1/Command/Count/Parameter/Delay",
                "Missing parameter 'Delay'",
            ));
        };
        let exec = Arc::new(CountExec {
            target: n_value.value - 1,
            delay: delay.value,
            start: Instant::now(),
            values: Mutex::new(Vec::new()),
        });
        spawn_count(Arc::clone(&exec));
        let identifier = Uuid::new_v4();
        guard(&self.counts).push((identifier, exec));
        Ok(Response::new(confirmation(identifier)))
    }

    async fn count_info(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<Self::Count_InfoStream>, Status> {
        let value = request.into_inner().value;
        let exec = self.count_exec(&value)?;
        let info_exec = Arc::clone(&exec);
        let done_exec = Arc::clone(&exec);
        Ok(Response::new(info_until_done(
            move || info_exec.info(),
            move || done_exec.done(),
        )))
    }

    async fn count_intermediate(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<Self::Count_IntermediateStream>, Status> {
        let value = request.into_inner().value;
        let exec = self.count_exec(&value)?;
        if exec.done() {
            return Ok(Response::new(Box::pin(stream::empty())));
        }
        Ok(Response::new(Box::pin(stream::unfold(
            0_usize,
            move |index| {
                let exec = Arc::clone(&exec);
                async move {
                    loop {
                        if let Some(current) = exec.values_from(index) {
                            return Some((
                                Ok(CountIntermediateResponses {
                                    current_iteration: Some(integer(current)),
                                }),
                                index + 1,
                            ));
                        }
                        if exec.done() {
                            return None;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            },
        ))))
    }

    async fn count_result(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<CountResponses>, Status> {
        let value = request.into_inner().value;
        let exec = self.count_exec(&value)?;
        if !exec.done() {
            return Err(support::not_finished());
        }
        Ok(Response::new(CountResponses {
            iteration_response: Some(integer(exec.target)),
        }))
    }

    async fn echo_value_after_delay(
        &self,
        request: Request<EchoValueAfterDelayParameters>,
    ) -> Result<Response<fw::CommandConfirmation>, Status> {
        let parameters = request.into_inner();
        let Some(value) = parameters.value else {
            return Err(missing(
                "org.silastandard/test/ObservableCommandTest/v1/Command/EchoValueAfterDelay/Parameter/Value",
                "Missing parameter 'N'",
            ));
        };
        let Some(delay) = parameters.delay else {
            return Err(missing(
                "org.silastandard/test/ObservableCommandTest/v1/Command/EchoValueAfterDelay/Parameter/Delay",
                "Missing parameter 'Delay'",
            ));
        };
        let exec = EchoExec {
            value: value.value,
            delay: delay.value,
            start: Instant::now(),
        };
        let identifier = Uuid::new_v4();
        guard(&self.echoes).push((identifier, exec));
        Ok(Response::new(confirmation(identifier)))
    }

    async fn echo_value_after_delay_info(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<Self::EchoValueAfterDelay_InfoStream>, Status> {
        let value = request.into_inner().value;
        let exec = self.echo_exec(&value)?;
        Ok(Response::new(Box::pin(stream::unfold(
            EchoPhase::Check,
            move |phase| async move { echo_info_step(exec, phase).await },
        ))))
    }

    async fn echo_value_after_delay_result(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<EchoValueAfterDelayResponses>, Status> {
        let value = request.into_inner().value;
        let exec = self.echo_exec(&value)?;
        if !exec.done() {
            return Err(support::not_finished());
        }
        Ok(Response::new(EchoValueAfterDelayResponses {
            received_value: Some(integer(exec.value)),
        }))
    }
}

impl Feature {
    fn count_exec(&self, value: &str) -> Result<Arc<CountExec>, Status> {
        let identifier = support::lookup_uuid(value)?;
        guard(&self.counts)
            .iter()
            .find(|(id, _)| *id == identifier)
            .map(|(_, exec)| Arc::clone(exec))
            .ok_or_else(|| support::unknown_execution(value))
    }

    fn echo_exec(&self, value: &str) -> Result<EchoExec, Status> {
        let identifier = support::lookup_uuid(value)?;
        guard(&self.echoes)
            .iter()
            .find(|(id, _)| *id == identifier)
            .map(|(_, exec)| *exec)
            .ok_or_else(|| support::unknown_execution(value))
    }
}

impl CountExec {
    fn done(&self) -> bool {
        let steps = self.target.saturating_add(1).max(0);
        self.start.elapsed()
            >= support::sleep_duration(
                self.delay * f64::from(u32::try_from(steps).unwrap_or(u32::MAX)),
            )
    }

    fn info(&self) -> fw::ExecutionInfo {
        let current = guard(&self.values).last().copied().unwrap_or(0);
        let progress = if self.target == 0 {
            0.0
        } else {
            i64_to_f64(current) / i64_to_f64(self.target)
        };
        let status = if self.done() {
            CommandStatus::FinishedSuccessfully
        } else {
            CommandStatus::Running
        };
        execution(status, progress, self.remaining())
    }

    fn remaining(&self) -> f64 {
        let steps = self.target.saturating_add(1).max(0);
        let total = self.delay * f64::from(u32::try_from(steps).unwrap_or(u32::MAX));
        (total - self.start.elapsed().as_secs_f64()).max(0.0)
    }

    fn values_from(&self, index: usize) -> Option<i64> {
        guard(&self.values).get(index).copied()
    }
}

impl EchoExec {
    fn remaining(&self) -> f64 {
        (self.delay - self.start.elapsed().as_secs_f64()).max(0.0)
    }

    fn done(&self) -> bool {
        self.remaining() <= 0.0
    }

    fn waiting(&self) -> fw::ExecutionInfo {
        execution(
            CommandStatus::Waiting,
            self.remaining() - self.delay,
            self.remaining(),
        )
    }

    fn running(&self) -> fw::ExecutionInfo {
        execution(
            CommandStatus::Running,
            self.remaining() - self.delay,
            self.remaining(),
        )
    }

    fn finished(&self) -> fw::ExecutionInfo {
        execution(
            CommandStatus::FinishedSuccessfully,
            self.remaining() - self.delay,
            self.remaining(),
        )
    }
}

#[derive(Clone, Copy)]
enum EchoPhase {
    Check,
    Sleep,
    Last,
    End,
}

async fn echo_info_step(
    exec: EchoExec,
    phase: EchoPhase,
) -> Option<(Result<fw::ExecutionInfo, Status>, EchoPhase)> {
    match phase {
        EchoPhase::Check if exec.done() => Some((Ok(exec.finished()), EchoPhase::End)),
        EchoPhase::Check | EchoPhase::Sleep => {
            if matches!(phase, EchoPhase::Sleep) {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            if exec.done() {
                Some((Ok(exec.running()), EchoPhase::Last))
            } else {
                Some((Ok(exec.waiting()), EchoPhase::Sleep))
            }
        }
        EchoPhase::Last => Some((Ok(exec.finished()), EchoPhase::End)),
        EchoPhase::End => None,
    }
}

fn spawn_count(exec: Arc<CountExec>) {
    tokio::spawn(async move {
        let steps = exec.target.saturating_add(1).max(0);
        let delay = support::sleep_duration(exec.delay);
        for value in 0..steps {
            guard(&exec.values).push(value);
            tokio::time::sleep(delay).await;
        }
    });
}

fn info_until_done<I, D>(info: I, done: D) -> RpcStream<fw::ExecutionInfo>
where
    I: Fn() -> fw::ExecutionInfo + Send + Sync + 'static,
    D: Fn() -> bool + Send + Sync + 'static,
{
    let info = Arc::new(info);
    let done = Arc::new(done);
    Box::pin(stream::unfold(false, move |sent_final| {
        let info = Arc::clone(&info);
        let done = Arc::clone(&done);
        async move {
            if sent_final {
                return None;
            }
            let snapshot = info();
            let finished = done();
            if !finished {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Some((Ok(snapshot), finished))
        }
    }))
}

fn missing(parameter: &str, message: &str) -> Status {
    error::validation(parameter, message)
}

#[allow(clippy::cast_precision_loss)]
fn i64_to_f64(value: i64) -> f64 {
    value as f64
}
