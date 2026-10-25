//! Pixels of linked files a host supplied (link sources), shared by every renderer in the process.
//!
//! A linked image shows its saved preview until its file is read. Where there is no file to read
//! (the web build), a host can supply the file's pixels, at most at the size its placements print
//! at (`links.addSources`). The renderer draws the preview, then the supplied pixels over it, so
//! pixels arriving or being dropped never leave a hole. They are display and output only: no
//! document holds them, so nothing saves, journals or undoes them, and undo never brings back a
//! preview.
//!
//! Supplied pixels are kept per image key (the hash of the file's bytes, the same in every
//! document that links the file). Their encoded bytes are held within [`budget`] bytes, the least
//! recently drawn dropped first; their decoded pixels live in the shared image store, under its
//! own budget. Dropped pixels draw the preview again until the host supplies them again.
//!
//! Linked images drawn magnified past their preview's pixels are noted ([`take_drawn`]): what the
//! engine asks link sources for, so only what shows at a size the preview can't fill is fetched.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use vectorcraft_doc::ImageBlob;

/// The default budget of the encoded bytes of supplied pixels.
#[cfg(not(target_arch = "wasm32"))]
const DEFAULT_BUDGET: usize = 512 << 20;
/// The web's (a tab's memory is tight).
#[cfg(target_arch = "wasm32")]
const DEFAULT_BUDGET: usize = 128 << 20;

/// A linked file's pixels as a host supplied them, possibly smaller than the file.
#[derive(Clone, Debug)]
pub struct Supplied {
    /// The pixels' size (width, height).
    pub size: (u32, u32),
    /// The encoded pixels.
    pub blob: ImageBlob,
}

impl Supplied {
    /// Pixels per file pixel for a file of `file` pixels (width, height), the smaller of the two axes.
    pub fn scale(&self, file: (u32, u32)) -> f64 {
        (self.size.0 as f64 / file.0.max(1) as f64).min(self.size.1 as f64 / file.1.max(1) as f64)
    }
}

struct Entry {
    pixels: Arc<Supplied>,
    used: u64,
}

struct Store {
    by_key: HashMap<String, Entry>,
    bytes: usize,
    budget: usize,
    tick: u64,
    /// Linked images drawn magnified past their preview since the last [`take_drawn`].
    drawn: BTreeSet<String>,
}

impl Store {
    /// Drop the least recently drawn while over budget.
    fn evict(&mut self) {
        if self.bytes <= self.budget {
            return;
        }
        let mut all: Vec<(u64, String)> = self.by_key.iter().map(|(k, e)| (e.used, k.clone())).collect();
        all.sort_unstable_by_key(|(used, _)| *used);
        for (_, k) in all {
            if self.bytes <= self.budget {
                break;
            }
            if let Some(e) = self.by_key.remove(&k) {
                self.bytes = self.bytes.saturating_sub(e.pixels.blob.bytes.len());
                GENERATION.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Moves whenever the pixels kept change ([`generation`]).
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Changes whenever supplied pixels arrive or are dropped: a screen showing linked images draws
/// again when it does.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

static STORE: LazyLock<Mutex<Store>> =
    LazyLock::new(|| Mutex::new(Store { by_key: HashMap::new(), bytes: 0, budget: DEFAULT_BUDGET, tick: 0, drawn: BTreeSet::new() }));

/// The process's supplied pixels (usable after a panic elsewhere while locked: each change is
/// complete when made).
fn store() -> MutexGuard<'static, Store> {
    STORE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Keep `pixels` for image `key`, in place of what it had; then drop the least recently drawn
/// while over budget (these too, when they alone are past it).
pub fn insert(key: &str, pixels: Supplied) {
    let mut s = store();
    s.tick += 1;
    let used = s.tick;
    let size = pixels.blob.bytes.len();
    if let Some(old) = s.by_key.insert(key.to_string(), Entry { pixels: Arc::new(pixels), used }) {
        s.bytes = s.bytes.saturating_sub(old.pixels.blob.bytes.len());
    }
    s.bytes += size;
    GENERATION.fetch_add(1, Ordering::Relaxed);
    s.evict();
}

/// The pixels to draw image `key` with, marked as just drawn.
pub fn get(key: &str) -> Option<Arc<Supplied>> {
    let mut s = store();
    s.tick += 1;
    let tick = s.tick;
    let e = s.by_key.get_mut(key)?;
    e.used = tick;
    Some(e.pixels.clone())
}

/// The pixels kept for image `key`, without marking them drawn.
pub fn peek(key: &str) -> Option<Arc<Supplied>> {
    store().by_key.get(key).map(|e| e.pixels.clone())
}

/// Note that linked image `key` was drawn magnified past its preview's pixels.
pub(crate) fn note_drawn(key: &str) {
    let mut s = store();
    if !s.drawn.contains(key) {
        s.drawn.insert(key.to_string());
    }
}

/// Were linked images drawn magnified past their previews since [`take_drawn`] was last called?
pub fn has_drawn() -> bool {
    !store().drawn.is_empty()
}

/// The linked images drawn magnified past their previews since the last call.
pub fn take_drawn() -> BTreeSet<String> {
    std::mem::take(&mut store().drawn)
}

/// The encoded bytes kept now.
pub fn bytes() -> usize {
    store().bytes
}

/// The most encoded bytes kept.
pub fn budget() -> usize {
    store().budget
}

/// Keep at most `bytes` of encoded pixels, dropping the least recently drawn beyond it.
pub fn set_budget(bytes: usize) {
    let mut s = store();
    s.budget = bytes;
    s.evict();
}

/// Drop the pixels kept for image `key`.
pub fn forget(key: &str) {
    let mut s = store();
    if let Some(e) = s.by_key.remove(key) {
        s.bytes = s.bytes.saturating_sub(e.pixels.blob.bytes.len());
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supplied(size: (u32, u32), bytes: usize) -> Supplied {
        Supplied { size, blob: ImageBlob::new("image/png", vec![0; bytes]) }
    }

    #[test]
    fn supplied_pixels_scale_by_the_smaller_axis() {
        assert_eq!(supplied((50, 30), 1).scale((100, 50)), 0.5);
    }

    #[test]
    fn the_least_recently_drawn_go_over_budget() {
        let old = budget();
        set_budget(1000);
        for k in ["lp1", "lp2", "lp3"] {
            forget(k);
        }
        insert("lp1", supplied((10, 10), 300));
        insert("lp1", supplied((20, 20), 400));
        assert_eq!(peek("lp1").map(|p| p.size), Some((20, 20)), "the newer pixels replace the older");
        insert("lp2", supplied((10, 10), 400));
        assert!(get("lp1").is_some(), "drawn again: now the most recent");
        insert("lp3", supplied((10, 10), 400));
        // 1200 > 1000: lp2, drawn least lately, goes.
        assert!(peek("lp2").is_none());
        assert!(peek("lp1").is_some() && peek("lp3").is_some());
        set_budget(old);
        forget("lp1");
        forget("lp3");
    }
}
