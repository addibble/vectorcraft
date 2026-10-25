//! Link sources: linked files a host can supply, for a build that reads no files (the web).
//!
//! A linked image shows its saved preview until its file is read ([`crate::cmd::links`]); on the
//! desktop the file is read when the document opens and for output. A page that embeds the web
//! build has the files but no file system to give them through, so it lists them as sources
//! (`links.addSources`): the path a link names and a URL the file can be fetched from at a size.
//! VectorCraft asks for a file once an image linking it is drawn magnified past its preview, at
//! the size its largest placement needs at the sources' resolution (the host's output resolution,
//! 300 ppi unless it says otherwise), never more than the file has; resized larger, it is asked
//! for again. What arrives (`links.supplySource`) is kept outside the document
//! ([`vectorcraft_render::link_pixels`], least recently drawn dropped first), drawn over the
//! preview, and used for PDF export ([`for_output`]); it is not an edit. `links.loadSources` asks
//! for every linked image of the document at once, as a host does before printing.
//!
//! Illustrator draws linked images from a low-resolution preview and reads the file for output,
//! and has no preference for the preview's resolution
//! (<https://community.adobe.com/t5/illustrator-discussions/linked-file-preview-resolution/m-p/14871637>):
//! the resolution here is the host's.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::Affine;
use vectorcraft_render::link_pixels::{self, Supplied};

use crate::Session;

/// The resolution a source is asked for at, unless the host says otherwise (pixels per inch).
pub const DEFAULT_RESOLUTION: f64 = 300.0;

/// Points per inch.
const POINTS_PER_INCH: f64 = 72.0;

/// The URL a source's file is fetched from: `{width}` and `{height}` in it are replaced by the
/// pixel size asked for, and `{side}` by the longer of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSource {
    pub url: String,
}

/// A file to fetch for a link source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRequest {
    /// The path the links name.
    pub path: String,
    /// The URL, its size filled in.
    pub url: String,
    /// The pixel size asked for.
    pub width: u32,
    pub height: u32,
}

#[derive(Default)]
struct Sources {
    /// By the path a link names (its relative path, else its path).
    by_path: BTreeMap<String, LinkSource>,
    resolution: Option<f64>,
    /// Paths asked for and not supplied yet, with the width asked for.
    pending: HashMap<String, u32>,
    /// Paths supplied, with the width last asked for: their pixels are what that asked for (a
    /// file smaller than asked comes as it is).
    answered: HashMap<String, u32>,
    /// Paths whose fetch failed, and why: not asked for again until given again.
    failed: HashMap<String, String>,
    /// Image keys `links.loadSources` asked for, wanted whether drawn or not.
    wanted: BTreeSet<String>,
}

static SOURCES: LazyLock<Mutex<Sources>> = LazyLock::new(Mutex::default);

/// The process's link sources (usable after a panic elsewhere while locked: each change is
/// complete when made).
fn sources() -> MutexGuard<'static, Sources> {
    SOURCES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The source of image `im`, by its relative path, else its path.
fn source_of<'a>(s: &'a Sources, im: &ImageObject) -> Option<(&'a str, &'a LinkSource)> {
    let link = im.link.as_ref()?;
    link.relative.iter().chain(std::iter::once(&link.path)).find_map(|p| s.by_path.get_key_value(p.as_str())).map(|(k, v)| (k.as_str(), v))
}

/// Give the sources the host can supply, by the path links name; a path already a source takes
/// the new URL (asked for again from it), and one that failed is asked for again. `resolution` (ppi), when given, is the
/// most any placement is asked for at from now on. Returns how many paths are new.
pub fn add(list: Vec<(String, LinkSource)>, resolution: Option<f64>) -> usize {
    let mut s = sources();
    if resolution.is_some() {
        s.resolution = resolution;
    }
    let mut added = 0;
    for (path, source) in list {
        s.failed.remove(&path);
        match s.by_path.insert(path.clone(), source.clone()) {
            None => added += 1,
            // Another URL may serve other pixels: what came from the old one is asked for again.
            Some(old) if old != source => {
                s.answered.remove(&path);
            }
            Some(_) => {}
        }
    }
    added
}

/// How many paths are sources, and the resolution they are asked for at.
pub fn summary() -> (usize, f64) {
    let s = sources();
    (s.by_path.len(), s.resolution.unwrap_or(DEFAULT_RESOLUTION))
}

/// The scale a pixel size asked for is rounded up to: steps of √2, so a small resize is still
/// covered and sizes asked for repeat; at most 1 (the file itself).
fn stepped(scale: f64) -> f64 {
    if !scale.is_finite() || scale >= 1.0 {
        return 1.0;
    }
    let step = (scale.max(1e-6).log2() * 2.0).ceil() / 2.0;
    2f64.powf(step).min(1.0)
}

/// For each linked image of `d`, by key: its file's pixel size and the scale (pixels per file
/// pixel, at most 1) its largest placement needs at `resolution`: its transform composed with each
/// symbol instance it is drawn through. Every layer counts, hidden and template ones too.
fn needs(d: &Document, resolution: f64) -> HashMap<String, ((u32, u32), f64)> {
    let mut out = HashMap::new();
    for layer in &d.layers {
        needs_in(d, layer, Affine::IDENTITY, resolution, &mut vec![], &mut out);
    }
    out
}

