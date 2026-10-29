//! Authenticated MJPEG fetch, incremental parse, and 1x canvas paint.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortController, Blob, BlobPropertyBag, CanvasRenderingContext2d, Event, EventTarget,
    HtmlCanvasElement, HtmlImageElement, HtmlVideoElement, ImageBitmap,
    ReadableStreamDefaultReader,
};

use crate::videos::mjpeg::{self, Event as MjpegEvent, Frame as MjpegFrame, Parser as MjpegParser};

const CANVAS: &str = "video-canvas";
const PLAYER: &str = "video-player";
const BOUND: &str = "__a2aMediaBound";
const GAP_THRESHOLD_US: i64 = 1_500_000;

type MediaListener = Closure<dyn FnMut(Event)>;

type PlayingCallback = fn(u32);
type FrameCallback = fn(u32, &str);
type GapSkippedCallback = fn(u32);

#[derive(Clone, Copy)]
pub struct StreamTicket {
    pub generation: u32,
}

pub enum StreamOutcome {
    Stopped {
        generation: u32,
    },
    Failed {
        generation: u32,
        message: String,
        body: String,
    },
}

struct Session {
    generation: u32,
    abort: AbortController,
    paused: bool,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    static ON_PLAYING: RefCell<Option<PlayingCallback>> = const { RefCell::new(None) };
    static ON_FRAME: RefCell<Option<FrameCallback>> = const { RefCell::new(None) };
    static ON_GAP_SKIPPED: RefCell<Option<GapSkippedCallback>> = const { RefCell::new(None) };
    static START_PAUSED: Cell<bool> = const { Cell::new(false) };
    static IGNORE_PAUSE: Cell<u32> = const { Cell::new(0) };
    static PLAY_LISTENER: RefCell<Option<MediaListener>> = const { RefCell::new(None) };
    static PAUSE_LISTENER: RefCell<Option<MediaListener>> = const { RefCell::new(None) };
}

pub fn bind(playing: PlayingCallback, frame: FrameCallback, gap_skipped: GapSkippedCallback) {
    ON_PLAYING.with(|slot| *slot.borrow_mut() = Some(playing));
    ON_FRAME.with(|slot| *slot.borrow_mut() = Some(frame));
    ON_GAP_SKIPPED.with(|slot| *slot.borrow_mut() = Some(gap_skipped));
}

pub fn bind_media(on_play: fn(), on_pause: fn()) {
    attach_player();
    let Some(video) = player() else {
        return;
    };
    if media_bound(&video) {
        return;
    }
    let play = Closure::wrap(Box::new(move |_event: Event| {
        on_play();
    }) as Box<dyn FnMut(Event)>);
    let pause = Closure::wrap(Box::new(move |_event: Event| {
        on_pause();
    }) as Box<dyn FnMut(Event)>);
    let target: &EventTarget = video.unchecked_ref();
    if target
        .add_event_listener_with_callback("play", play.as_ref().unchecked_ref())
        .is_err()
        || target
            .add_event_listener_with_callback("pause", pause.as_ref().unchecked_ref())
            .is_err()
    {
        return;
    }
    if let Some(window) = web_sys::window() {
        let _ = js_sys::Reflect::set(
            &window,
            &JsValue::from_str("__a2aNativePlay"),
            play.as_ref().unchecked_ref(),
        );
        let _ = js_sys::Reflect::set(
            &window,
            &JsValue::from_str("__a2aNativePause"),
            pause.as_ref().unchecked_ref(),
        );
    }
    mark_bound(&video);
    PLAY_LISTENER.with(|slot| *slot.borrow_mut() = Some(play));
    PAUSE_LISTENER.with(|slot| *slot.borrow_mut() = Some(pause));
}

pub fn attach_player() {
    let Some(canvas) = canvas() else {
        return;
    };
    let Some(video) = player() else {
        return;
    };
    video.set_autoplay(false);
    video.set_muted(true);
    if video.src_object().is_some() {
        return;
    }
    if let Ok(stream) = canvas.capture_stream() {
        video.set_src_object(Some(&stream));
    }
    if !video.paused() {
        pause_element();
    }
}

pub fn play_element() {
    if START_PAUSED.with(Cell::get) {
        return;
    }
    let Some(video) = player() else {
        return;
    };
    let _ = video.play();
}

pub fn pause_element() {
    bump_ignore_pause();
    if let Some(video) = player() {
        let _ = video.pause();
    }
}

pub fn take_ignore_pause() -> bool {
    IGNORE_PAUSE.with(|cell| {
        let n = cell.get();
        if n == 0 {
            return false;
        }
        cell.set(n - 1);
        true
    })
}

