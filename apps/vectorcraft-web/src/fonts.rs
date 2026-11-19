//! Craft-fonts faces served beside the wasm (`fonts/<sha16>/…`, copied by
//! `cargo xtask web-fonts`) instead of embedded: [`load_startup_fonts`] fetches the `startup`
//! ones before the app starts, [`spawn_background_fonts`] the `background` ones once it runs, and
//! [`want_chars`] a `demand` face (Korean, Devanagari) when the interface language or the
//! interface's own text needs its script. The browser checks each file against its SHA-256
//! (Subresource Integrity). A face that fails is logged and skipped: the app runs without it. With
//! `WEB_FONTS` empty (built without craft-fonts) these do nothing.
//!
//! Fonts a host supplies (RFC-0034): the host gives font indexes in craft-font-index format
//! through the hosting contract's `fonts` message ([`give_indexes`]), each as its bytes or as an
//! https URL with the index's SHA-256; the app lists their families at once. [`fetch_requested`]
//! fetches each face type has asked for (`FontDb::take_source_requests`) from the index's URLs, the
//! best first, with the browser checking it against its SHA-256 (Subresource Integrity), sending no
//! referrer and credentials only to the page's own origin (section 2.4); [`take_fetched`] hands its
//! bytes to the app's next frame, which checks the face against its entry
//! (`FontDb::settle`). At most [`BACKGROUND_PARALLEL`] are fetched at once, and a file
//! larger than its entry says is refused before its bytes are copied into the app's memory.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use vectorcraft_text::craft_fonts::demand_fonts;
use vectorcraft_text::{FontDb, WEB_FONTS, WebFont};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;

use crate::demand::Demand;

/// Per startup face: the page shows its loading text meanwhile.
const STARTUP_TIMEOUT_MS: i32 = 20_000;
/// Per background face.
const BACKGROUND_TIMEOUT_MS: i32 = 120_000;
/// Background fetches at once, leaving the connection to the app's own requests.
const BACKGROUND_PARALLEL: usize = 4;
/// The most indexes one `fonts` message may give, as the hosting contract bounds them
/// (craft-web-host's `MAX_FONT_INDEXES`).
pub const MAX_FONT_INDEXES: usize = 16;

/// A set of font indexes the host gave, read (each `(id, bytes)`), or why it couldn't be.
pub type IndexList = Result<Vec<(String, Vec<u8>)>, String>;

thread_local! {
    /// The background fetch runs once per page, though a graphics restart creates the app again.
    static BACKGROUND_STARTED: Cell<bool> = const { Cell::new(false) };
    /// Faces whose fetch ended and that the app hasn't settled yet: each its file's bytes, or why
    /// it couldn't be fetched ([`take_fetched`]).
    static FETCHED: std::cell::RefCell<Vec<crate::settle::Fetched>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The latest index set a host gave, once the shell has read it (each `(id, bytes)`), or why it
    /// couldn't be read, for the app's next frame ([`take_indexes`]); a set given before it that
    /// is still being read is dropped when it arrives.
    static INDEXES: std::cell::RefCell<crate::latest::Latest<IndexList>> = std::cell::RefCell::new(crate::latest::Latest::default());
    /// Font sources being fetched now (at most [`BACKGROUND_PARALLEL`]).
    static IN_FLIGHT: Cell<usize> = const { Cell::new(0) };
    /// The `demand` faces' fetches.
    static DEMAND: RefCell<Demand> = RefCell::new(Demand::default());
    /// The running app's context, asked for a frame when a `demand` face arrives (none before the
    /// app starts: its first frame looks through the faces loaded by then).
    static REPAINT: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
}

/// Fetch, in the background, the `demand` faces for the scripts of `chars` that are not loaded or
/// being fetched. Each one that arrives asks the app for a frame, and the interface draws those
/// characters with it from then on. Set as the interface's missing-glyph handler
/// (`vectorcraft_ui_egui::on_missing_glyphs`), and called with the interface language's name
/// before the app starts.
pub fn want_chars(chars: &[char]) {
    let wanted = demand_fonts(WEB_FONTS, chars.iter().copied());
    if !wanted.is_empty() {
        fetch_demand(&wanted);
    }
}

