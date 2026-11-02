//! Authenticated gateway HLS playback through Media Source Extensions.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortController, HtmlVideoElement, MediaSource, MediaSourceReadyState, SourceBuffer,
};

use crate::shell::http::Transport;
use crate::shell::playback::{self, StreamOutcome, StreamTicket};
use crate::videos::hls::{
    CodecMime, CodecProbe, parse_playlist, probe_codec_from_init, resolve_playlist_asset,
};

const PLAYER: &str = "video-player";
const TRACE: &str = "__a2aHlsTrace";

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static OBJECT_URL: RefCell<Option<String>> = const { RefCell::new(None) };
    static SESSION_EPOCH: Cell<u32> = const { Cell::new(0) };
}

fn bump_session_epoch() -> u32 {
    SESSION_EPOCH.with(|slot| {
        let next = slot.get().wrapping_add(1);
        slot.set(next);
        next
    })
}

fn current_session_epoch() -> u32 {
    SESSION_EPOCH.with(Cell::get)
}

thread_local! {
    static HLS_GUARD: Cell<(u32, u32)> = const { Cell::new((u32::MAX, u32::MAX)) };
}

fn activate_hls_guard(generation: u32, epoch: u32) {
    HLS_GUARD.with(|slot| slot.set((generation, epoch)));
}

fn deactivate_hls_guard(epoch: u32) {
    HLS_GUARD.with(|slot| {
        if slot.get().1 == epoch {
            slot.set((u32::MAX, u32::MAX));
        }
    });
}

fn clear_hls_guard() {
    HLS_GUARD.with(|slot| slot.set((u32::MAX, u32::MAX)));
}

fn hls_alive(generation: u32) -> bool {
    if playback::live(generation) {
        return true;
    }
    HLS_GUARD.with(|slot| {
        let (guard_generation, epoch) = slot.get();
        guard_generation == generation && epoch == current_session_epoch()
    })
}

pub fn is_active() -> bool {
    ACTIVE.with(Cell::get)
}

pub fn teardown() {
    ACTIVE.with(|slot| slot.set(false));
    clear_hls_guard();
    if let Some(url) = OBJECT_URL.with(|slot| slot.borrow_mut().take()) {
        let _ = web_sys::Url::revoke_object_url(&url);
    }
    if let Some(video) = player() {
        video.set_src("");
        let _ = video.remove_attribute("src");
        video.set_src_object(None);
    }
    publish_trace(&Trace {
        append_order: Vec::new(),
        cleaned: true,
        unsupported_codec: false,
        trick_playlist: false,
    });
}

pub fn prepare_player() {
    teardown();
    ACTIVE.with(|slot| slot.set(true));
    if let Some(video) = player() {
        video.set_src_object(None);
        video.set_src("");
        video.set_muted(true);
        video.set_hidden(false);
        let _ = video.remove_attribute("hidden");
    }
}

pub async fn run(
    ticket: StreamTicket,
    playlist_path: String,
    secret: Option<String>,
    abort: &AbortController,
    from_start: bool,
) -> StreamOutcome {
    let epoch = bump_session_epoch();
    prepare_player();
    let generation = ticket.generation;
    activate_hls_guard(generation, epoch);
    let mut trace = Trace {
        trick_playlist: playlist_path.contains("index-8x.m3u8"),
        ..Trace::default()
    };
    let outcome = play_session(
        generation,
        &playlist_path,
        secret.as_deref(),
        abort,
        from_start,
        &mut trace,
    )
    .await;
    if current_session_epoch() == epoch {
        deactivate_hls_guard(epoch);
        teardown();
        trace.cleaned = true;
        publish_trace(&trace);
        playback::end_session(generation);
    }
    outcome
}

#[derive(Default, Clone)]
struct Trace {
    append_order: Vec<String>,
    cleaned: bool,
    unsupported_codec: bool,
    trick_playlist: bool,
}

