//! MultiClientTest feature.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures_util::stream;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::error::{self, framework};
use crate::support::{
    self, RpcStream, confirmation, duration_from_seconds, execution, guard, real,
};
use crate::wire::sila2::org::silastandard::framework_error::ErrorType;
use crate::wire::sila2::org::silastandard::test::multiclienttest::v1::multi_client_test_server::MultiClientTest;
use crate::wire::sila2::org::silastandard::test::multiclienttest::v1::{
    RejectParallelExecutionParameters, RejectParallelExecutionResponses, RunInParallelParameters,
    RunInParallelResponses, RunQueuedParameters, RunQueuedResponses,
};
use crate::wire::sila2::org::silastandard::{self as fw, execution_info::CommandStatus};

#[derive(Clone, Copy)]
struct Period {
    start: Instant,
    end: Instant,
}

pub struct Feature {
    parallel: Mutex<Vec<(Uuid, Period)>>,
    queued: Mutex<Vec<(Uuid, Period)>>,
    queue: Mutex<Vec<Period>>,
    reject_end: Mutex<Instant>,
    rejected: Mutex<Vec<(Uuid, Period)>>,
    queue_lock: Mutex<()>,
    reject_lock: Mutex<()>,
}

impl Default for Feature {
    fn default() -> Self {
        Self {
            parallel: Mutex::new(Vec::new()),
            queued: Mutex::new(Vec::new()),
            queue: Mutex::new(Vec::new()),
            reject_end: Mutex::new(Instant::now()),
            rejected: Mutex::new(Vec::new()),
            queue_lock: Mutex::new(()),
            reject_lock: Mutex::new(()),
        }
    }
}

#[tonic::async_trait]
impl MultiClientTest for Feature {
    type RunInParallel_InfoStream = RpcStream<fw::ExecutionInfo>;
    type RunQueued_InfoStream = RpcStream<fw::ExecutionInfo>;
    type RejectParallelExecution_InfoStream = RpcStream<fw::ExecutionInfo>;

    async fn run_in_parallel(
        &self,
        request: Request<RunInParallelParameters>,
    ) -> Result<Response<fw::CommandConfirmation>, Status> {
        let duration = require_duration(request.into_inner().duration, "RunInParallel")?;
        let now = Instant::now();
        Ok(Response::new(self.store(
            &self.parallel,
            Period {
                start: now,
                end: now + duration,
            },
        )))
    }

