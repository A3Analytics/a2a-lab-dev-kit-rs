//! Cooperative stop signal for capture worker threads.

use tokio::sync::watch;

/// Shared shutdown handle returned from [`super::DeclarativeVideoRuntime::start`].
#[derive(Clone)]
pub struct VideoRuntimeShutdown {
    stop: watch::Sender<bool>,
}

impl VideoRuntimeShutdown {
    pub(crate) fn new(stop: watch::Sender<bool>) -> Self {
        Self { stop }
    }

    /// Signals every capture worker to exit its loop.
    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }
}
