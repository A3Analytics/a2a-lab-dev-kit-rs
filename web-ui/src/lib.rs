//! Browser shell and same-origin A2A HTTP+JSON client.

mod card;
mod codec;
mod inventory;
mod logs;
mod metrics;
mod tasks;
mod workbench;

#[cfg(target_arch = "wasm32")]
mod shell;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    shell::start();
}
