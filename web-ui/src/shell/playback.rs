//! Authenticated MJPEG fetch, incremental parse, and 1x canvas paint.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortController, Blob, BlobPropertyBag, CanvasRenderingContext2d, Event, EventTarget,
    HtmlCanvasElement, HtmlVideoElement, ImageBitmap, ReadableStreamDefaultReader,
};

use crate::videos::StreamPlan;
use crate::videos::mjpeg::{self, Event as MjpegEvent, Frame as MjpegFrame, Parser as MjpegParser};

use super::hls_playback;

const CANVAS: &str = "video-canvas";
const PLAYER: &str = "video-player";
const BOUND: &str = "__a2aMediaBound";
const BUFFER_GRACE_MS: i32 = 500;

type MediaListener = Closure<dyn FnMut(Event)>;

type PlayingCallback = fn(u32);
type FrameCallback = fn(u32, &str);
type GapSkippedCallback = fn(u32);
type BufferingCallback = fn(u32, bool);

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
    static ON_BUFFERING: RefCell<Option<BufferingCallback>> = const { RefCell::new(None) };
    static START_PAUSED: Cell<bool> = const { Cell::new(false) };
    static HOLD: Cell<bool> = const { Cell::new(false) };
    static IGNORE_PAUSE: Cell<u32> = const { Cell::new(0) };
    static MJPEG_PACE: Cell<u32> = const { Cell::new(1) };
    static HLS_PLAYBACK_RATE: Cell<f64> = const { Cell::new(1.0) };
    static REBASE_PACING: Cell<bool> = const { Cell::new(false) };
    static PLAY_LISTENER: RefCell<Option<MediaListener>> = const { RefCell::new(None) };
    static PAUSE_LISTENER: RefCell<Option<MediaListener>> = const { RefCell::new(None) };
    static WAITING_LISTENER: RefCell<Option<MediaListener>> = const { RefCell::new(None) };
    static RESUMED_LISTENER: RefCell<Option<MediaListener>> = const { RefCell::new(None) };
}

pub fn bind(
    playing: PlayingCallback,
    frame: FrameCallback,
    gap_skipped: GapSkippedCallback,
    buffering: BufferingCallback,
) {
    ON_PLAYING.with(|slot| *slot.borrow_mut() = Some(playing));
    ON_FRAME.with(|slot| *slot.borrow_mut() = Some(frame));
    ON_GAP_SKIPPED.with(|slot| *slot.borrow_mut() = Some(gap_skipped));
    ON_BUFFERING.with(|slot| *slot.borrow_mut() = Some(buffering));
}

pub fn bind_media(on_play: fn(), on_pause: fn(), on_force_pause: fn()) {
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
    let waiting = Closure::wrap(Box::new(move |_event: Event| {
        if let Some(generation) = session_generation() {
            notify_buffering(generation, true);
        }
    }) as Box<dyn FnMut(Event)>);
    let resumed = Closure::wrap(Box::new(move |_event: Event| {
        if hls_playback::is_active()
            && let Some(generation) = session_generation()
        {
            notify_buffering(generation, false);
        }
    }) as Box<dyn FnMut(Event)>);
    let target: &EventTarget = video.unchecked_ref();
    if target
        .add_event_listener_with_callback("play", play.as_ref().unchecked_ref())
        .is_err()
        || target
            .add_event_listener_with_callback("pause", pause.as_ref().unchecked_ref())
            .is_err()
        || target
            .add_event_listener_with_callback("waiting", waiting.as_ref().unchecked_ref())
            .is_err()
        || target
            .add_event_listener_with_callback("playing", resumed.as_ref().unchecked_ref())
            .is_err()
    {
        return;
    }
    if let Some(window) = web_sys::window() {
        let resume = Closure::wrap(Box::new(move || {
            set_paused(false);
            play_element();
            on_play();
        }) as Box<dyn FnMut()>);
        let _ = js_sys::Reflect::set(
            &window,
            &JsValue::from_str("__a2aNativePlay"),
            resume.as_ref().unchecked_ref(),
        );
        let force_pause = Closure::wrap(Box::new(move || {
            on_force_pause();
        }) as Box<dyn FnMut()>);
        let _ = js_sys::Reflect::set(
            &window,
            &JsValue::from_str("__a2aNativePause"),
            force_pause.as_ref().unchecked_ref(),
        );
        resume.forget();
        force_pause.forget();
    }
    mark_bound(&video);
    PLAY_LISTENER.with(|slot| *slot.borrow_mut() = Some(play));
    PAUSE_LISTENER.with(|slot| *slot.borrow_mut() = Some(pause));
    WAITING_LISTENER.with(|slot| *slot.borrow_mut() = Some(waiting));
    RESUMED_LISTENER.with(|slot| *slot.borrow_mut() = Some(resumed));
}

