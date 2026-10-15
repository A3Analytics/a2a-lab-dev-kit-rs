//! WASM shell: DOM, fetch, and page session.

mod chat;
mod http;
mod inventory;
mod logs;
mod metrics;
mod nav;
mod page;
mod present;
mod session;
mod workbench;

pub fn start() {
    session::start();
}