pub fn abort() {
    SESSION.with(|slot| {
        if let Some(session) = slot.borrow_mut().take() {
            session.abort.abort();
        }
    });
}

pub fn set_paused(paused: bool) {
    START_PAUSED.with(|slot| slot.set(paused));
    SESSION.with(|slot| {
        if let Some(session) = slot.borrow_mut().as_mut() {
            session.paused = paused;
        }
    });
}

pub fn live(generation: u32) -> bool {
    SESSION.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|session| session.generation == generation)
    })
}

pub async fn run(ticket: StreamTicket, path: String, secret: Option<String>) -> StreamOutcome {
    abort();
    let Ok(abort) = AbortController::new() else {
        return failed(ticket.generation, "The browser could not start playback.");
    };
    let paused = START_PAUSED.with(Cell::get);
    SESSION.with(|slot| {
        *slot.borrow_mut() = Some(Session {
            generation: ticket.generation,
            abort: abort.clone(),
            paused,
        });
    });
    let from_start = path.starts_with(crate::videos::CLIP_PATH_PREFIX);
    let outcome = play(
        ticket.generation,
        &path,
        secret.as_deref(),
        &abort,
        from_start,
    )
    .await;
    SESSION.with(|slot| {
        if slot
            .borrow()
            .as_ref()
            .is_some_and(|session| session.generation == ticket.generation)
        {
            slot.borrow_mut().take();
        }
    });
    outcome
}

pub async fn paint_frame(media_type: &str, data: &str) -> bool {
    let url = format!("data:{media_type};base64,{data}");
    paint_data_url(&url).await
}

async fn play(
    generation: u32,
    path: &str,
    secret: Option<&str>,
    abort: &AbortController,
    from_start: bool,
) -> StreamOutcome {
    match open_reader(generation, path, secret, abort).await {
        Ok(reader) => {
            notify_playing(generation);
            consume(generation, reader, from_start).await
        }
        Err(outcome) => outcome,
    }
}

async fn open_reader(
    generation: u32,
    path: &str,
    secret: Option<&str>,
    abort: &AbortController,
) -> Result<ReadableStreamDefaultReader, StreamOutcome> {
    let Some(transport) = super::session::transport() else {
        return Err(failed(
            generation,
            "The browser could not reach this origin.",
        ));
    };
    let response = match transport
        .get_stream(path, secret, Some(&abort.signal()))
        .await
    {
        Ok(response) => response,
        Err(()) => {
            return Err(interrupted(
                generation,
                "The browser could not reach this origin.",
            ));
        }
    };
    if !live(generation) {
        return Err(StreamOutcome::Stopped { generation });
    }
    let message = match response.status() {
        200 => None,
        401 => Some("The clip was not authorized."),
        403 => Some("The clip was forbidden."),
        _ => Some("The clip could not be read."),
    };
    if let Some(message) = message {
        let body = error_body(&response).await;
        return Err(failed_body(generation, message, body));
    }
    let Some(body) = response.body() else {
        return Err(failed(generation, "The clip stream was empty."));
    };
    body.get_reader()
        .dyn_into::<ReadableStreamDefaultReader>()
        .map_err(|_| failed(generation, "The clip stream could not be read."))
}

async fn consume(
    generation: u32,
    reader: ReadableStreamDefaultReader,
    from_start: bool,
) -> StreamOutcome {
    let reduced = prefers_reduced_motion();
    let mut parser = MjpegParser::new();
    let mut queue = VecDeque::new();
    let mut origin_capture = None;
    let mut origin_wall = None;
    let mut prev_capture = None;
    let mut painted = false;
    loop {
        if let Some(outcome) = halted(generation, &mut queue) {
            return outcome;
        }
        wait_if_paused(generation).await;
        if let Some(outcome) = halted(generation, &mut queue) {
            return outcome;
        }
        if queue.len() < mjpeg::MAX_QUEUE {
            match read_chunk(&reader).await {
                Ok(None) => break,
                Ok(Some(chunk)) => match parser.push(&chunk) {
                    Ok(events) => {
                        if let Some(outcome) = enqueue(
                            generation,
                            events,
                            &mut queue,
                            &mut origin_capture,
                            &mut origin_wall,
                            &mut prev_capture,
                            &mut painted,
                            reduced,
                            from_start,
                        )
                        .await
                        {
                            return outcome;
                        }
                    }
                    Err(_) => {
                        queue.clear();
                        return failed(generation, "The clip stream was malformed.");
                    }
                },
                Err(()) => {
                    queue.clear();
                    return interrupted(generation, "The clip stream was interrupted.");
                }
            }
        }
        if let Err(outcome) = drain(
            generation,
            &mut queue,
            &mut origin_capture,
            &mut origin_wall,
            &mut prev_capture,
            &mut painted,
            reduced,
        )
        .await
        {
            return outcome;
        }
    }
    queue.clear();
    StreamOutcome::Stopped { generation }
}

