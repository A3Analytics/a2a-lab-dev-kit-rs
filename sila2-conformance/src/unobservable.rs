//! Unobservable property and command test features.

use std::time::{SystemTime, UNIX_EPOCH};

use tonic::{Request, Response, Status};

use crate::error;
use crate::support::{integer, string};
use crate::wire::sila2::org::silastandard::test::unobservablecommandtest::v1::unobservable_command_test_server::UnobservableCommandTest;
use crate::wire::sila2::org::silastandard::test::unobservablecommandtest::v1::{
    CommandWithoutParametersAndResponsesParameters, CommandWithoutParametersAndResponsesResponses,
    ConvertIntegerToStringParameters, ConvertIntegerToStringResponses, JoinIntegerAndStringParameters,
    JoinIntegerAndStringResponses, SplitStringAfterFirstCharacterParameters, SplitStringAfterFirstCharacterResponses,
};
use crate::wire::sila2::org::silastandard::test::unobservablepropertytest::v1::unobservable_property_test_server::UnobservablePropertyTest;
use crate::wire::sila2::org::silastandard::test::unobservablepropertytest::v1::{
    GetAnswerToEverythingParameters, GetAnswerToEverythingResponses, GetSecondsSince1970Parameters,
    GetSecondsSince1970Responses,
};

#[derive(Clone, Copy, Default)]
pub struct Properties;

#[derive(Clone, Copy, Default)]
pub struct Commands;

#[tonic::async_trait]
impl UnobservablePropertyTest for Properties {
    async fn get_answer_to_everything(
        &self,
        _request: Request<GetAnswerToEverythingParameters>,
    ) -> Result<Response<GetAnswerToEverythingResponses>, Status> {
        Ok(Response::new(GetAnswerToEverythingResponses {
            answer_to_everything: Some(integer(42)),
        }))
    }

    async fn get_seconds_since1970(
        &self,
        _request: Request<GetSecondsSince1970Parameters>,
    ) -> Result<Response<GetSecondsSince1970Responses>, Status> {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        Ok(Response::new(GetSecondsSince1970Responses {
            seconds_since1970: Some(integer(i64::try_from(seconds).unwrap_or(i64::MAX))),
        }))
    }
}

#[tonic::async_trait]
impl UnobservableCommandTest for Commands {
    async fn command_without_parameters_and_responses(
        &self,
        _request: Request<CommandWithoutParametersAndResponsesParameters>,
    ) -> Result<Response<CommandWithoutParametersAndResponsesResponses>, Status> {
        Ok(Response::new(
            CommandWithoutParametersAndResponsesResponses {},
        ))
    }

    async fn convert_integer_to_string(
        &self,
        request: Request<ConvertIntegerToStringParameters>,
    ) -> Result<Response<ConvertIntegerToStringResponses>, Status> {
        let Some(value) = request.into_inner().integer else {
            return Err(error::validation(
                "org.silastandard/test/UnobservableCommandTest/v1/Command/ConvertIntegerToString/Parameter/Integer",
                "Missing parameter 'Integer'",
            ));
        };
        Ok(Response::new(ConvertIntegerToStringResponses {
            string_representation: Some(string(value.value.to_string())),
        }))
    }

    async fn join_integer_and_string(
        &self,
        request: Request<JoinIntegerAndStringParameters>,
    ) -> Result<Response<JoinIntegerAndStringResponses>, Status> {
        let parameters = request.into_inner();
        let Some(integer_value) = parameters.integer else {
            return Err(error::validation(
                "org.silastandard/test/UnobservableCommandTest/v1/Command/JoinIntegerAndString/Parameter/Integer",
                "Missing parameter 'Integer'",
            ));
        };
        let Some(string_value) = parameters.string else {
            return Err(error::validation(
                "org.silastandard/test/UnobservableCommandTest/v1/Command/JoinIntegerAndString/Parameter/String",
                "Missing parameter 'String'",
            ));
        };
        Ok(Response::new(JoinIntegerAndStringResponses {
            joined_parameters: Some(string(format!(
                "{}{}",
                integer_value.value, string_value.value
            ))),
        }))
    }

    async fn split_string_after_first_character(
        &self,
        request: Request<SplitStringAfterFirstCharacterParameters>,
    ) -> Result<Response<SplitStringAfterFirstCharacterResponses>, Status> {
        let Some(value) = request.into_inner().string else {
            return Err(error::validation(
                "org.silastandard/test/UnobservableCommandTest/v1/Command/SplitStringAfterFirstCharacter/Parameter/String",
                "Missing parameter 'String'",
            ));
        };
        let mut chars = value.value.chars();
        let first = chars
            .next()
            .map(|character| character.to_string())
            .unwrap_or_default();
        Ok(Response::new(SplitStringAfterFirstCharacterResponses {
            first_character: Some(string(first)),
            remainder: Some(string(chars.collect::<String>())),
        }))
    }
}
