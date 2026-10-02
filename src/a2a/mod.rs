//! A2A 1.0 adapter for lab providers.
//!
//! `A2aServer` serves HTTP+JSON, JSON-RPC, and gRPC. `A2aClient` speaks HTTP+JSON.

mod access;
mod card;
mod client;
mod executor;
mod message;
mod server;
mod tck;
mod wire;

pub use client::{A2aClient, AgentMessageResponse};
pub use message::{
    AgentMessageFuture, AgentMessageHandler, AgentMessageReply, AgentMessageRequest,
};
pub use server::{A2aServer, bind_local};
pub use wire::{A2A_PROTOCOL_VERSION, LAB_MEDIA_TYPE};

pub use a2a_types::{
    AgentCard, HttpAuthSecurityScheme, ListTasksResponse, SecurityScheme, StreamResponse, Task,
    TaskPushNotificationConfig,
};