async fn enqueue(
    generation: u32,
    events: Vec<MjpegEvent>,
    queue: &mut VecDeque<MjpegFrame>,
    origin_capture: &mut Option<i64>,
    origin_wall: &mut Option<f64>,
    prev_capture: &mut Option<i64>,
    painted: &mut bool,
    reduced: bool,
    from_start: bool,
) -> Option<StreamOutcome> {
    for event in events {
        match event {
            MjpegEvent::Frame(frame) => {
                if from_start && queue.len() >= mjpeg::MAX_QUEUE {
                    if let Err(outcome) = drain(
                        generation,
                        queue,
                        origin_capture,
                        origin_wall,
                        prev_capture,
                        painted,
                        reduced,
                    )
                    .await
                    {
                        queue.clear();
                        return Some(outcome);
                    }
                } else if queue.len() >= mjpeg::MAX_QUEUE {
                    queue.pop_front();
                }
                queue.push_back(frame);
            }
            MjpegEvent::Done => {
                let outcome = drain(
                    generation,
                    queue,
                    origin_capture,
                    origin_wall,
                    prev_capture,
                    painted,
                    reduced,
                )
                .await;
                queue.clear();
                return Some(
                    outcome
                        .err()
                        .unwrap_or(StreamOutcome::Stopped { generation }),
                );
            }
        }
    }
    None
}

fn halted(generation: u32, queue: &mut VecDeque<MjpegFrame>) -> Option<StreamOutcome> {
    if live(generation) {
        return None;
    }
    queue.clear();
    Some(StreamOutcome::Stopped { generation })
}

fn interrupted(generation: u32, message: &str) -> StreamOutcome {
    if live(generation) {
        failed(generation, message)
    } else {
        StreamOutcome::Stopped { generation }
    }
}

async fn drain(
    generation: u32,
    queue: &mut VecDeque<MjpegFrame>,
    origin_capture: &mut Option<i64>,
    origin_wall: &mut Option<f64>,
    prev_capture: &mut Option<i64>,
    painted: &mut bool,
    reduced: bool,
) -> Result<(), StreamOutcome> {
    while let Some(frame) = queue.pop_front() {
        if !live(generation) {
            queue.clear();
            return Err(StreamOutcome::Stopped { generation });
        }
        wait_if_paused(generation).await;
        if !live(generation) {
            queue.clear();
            return Err(StreamOutcome::Stopped { generation });
        }
        let jumped = prev_capture
            .map(|previous| frame.captured_us.saturating_sub(previous) > GAP_THRESHOLD_US)
            .unwrap_or(false);
        if jumped {
            *origin_capture = Some(frame.captured_us);
            *origin_wall = Some(js_sys::Date::now());
            notify_gap_skipped(generation);
        }
        let capture = *origin_capture.get_or_insert(frame.captured_us);
        let wall = *origin_wall.get_or_insert_with(js_sys::Date::now);
        if reduced && *painted && !jumped {
            notify_frame(generation, &frame.captured_at);
            *prev_capture = Some(frame.captured_us);
            continue;
        }
        if !jumped {
            let due = wall + ((frame.captured_us - capture) as f64 / 1_000.0);
            sleep_until(due).await;
            if !live(generation) {
                queue.clear();
                return Err(StreamOutcome::Stopped { generation });
            }
        }
        if paint_jpeg(&frame.jpeg).await {
            *painted = true;
            notify_frame(generation, &frame.captured_at);
        }
        *prev_capture = Some(frame.captured_us);
    }
    Ok(())
}

async fn wait_if_paused(generation: u32) {
    while SESSION.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|session| session.generation == generation && session.paused)
    }) {
        sleep_ms(50).await;
    }
}

async fn read_chunk(reader: &ReadableStreamDefaultReader) -> Result<Option<Vec<u8>>, ()> {
    let result = JsFuture::from(reader.read()).await.map_err(|_| ())?;
    let done = js_sys::Reflect::get(&result, &JsValue::from_str("done"))
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    if done {
        return Ok(None);
    }
    let value = js_sys::Reflect::get(&result, &JsValue::from_str("value")).map_err(|_| ())?;
    let array: js_sys::Uint8Array = value.dyn_into().map_err(|_| ())?;
    let mut bytes = vec![0; array.length() as usize];
    array.copy_to(&mut bytes);
    Ok(Some(bytes))
}

