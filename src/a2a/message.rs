//! Plain-text agent messages handled outside the lab data-part profile.

use std::future::Future;
use std::pin::Pin;

use crate::error::A2aLabError;

/// Future returned by [`AgentMessageHandler`].
pub type AgentMessageFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One plain-text turn for an agent message handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMessageRequest {
    /// Concatenated text parts from the A2A message.
    pub text: String,
    /// Server-authoritative conversation identifier.
    pub context_id: String,
    /// Protocol task created for this turn.
    pub task_id: String,
    /// Earlier tasks this turn references.
    pub reference_task_ids: Vec<String>,
}

/// Text produced for one agent-message turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMessageReply {
    /// Assistant text returned as a `text/plain` artifact.
    pub text: String,
}

/// Handles a plain-text A2A message.
pub trait AgentMessageHandler: Send + Sync {
    /// Answers `request` using the conversation identified by its context.
    fn handle(
        &self,
        request: AgentMessageRequest,
    ) -> AgentMessageFuture<'_, Result<AgentMessageReply, A2aLabError>>;
}
