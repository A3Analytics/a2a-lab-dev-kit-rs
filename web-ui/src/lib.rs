//! Browser shell and same-origin A2A HTTP+JSON client.

mod card;
mod codec;
mod images;
mod inventory;
mod logs;
mod metrics;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(dead_code)]
#[path = "shell/settings/document.rs"]
mod settings_document;
#[cfg(any(test, target_arch = "wasm32"))]
mod settings_semantic;
mod tasks;
mod videos;
mod workbench;

#[cfg(target_arch = "wasm32")]
mod shell;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    shell::start();
}
