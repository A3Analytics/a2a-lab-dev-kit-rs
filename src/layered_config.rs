//! The external `config` crate under another name.
//!
//! `config/load.rs` imports it through this module so the package name does not
//! collide with this crate's `config` module.

pub(crate) use ::config::{Config, File, FileFormat};