async fn play_session(
    generation: u32,
    playlist_path: &str,
    secret: Option<&str>,
    abort: &AbortController,
    from_start: bool,
    trace: &mut Trace,
) -> StreamOutcome {
    let Some(transport) = crate::shell::session::transport() else {
        return failed(generation, "The browser could not reach this origin.");
    };
    let playlist_bytes =
        match fetch_bytes(&transport, playlist_path, secret, abort, generation).await {
            Ok(bytes) => bytes,
            Err(outcome) => return outcome,
        };
    trace.append_order.push(
        playlist_path
            .rsplit('/')
            .next()
            .unwrap_or("index.m3u8")
            .to_owned(),
    );
    publish_trace(trace);
    let Some(parsed) = parse_playlist(std::str::from_utf8(&playlist_bytes).unwrap_or("")) else {
        return failed(generation, "The clip stream was malformed.");
    };
    let init_relative = parsed.map_uri.as_deref().unwrap_or("init.mp4");
    let Some(init_path) = resolve_playlist_asset(playlist_path, init_relative) else {
        return failed(generation, "The clip stream was malformed.");
    };
    let init_bytes = match fetch_bytes(&transport, &init_path, secret, abort, generation).await {
        Ok(bytes) => bytes,
        Err(outcome) => return outcome,
    };
    let mime = match probe_codec_from_init(&init_bytes) {
        CodecProbe::Supported(codec) => select_source_buffer_mime(&codec),
        CodecProbe::Unsupported => None,
    };
    let Some(mime) = mime else {
        trace.unsupported_codec = true;
        return failed(generation, "This browser cannot play the packaged codec.");
    };
    let Some(video) = player() else {
        return failed(generation, "The browser could not start playback.");
    };
    let Ok(media_source) = MediaSource::new() else {
        return failed(generation, "The browser could not start playback.");
    };
    let Ok(object_url) = web_sys::Url::create_object_url_with_source(&media_source) else {
        return failed(generation, "The browser could not start playback.");
    };
    OBJECT_URL.with(|slot| *slot.borrow_mut() = Some(object_url.clone()));
    video.set_src(&object_url);
    video.load();
    next_frame().await;
    if wait_source_open(&media_source, abort, generation)
        .await
        .is_err()
    {
        return stopped(generation);
    }
    if abort.signal().aborted() {
        return stopped(generation);
    }
    let source_buffer = match media_source.add_source_buffer(&mime) {
        Ok(buffer) => buffer,
        Err(_) => {
            trace.unsupported_codec = true;
            return failed(generation, "This browser cannot play the packaged codec.");
        }
    };
    if append_buffer(&source_buffer, &init_bytes, abort, generation)
        .await
        .is_err()
    {
        return stopped(generation);
    }
    trace.append_order.push(init_relative.to_owned());
    publish_trace(trace);
    playback::set_paused(false);
    playback::notify_playing(generation);
    playback::bump_ignore_pause();
    let _ = video.play();
    let mut appended = HashSet::new();
    appended.insert(init_relative.to_owned());
    play_appended_segments(
        generation,
        playlist_path,
        secret,
        abort,
        from_start,
        &transport,
        &media_source,
        &source_buffer,
        &video,
        &parsed,
        &mut appended,
        trace,
    )
    .await
}

async fn play_appended_segments(
    generation: u32,
    playlist_path: &str,
    secret: Option<&str>,
    abort: &AbortController,
    from_start: bool,
    transport: &Transport,
    media_source: &MediaSource,
    source_buffer: &SourceBuffer,
    video: &HtmlVideoElement,
    parsed: &crate::videos::hls::ParsedPlaylist,
    appended: &mut HashSet<String>,
    trace: &mut Trace,
) -> StreamOutcome {
    if let Err(outcome) = append_segments(
        generation,
        playlist_path,
        secret,
        abort,
        transport,
        parsed,
        source_buffer,
        appended,
        trace,
    )
    .await
    {
        return outcome;
    }
    trace.append_order.push(
        playlist_path
            .rsplit('/')
            .next()
            .unwrap_or("index.m3u8")
            .to_owned(),
    );
    publish_trace(trace);
    if parsed.end_list && !live_playlist_session(playlist_path) {
        let _ = media_source.end_of_stream();
        playback::apply_hls_playback_rate();
        if from_start {
            video.set_current_time(0.0);
        }
        let _ = video.play();
        return wait_until_end(generation, abort, true, video, source_buffer).await;
    }
    playback::apply_hls_playback_rate();
    if from_start {
        video.set_current_time(0.0);
    }
    let _ = video.play();
    refresh_live(
        generation,
        playlist_path,
        secret,
        abort,
        transport,
        media_source,
        source_buffer,
        video,
        appended,
        trace,
    )
    .await
}

