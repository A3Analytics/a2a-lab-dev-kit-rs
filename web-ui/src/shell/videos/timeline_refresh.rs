//! Periodic timeline refresh while the browse panel is active.

use std::cell::Cell;

use dioxus::prelude::*;
use wasm_bindgen::JsCast;

use crate::shell::session::{self, calls_allowed, take_secret};
use crate::videos::Timeline;

use super::video_tabs::browse_streaming_allowed;
use super::{bound, run};

thread_local! {
    static REFRESH: Cell<Option<i32>> = const { Cell::new(None) };
}

pub(super) fn start_coverage_refresh() {
    REFRESH.with(|slot| {
        if slot.get().is_some() {
            return;
        }
        let closure = wasm_bindgen::closure::Closure::wrap(Box::new(|| {
            if browse_streaming_allowed() {
                fetch_timeline(|_| {});
            }
        }) as Box<dyn FnMut()>);
        let id = web_sys::window().and_then(|window| {
            window
                .set_interval_with_callback_and_timeout_and_arguments_0(
                    closure.as_ref().unchecked_ref(),
                    2000,
                )
                .ok()
        });
        closure.forget();
        slot.set(id);
    });
}

pub(super) fn stop_coverage_refresh() {
    REFRESH.with(|slot| {
        let Some(id) = slot.get() else {
            return;
        };
        if let Some(window) = web_sys::window() {
            window.clear_interval_with_handle(id);
        }
        slot.set(None);
    });
}

pub(super) fn fetch_timeline(done: impl FnOnce(Option<Timeline>) + 'static) {
    let Some(ui) = bound() else {
        done(None);
        return;
    };
    let source_id = ui.peek().reader.source_id.clone();
    if source_id.is_empty() || !calls_allowed() || take_secret(&[source_id.as_str()]).is_err() {
        done(None);
        return;
    }
    wasm_bindgen_futures::spawn_local(async move {
        let secret = session::native_secret();
        let timeline = request_timeline(&source_id, secret).await;
        run(move || {
            if browse_streaming_allowed()
                && let Some(timeline) = timeline.clone()
                && let Some(mut ui) = bound()
            {
                let mut next = ui.peek().clone();
                if next.reader.install_timeline(timeline) {
                    ui.set(next);
                }
            }
            done(timeline);
        });
    });
}

async fn request_timeline(source_id: &str, secret: Option<String>) -> Option<Timeline> {
    let transport = session::transport()?;
    let command = crate::videos::timeline_command(source_id);
    let body =
        crate::codec::command_message(&crate::shell::session::message_id(), &command, "").ok()?;
    let reply = transport
        .send_signaled(&body, secret.as_deref(), None)
        .await
        .ok()?;
    crate::videos::interpret_timeline(&reply, secret.as_deref()).ok()
}
