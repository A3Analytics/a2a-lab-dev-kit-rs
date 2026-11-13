//! Declarative video runtime composition (DK-10).

mod camera_open;
mod config;
mod disk_job;
mod dyn_camera;
mod injections;
mod pump;
mod runtime;
mod shutdown;

pub use config::{DEFAULT_MAX_FRAME_BYTES, DvrReady, VideoRuntimeConfig};
pub use injections::VideoRuntimeInjections;
pub use runtime::DeclarativeVideoRuntime;
pub use shutdown::VideoRuntimeShutdown;
