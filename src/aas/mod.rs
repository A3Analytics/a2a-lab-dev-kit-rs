//! Outbound client for IDTA Asset Administration Shell HTTP repositories.

mod client;
mod codec;
mod token;

pub use client::AasClient;
pub use token::{AccessTokenSource, StaticToken};
