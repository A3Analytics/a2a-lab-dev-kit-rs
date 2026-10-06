//! Authentication, authorization, and the authentication test feature.

use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::error::{self, framework};
use crate::metadata::{self, CallMetadata};
use crate::shared::Shared;
use crate::support::{self, integer, string};
use crate::wire::sila2::org::silastandard::core::authenticationservice::v1::authentication_service_server::AuthenticationService;
use crate::wire::sila2::org::silastandard::core::authenticationservice::v1::{
    LoginParameters, LoginResponses, LogoutParameters, LogoutResponses,
};
use crate::wire::sila2::org::silastandard::core::authorizationservice::v1::authorization_service_server::AuthorizationService;
use crate::wire::sila2::org::silastandard::core::authorizationservice::v1::{
    GetFcpAffectedByMetadataAccessTokenParameters, GetFcpAffectedByMetadataAccessTokenResponses,
};
use crate::wire::sila2::org::silastandard::framework_error::ErrorType;
use crate::wire::sila2::org::silastandard::test::authenticationtest::v1::authentication_test_server::AuthenticationTest;
use crate::wire::sila2::org::silastandard::test::authenticationtest::v1::{
    RequiresTokenForBinaryUploadParameters, RequiresTokenForBinaryUploadResponses, RequiresTokenParameters,
    RequiresTokenResponses,
};

const AUTH_FAILED: &str =
    "org.silastandard/core/AuthenticationService/v1/DefinedExecutionError/AuthenticationFailed";
const INVALID_TOKEN: &str =
    "org.silastandard/core/AuthorizationService/v1/DefinedExecutionError/InvalidAccessToken";
const LOGOUT_INVALID: &str =
    "org.silastandard/core/AuthenticationService/v1/DefinedExecutionError/InvalidAccessToken";
const LOGIN: &str = "org.silastandard/core/AuthenticationService/v1/Command/Login/Parameter";
const UPLOAD: &str = "org.silastandard/test/AuthenticationTest/v1/Command/RequiresTokenForBinaryUpload/Parameter/BinaryToUpload";

#[derive(Clone, Copy, Default)]
pub struct Authorization;

pub struct Authentication {
    shared: Arc<Shared>,
}

pub struct Test {
    shared: Arc<Shared>,
}

impl Authentication {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

impl Test {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

#[tonic::async_trait]
impl AuthorizationService for Authorization {
    async fn get_fcp_affected_by_metadata_access_token(
        &self,
        _request: Request<GetFcpAffectedByMetadataAccessTokenParameters>,
    ) -> Result<Response<GetFcpAffectedByMetadataAccessTokenResponses>, Status> {
        Ok(Response::new(
            GetFcpAffectedByMetadataAccessTokenResponses {
                affected_calls: vec![string("org.silastandard/test/AuthenticationTest/v1")],
            },
        ))
    }
}

#[tonic::async_trait]
impl AuthenticationService for Authentication {
    async fn login(
        &self,
        request: Request<LoginParameters>,
    ) -> Result<Response<LoginResponses>, Status> {
        let parameters = request.into_inner();
        let user = require_login(&parameters.user_identification, "UserIdentification")?;
        let password = require_login(&parameters.password, "Password")?;
        let server = require_login(&parameters.requested_server, "RequestedServer")?;
        if !support::is_lowercase_server_uuid(&server) {
            return Err(error::validation(
                &format!("{LOGIN}/RequestedServer"),
                &format!(
                    "Parameter 'RequestedServer' does not match pattern '[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}', was '{server}'"
                ),
            ));
        }
        let features = parameters
            .requested_features
            .iter()
            .map(|feature| feature.value.clone())
            .collect::<Vec<_>>();
        for feature in &features {
            if !support::is_feature_identifier(feature) {
                return Err(error::validation(
                    &format!("{LOGIN}/RequestedFeatures"),
                    &format!(
                        "'Requested Feature' parameter is not a fully qualified feature identifier: {feature}"
                    ),
                ));
            }
        }
        if support::parse_uuid(&server) != Some(self.shared.server_uuid) {
            return Err(error::defined(
                AUTH_FAILED,
                &format!("Unknown server UUID: '{server}'"),
            ));
        }
        if features != ["org.silastandard/test/AuthenticationTest/v1"] {
            return Err(error::defined(
                AUTH_FAILED,
                &format!(
                    "Login is only possible for the 'Authentication Test' feature, not {}",
                    support::python_list(&features)
                ),
            ));
        }
        if user != "test" || password != "test" {
            return Err(error::defined(AUTH_FAILED, "Invalid credentials"));
        }
        let (token, lifetime) = self.shared.issue_token();
        Ok(Response::new(LoginResponses {
            access_token: Some(string(token)),
            token_lifetime: Some(integer(lifetime)),
        }))
    }

