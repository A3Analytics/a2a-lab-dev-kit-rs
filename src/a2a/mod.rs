//! A2A 1.0 adapter for lab providers.
//!
//! `A2aServer` serves HTTP+JSON, JSON-RPC, and gRPC. `A2aClient` speaks HTTP+JSON.

mod card;
mod client;
mod executor;
mod server;
mod tck;
mod wire;

pub use client::A2aClient;
pub use server::{A2aServer, bind_local};
pub use wire::{A2A_PROTOCOL_VERSION, LAB_MEDIA_TYPE};

pub use a2a_types::{
    AgentCard, HttpAuthSecurityScheme, ListTasksResponse, SecurityScheme, StreamResponse, Task,
    TaskPushNotificationConfig,
};