fn needs_in<'a>(
    d: &'a Document,
    n: &'a Node,
    placed: Affine,
    resolution: f64,
    expanding: &mut Vec<&'a str>,
    out: &mut HashMap<String, ((u32, u32), f64)>,
) {
    if let Some(mask) = &n.extras().mask {
        needs_in(d, &mask.art, placed, resolution, expanding, out);
    }
    match &n.kind {
        NodeKind::Image(im) if im.link.is_some() => {
            let [a, b, c, dd, _, _] = (placed * im.xf).as_coeffs();
            // Points per file pixel along the image's axes; the larger decides.
            let pt = a.hypot(b).max(c.hypot(dd));
            let scale = pt / POINTS_PER_INCH * resolution;
            if scale.is_finite() {
                let e = out.entry(im.key.clone()).or_insert(((im.width, im.height), 0.0));
                e.1 = e.1.max(scale.min(1.0));
            }
        }
        NodeKind::SymbolInstance { symbol, xf } => {
            let Some(sym) = d.symbols.iter().find(|s| &s.name == symbol) else { return };
            if expanding.contains(&sym.name.as_str()) {
                return;
            }
            expanding.push(&sym.name);
            needs_in(d, &sym.art, placed * *xf, resolution, expanding, out);
            expanding.pop();
        }
        _ => {
            for c in n.children().into_iter().flatten() {
                needs_in(d, c, placed, resolution, expanding, out);
            }
        }
    }
}

/// Are image `key`'s pixels, from `path`, kept and what `width` asks for (or more)?
fn covered(s: &Sources, key: &str, path: &str, width: u32) -> bool {
    s.answered.get(path).is_some_and(|asked| *asked >= width) && link_pixels::peek(key).is_some()
}

/// The URL of `source` for `width` × `height` pixels.
fn url_for(source: &LinkSource, width: u32, height: u32) -> String {
    source.url.replace("{width}", &width.to_string()).replace("{height}", &height.to_string()).replace("{side}", &width.max(height).to_string())
}

/// The files to fetch now: for each linked image with a source that was drawn magnified past its
/// preview since the last call (or that `links.loadSources` asked for) and whose supplied pixels
/// are fewer than its placements need, its file at the size they need; each path once until it is
/// supplied or fails, and again when a larger size is needed.
pub fn requests(session: &Session) -> Vec<SourceRequest> {
    let drawn = link_pixels::take_drawn();
    let mut s = sources();
    if s.by_path.is_empty() {
        return vec![];
    }
    let resolution = s.resolution.unwrap_or(DEFAULT_RESOLUTION);
    let wanted: BTreeSet<String> = drawn.into_iter().chain(s.wanted.iter().cloned()).collect();
    if wanted.is_empty() {
        return vec![];
    }
    // What each wanted image needs, the most any open document asks.
    let mut need: HashMap<String, ((u32, u32), f64)> = HashMap::new();
    let mut paths: HashMap<String, (String, LinkSource)> = HashMap::new();
    for st in session.documents() {
        let found = needs(&st.doc, resolution);
        st.doc.visit_images(|_, im| {
            if !wanted.contains(&im.key) || paths.contains_key(&im.key) {
                return;
            }
            if let Some((path, source)) = source_of(&s, im) {
                paths.insert(im.key.clone(), (path.to_string(), source.clone()));
            }
        });
        for (key, (file, scale)) in found {
            if wanted.contains(&key) {
                let e = need.entry(key).or_insert((file, 0.0));
                e.1 = e.1.max(scale);
            }
        }
    }
    let mut out = vec![];
    for (key, (path, source)) in paths {
        let Some(&((w, h), scale)) = need.get(&key) else { continue };
        if w == 0 || h == 0 || s.failed.contains_key(&path) {
            continue;
        }
        let scale = stepped(scale);
        let width = ((f64::from(w) * scale).ceil() as u32).clamp(1, w);
        let height = ((f64::from(h) * scale).ceil() as u32).clamp(1, h);
        if covered(&s, &key, &path, width) {
            s.wanted.remove(&key);
            continue;
        }
        if s.pending.get(&path).is_some_and(|asked| *asked >= width) {
            continue;
        }
        s.pending.insert(path.clone(), width);
        out.push(SourceRequest { url: url_for(&source, width, height), path, width, height });
    }
    out
}

/// Were linked images drawn larger than their previews since [`requests`] last looked? (A web
/// shell asks for another frame then, whose [`requests`] fetches them.)
pub fn drawn_since() -> bool {
    link_pixels::has_drawn()
}

/// The most bytes one supplied file may have.
pub const MAX_SUPPLIED: usize = 256 << 20;