    async fn logout(
        &self,
        request: Request<LogoutParameters>,
    ) -> Result<Response<LogoutResponses>, Status> {
        let Some(token) = request.into_inner().access_token else {
            return Err(error::validation(
                "org.silastandard/core/AuthenticationService/v1/Command/Logout/Parameter/AccessToken",
                "Missing parameter 'AccessToken'",
            ));
        };
        if !self.shared.token_valid(&token.value) {
            return Err(error::defined(LOGOUT_INVALID, "Invalid access token"));
        }
        self.shared.revoke_token(&token.value);
        Ok(Response::new(LogoutResponses {}))
    }
}

#[tonic::async_trait]
impl AuthenticationTest for Test {
    async fn requires_token(
        &self,
        request: Request<RequiresTokenParameters>,
    ) -> Result<Response<RequiresTokenResponses>, Status> {
        require_live_token(&metadata::extract(request.metadata()), &self.shared)?;
        Ok(Response::new(RequiresTokenResponses {}))
    }

    async fn requires_token_for_binary_upload(
        &self,
        request: Request<RequiresTokenForBinaryUploadParameters>,
    ) -> Result<Response<RequiresTokenForBinaryUploadResponses>, Status> {
        require_live_token(&metadata::extract(request.metadata()), &self.shared)?;
        let Some(binary) = request.into_inner().binary_to_upload else {
            return Err(error::validation(
                UPLOAD,
                "Missing parameter 'Binary To Upload",
            ));
        };
        self.shared.read_binary(&binary, UPLOAD)?;
        Ok(Response::new(RequiresTokenForBinaryUploadResponses {}))
    }
}

pub fn validate_upload(parsed: &CallMetadata, shared: &Shared) -> Result<(), Status> {
    let Some(metadata) = &parsed.access_token else {
        return Err(framework(
            ErrorType::InvalidMetadata,
            "Missing metadata: 'org.silastandard/core/AuthorizationService/v1/Metadata/AccessToken'",
        ));
    };
    let token = support::text_of(metadata.access_token.as_ref());
    if shared.token_known(token) {
        Ok(())
    } else {
        Err(error::defined(INVALID_TOKEN, "Invalid access token"))
    }
}

fn require_live_token(parsed: &CallMetadata, shared: &Shared) -> Result<(), Status> {
    let Some(metadata) = &parsed.access_token else {
        return Err(framework(
            ErrorType::InvalidMetadata,
            "Missing metadata 'Access Token'",
        ));
    };
    let token = support::text_of(metadata.access_token.as_ref());
    if shared.token_valid(token) {
        Ok(())
    } else {
        Err(error::defined(INVALID_TOKEN, "Invalid access token"))
    }
}

fn require_login(
    value: &Option<crate::wire::sila2::org::silastandard::String>,
    name: &str,
) -> Result<String, Status> {
    value
        .as_ref()
        .map(|item| item.value.clone())
        .ok_or_else(|| {
            error::validation(
                &format!("{LOGIN}/{name}"),
                &format!("Missing parameter '{name}'"),
            )
        })
}
