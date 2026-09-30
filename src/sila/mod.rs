//! Generated SiLA 2 client for pinned lab features.

mod client;

pub mod proto {
    #![allow(clippy::all, clippy::pedantic, clippy::nursery)]
    include!(concat!(env!("OUT_DIR"), "/sila.lab.rs"));
}

pub use client::{
    MAX_CHUNK, SilaEndpoint, certificate_accepted, chunk_binary, execution_state, parse_discovery,
    start_task,
};
