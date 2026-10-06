//! ErrorHandlingTest feature.

use std::collections::HashSet;
use std::sync::Mutex;

use futures_util::stream;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::error::{defined, undefined};
use crate::support::{self, confirmation, guard, integer, status_only, RpcStream};
use crate::wire::sila2::org::silastandard::test::errorhandlingtest::v1::error_handling_test_server::ErrorHandlingTest;
use crate::wire::sila2::org::silastandard::test::errorhandlingtest::v1::{
    GetRaiseDefinedExecutionErrorOnGetParameters, GetRaiseDefinedExecutionErrorOnGetResponses,
    GetRaiseUndefinedExecutionErrorOnGetParameters, GetRaiseUndefinedExecutionErrorOnGetResponses,
    RaiseDefinedExecutionErrorObservablyParameters, RaiseDefinedExecutionErrorObservablyResponses,
    RaiseDefinedExecutionErrorParameters, RaiseDefinedExecutionErrorResponses,
    RaiseUndefinedExecutionErrorObservablyParameters, RaiseUndefinedExecutionErrorObservablyResponses,
    RaiseUndefinedExecutionErrorParameters, RaiseUndefinedExecutionErrorResponses,
    SubscribeRaiseDefinedExecutionErrorAfterValueWasSentParameters,
    SubscribeRaiseDefinedExecutionErrorAfterValueWasSentResponses,
    SubscribeRaiseDefinedExecutionErrorOnSubscribeParameters, SubscribeRaiseDefinedExecutionErrorOnSubscribeResponses,
    SubscribeRaiseUndefinedExecutionErrorAfterValueWasSentParameters,
    SubscribeRaiseUndefinedExecutionErrorAfterValueWasSentResponses,
    SubscribeRaiseUndefinedExecutionErrorOnSubscribeParameters,
    SubscribeRaiseUndefinedExecutionErrorOnSubscribeResponses,
};
use crate::wire::sila2::org::silastandard::{self as fw, execution_info};

const TEST_ERROR: &str =
    "org.silastandard/test/ErrorHandlingTest/v1/DefinedExecutionError/TestError";
const TEST_MESSAGE: &str = "SiLA2_test_error_message";

pub struct Feature {
    defined_ids: Mutex<HashSet<Uuid>>,
    undefined_ids: Mutex<HashSet<Uuid>>,
}

impl Default for Feature {
    fn default() -> Self {
        Self {
            defined_ids: Mutex::new(HashSet::new()),
            undefined_ids: Mutex::new(HashSet::new()),
        }
    }
}

#[tonic::async_trait]
impl ErrorHandlingTest for Feature {
    type RaiseDefinedExecutionErrorObservably_InfoStream = RpcStream<fw::ExecutionInfo>;
    type RaiseUndefinedExecutionErrorObservably_InfoStream = RpcStream<fw::ExecutionInfo>;
    type Subscribe_RaiseDefinedExecutionErrorOnSubscribeStream =
        RpcStream<SubscribeRaiseDefinedExecutionErrorOnSubscribeResponses>;
    type Subscribe_RaiseUndefinedExecutionErrorOnSubscribeStream =
        RpcStream<SubscribeRaiseUndefinedExecutionErrorOnSubscribeResponses>;
    type Subscribe_RaiseDefinedExecutionErrorAfterValueWasSentStream =
        RpcStream<SubscribeRaiseDefinedExecutionErrorAfterValueWasSentResponses>;
    type Subscribe_RaiseUndefinedExecutionErrorAfterValueWasSentStream =
        RpcStream<SubscribeRaiseUndefinedExecutionErrorAfterValueWasSentResponses>;

    async fn raise_defined_execution_error(
        &self,
        _request: Request<RaiseDefinedExecutionErrorParameters>,
    ) -> Result<Response<RaiseDefinedExecutionErrorResponses>, Status> {
        Err(test_defined())
    }

    async fn raise_defined_execution_error_observably(
        &self,
        _request: Request<RaiseDefinedExecutionErrorObservablyParameters>,
    ) -> Result<Response<fw::CommandConfirmation>, Status> {
        Ok(Response::new(self.confirm(&self.defined_ids)))
    }

