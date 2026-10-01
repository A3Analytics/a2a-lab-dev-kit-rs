//! MCP adapter for lab providers.

mod client;
mod server;

pub use client::{DEFAULT_MCP_URL, McpLab};
pub use server::McpServer;
