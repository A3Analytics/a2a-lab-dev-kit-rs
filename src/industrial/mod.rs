//! Providers that resolve AAS bindings and read OPC UA or SiLA 2.

mod live;
mod logs;
mod metrics;
mod scripted;
mod tasks;

pub use live::{LiveRun, LiveSource};
pub use logs::IndustrialLogs;
pub use metrics::IndustrialMetrics;
pub use scripted::ScriptedLive;
pub use tasks::IndustrialTasks;

use std::sync::Arc;

use crate::catalog::AssetCatalogProvider;

/// Shares one catalog and live source across the three lab providers.
pub struct IndustrialLabBuilder<C, L> {
    catalog: Arc<C>,
    live: Arc<L>,
}

impl<C, L> IndustrialLabBuilder<C, L>
where
    C: AssetCatalogProvider,
    L: LiveSource,
{
    /// Stores the shared catalog and live source.
    #[must_use]
    pub fn new(catalog: C, live: L) -> Self {
        Self {
            catalog: Arc::new(catalog),
            live: Arc::new(live),
        }
    }

    /// Log provider using the shared instance.
    #[must_use]
    pub fn logs(&self) -> IndustrialLogs<C, L> {
        IndustrialLogs::new(Arc::clone(&self.catalog), Arc::clone(&self.live))
    }

    /// Metric provider using the shared instance.
    #[must_use]
    pub fn metrics(&self) -> IndustrialMetrics<C, L> {
        IndustrialMetrics::new(Arc::clone(&self.catalog), Arc::clone(&self.live))
    }

    /// Task provider using the shared instance.
    #[must_use]
    pub fn tasks(&self) -> IndustrialTasks<C, L> {
        IndustrialTasks::new(Arc::clone(&self.catalog), Arc::clone(&self.live))
    }
}