/// Fetch the `demand` faces `wanted` (indexes into `WEB_FONTS`) that [`Demand::start`] lets start.
/// A face whose fetch fails is fetched again after the wait [`Demand::finished`] gives.
fn fetch_demand(wanted: &[usize]) {
    for i in DEMAND.with(|d| d.borrow_mut().start(wanted, vectorcraft_ui_egui::now_ms())) {
        let Some(f) = WEB_FONTS.get(i) else { continue };
        wasm_bindgen_futures::spawn_local(async move {
            let ok = load(f, BACKGROUND_TIMEOUT_MS).await;
            let retry = DEMAND.with(|d| d.borrow_mut().finished(i, ok, vectorcraft_ui_egui::now_ms()));
            if ok && let Some(ctx) = REPAINT.with(|r| r.borrow().clone()) {
                ctx.request_repaint();
            }
            if let Some(wait) = retry {
                // A second longer, so a clock the browser coarsens still reads the wait as over.
                crate::api::sleep(wait.saturating_add(1_000)).await;
                fetch_demand(&[i]);
            }
        });
    }
}

/// Fetch, in the background, the faces type has asked for: at most [`BACKGROUND_PARALLEL`] at
/// once, taken from the font database's queue only as fetches are free, so it is the only queue
/// and a new index set prunes it (each fetch's end asks `ctx` for a frame, whose call here takes
/// the next). A fetch's end, done or failed, waits for [`take_fetched`].
pub fn fetch_requested(ctx: &egui::Context) {
    let free = BACKGROUND_PARALLEL.saturating_sub(IN_FLIGHT.get());
    for src in FontDb::global().take_source_requests_up_to(free) {
        IN_FLIGHT.set(IN_FLIGHT.get() + 1);
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            // The index's URLs, the best first: the next after any failure, as the font store
            // tries them.
            let (integrity, size) = (src.integrity(), usize::try_from(src.file.size).unwrap_or(usize::MAX));
            let mut last = String::from("the index gives no URL for it");
            let mut fetched = None;
            for url in &src.urls {
                match fetch(url, &integrity, BACKGROUND_TIMEOUT_MS, Some(size)).await {
                    Ok(bytes) => {
                        fetched = Some(bytes);
                        break;
                    }
                    Err(e) => last = e,
                }
            }
            FETCHED.with(|f| f.borrow_mut().push((src, fetched.ok_or(last))));
            IN_FLIGHT.set(IN_FLIGHT.get().saturating_sub(1));
            ctx.request_repaint();
        });
    }
}

/// Whether the shell asks for another frame to take the faces type queued
/// ([`crate::settle::wants_frame`]): only while a fetch is free.
pub fn wants_frame() -> bool {
    crate::settle::wants_frame(FontDb::global().has_source_requests(), IN_FLIGHT.get(), BACKGROUND_PARALLEL)
}