    async fn raise_defined_execution_error_observably_info(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<Self::RaiseDefinedExecutionErrorObservably_InfoStream>, Status> {
        self.known(&self.defined_ids, &request.into_inner().value)?;
        Ok(Response::new(finished_with_error()))
    }

    async fn raise_defined_execution_error_observably_result(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<RaiseDefinedExecutionErrorObservablyResponses>, Status> {
        self.known(&self.defined_ids, &request.into_inner().value)?;
        Err(test_defined())
    }

    async fn raise_undefined_execution_error(
        &self,
        _request: Request<RaiseUndefinedExecutionErrorParameters>,
    ) -> Result<Response<RaiseUndefinedExecutionErrorResponses>, Status> {
        Err(undefined(TEST_MESSAGE))
    }

    async fn raise_undefined_execution_error_observably(
        &self,
        _request: Request<RaiseUndefinedExecutionErrorObservablyParameters>,
    ) -> Result<Response<fw::CommandConfirmation>, Status> {
        Ok(Response::new(self.confirm(&self.undefined_ids)))
    }

    async fn raise_undefined_execution_error_observably_info(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<Self::RaiseUndefinedExecutionErrorObservably_InfoStream>, Status> {
        self.known(&self.undefined_ids, &request.into_inner().value)?;
        Ok(Response::new(finished_with_error()))
    }

    async fn raise_undefined_execution_error_observably_result(
        &self,
        request: Request<fw::CommandExecutionUuid>,
    ) -> Result<Response<RaiseUndefinedExecutionErrorObservablyResponses>, Status> {
        self.known(&self.undefined_ids, &request.into_inner().value)?;
        Err(undefined(TEST_MESSAGE))
    }

    async fn get_raise_defined_execution_error_on_get(
        &self,
        _request: Request<GetRaiseDefinedExecutionErrorOnGetParameters>,
    ) -> Result<Response<GetRaiseDefinedExecutionErrorOnGetResponses>, Status> {
        Err(test_defined())
    }

    async fn subscribe_raise_defined_execution_error_on_subscribe(
        &self,
        _request: Request<SubscribeRaiseDefinedExecutionErrorOnSubscribeParameters>,
    ) -> Result<Response<Self::Subscribe_RaiseDefinedExecutionErrorOnSubscribeStream>, Status> {
        Err(test_defined())
    }

    async fn get_raise_undefined_execution_error_on_get(
        &self,
        _request: Request<GetRaiseUndefinedExecutionErrorOnGetParameters>,
    ) -> Result<Response<GetRaiseUndefinedExecutionErrorOnGetResponses>, Status> {
        Err(undefined(TEST_MESSAGE))
    }

    async fn subscribe_raise_undefined_execution_error_on_subscribe(
        &self,
        _request: Request<SubscribeRaiseUndefinedExecutionErrorOnSubscribeParameters>,
    ) -> Result<Response<Self::Subscribe_RaiseUndefinedExecutionErrorOnSubscribeStream>, Status>
    {
        Err(undefined(TEST_MESSAGE))
    }

    async fn subscribe_raise_defined_execution_error_after_value_was_sent(
        &self,
        _request: Request<SubscribeRaiseDefinedExecutionErrorAfterValueWasSentParameters>,
    ) -> Result<Response<Self::Subscribe_RaiseDefinedExecutionErrorAfterValueWasSentStream>, Status>
    {
        Ok(Response::new(Box::pin(stream::iter([
            Ok(
                SubscribeRaiseDefinedExecutionErrorAfterValueWasSentResponses {
                    raise_defined_execution_error_after_value_was_sent: Some(integer(1)),
                },
            ),
            Err(test_defined()),
        ]))))
    }

    async fn subscribe_raise_undefined_execution_error_after_value_was_sent(
        &self,
        _request: Request<SubscribeRaiseUndefinedExecutionErrorAfterValueWasSentParameters>,
    ) -> Result<Response<Self::Subscribe_RaiseUndefinedExecutionErrorAfterValueWasSentStream>, Status>
    {
        Ok(Response::new(Box::pin(stream::iter([
            Ok(
                SubscribeRaiseUndefinedExecutionErrorAfterValueWasSentResponses {
                    raise_undefined_execution_error_after_value_was_sent: Some(integer(1)),
                },
            ),
            Err(undefined(TEST_MESSAGE)),
        ]))))
    }
}

impl Feature {
    fn confirm(&self, ids: &Mutex<HashSet<Uuid>>) -> fw::CommandConfirmation {
        let identifier = Uuid::new_v4();
        guard(ids).insert(identifier);
        confirmation(identifier)
    }

    fn known(&self, ids: &Mutex<HashSet<Uuid>>, value: &str) -> Result<(), Status> {
        let identifier = support::lookup_uuid(value)?;
        if guard(ids).contains(&identifier) {
            Ok(())
        } else {
            Err(support::unknown_execution(value))
        }
    }
}

fn test_defined() -> Status {
    defined(TEST_ERROR, TEST_MESSAGE)
}

fn finished_with_error() -> RpcStream<fw::ExecutionInfo> {
    Box::pin(stream::iter([Ok(status_only(
        execution_info::CommandStatus::FinishedWithError,
    ))]))
}
