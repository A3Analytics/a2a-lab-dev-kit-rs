//! Bearer tokens for the AAS HTTP API.

use crate::error::SdkError;

/// Supplies an AAS access token. An empty token means the request is anonymous.
pub trait AccessTokenSource: Send + Sync {
    /// Returns the current bearer token.
    fn bearer_token(&self) -> impl Future<Output = Result<Option<String>, SdkError>> + Send;
}

/// A token that does not change.
#[derive(Debug, Clone, Default)]
pub struct StaticToken {
    token: Option<String>,
}

impl StaticToken {
    /// Creates a static token source. `None` sends no authorization header.
    #[must_use]
    pub const fn new(token: Option<String>) -> Self {
        Self { token }
    }
}

impl AccessTokenSource for StaticToken {
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
    async fn bearer_token(&self) -> Result<Option<String>, SdkError> {
        Ok(self.token.clone())
    }
}
