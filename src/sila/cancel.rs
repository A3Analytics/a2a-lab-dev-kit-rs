//! Standard `CancelController` mapped onto lab task cancellation.

use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::error::A2aLabError;
use crate::service::A2aLabApi;
use crate::sila::constraints::execution_uuid;
use crate::sila::errors::{cancel_defined, framework, reject_metadata, undefined, validation};
use crate::sila::executions::{status_request, Executions};
use crate::sila::wire::sila2::org::silastandard::core::commands::cancelcontroller::v1::cancel_controller_server::CancelController;
use crate::sila::wire::sila2::org::silastandard::core::commands::cancelcontroller::v1::{
    CancelAllParameters, CancelAllResponses, CancelCommandParameters, CancelCommandResponses,
};
use crate::sila::wire::sila2::org::silastandard::framework_error::ErrorType;
use crate::tasks::TaskState;

const EXECUTION_PARAMETER: &str = "org.silastandard/core/commands/CancelController/v1/Command/CancelCommand/Parameter/CommandExecutionUUID";

#[derive(Clone)]
pub(crate) struct CancelFeature {
    pub lab: Arc<dyn A2aLabApi>,
    pub executions: Executions,
}

#[tonic::async_trait]
impl CancelController for CancelFeature {
    async fn cancel_command(
        &self,
        request: Request<CancelCommandParameters>,
    ) -> Result<Response<CancelCommandResponses>, Status> {
        reject_metadata(&request)?;
        let execution = execution_parameter(request.into_inner().command_execution_uuid)?;
        self.cancel_one(&execution).await?;
        Ok(Response::new(CancelCommandResponses {}))
    }

    async fn cancel_all(
        &self,
        request: Request<CancelAllParameters>,
    ) -> Result<Response<CancelAllResponses>, Status> {
        reject_metadata(&request)?;
        for (execution, _) in self.executions.active_run_ids().await {
            self.cancel_one(&execution).await?;
        }
        Ok(Response::new(CancelAllResponses {}))
    }
}

impl CancelFeature {
    async fn cancel_one(&self, execution: &str) -> Result<(), Status> {
        if !execution_uuid(execution) {
            return Err(validation(EXECUTION_PARAMETER, "must be a lowercase UUID"));
        }
        let updates = self.executions.subscribe(execution).await.map_err(|_| {
            cancel_defined(
                "InvalidCommandExecutionUUID",
                "the command execution is not running",
            )
        })?;
        if updates.borrow().state.is_terminal() {
            return Err(cancel_defined(
                "InvalidCommandExecutionUUID",
                "the command execution is not running",
            ));
        }
        let run_id = self.executions.run_id(execution).await.map_err(|_| {
            framework(
                ErrorType::InvalidCommandExecutionUuid,
                "unknown command execution",
            )
        })?;
        let request = status_request(&run_id).map_err(|error| undefined(error.to_string()))?;
        match self.lab.cancel(request).await {
            Ok(snapshot) => {
                let crate::service::A2aLabResult::StartTask(run) = snapshot.result else {
                    return Err(undefined("cancel did not return a task run"));
                };
                if run.state != TaskState::Canceled {
                    return Err(undefined("the provider did not cancel the run"));
                }
                self.executions.mark_canceled(execution, run).await;
                Ok(())
            }
            Err(A2aLabError::Unavailable { message }) if message.contains("not cancelable") => {
                Err(cancel_defined("OperationNotSupported", message))
            }
            Err(A2aLabError::NotFound { .. }) => Err(cancel_defined(
                "InvalidCommandExecutionUUID",
                "the command execution is not running",
            )),
            Err(error) => Err(undefined(error.to_string())),
        }
    }
}

fn execution_parameter(
    value: Option<
        crate::sila::wire::sila2::org::silastandard::core::commands::cancelcontroller::v1::DataTypeUuid,
    >,
) -> Result<String, Status> {
    let value = value
        .and_then(|value| value.uuid)
        .map(|value| value.value)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| validation(EXECUTION_PARAMETER, "is required"))?;
    if !execution_uuid(&value) {
        return Err(validation(EXECUTION_PARAMETER, "must be a lowercase UUID"));
    }
    Ok(value)
}