async fn paint_jpeg(jpeg: &[u8]) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let array = js_sys::Uint8Array::new_with_length(jpeg.len() as u32);
    array.copy_from(jpeg);
    let parts = js_sys::Array::new();
    parts.push(&array);
    let options = BlobPropertyBag::new();
    options.set_type("image/jpeg");
    let Ok(blob) = Blob::new_with_u8_array_sequence_and_options(&parts, &options) else {
        return false;
    };
    let Ok(promise) = window.create_image_bitmap_with_blob(&blob) else {
        return false;
    };
    let Ok(value) = JsFuture::from(promise).await else {
        return false;
    };
    let Ok(bitmap) = value.dyn_into::<ImageBitmap>() else {
        return false;
    };
    draw_bitmap(&bitmap)
}

async fn paint_data_url(url: &str) -> bool {
    let Ok(image) = HtmlImageElement::new() else {
        return false;
    };
    let loaded = js_sys::Promise::new(&mut |resolve, reject| {
        let onload = wasm_bindgen::closure::Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::NULL);
        });
        let onerror = wasm_bindgen::closure::Closure::once_into_js(move || {
            let _ = reject.call0(&JsValue::NULL);
        });
        image.set_onload(Some(onload.unchecked_ref()));
        image.set_onerror(Some(onerror.unchecked_ref()));
        image.set_src(url);
    });
    if JsFuture::from(loaded).await.is_err() {
        return false;
    }
    let Some(canvas) = canvas() else {
        return false;
    };
    canvas.set_width(image.natural_width());
    canvas.set_height(image.natural_height());
    let Some(ctx) = context(&canvas) else {
        return false;
    };
    ctx.draw_image_with_html_image_element(&image, 0.0, 0.0)
        .is_ok()
        && {
            attach_player();
            true
        }
}

fn draw_bitmap(bitmap: &ImageBitmap) -> bool {
    let Some(canvas) = canvas() else {
        return false;
    };
    canvas.set_width(bitmap.width());
    canvas.set_height(bitmap.height());
    context(&canvas)
        .and_then(|ctx| ctx.draw_image_with_image_bitmap(bitmap, 0.0, 0.0).ok())
        .is_some()
        && {
            attach_player();
            true
        }
}

fn bump_ignore_pause() {
    IGNORE_PAUSE.with(|slot| slot.set(slot.get().saturating_add(1)));
}

fn media_bound(video: &HtmlVideoElement) -> bool {
    js_sys::Reflect::get(video.as_ref(), &JsValue::from_str(BOUND))
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn mark_bound(video: &HtmlVideoElement) {
    let _ = js_sys::Reflect::set(video.as_ref(), &JsValue::from_str(BOUND), &JsValue::TRUE);
}

fn player() -> Option<HtmlVideoElement> {
    web_sys::window()?
        .document()?
        .get_element_by_id(PLAYER)?
        .dyn_into()
        .ok()
}

fn canvas() -> Option<HtmlCanvasElement> {
    web_sys::window()?
        .document()?
        .get_element_by_id(CANVAS)?
        .dyn_into()
        .ok()
}

fn context(canvas: &HtmlCanvasElement) -> Option<CanvasRenderingContext2d> {
    canvas.get_context("2d").ok().flatten()?.dyn_into().ok()
}

fn prefers_reduced_motion() -> bool {
    web_sys::window()
        .and_then(|window| {
            window
                .match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
        })
        .is_some_and(|query| query.matches())
}

fn notify_playing(generation: u32) {
    ON_PLAYING.with(|slot| {
        if let Some(callback) = *slot.borrow() {
            callback(generation);
        }
    });
}

fn notify_gap_skipped(generation: u32) {
    ON_GAP_SKIPPED.with(|slot| {
        if let Some(callback) = *slot.borrow() {
            callback(generation);
        }
    });
}

fn notify_frame(generation: u32, captured_at: &str) {
    ON_FRAME.with(|slot| {
        if let Some(callback) = *slot.borrow() {
            callback(generation, captured_at);
        }
    });
}

fn failed(generation: u32, message: &str) -> StreamOutcome {
    failed_body(generation, message, String::new())
}

fn failed_body(generation: u32, message: &str, body: String) -> StreamOutcome {
    StreamOutcome::Failed {
        generation,
        message: message.to_owned(),
        body,
    }
}

async fn error_body(response: &web_sys::Response) -> String {
    let Ok(promise) = response.text() else {
        return String::new();
    };
    JsFuture::from(promise)
        .await
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default()
}

async fn sleep_until(deadline: f64) {
    let delay = deadline - js_sys::Date::now();
    if delay > 0.0 {
        sleep_ms(delay as i32).await;
    }
}

async fn sleep_ms(ms: i32) {
    if ms <= 0 {
        return;
    }
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = JsFuture::from(promise).await;
}