async fn append_segments(
    generation: u32,
    playlist_path: &str,
    secret: Option<&str>,
    abort: &AbortController,
    transport: &Transport,
    parsed: &crate::videos::hls::ParsedPlaylist,
    source_buffer: &SourceBuffer,
    appended: &mut HashSet<String>,
    trace: &mut Trace,
) -> Result<(), StreamOutcome> {
    for segment in &parsed.segments {
        if abort.signal().aborted() {
            return Err(stopped(generation));
        }
        playback::wait_if_paused(generation).await;
        if appended.contains(&segment.uri) {
            continue;
        }
        let Some(segment_path) = resolve_playlist_asset(playlist_path, &segment.uri) else {
            return Err(failed(generation, "The clip stream was malformed."));
        };
        let segment_bytes =
            match fetch_bytes(transport, &segment_path, secret, abort, generation).await {
                Ok(bytes) => bytes,
                Err(outcome) => return Err(outcome),
            };
        trace.append_order.push(segment.uri.clone());
        publish_trace(trace);
        if append_buffer(source_buffer, &segment_bytes, abort, generation)
            .await
            .is_err()
        {
            if live_playlist_session(playlist_path) {
                continue;
            }
            return Err(stopped(generation));
        }
        appended.insert(segment.uri.clone());
        if segment.discontinuity_before {
            playback::notify_gap_skipped(generation);
        }
        if let Some(stamp) = segment.program_date_time.as_deref() {
            playback::notify_frame(generation, stamp);
        }
    }
    Ok(())
}

async fn refresh_live(
    generation: u32,
    playlist_path: &str,
    secret: Option<&str>,
    abort: &AbortController,
    transport: &Transport,
    media_source: &MediaSource,
    source_buffer: &SourceBuffer,
    video: &HtmlVideoElement,
    appended: &mut HashSet<String>,
    trace: &mut Trace,
) -> StreamOutcome {
    loop {
        if abort.signal().aborted() {
            return stopped(generation);
        }
        playback::wait_if_paused(generation).await;
        let playlist_bytes =
            match fetch_bytes(transport, playlist_path, secret, abort, generation).await {
                Ok(bytes) => bytes,
                Err(outcome) => return outcome,
            };
        trace.append_order.push(
            playlist_path
                .rsplit('/')
                .next()
                .unwrap_or("index.m3u8")
                .to_owned(),
        );
        publish_trace(trace);
        let Some(parsed) = parse_playlist(std::str::from_utf8(&playlist_bytes).unwrap_or(""))
        else {
            return failed(generation, "The clip stream was malformed.");
        };
        if let Err(outcome) = append_segments(
            generation,
            playlist_path,
            secret,
            abort,
            transport,
            &parsed,
            source_buffer,
            appended,
            trace,
        )
        .await
        {
            return outcome;
        }
        skip_forward_if_gapped(video, source_buffer);
        if parsed.end_list && !live_playlist_session(playlist_path) {
            let _ = media_source.end_of_stream();
            return wait_until_end(generation, abort, true, video, source_buffer).await;
        }
        playback::sleep_ms(500).await;
    }
}

async fn wait_until_end(
    generation: u32,
    abort: &AbortController,
    finite: bool,
    video: &HtmlVideoElement,
    source_buffer: &SourceBuffer,
) -> StreamOutcome {
    if !finite {
        loop {
            if !hls_alive(generation) || abort.signal().aborted() {
                return stopped(generation);
            }
            playback::wait_if_paused(generation).await;
            skip_forward_if_gapped(video, source_buffer);
            playback::sleep_ms(200).await;
        }
    }
    loop {
        if abort.signal().aborted() {
            return stopped(generation);
        }
        playback::wait_if_paused(generation).await;
        skip_forward_if_gapped(video, source_buffer);
        if video.ended() {
            return stopped(generation);
        }
        playback::sleep_ms(100).await;
    }
}

fn skip_forward_if_gapped(video: &HtmlVideoElement, buffer: &SourceBuffer) {
    let t = video.current_time();
    let Ok(ranges) = buffer.buffered() else {
        return;
    };
    for index in 0..ranges.length() {
        let Ok(start) = ranges.start(index) else {
            continue;
        };
        let Ok(end) = ranges.end(index) else {
            continue;
        };
        if t >= start - 0.05 && t < end {
            return;
        }
        if start > t + 0.05 {
            video.set_current_time(start);
            return;
        }
    }
}

