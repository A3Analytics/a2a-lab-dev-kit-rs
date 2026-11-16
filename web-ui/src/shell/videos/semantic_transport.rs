//! Abortable semantic command transport for the Videos reader.

use std::cell::RefCell;

use dioxus::prelude::ReadableExt;

use crate::inventory::Fault;
use crate::shell::session::{self, message_id};
use crate::videos::Timeline;
use web_sys::AbortController;

use super::{VideoUi, bound};

struct SemanticFlight {
    generation: u32,
    abort: AbortController,
}

thread_local! {
    static FLIGHTS: RefCell<Option<SemanticFlight>> = const { RefCell::new(None) };
}

pub(super) async fn post_with_abort(
    command: serde_json::Value,
    secret: Option<String>,
    generation: u32,
) -> Result<crate::codec::Reply, String> {
    let Some(signal) = begin_semantic_command(generation) else {
        return Err("The semantic request was canceled.".to_owned());
    };
    let Some(transport) = session::transport() else {
        return Err("The browser could not reach this origin.".to_owned());
    };
    let Ok(body) = crate::codec::command_message(&message_id(), &command, "") else {
        return Err("The semantic query could not be built.".to_owned());
    };
    match transport
        .send_signaled(&body, secret.as_deref(), Some(&signal))
        .await
    {
        Err(()) => Err("The browser could not reach this origin.".to_owned()),
        Ok(reply) => Ok(reply),
    }
}

fn begin_semantic_command(generation: u32) -> Option<web_sys::AbortSignal> {
    let abort = AbortController::new().ok()?;
    let signal = abort.signal();
    let flight = SemanticFlight { generation, abort };
    let previous = FLIGHTS.with(|slot| {
        let mut current = slot.borrow_mut();
        if current
            .as_ref()
            .is_some_and(|existing| existing.generation > generation)
        {
            return Err(flight);
        }
        Ok(current.replace(flight))
    });
    match previous {
        Ok(previous) => {
            if let Some(previous) = previous {
                previous.abort.abort();
            }
            Some(signal)
        }
        Err(_flight) => None,
    }
}

pub fn abort_semantic_commands() {
    FLIGHTS.with(|slot| {
        if let Some(flight) = slot.borrow_mut().take() {
            flight.abort.abort();
        }
    });
}

pub(super) fn coverage_bounds(
    source_id: &str,
    search_window: impl Fn(&VideoUi) -> Option<(String, String)>,
    retained_search_window: impl Fn(&Timeline) -> Option<(String, String)>,
) -> (Option<String>, Option<String>) {
    let Some(ui) = bound() else {
        return (None, None);
    };
    let state = ui.peek().clone();
    if let Some((start, end)) = search_window(&state) {
        return (Some(start), Some(end));
    }
    let Some((start, end)) = state
        .reader
        .timeline
        .as_ref()
        .and_then(retained_search_window)
    else {
        let _ = source_id;
        return (None, None);
    };
    (Some(start), Some(end))
}

pub(super) fn fault_text(fault: &Fault) -> String {
    match fault {
        Fault::Unauthorized(message) | Fault::Forbidden(message) => message.clone(),
        Fault::Unavailable(message) => message.clone(),
        Fault::Malformed | Fault::Transport => "The response could not be read.".to_owned(),
    }
}
