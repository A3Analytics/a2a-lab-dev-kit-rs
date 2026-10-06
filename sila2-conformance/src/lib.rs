//! Communication-tester Feature Provider. This binary is not the lab Feature Provider.

mod any_type;
mod any_xml;
mod app;
mod auth;
mod basic;
mod binary;
mod error;
mod error_handling;
mod lists;
mod metadata;
mod metadata_test;
mod multi_client;
mod observable_command;
mod observable_property;
mod shared;
mod sila_service;
mod structures;
mod support;
mod unobservable;
mod wire;

pub use app::serve;