async fn fetch_bytes(
    transport: &Transport,
    path: &str,
    secret: Option<&str>,
    abort: &AbortController,
    generation: u32,
) -> Result<Vec<u8>, StreamOutcome> {
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
    if abort.signal().aborted() {
        return Err(stopped(generation));
    }
    let status = response.status();
    if status != 200 {
        let message = match status {
            401 => "The clip was not authorized.",
            403 => "The clip was forbidden.",
            _ => "The clip could not be read.",
        };
        return Err(failed(generation, message));
    }
    read_body(&response).await.map_err(|_| stopped(generation))
}

async fn read_body(response: &web_sys::Response) -> Result<Vec<u8>, ()> {
    let Ok(promise) = response.array_buffer() else {
        return Err(());
    };
    let Ok(value) = JsFuture::from(promise).await else {
        return Err(());
    };
    let buffer: js_sys::ArrayBuffer = value.dyn_into().map_err(|_| ())?;
    let array = js_sys::Uint8Array::new(&buffer);
    let mut bytes = vec![0; array.length() as usize];
    array.copy_to(&mut bytes);
    Ok(bytes)
}

async fn append_buffer(
    source_buffer: &SourceBuffer,
    bytes: &[u8],
    abort: &AbortController,
    generation: u32,
) -> Result<(), ()> {
    let _ = generation;
    wait_update_end(source_buffer).await?;
    if abort.signal().aborted() {
        return Err(());
    }
    let mut payload = bytes.to_vec();
    source_buffer
        .append_buffer_with_u8_array(payload.as_mut_slice())
        .map_err(|_| ())?;
    wait_update_end(source_buffer).await
}

async fn wait_update_end(source_buffer: &SourceBuffer) -> Result<(), ()> {
    while source_buffer.updating() {
        playback::sleep_ms(10).await;
    }
    Ok(())
}

async fn next_frame() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let _ = window.request_animation_frame(&resolve);
    });
    let _ = JsFuture::from(promise).await;
}

async fn wait_source_open(
    media_source: &MediaSource,
    abort: &AbortController,
    generation: u32,
) -> Result<(), ()> {
    let _ = generation;
    for _ in 0..1000 {
        if abort.signal().aborted() {
            return Err(());
        }
        if media_source.ready_state() == MediaSourceReadyState::Open {
            return Ok(());
        }
        playback::sleep_ms(10).await;
    }
    Err(())
}

fn live_playlist_session(playlist_path: &str) -> bool {
    playlist_path.contains(crate::videos::LIVE_PATH_PREFIX)
}

fn publish_trace(trace: &Trace) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let object = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &object,
        &wasm_bindgen::JsValue::from_str("appendOrder"),
        &js_sys::Array::from_iter(
            trace
                .append_order
                .iter()
                .map(|item| wasm_bindgen::JsValue::from_str(item)),
        ),
    );
    let _ = js_sys::Reflect::set(
        &object,
        &wasm_bindgen::JsValue::from_str("cleaned"),
        &wasm_bindgen::JsValue::from_bool(trace.cleaned),
    );
    let _ = js_sys::Reflect::set(
        &object,
        &wasm_bindgen::JsValue::from_str("unsupportedCodec"),
        &wasm_bindgen::JsValue::from_bool(trace.unsupported_codec),
    );
    let _ = js_sys::Reflect::set(
        &object,
        &wasm_bindgen::JsValue::from_str("trickPlaylist"),
        &wasm_bindgen::JsValue::from_bool(trace.trick_playlist),
    );
    let _ = js_sys::Reflect::set(&window, &wasm_bindgen::JsValue::from_str(TRACE), &object);
}

fn player() -> Option<HtmlVideoElement> {
    web_sys::window()?
        .document()?
        .get_element_by_id(PLAYER)?
        .dyn_into()
        .ok()
}

fn failed(generation: u32, message: &str) -> StreamOutcome {
    StreamOutcome::Failed {
        generation,
        message: message.to_owned(),
        body: String::new(),
    }
}

fn interrupted(generation: u32, message: &str) -> StreamOutcome {
    if hls_alive(generation) {
        failed(generation, message)
    } else {
        stopped(generation)
    }
}

fn stopped(generation: u32) -> StreamOutcome {
    StreamOutcome::Stopped { generation }
}

fn select_source_buffer_mime(codec: &CodecMime) -> Option<String> {
    codec
        .mime_candidates()
        .into_iter()
        .find(|mime| MediaSource::is_type_supported(mime))
}
