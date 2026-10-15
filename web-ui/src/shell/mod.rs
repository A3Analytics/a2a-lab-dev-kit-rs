//! WASM shell: DOM, fetch, and page session.

mod chat;
mod http;
mod inventory;
mod log_chrome;
mod logs;
mod metrics;
mod nav;
mod page;
mod present;
mod session;
mod task_runs;
mod tasks;
mod workbench;

pub fn start() {
    session::start();
}
