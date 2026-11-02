//! WASM shell: DOM, fetch, and page session.

mod app;
mod chat;
mod hls_playback;
mod http;
mod images;
mod inventory;
mod loading;
mod logs;
mod metric_chart;
mod metrics;
mod nav;
mod playback;
mod session;
mod tasks;
mod videos;
mod workbench;

pub fn start() {
    app::start();
}
