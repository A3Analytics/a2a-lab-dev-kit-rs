//! WASM shell: DOM, fetch, and page session.

mod agent;
mod app;
mod chat;
mod http;
mod images;
mod inventory;
mod loading;
mod logs;
mod metric_chart;
mod metrics;
mod nav;
mod session;
mod tasks;
mod workbench;

pub fn start() {
    app::start();
}