pub fn attach_player() {
    if hls_playback::is_active() {
        return;
    }
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
    if session_wants_play() {
        play_element();
    } else if !video.paused() {
        pause_element();
    }
}

fn session_wants_play() -> bool {
    !START_PAUSED.with(Cell::get)
        && SESSION.with(|slot| {
            slot.borrow()
                .as_ref()
                .is_some_and(|session| !session.paused)
        })
}

pub fn start_paused() -> bool {
    START_PAUSED.with(Cell::get)
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

pub fn toggle_element() {
    let Some(video) = player() else {
        return;
    };
    if video.paused() {
        play_element();
    } else {
        pause_element();
    }
}

pub fn take_ignore_pause() -> bool {
    IGNORE_PAUSE.with(|cell| {
        let n = cell.get();
        if n == 0 {
            return false;
        }
        cell.set(0);
        true
    })
}

pub fn abort() {
    HOLD.with(|slot| slot.set(false));
    hls_playback::teardown();
    stop_session();
    reset_surface();
}

pub fn hold_stream() {
    HOLD.with(|slot| slot.set(true));
    hls_playback::teardown();
    stop_session();
}

fn stop_session() {
    SESSION.with(|slot| {
        if let Some(session) = slot.borrow_mut().take() {
            session.abort.abort();
        }
    });
}

fn frames_held() -> bool {
    HOLD.with(|slot| slot.get())
}

pub fn set_paused(paused: bool) {
    START_PAUSED.with(|slot| slot.set(paused));
    SESSION.with(|slot| {
        if let Some(session) = slot.borrow_mut().as_mut() {
            session.paused = paused;
        }
    });
}

pub fn apply_plan(plan: &StreamPlan) {
    MJPEG_PACE.with(|slot| slot.set(plan.mjpeg_pace.max(1)));
    HLS_PLAYBACK_RATE.with(|slot| slot.set(plan.hls_playback_rate.max(0.1)));
    apply_hls_playback_rate();
    rebase_pacing();
}

fn mjpeg_pace() -> u32 {
    MJPEG_PACE.with(Cell::get).max(1)
}

fn rebase_pacing() {
    REBASE_PACING.with(|slot| slot.set(true));
}

pub(crate) fn apply_hls_playback_rate() {
    let Some(video) = player() else {
        return;
    };
    video.set_playback_rate(HLS_PLAYBACK_RATE.with(Cell::get));
}

fn maybe_rebase_pacing(
    origin_capture: &mut Option<i64>,
    origin_wall: &mut Option<f64>,
    prev_capture: &mut Option<i64>,
) {
    if !REBASE_PACING.with(Cell::get) {
        return;
    }
    REBASE_PACING.with(|slot| slot.set(false));
    *origin_capture = None;
    *origin_wall = None;
    *prev_capture = None;
}

pub fn live(generation: u32) -> bool {
    SESSION.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|session| session.generation == generation)
    })
}