/// The faces whose fetch ended since the last call, each its file's bytes or why it couldn't be
/// fetched, for [`crate::settle::settle_fetches`].
pub fn take_fetched() -> Vec<crate::settle::Fetched> {
    FETCHED.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

/// One font index a host gives (the hosting contract's `fonts` message): its bytes, or an https
/// URL and the SHA-256 of the file there.
#[allow(dead_code, reason = "the hosting contract's fonts message builds them: the embedding bridge, which a build without one lacks")]
pub enum HostFontIndex {
    Bytes(Vec<u8>),
    Url { url: String, sha256: String },
}

/// The most bytes an index fetched from a URL may have: what the font store reads
/// (`craft_font_store::MAX_INDEX_BYTES`).
const INDEX_MAX_BYTES: usize = craft_font_store::MAX_INDEX_BYTES;

/// The font indexes a host gives (RFC-0034 section 1), its whole set in place of the one it gave
/// before, read for the app's next frame ([`take_indexes`]), all or nothing: an index given by URL
/// is fetched (https only, checked against its SHA-256 by the browser and again here, no referrer,
/// credentials to the page's own origin only), and one that can't be fetched gives nothing of the
/// set. The `n`th index of a set is the host's index `page` (`page-2`, `page-3`, … after it). The
/// last set given wins: one given before it whose fetches end later is dropped. Only the hosting
/// contract calls this (section 7): it is no command.
#[allow(dead_code, reason = "the hosting contract's fonts message calls it: the embedding bridge, which a build without one lacks")]
pub fn give_indexes(list: Vec<HostFontIndex>, ctx: &egui::Context) {
    let generation = INDEXES.with(|q| q.borrow_mut().start());
    if list.is_empty() || list.len() > MAX_FONT_INDEXES {
        INDEXES.with(|q| q.borrow_mut().finish_and_wake(generation, Err(format!("give 1 to {MAX_FONT_INDEXES} font indexes")), ctx));
        return;
    }
    let ctx = ctx.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let mut read = Vec::with_capacity(list.len());
        let mut failed = None;
        for (i, index) in list.into_iter().enumerate() {
            let id = if i == 0 { "page".to_string() } else { format!("page-{}", i + 1) };
            let bytes = match index {
                HostFontIndex::Bytes(bytes) => Ok(bytes),
                HostFontIndex::Url { url, sha256 } => fetch_index(&url, &sha256).await,
            };
            match bytes {
                Ok(bytes) => read.push((id, bytes)),
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        INDEXES.with(|q| q.borrow_mut().finish_and_wake(generation, failed.map_or(Ok(read), Err), &ctx));
    });
}

/// The index at `url`, an https URL, whose file must have the SHA-256 `sha256` (64 lowercase hex
/// digits).
async fn fetch_index(url: &str, sha256: &str) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err(format!("a font index's URL is https: {url}"));
    }
    let digest: Option<Vec<u8>> = (sha256.len() == 64 && sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .then(|| (0..32).filter_map(|i| u8::from_str_radix(sha256.get(2 * i..2 * i + 2)?, 16).ok()).collect());
    let digest = digest.ok_or_else(|| format!("the SHA-256 of {url} is 64 lowercase hexadecimal digits"))?;
    let integrity = format!("sha256-{}", vectorcraft_format::base64_encode(&digest));
    let bytes = fetch(url, &integrity, BACKGROUND_TIMEOUT_MS, Some(INDEX_MAX_BYTES)).await?;
    if craft_font_store::sha256_hex(&bytes) != sha256 {
        return Err(format!("{url} doesn't have the SHA-256 given"));
    }
    Ok(bytes)
}

/// The latest index set the host gave ([`give_indexes`]), once it is read, or why it couldn't be.
pub fn take_indexes() -> Option<IndexList> {
    INDEXES.with(|q| q.borrow_mut().take())
}

/// Fetch and register the `startup` faces (one after the other). Returns how many loaded.
pub async fn load_startup_fonts() -> usize {
    let mut loaded = 0;
    for f in WEB_FONTS.iter().filter(|f| f.startup && !f.demand) {
        if load(f, STARTUP_TIMEOUT_MS).await {
            loaded += 1;
        }
    }
    loaded
}

/// Fetch and register the `background` faces, [`BACKGROUND_PARALLEL`] at a time, asking `ctx` for
/// a frame after each so the font menus and the canvas pick it up. Only the first call fetches
/// anything; each call makes `ctx` the context a `demand` face asks for a frame.
pub fn spawn_background_fonts(ctx: egui::Context) {
    REPAINT.with(|r| *r.borrow_mut() = Some(ctx.clone()));
    if BACKGROUND_STARTED.with(|s| s.replace(true)) {
        return;
    }
    let queue: Rc<Vec<&'static WebFont>> = Rc::new(WEB_FONTS.iter().filter(|f| !f.startup && !f.demand).collect());
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
    // A fetch tells nothing about the document or the user beyond its URL (RFC-0034 section 2.4):
    // no referrer, and credentials only to the page's own origin.
    init.set_referrer_policy(web_sys::ReferrerPolicy::NoReferrer);
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
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
            return Err(format!("{url} is larger than {} bytes", max_bytes.unwrap_or_default()));
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
