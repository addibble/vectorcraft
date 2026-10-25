//! Link sources (`links.addSources`): the linked files the embedding page can supply.
//! [`fetch_requested`] fetches each file the engine asks for
//! (`vectorcraft_engine::link_sources::requests`), at most [`PARALLEL`] at once, and keeps the
//! bytes until the app's next frame hands them to the engine ([`take_fetched`]); a file larger than
//! the engine takes is refused before its bytes are copied into the app's memory. The page's own
//! cookies go with each fetch, as with any same-origin request.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use vectorcraft_engine::link_sources::{MAX_SUPPLIED, SourceRequest};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;

/// Fetches at once, leaving the connection to the app's own requests.
const PARALLEL: usize = 4;
/// A fetch given up after this long.
const TIMEOUT_MS: i32 = 120_000;

thread_local! {
    /// Files fetched and not yet handed to the engine: (path, bytes).
    static FETCHED: RefCell<Vec<(String, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
    /// Fetches that failed: (path, why).
    static FAILED: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
    /// Files asked for and not fetched yet, waiting for a free fetch.
    static WAITING: RefCell<VecDeque<SourceRequest>> = const { RefCell::new(VecDeque::new()) };
    /// Fetches under way (at most [`PARALLEL`]).
    static IN_FLIGHT: Cell<usize> = const { Cell::new(0) };
}

/// Fetch, in the background, the files the engine asks for now: at most [`PARALLEL`] at once, the
/// rest when one finishes (each asks `ctx` for a frame, whose call here starts the next).
pub fn fetch_requested(ctx: &egui::Context, session: &vectorcraft_engine::Session) {
    WAITING.with(|w| w.borrow_mut().extend(vectorcraft_engine::link_sources::requests(session)));
    while IN_FLIGHT.get() < PARALLEL {
        let Some(req) = WAITING.with(|w| w.borrow_mut().pop_front()) else { break };
        IN_FLIGHT.set(IN_FLIGHT.get() + 1);
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match fetch(&req.url).await {
                Ok(bytes) => FETCHED.with(|f| f.borrow_mut().push((req.path, bytes))),
                Err(e) => {
                    log::warn!("link source {}: {e}", req.path);
                    FAILED.with(|f| f.borrow_mut().push((req.path, e)));
                }
            }
            IN_FLIGHT.set(IN_FLIGHT.get().saturating_sub(1));
            ctx.request_repaint();
        });
    }
}

/// The files fetched since the last call: (path, bytes).
pub fn take_fetched() -> Vec<(String, Vec<u8>)> {
    FETCHED.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

/// The fetches that failed since the last call: (path, why).
pub fn take_failed() -> Vec<(String, String)> {
    FAILED.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

/// The file at `url` (relative to the page), given up after [`TIMEOUT_MS`]; one larger than the
/// engine takes is refused before its bytes are copied into the app's memory.
async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("no window")?;
    let abort = web_sys::AbortController::new().map_err(js)?;
    let init = web_sys::RequestInit::new();
    init.set_method("GET");
    init.set_signal(Some(&abort.signal()));
    let on_timeout = {
        let abort = abort.clone();
        Closure::<dyn FnMut()>::new(move || abort.abort())
    };
    let timer = window.set_timeout_with_callback_and_timeout_and_arguments_0(on_timeout.as_ref().unchecked_ref(), TIMEOUT_MS).map_err(js)?;
    let result = async {
        let response: web_sys::Response = JsFuture::from(window.fetch_with_str_and_init(url, &init)).await.map_err(js)?.dyn_into().map_err(js)?;
        if !response.ok() {
            return Err(format!("HTTP {} for {url}", response.status()));
        }
        let buffer = JsFuture::from(response.array_buffer().map_err(js)?).await.map_err(js)?;
        let bytes = js_sys::Uint8Array::new(&buffer);
        if bytes.length() as usize > MAX_SUPPLIED {
            return Err(format!("{url} is larger than {} MB", MAX_SUPPLIED >> 20));
        }
        Ok(bytes.to_vec())
    }
    .await;
    window.clear_timeout_with_handle(timer);
    // Kept alive until the timer can no longer fire.
    drop(on_timeout);
    result
}

fn js(e: wasm_bindgen::JsValue) -> String {
    format!("{e:?}")
}
