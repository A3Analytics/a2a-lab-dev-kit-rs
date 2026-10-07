//! Dynamic SiLA Feature Consumer used as an A2A-LAB provider.

mod cloud;
mod codec;
mod compile;
mod config;
mod device;
mod fdl;
mod model;
mod provider;
mod rpc;
mod select;
mod session;

#[cfg(test)]
mod checks;

pub use config::{
    LogBinding, MemberKind, MetricBinding, RequestBinding, SilaBinding, SilaMember,
    SilaProviderConfig, TaskBinding,
};
pub use device::{SilaDevice, SilaTasks};
pub use provider::SilaProvider;
pub use session::{SILA_SERVICE, SilaEndpoint, SilaSession, SilaTrust};