/// The pixel size and MIME type of the image `bytes` hold, from their header alone.
fn header(bytes: &[u8]) -> Result<((u32, u32), &'static str), String> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| format!("the file does not read: {e}"))?;
    let format = reader.format().ok_or("the file is no picture format VectorCraft reads")?;
    let size = reader.into_dimensions().map_err(|e| format!("the picture's header does not read: {e}"))?;
    if size.0 == 0 || size.1 == 0 {
        return Err("the picture has no pixels".into());
    }
    Ok((size, format.to_mime_type()))
}

/// The file at `path` as a source supplied it: kept for every linked image of the open documents
/// that links it, drawn over its preview and used for output. Returns how many images (by key)
/// it is kept for and its pixel size. Not an edit.
pub fn supply(session: &mut Session, path: &str, bytes: Vec<u8>) -> Result<(usize, (u32, u32)), String> {
    if bytes.len() > MAX_SUPPLIED {
        return Err(format!("{path} is larger than {} MB", MAX_SUPPLIED >> 20));
    }
    let checked = header(&bytes);
    let mut s = sources();
    let asked = s.pending.remove(path);
    let (size, mime) = match checked {
        Ok(found) => found,
        Err(e) => {
            s.failed.insert(path.to_string(), e.clone());
            return Err(format!("{path}: {e}"));
        }
    };
    let blob = ImageBlob::new(mime, bytes);
    let mut keys = BTreeSet::new();
    for st in &mut session.docs {
        let mut shows = false;
        st.doc.visit_images(|_, im| {
            let link = im.link.as_ref();
            if link.is_some_and(|l| l.relative.as_deref() == Some(path) || l.path == path) {
                keys.insert(im.key.clone());
                shows = true;
            }
        });
        if shows {
            // Draw again; nothing in the document changed.
            st.revision += 1;
        }
    }
    for key in &keys {
        link_pixels::insert(key, Supplied { size, blob: blob.clone() });
        s.wanted.remove(key);
    }
    // Supplied unasked (by hand), it answers for its own width.
    s.answered.insert(path.to_string(), asked.unwrap_or(size.0).max(size.0));
    Ok((keys.len(), size))
}

/// The fetch of `path`'s file failed: it is not asked for again until the host gives its source
/// again.
pub fn failed(path: &str, reason: &str) {
    let mut s = sources();
    s.pending.remove(path);
    s.failed.insert(path.to_string(), reason.to_string());
}

/// How the active document's linked images with sources stand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Loading {
    /// Images whose supplied pixels are what their placements need.
    pub loaded: usize,
    /// Images still to be supplied.
    pub loading: usize,
    /// Images whose source failed.
    pub failed: usize,
}

/// Ask for every linked image of `d` that has a source and lacks the pixels its placements need,
/// drawn or not (a host does this before printing); tells how they stand.
pub fn load_all(d: &Document) -> Loading {
    let mut s = sources();
    let resolution = s.resolution.unwrap_or(DEFAULT_RESOLUTION);
    let need = needs(d, resolution);
    let mut seen = BTreeSet::new();
    let mut out = Loading::default();
    let mut images = vec![];
    d.visit_images(|_, im| {
        if seen.insert(im.key.clone()) {
            images.push(im.clone());
        }
    });
    for im in images {
        let Some((path, _)) = source_of(&s, &im) else { continue };
        let path = path.to_string();
        if !d.images.get(&im.key).is_some_and(ImageBlob::is_proxy) {
            continue;
        }
        let Some(&((w, _), scale)) = need.get(&im.key) else { continue };
        let scale = stepped(scale);
        let width = ((f64::from(w) * scale).ceil() as u32).clamp(1, w.max(1));
        if s.failed.contains_key(&path) {
            out.failed += 1;
        } else if covered(&s, &im.key, &path, width) {
            out.loaded += 1;
        } else {
            s.wanted.insert(im.key.clone());
            out.loading += 1;
        }
    }
    out
}

/// `d` for output: each linked image whose bytes are its preview shows the pixels a source
/// supplied for it, when there are any (the preview stays its preview). Images without them stay
/// as they are.
pub fn for_output(d: &Document) -> Cow<'_, Document> {
    let swap: Vec<(String, Arc<Supplied>)> =
        d.images.iter().filter(|(_, b)| b.is_proxy()).filter_map(|(k, _)| link_pixels::peek(k).map(|p| (k.clone(), p))).collect();
    if swap.is_empty() {
        return Cow::Borrowed(d);
    }
    let mut out = d.clone();
    for (key, supplied) in swap {
        let preview = out.images.get(&key).map(|b| b.bytes.clone());
        out.images.insert(key, ImageBlob { mime: supplied.blob.mime.clone(), bytes: supplied.blob.bytes.clone(), proxy: preview });
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_step_up_by_root_two_to_at_most_one() {
        assert_eq!(stepped(1.5), 1.0);
        assert_eq!(stepped(0.5), 0.5);
        assert!((stepped(0.6) - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-9);
        assert_eq!(stepped(f64::NAN), 1.0);
    }

    #[test]
    fn a_source_s_url_takes_the_size_asked_for() {
        let s = LinkSource { url: "p.png?w={width}&h={height}&s={side}".into() };
        assert_eq!(url_for(&s, 30, 40), "p.png?w=30&h=40&s=40");
    }
}