pub fn end_session(generation: u32) {
    SESSION.with(|slot| {
        if slot
            .borrow()
            .as_ref()
            .is_some_and(|session| session.generation == generation)
        {
            slot.borrow_mut().take();
        }
    });
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
    if !paused {
        play_element();
    }
    let from_start = path.starts_with(crate::videos::CLIP_PATH_PREFIX);
    if crate::videos::speed::is_hls_fetch_path(&path) {
        return hls_playback::run(ticket, path, secret, &abort, from_start).await;
    }
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
    let mut stalling = false;
    loop {
        if let Some(outcome) = halted(generation, &mut queue) {
            return outcome;
        }
        wait_if_paused(generation).await;
        if let Some(outcome) = halted(generation, &mut queue) {
            return outcome;
        }
        if queue.len() < mjpeg::MAX_QUEUE {
            match read_playback_chunk(
                generation,
                &reader,
                queue.is_empty(),
                painted,
                &mut stalling,
            )
            .await
            {
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
                            &mut stalling,
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
            &mut stalling,
            reduced,
            from_start,
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
    stalling: &mut bool,
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
                        stalling,
                        reduced,
                        from_start,
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
                    stalling,
                    reduced,
                    from_start,
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
    stalling: &mut bool,
    reduced: bool,
    from_start: bool,
) -> Result<(), StreamOutcome> {
    if !from_start && queue.len() > 1 {
        let newest = queue.pop_back();
        queue.clear();
        if let Some(frame) = newest {
            queue.push_back(frame);
        }
    }
    while let Some(frame) = queue.pop_front() {
        if !live(generation) {
            queue.clear();
            return Err(StreamOutcome::Stopped { generation });
        }
        maybe_rebase_pacing(origin_capture, origin_wall, prev_capture);
        wait_if_paused(generation).await;
        if !live(generation) {
            queue.clear();
            return Err(StreamOutcome::Stopped { generation });
        }
        let jumped =
            prev_capture.is_some_and(|previous| mjpeg::is_capture_gap(previous, frame.captured_us));
        if jumped {
            *origin_capture = Some(frame.captured_us);
            *origin_wall = Some(js_sys::Date::now());
            notify_gap_skipped(generation);
        }
        if frames_held() || !live(generation) {
            queue.clear();
            return Err(StreamOutcome::Stopped { generation });
        }
        if !from_start {
            if paint_jpeg(&frame.jpeg).await {
                *painted = true;
                clear_stall(generation, stalling);
            }
            notify_frame(generation, &frame.captured_at);
            *origin_capture = Some(frame.captured_us);
            *origin_wall = Some(js_sys::Date::now());
            *prev_capture = Some(frame.captured_us);
            continue;
        }
        let capture = *origin_capture.get_or_insert(frame.captured_us);
        let wall = *origin_wall.get_or_insert_with(js_sys::Date::now);
        if reduced && *painted && !jumped {
            *prev_capture = Some(frame.captured_us);
            continue;
        }
        if !jumped {
            let pace = mjpeg_pace();
            if mjpeg::is_late(frame.captured_us, capture, wall, js_sys::Date::now(), pace) {
                notify_frame(generation, &frame.captured_at);
                *prev_capture = Some(frame.captured_us);
                continue;
            }
            sleep_until(mjpeg::due_wall_ms(frame.captured_us, capture, wall, pace)).await;
            if !live(generation) {
                queue.clear();
                return Err(StreamOutcome::Stopped { generation });
            }
        }
        if paint_jpeg(&frame.jpeg).await {
            *painted = true;
            clear_stall(generation, stalling);
        }
        notify_frame(generation, &frame.captured_at);
        *prev_capture = Some(frame.captured_us);
    }
    Ok(())
}

pub(crate) async fn wait_if_paused(generation: u32) {
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

pub(crate) async fn show_jpeg(jpeg: &[u8]) -> bool {
    paint_jpeg(jpeg).await
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

pub(crate) fn bump_ignore_pause() {
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

fn reset_surface() {
    if let Some(canvas) = canvas() {
        canvas.set_width(0);
        canvas.set_height(0);
    }
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

fn session_generation() -> Option<u32> {
    SESSION.with(|slot| slot.borrow().as_ref().map(|session| session.generation))
}

fn clear_stall(generation: u32, stalling: &mut bool) {
    if *stalling {
        *stalling = false;
        notify_buffering(generation, false);
    }
}

async fn read_playback_chunk(
    generation: u32,
    reader: &ReadableStreamDefaultReader,
    queue_empty: bool,
    painted: bool,
    stalling: &mut bool,
) -> Result<Option<Vec<u8>>, ()> {
    let mut read = std::pin::pin!(read_chunk(reader));
    if !queue_empty || !painted || *stalling {
        return read.await;
    }
    let mut grace = std::pin::pin!(sleep_ms(BUFFER_GRACE_MS));
    std::future::poll_fn(|context| {
        if !*stalling && grace.as_mut().poll(context).is_ready() {
            *stalling = true;
            notify_buffering(generation, true);
        }
        read.as_mut().poll(context)
    })
    .await
}

pub(crate) fn notify_buffering(generation: u32, buffering: bool) {
    ON_BUFFERING.with(|slot| {
        if let Some(callback) = *slot.borrow() {
            callback(generation, buffering);
        }
    });
}

pub(crate) fn notify_playing(generation: u32) {
    ON_PLAYING.with(|slot| {
        if let Some(callback) = *slot.borrow() {
            callback(generation);
        }
    });
}

pub(crate) fn notify_gap_skipped(generation: u32) {
    ON_GAP_SKIPPED.with(|slot| {
        if let Some(callback) = *slot.borrow() {
            callback(generation);
        }
    });
}

pub(crate) fn notify_frame(generation: u32, captured_at: &str) {
    if let Some(status) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("video-status"))
    {
        let _ = status.set_attribute("data-frame-time", captured_at);
    }
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

pub(crate) async fn sleep_ms(ms: i32) {
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
