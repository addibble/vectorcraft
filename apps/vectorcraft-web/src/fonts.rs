//! Craft-fonts faces served beside the wasm (`fonts/<sha16>/…`, copied by
//! `cargo xtask web-fonts`) instead of embedded: [`load_startup_fonts`] fetches the `startup`
//! ones before the app starts, [`spawn_background_fonts`] the rest once it runs. The browser checks
//! each file against its SHA-256 (Subresource Integrity). A face that fails is logged and skipped:
//! the app runs without it. With `WEB_FONTS` empty (built without craft-fonts) both do nothing.
//!
//! Font sources (`text.addFontSources`): the faces a page can supply. [`fetch_requested`] fetches
//! each one type has asked for (`FontDb::take_source_requests`), checked against its SHA-256, and
//! keeps the bytes until the app's next frame hands them to `text.addFont` ([`take_fetched`]). At
//! most [`BACKGROUND_PARALLEL`] are fetched at once, and a file larger than `text.addFont` takes
//! ([`SOURCE_MAX_BYTES`]) is refused before its bytes are copied into the app's memory.

use std::cell::Cell;
use std::rc::Rc;

use vectorcraft_text::{FontDb, FontSource, WEB_FONTS, WebFont};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;

/// Per startup face: the page shows its loading text meanwhile.
const STARTUP_TIMEOUT_MS: i32 = 20_000;
/// Per background face.
const BACKGROUND_TIMEOUT_MS: i32 = 120_000;
/// Background fetches at once, leaving the connection to the app's own requests.
const BACKGROUND_PARALLEL: usize = 4;
/// The largest font source fetched: the most `text.addFont` takes.
const SOURCE_MAX_BYTES: usize = 64 << 20;

thread_local! {
    /// The background fetch runs once per page, though a graphics restart creates the app again.
    static BACKGROUND_STARTED: Cell<bool> = const { Cell::new(false) };
    /// Font sources fetched and not yet added, with the file's bytes.
    static FETCHED: std::cell::RefCell<Vec<(FontSource, Vec<u8>)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Font sources that failed to fetch, and why, for the status line.
    static FAILED: std::cell::RefCell<Vec<(FontSource, String)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Font sources asked for and not fetched yet, waiting for a free fetch.
    static WAITING: std::cell::RefCell<std::collections::VecDeque<FontSource>> = const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    /// Font sources being fetched now (at most [`BACKGROUND_PARALLEL`]).
    static IN_FLIGHT: Cell<usize> = const { Cell::new(0) };
}

/// Fetch, in the background, the font sources type has asked for: at most
/// [`BACKGROUND_PARALLEL`] at once, the rest when one finishes (each asks `ctx` for a frame, whose
/// call here starts the next). A fetched file waits for [`take_fetched`]. A source that fails is
/// logged; its type stays in the fallback face.
pub fn fetch_requested(ctx: &egui::Context) {
    WAITING.with(|w| w.borrow_mut().extend(FontDb::global().take_source_requests()));
    while IN_FLIGHT.get() < BACKGROUND_PARALLEL {
        let Some(src) = WAITING.with(|w| w.borrow_mut().pop_front()) else { break };
        IN_FLIGHT.set(IN_FLIGHT.get() + 1);
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match fetch(&src.url, &src.integrity, BACKGROUND_TIMEOUT_MS, Some(SOURCE_MAX_BYTES)).await {
                Ok(bytes) => FETCHED.with(|f| f.borrow_mut().push((src, bytes))),
                Err(e) => {
                    log::warn!("font source {} {}: {e}", src.family, src.style);
                    FAILED.with(|f| f.borrow_mut().push((src, e)));
                }
            }
            IN_FLIGHT.set(IN_FLIGHT.get().saturating_sub(1));
            ctx.request_repaint();
        });
    }
}

/// The font sources fetched since the last call, with their files' bytes, for `text.addFont`.
pub fn take_fetched() -> Vec<(FontSource, Vec<u8>)> {
    FETCHED.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

/// The font sources that failed to fetch since the last call, and why.
pub fn take_failed() -> Vec<(FontSource, String)> {
    FAILED.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

/// Fetch and register the `startup` faces (one after the other). Returns how many loaded.
pub async fn load_startup_fonts() -> usize {
    let mut loaded = 0;
    for f in WEB_FONTS.iter().filter(|f| f.startup) {
        if load(f, STARTUP_TIMEOUT_MS).await {
            loaded += 1;
        }
    }
    loaded
}

/// Fetch and register the `background` faces, [`BACKGROUND_PARALLEL`] at a time, asking `ctx` for
/// a frame after each so the font menus and the canvas pick it up. Only the first call does
/// anything.
pub fn spawn_background_fonts(ctx: egui::Context) {
    if BACKGROUND_STARTED.with(|s| s.replace(true)) {
        return;
    }
    let queue: Rc<Vec<&'static WebFont>> = Rc::new(WEB_FONTS.iter().filter(|f| !f.startup).collect());
    let next = Rc::new(Cell::new(0usize));
    for _ in 0..BACKGROUND_PARALLEL.min(queue.len()) {
        let (queue, next, ctx) = (queue.clone(), next.clone(), ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let i = next.get();
                next.set(i + 1);
                let Some(f) = queue.get(i) else { break };
                if load(f, BACKGROUND_TIMEOUT_MS).await {
                    ctx.request_repaint();
                }
            }
        });
    }
}

/// Fetch one face and add it to the font database. False (and a console warning) on failure.
async fn load(f: &WebFont, timeout_ms: i32) -> bool {
    let bytes = match fetch(f.url, f.integrity, timeout_ms, None).await {
        Ok(bytes) => bytes,
        Err(e) => {
            log::warn!("web font {} {}: {e}", f.family, f.style);
            return false;
        }
    };
    if FontDb::global().add_font(bytes) == 0 {
        log::warn!("web font {} {}: not a font, or already loaded", f.family, f.style);
        return false;
    }
    true
}

/// GET `url` (relative to the page) with Subresource Integrity `integrity`, aborted after
/// `timeout_ms`.
/// The file at `url`, checked against `integrity` (Subresource Integrity), given up after
/// `timeout_ms`. With `max_bytes`, a larger file is refused before its bytes are copied into the
/// app's memory. The browser has read and checked the whole body by the time the response
/// arrives (it must, to check its integrity), so the download itself is not cut short.
async fn fetch(url: &str, integrity: &str, timeout_ms: i32, max_bytes: Option<usize>) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("no window")?;
    let abort = web_sys::AbortController::new().map_err(js)?;
    let init = web_sys::RequestInit::new();
    init.set_method("GET");
    init.set_integrity(integrity);
    init.set_signal(Some(&abort.signal()));
    let on_timeout = {
        let abort = abort.clone();
        Closure::<dyn FnMut()>::new(move || abort.abort())
    };
    let timer = window.set_timeout_with_callback_and_timeout_and_arguments_0(on_timeout.as_ref().unchecked_ref(), timeout_ms).map_err(js)?;
    let result = async {
        let response: web_sys::Response = JsFuture::from(window.fetch_with_str_and_init(url, &init)).await.map_err(js)?.dyn_into().map_err(js)?;
        if !response.ok() {
            return Err(format!("HTTP {} for {url}", response.status()));
        }
        let buffer = JsFuture::from(response.array_buffer().map_err(js)?).await.map_err(js)?;
        let bytes = js_sys::Uint8Array::new(&buffer);
        if max_bytes.is_some_and(|max| bytes.length() as usize > max) {
            return Err(format!("{url} is larger than {} MB", max_bytes.unwrap_or_default() >> 20));
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