    async fn run_in_parallel_info(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<Self::RunInParallel_InfoStream>, Status> {
        let value = request.into_inner().value;
        let period = self.find(&self.parallel, &value)?;
        Ok(Response::new(parallel_info(period)))
    }

    async fn run_in_parallel_result(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<RunInParallelResponses>, Status> {
        let value = request.into_inner().value;
        finished(&self.find(&self.parallel, &value)?)?;
        Ok(Response::new(RunInParallelResponses {}))
    }

    async fn run_queued(
        &self,
        request: Request<RunQueuedParameters>,
    ) -> Result<Response<fw::CommandConfirmation>, Status> {
        let duration = require_duration(request.into_inner().duration, "RunQueued")?;
        let _registration = guard(&self.queue_lock);
        let mut start = Instant::now();
        if let Some(last) = guard(&self.queue).last().copied()
            && !last.is_done()
        {
            start = last.end;
        }
        let period = Period {
            start,
            end: start + duration,
        };
        guard(&self.queue).push(period);
        Ok(Response::new(self.store(&self.queued, period)))
    }

    async fn run_queued_info(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<Self::RunQueued_InfoStream>, Status> {
        let value = request.into_inner().value;
        let period = self.find(&self.queued, &value)?;
        Ok(Response::new(queued_info(period)))
    }

    async fn run_queued_result(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<RunQueuedResponses>, Status> {
        let value = request.into_inner().value;
        finished(&self.find(&self.queued, &value)?)?;
        Ok(Response::new(RunQueuedResponses {}))
    }

    async fn reject_parallel_execution(
        &self,
        request: Request<RejectParallelExecutionParameters>,
    ) -> Result<Response<fw::CommandConfirmation>, Status> {
        let duration = require_duration(request.into_inner().duration, "RejectParallelExecution")?;
        let _registration = guard(&self.reject_lock);
        let now = Instant::now();
        if *guard(&self.reject_end) > now {
            return Err(framework(
                ErrorType::CommandExecutionNotAccepted,
                "Another instance of this command is already running",
            ));
        }
        let end = now + duration;
        *guard(&self.reject_end) = end;
        Ok(Response::new(
            self.store(&self.rejected, Period { start: now, end }),
        ))
    }

    async fn reject_parallel_execution_info(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<Self::RejectParallelExecution_InfoStream>, Status> {
        let value = request.into_inner().value;
        // The reference server checks the queued-command map before reading the reject-parallel instance.
        self.find(&self.queued, &value)?;
        let period = self.find(&self.rejected, &value)?;
        Ok(Response::new(parallel_info(period)))
    }

    async fn reject_parallel_execution_result(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<RejectParallelExecutionResponses>, Status> {
        let value = request.into_inner().value;
        finished(&self.find(&self.rejected, &value)?)?;
        Ok(Response::new(RejectParallelExecutionResponses {}))
    }
}

impl Feature {
    fn store(
        &self,
        commands: &Mutex<Vec<(Uuid, Period)>>,
        period: Period,
    ) -> fw::CommandConfirmation {
        let identifier = Uuid::new_v4();
        guard(commands).push((identifier, period));
        confirmation(identifier)
    }

    fn find(&self, commands: &Mutex<Vec<(Uuid, Period)>>, value: &str) -> Result<Period, Status> {
        let identifier = support::lookup_uuid(value)?;
        guard(commands)
            .iter()
            .find(|(id, _)| *id == identifier)
            .map(|(_, period)| *period)
            .ok_or_else(|| support::unknown_execution(value))
    }
}

impl Period {
    fn is_running(self) -> bool {
        let now = Instant::now();
        self.start < now && now < self.end
    }

    fn is_waiting(self) -> bool {
        self.start > Instant::now()
    }

    fn is_done(self) -> bool {
        Instant::now() > self.end
    }

    fn remaining(self) -> f64 {
        if self.is_done() {
            0.0
        } else {
            self.end
                .saturating_duration_since(Instant::now())
                .as_secs_f64()
        }
    }
}

fn parallel_info(period: Period) -> RpcStream<fw::ExecutionInfo> {
    Box::pin(stream::unfold(InfoPhase::Start, move |phase| async move {
        step_parallel(period, phase).await
    }))
}

fn queued_info(period: Period) -> RpcStream<fw::ExecutionInfo> {
    Box::pin(stream::unfold(InfoPhase::Start, move |phase| async move {
        step_queued(period, phase).await
    }))
}

#[derive(Clone, Copy)]
enum InfoPhase {
    Start,
    Wait,
    Running,
    Pause,
    Done,
}

async fn step_parallel(
    period: Period,
    phase: InfoPhase,
) -> Option<(Result<fw::ExecutionInfo, Status>, InfoPhase)> {
    match phase {
        InfoPhase::Start if period.start > Instant::now() => {
            Some((Ok(waiting(period)), InfoPhase::Running))
        }
        InfoPhase::Start | InfoPhase::Running if period.is_running() => {
            Some((Ok(running(period)), InfoPhase::Pause))
        }
        InfoPhase::Pause => {
            tokio::time::sleep(Duration::from_secs_f64(period.remaining().min(1.0))).await;
            next_after_pause(period)
        }
        InfoPhase::Done => None,
        _ => Some((Ok(finished_info(period)), InfoPhase::Done)),
    }
}

async fn step_queued(
    period: Period,
    phase: InfoPhase,
) -> Option<(Result<fw::ExecutionInfo, Status>, InfoPhase)> {
    match phase {
        InfoPhase::Start if period.is_waiting() => Some((Ok(waiting(period)), InfoPhase::Wait)),
        InfoPhase::Wait => {
            tokio::time::sleep(period.start.saturating_duration_since(Instant::now())).await;
            Some((Ok(running(period)), InfoPhase::Pause))
        }
        InfoPhase::Start | InfoPhase::Running if period.is_running() => {
            Some((Ok(running(period)), InfoPhase::Pause))
        }
        InfoPhase::Pause => {
            tokio::time::sleep(Duration::from_secs_f64(period.remaining().min(1.0))).await;
            next_after_pause(period)
        }
        InfoPhase::Done => None,
        _ => Some((Ok(finished_info(period)), InfoPhase::Done)),
    }
}

fn next_after_pause(period: Period) -> Option<(Result<fw::ExecutionInfo, Status>, InfoPhase)> {
    if period.is_running() {
        Some((Ok(running(period)), InfoPhase::Pause))
    } else {
        Some((Ok(finished_info(period)), InfoPhase::Done))
    }
}

fn waiting(period: Period) -> fw::ExecutionInfo {
    fw::ExecutionInfo {
        command_status: CommandStatus::Waiting as i32,
        progress_info: Some(real(0.0)),
        estimated_remaining_time: Some(duration_from_seconds(
            period
                .end
                .saturating_duration_since(Instant::now())
                .as_secs_f64(),
        )),
        updated_lifetime_of_execution: None,
    }
}

fn running(period: Period) -> fw::ExecutionInfo {
    fw::ExecutionInfo {
        command_status: CommandStatus::Running as i32,
        progress_info: None,
        estimated_remaining_time: Some(duration_from_seconds(period.remaining())),
        updated_lifetime_of_execution: None,
    }
}

fn finished_info(period: Period) -> fw::ExecutionInfo {
    execution(CommandStatus::FinishedSuccessfully, 1.0, period.remaining())
}

fn finished(period: &Period) -> Result<(), Status> {
    if period.is_done() {
        Ok(())
    } else {
        Err(support::not_finished())
    }
}

fn require_duration(value: Option<fw::Real>, command: &str) -> Result<Duration, Status> {
    let Some(duration) = value else {
        return Err(error::validation(
            &format!(
                "org.silastandard/test/MultiClientTest/v1/Command/{command}/Parameter/Duration"
            ),
            "Missing parameter 'Duration'",
        ));
    };
    Ok(support::sleep_duration(duration.value))
}
