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
//! Sources are the process's (the editor's), not a document's: a path names the same file in every
//! open document, so a host gives each file a path of its own (content-addressed, say) and never
//! one path two files.
//!
//! Illustrator draws linked images from a low-resolution preview and reads the file for output,
//! and has no preference for the preview's resolution
//! (<https://community.adobe.com/t5/illustrator-discussions/linked-file-preview-resolution/m-p/14871637>):
//! the resolution here is the host's.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::Affine;
use vectorcraft_render::link_pixels::{self, Supplied};

use crate::Session;

/// The resolution a source is asked for at, unless the host says otherwise (pixels per inch).
pub const DEFAULT_RESOLUTION: f64 = 300.0;

/// Points per inch.
const POINTS_PER_INCH: f64 = 72.0;

/// The most paths that may be sources at once.
pub const MAX_SOURCES: usize = 100_000;

/// The URL a source's file is fetched from: `{width}` and `{height}` in it are replaced by the
/// pixel size asked for, and `{side}` by the longer of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSource {
    pub url: String,
    /// The file's XMP sidecar, as text, when the host has one: what a `<name>.xmp` beside the file
    /// would hold. Its main subject (an IPTC image region) is the link's, read as a sidecar on disk
    /// is ([`crate::cmd::fileio::regions`]).
    pub sidecar: Option<String>,
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

/// What a path's supplied pixels are.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Kept {
    /// Their width in pixels: what they answer for.
    width: u32,
    /// Their encoded size.
    size: usize,
    /// The image keys they are kept for: dropped when the path's source moves.
    keys: BTreeSet<String>,
}

/// Where a path's file stands. Pixels kept from an earlier fetch (`kept`) are drawn and printed
/// until the source moves, whatever comes after them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum State {
    /// Nothing asked for, nothing kept.
    #[default]
    Idle,
    /// Asked for at `width` from `url` (the request a fetch is credited to, not a later one).
    Pending { url: String, width: u32, kept: Option<Kept> },
    /// Supplied.
    Kept(Kept),
    /// Pixels answering for `width`, of `size` encoded bytes, the linked budget refused: not asked
    /// for again at that width while they would not fit.
    Refused { width: u32, size: usize, kept: Option<Kept> },
    /// The fetch failed, or what it brought was refused: not asked for again until the source is
    /// given again or supplied after all.
    Failed { reason: String, kept: Option<Kept> },
}

impl State {
    fn kept(&self) -> Option<&Kept> {
        match self {
            State::Idle => None,
            State::Kept(k) => Some(k),
            State::Pending { kept, .. } | State::Refused { kept, .. } | State::Failed { kept, .. } => kept.as_ref(),
        }
    }

    /// The state with nothing asked for and `kept` kept.
    fn settled(kept: Option<Kept>) -> Self {
        kept.map_or(State::Idle, State::Kept)
    }
}

/// A path that is a source: where its file is, and how it stands.
struct Source {
    link: LinkSource,
    state: State,
}

#[derive(Default)]
struct Sources {
    /// By the path a link names (its relative path, else its path).
    by_path: BTreeMap<String, Source>,
    resolution: Option<f64>,
    /// What `links.loadSources` asked for, handed out by the next [`requests`].
    outbox: Vec<SourceRequest>,
    /// Image keys drawn larger than their previews whose link had no source then, or whose source
    /// failed: looked at again once sources are given ([`requests`]). At most [`MAX_SOURCES`].
    waiting: BTreeSet<String>,
    /// Have sources been given since [`Sources::waiting`] was looked at?
    given: bool,
}

static SOURCES: LazyLock<Mutex<Sources>> = LazyLock::new(Mutex::default);

/// Moves whenever what the sources say of a link changes ([`generation`]).
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Changes whenever sources are given, supplied or fail: a panel showing whether links are found
/// looks again when it does. Not a document's revision: nothing in a document changes.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

fn moved() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// The process's link sources (usable after a panic elsewhere while locked: each change is
/// complete when made).
fn sources() -> MutexGuard<'static, Sources> {
    SOURCES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The path of the source of image `im`, by its relative path, else its path.
fn source_of<'a>(s: &'a Sources, im: &ImageObject) -> Option<&'a str> {
    let link = im.link.as_ref()?;
    link.relative.iter().chain(std::iter::once(&link.path)).find_map(|p| s.by_path.get_key_value(p.as_str())).map(|(k, _)| k.as_str())
}

/// The pixels `kept` was are no longer drawn or printed: its images show their previews.
fn let_go(kept: Option<&Kept>) {
    for key in kept.into_iter().flat_map(|k| &k.keys) {
        link_pixels::forget(key);
    }
}

/// Give the sources the host can supply, by the path links name; a path already a source takes
/// the new URL (asked for again from it, the pixels the old one supplied dropped at once and what
/// is on its way from it dropped when it comes), and one that failed is asked for again.
/// `resolution` (ppi), when given, is the most any placement is asked for at from now on. Returns
/// how many paths are new; refused, adding none, when the sources would pass [`MAX_SOURCES`].
pub fn add(list: Vec<(String, LinkSource)>, resolution: Option<f64>) -> Result<usize, String> {
    let mut s = sources();
    let new: BTreeSet<&str> = list.iter().map(|(p, _)| p.as_str()).filter(|p| !s.by_path.contains_key(*p)).collect();
    if s.by_path.len() + new.len() > MAX_SOURCES {
        return Err(format!("there may be at most {MAX_SOURCES} sources"));
    }
    if resolution.is_some() {
        s.resolution = resolution;
    }
    let mut added = 0;
    for (path, link) in list {
        let Some(src) = s.by_path.get_mut(&path) else {
            s.by_path.insert(path, Source { link, state: State::Idle });
            added += 1;
            continue;
        };
        if src.link.url != link.url {
            // Another URL may serve other pixels: what came from the old one goes (a new sidecar
            // alone changes no pixels).
            let_go(src.state.kept());
            src.state = State::Idle;
        } else if let State::Failed { kept, .. } = &mut src.state {
            src.state = State::settled(kept.take());
        }
        src.link = link;
    }
    s.given = true;
    moved();
    Ok(added)
}

/// Does the host supply the file `link` names (a source by its relative path or its path, whose
/// fetch has not failed)? The Links panel shows such a link as found.
pub fn sourced(link: &vectorcraft_doc::LinkInfo) -> bool {
    let s = sources();
    link.relative
        .iter()
        .chain(std::iter::once(&link.path))
        .any(|p| s.by_path.get(p.as_str()).is_some_and(|src| !matches!(src.state, State::Failed { .. })))
}

/// The main subject the sidecar the host gave for the file `link` names (a source by its relative
/// path or its path), as fractions of the file's `pixels` (width, height: for regions in pixels).
/// A supplied file is upright ([`supply`]), so its stored pixels are the ones it shows.
pub fn subject(link: &vectorcraft_doc::LinkInfo, pixels: (u32, u32)) -> Option<vectorcraft_doc::links::ImageRegion> {
    let s = sources();
    let sidecar = link.relative.iter().chain(std::iter::once(&link.path)).find_map(|p| s.by_path.get(p.as_str()))?.link.sidecar.as_ref()?;
    crate::cmd::fileio::regions::main_subject_in(sidecar.as_bytes(), pixels)
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
/// symbol instance it is drawn through. Every layer counts, hidden and template ones too, and the
/// art of patterns and graph designs, at its own size (a fill scales a pattern as it paints).
fn needs(d: &Document, resolution: f64) -> HashMap<String, ((u32, u32), f64)> {
    let mut out = HashMap::new();
    let roots = d.layers.iter().chain(d.patterns.iter().flat_map(|p| &p.art)).chain(d.graph_designs.iter().map(|g| &g.art));
    for root in roots {
        needs_in(d, root, Affine::IDENTITY, resolution, &mut vec![], &mut out);
    }
    out
}

/// The pixel size of the preview image `key` of `d` shows while its file is unread.
fn preview_size(d: &Document, key: &str) -> Option<(u32, u32)> {
    let blob = d.images.get(key).filter(|b| b.is_proxy())?;
    image::ImageReader::new(std::io::Cursor::new(&blob.bytes[..])).with_guessed_format().ok()?.into_dimensions().ok()
}

/// The pixel size to ask for of a file of `file` pixels whose placements need `scale` of them:
/// rounded up a step ([`stepped`]), never fewer than its preview has (`preview`: what is drawn
/// meanwhile, which fewer pixels would blur), never more than the file has.
fn size_for(file: (u32, u32), scale: f64, preview: Option<(u32, u32)>) -> (u32, u32) {
    let (w, h) = file;
    let least = preview.map_or(0.0, |(pw, ph)| (f64::from(pw) / f64::from(w.max(1))).max(f64::from(ph) / f64::from(h.max(1))));
    let scale = stepped(scale).max(least).min(1.0);
    (((f64::from(w) * scale).ceil() as u32).clamp(1, w.max(1)), ((f64::from(h) * scale).ceil() as u32).clamp(1, h.max(1)))
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

/// Do pixels kept at `kept`'s width answer for `width`? To the pixel: a host that rounds the other
/// side of a size it is asked for answers too.
fn covers(kept: &Kept, width: u32) -> bool {
    kept.width.saturating_add(1) >= width
}

/// The URL of `source` for `width` × `height` pixels.
fn url_for(source: &LinkSource, width: u32, height: u32) -> String {
    source.url.replace("{width}", &width.to_string()).replace("{height}", &height.to_string()).replace("{side}", &width.max(height).to_string())
}

/// Ask for `path`'s file at `width` × `height`, unless what is kept or on its way answers for it,
/// or its source failed; with `again`, also when the pixels kept, or refused, for it went for the
/// linked budget (the caller found room for them). The request, when one is made.
fn ask(s: &mut Sources, path: &str, width: u32, height: u32, again: bool) -> Option<SourceRequest> {
    let src = s.by_path.get_mut(path)?;
    let kept = match &src.state {
        State::Failed { .. } => return None,
        State::Pending { width: w, .. } if *w >= width => return None,
        State::Kept(k) if covers(k, width) && !again => return None,
        State::Refused { width: w, .. } if w.saturating_add(1) >= width && !again => return None,
        state => state.kept().cloned(),
    };
    let url = url_for(&src.link, width, height);
    src.state = State::Pending { url: url.clone(), width, kept };
    Some(SourceRequest { path: path.to_string(), url, width, height })
}

/// The files to fetch now: those `links.loadSources` asked for, and for each linked image with a
/// source that was drawn magnified past its preview since the last call (or before its source was
/// given) and whose supplied pixels are fewer than its placements need, its file at the size they
/// need; each path once until it is supplied or fails, and again when a larger size is needed.
/// Pixels the linked budget dropped are not asked for again by being drawn (each fetch could drop
/// another, forever): `links.loadSources` asks for them when they fit.
pub fn requests(session: &Session) -> Vec<SourceRequest> {
    let mut keys = link_pixels::take_drawn();
    let mut s = sources();
    // What loadSources asked for, unless a new URL or a larger request has overtaken it.
    let asked = std::mem::take(&mut s.outbox);
    let mut out: Vec<SourceRequest> = asked
        .into_iter()
        .filter(|r| matches!(s.by_path.get(&r.path).map(|src| &src.state), Some(State::Pending { url, .. }) if *url == r.url))
        .collect();
    // Those drawn before their source came, looked at again once sources are given.
    if std::mem::take(&mut s.given) {
        keys.append(&mut std::mem::take(&mut s.waiting));
    }
    if keys.is_empty() {
        return out;
    }
    let resolution = s.resolution.unwrap_or(DEFAULT_RESOLUTION);
    // What each image drawn needs, the most any open document asks, and its source's path.
    let mut need: HashMap<String, ((u32, u32), f64)> = HashMap::new();
    let mut paths: HashMap<String, String> = HashMap::new();
    let mut shown: BTreeSet<String> = BTreeSet::new();
    for st in session.documents() {
        st.doc.visit_images(|_, im| {
            if !keys.contains(&im.key) || paths.contains_key(&im.key) {
                return;
            }
            shown.insert(im.key.clone());
            if let Some(path) = source_of(&s, im) {
                paths.insert(im.key.clone(), path.to_string());
            }
        });
        for (key, (file, scale)) in needs(&st.doc, resolution) {
            if keys.contains(&key) {
                let e = need.entry(key).or_insert((file, 0.0));
                e.1 = e.1.max(scale);
            }
        }
    }
    // Drawn images an open document shows without a source, or whose source failed, wait for one.
    for key in shown {
        let failed = paths.get(&key).is_none_or(|p| matches!(s.by_path.get(p).map(|src| &src.state), Some(State::Failed { .. })));
        if failed && s.waiting.len() < MAX_SOURCES {
            s.waiting.insert(key);
        }
    }
    for (key, path) in paths {
        let Some(&((w, h), scale)) = need.get(&key) else { continue };
        if w == 0 || h == 0 {
            continue;
        }
        let preview = session.documents().iter().find_map(|st| preview_size(&st.doc, &key));
        let (width, height) = size_for((w, h), scale, preview);
        out.extend(ask(&mut s, &path, width, height, false));
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

/// The image `bytes` hold, kept as they are, and its pixel size: a format the image decoder reads
/// (PNG, JPEG, GIF, WebP, TIFF, BMP), not one Open converts first (PSD, TGA, HEIC, JPEG XL, ...). A picture whose EXIF orientation
/// turns or flips it is refused: a supplied file is drawn and printed as its pixels are stored, so
/// a host supplies it upright. Its pixels are decoded once whole, within the decode allowance
/// (`MAX_RASTER_ALLOC`, as Open's), so a file whose header reads but whose pixels are cut short is
/// refused, not drawn or printed in part; a picture past the allowance is kept, and the renderer
/// draws it at reduced size as it does an opened one.
fn checked(bytes: Vec<u8>) -> Result<((u32, u32), ImageBlob), String> {
    if let Some(o) = vectorcraft_doc::cmyk::exif_orientation(&bytes) {
        return Err(format!("supply the picture upright (its EXIF orientation is {})", o.to_exif()));
    }
    let reader =
        image::ImageReader::new(std::io::Cursor::new(&bytes[..])).with_guessed_format().map_err(|e| format!("the file does not read: {e}"))?;
    let format = reader.format().ok_or("the file is no picture format VectorCraft reads")?;
    let size = reader.into_dimensions().map_err(|e| format!("the picture's header does not read: {e}"))?;
    if size.0 == 0 || size.1 == 0 {
        return Err("the picture has no pixels".into());
    }
    let blob = ImageBlob::new(format.to_mime_type(), bytes).checked_with_proxy(false).ok_or("the picture's pixels do not read")?;
    Ok((size, blob))
}

/// Do pixels of `size` show a file of `file` pixels in its proportions, to the pixel either way?
fn same_shape(size: (u32, u32), file: (u32, u32)) -> bool {
    let (sw, sh, w, h) = (u64::from(size.0), u64::from(size.1), u64::from(file.0), u64::from(file.1));
    (sw * h).abs_diff(sh * w) <= w.max(h)
}

/// The file at `path` (a source) as the host supplied it: kept for every linked image of the open
/// documents that links it, drawn in its preview's place and used for output. Returns how many
/// images (by key) it is kept for and its pixel size. Not an edit. Supplied by hand, it answers for
/// its own width.
pub fn supply(session: &mut Session, path: &str, bytes: Vec<u8>) -> Result<(usize, (u32, u32)), String> {
    supply_as(session, path, bytes, None)
}

/// What a fetch for request `req` brought ([`supply`]). The fetch the path waits for settles it:
/// what it brought is kept, unless it is fewer pixels than are kept already (more than a pixel
/// fewer than it asked for are kept, but the source fails: asked again, the host would send no
/// more), refused for the budget, or no picture (the source fails, what was kept stays). One it no
/// longer waits for changes nothing (an error), unless nothing is kept or asked for, or it brings
/// more pixels than are kept: its source's URL changed since (another URL may serve other pixels),
/// or another fetch overtook it.
pub fn supply_fetched(session: &mut Session, req: &SourceRequest, bytes: Vec<u8>) -> Result<(usize, (u32, u32)), String> {
    // Before the file is decoded: a reply nothing waits for costs no decode.
    fetch_taken(&sources(), &req.path, req)?;
    supply_as(session, &req.path, bytes, Some(req))
}

/// Is a fetch for `req` taken for `path` now: `Ok(true)` when it is the one the path waits for,
/// `Ok(false)` when it may still settle an idle source or add pixels to those kept?
fn fetch_taken(s: &Sources, path: &str, req: &SourceRequest) -> Result<bool, String> {
    let src = s.by_path.get(path).ok_or_else(|| format!("{path} is no link source: give it with links.addSources first"))?;
    if url_for(&src.link, req.width, req.height) != req.url {
        return Err(format!("{path}: its source has moved since it was asked for"));
    }
    match &src.state {
        State::Pending { url, .. } if *url == req.url => Ok(true),
        State::Idle => Ok(false),
        State::Kept(k) if req.width > k.width => Ok(false),
        _ => Err(format!("{path}: its source no longer waits for this fetch")),
    }
}

fn supply_as(session: &mut Session, path: &str, bytes: Vec<u8>, req: Option<&SourceRequest>) -> Result<(usize, (u32, u32)), String> {
    let read = if bytes.len() > MAX_SUPPLIED { Err(format!("larger than {} MB", MAX_SUPPLIED >> 20)) } else { checked(bytes) };
    let mut keys = BTreeSet::new();
    let mut shapes = true;
    if let Ok((size, _)) = &read {
        for st in &session.docs {
            st.doc.visit_images(|_, im| {
                if im.link.as_ref().is_some_and(|l| l.relative.as_deref() == Some(path) || l.path == path) {
                    keys.insert(im.key.clone());
                    shapes &= same_shape(*size, (im.width, im.height));
                }
            });
        }
    }
    let read = read.and_then(|(size, blob)| {
        if shapes { Ok((size, blob)) } else { Err(format!("its {} × {} pixels are not in the linked file's proportions", size.0, size.1)) }
    });
    let mut s = sources();
    // Looked at again: the source may have moved while the file was decoded.
    let awaited = match req {
        Some(r) => fetch_taken(&s, path, r)?,
        None => false,
    };
    let Some(src) = s.by_path.get_mut(path) else {
        return Err(format!("{path} is no link source: give it with links.addSources first"));
    };
    let old = src.state.kept().cloned();
    // What settles the path: the fetch it waits for, or pixels supplied by hand (unless a fetch for
    // more pixels than they have is on its way, which will).
    let settles = |own: u32| awaited || (req.is_none() && !matches!(src.state, State::Pending { width, .. } if width > own.saturating_add(1)));
    let (size, blob) = match read {
        Ok(found) => found,
        Err(e) => {
            // Bad bytes supplied by hand fail the source whatever is on its way.
            if awaited || req.is_none() {
                src.state = State::Failed { reason: e.clone(), kept: old };
                moved();
            }
            return Err(format!("{path}: {e}"));
        }
    };
    // Kept pixels never give way to fewer: the reply the path waited for fails it, others are let be.
    if let Some(o) = old.as_ref().filter(|o| size.0.saturating_add(1) < o.width) {
        let reason = format!("its {} pixels across are fewer than the {} kept", size.0, o.width);
        if awaited {
            src.state = State::Failed { reason: reason.clone(), kept: old };
            moved();
        }
        return Err(format!("{path}: {reason}"));
    }
    if keys.is_empty() {
        // No open document links it: nothing is kept.
        if awaited {
            src.state = State::settled(old);
            moved();
        }
        return Ok((0, size));
    }
    let mut kept = BTreeSet::new();
    for key in &keys {
        if link_pixels::insert(key, Supplied { size, blob: blob.clone() }) {
            kept.insert(key.clone());
        }
    }
    moved();
    if kept.is_empty() {
        // The linked budget refused them: those kept before stay, and are not taken for these.
        if settles(size.0) {
            src.state = State::Refused { width: size.0, size: blob.bytes.len(), kept: old };
        }
        return Err(format!(
            "{path} is larger than the linked images' budget ({} MB, view.imageCache linked): it shows its preview",
            link_pixels::budget() >> 20
        ));
    }
    // Images the older pixels were kept for and these are not show their previews: nothing older stays.
    for key in old.iter().flat_map(|o| o.keys.difference(&kept)) {
        link_pixels::forget(key);
    }
    let new = Kept { width: size.0, size: blob.bytes.len(), keys: kept };
    let count = new.keys.len();
    let settled = settles(size.0);
    match (&mut src.state, req) {
        (State::Pending { kept, .. }, None) if !settled => *kept = Some(new),
        (_, Some(r)) if awaited && size.0.saturating_add(1) < r.width => {
            let reason = format!("its {} × {} pixels are fewer than the {} × {} asked for", size.0, size.1, r.width, r.height);
            src.state = State::Failed { reason: reason.clone(), kept: Some(new) };
            return Err(format!("{path}: {reason}"));
        }
        _ => src.state = State::Kept(new),
    }
    Ok((count, size))
}

/// The fetch for request `req` failed: when it is the one its path waits for, the path's file is
/// not asked for again until the host gives its source again, and its images show, and print, what
/// was kept before (their previews when nothing was); counted failed by `links.loadSources`. One
/// its path no longer waits for (its source moved, or a larger fetch overtook it) says nothing of
/// the source now and is let be (false).
pub fn fetch_failed(req: &SourceRequest, reason: &str) -> bool {
    let mut s = sources();
    let Some(src) = s.by_path.get_mut(&req.path) else { return false };
    let State::Pending { url, kept, .. } = &mut src.state else { return false };
    if *url != req.url {
        return false;
    }
    src.state = State::Failed { reason: reason.to_string(), kept: kept.take() };
    moved();
    true
}

/// Does a host fetch what [`requests`] asks for (the web shell)? Without one (the desktop, a
/// headless host), nothing is asked for: a host supplies files by hand ([`supply`]).
static FETCHER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Say whether a host fetches what [`requests`] asks for: the web shell does, once it runs.
pub fn set_fetcher(fetches: bool) {
    FETCHER.store(fetches, Ordering::Relaxed);
}

/// Does `path`'s source still wait for request `req` (its URL is the one asked for now)? A fetch
/// it no longer waits for is not started, or is given up.
pub fn awaits(req: &SourceRequest) -> bool {
    matches!(sources().by_path.get(&req.path).map(|src| &src.state), Some(State::Pending { url, .. }) if *url == req.url)
}

/// Has `path`'s source failed?
pub fn is_failed(path: &str) -> bool {
    matches!(sources().by_path.get(path).map(|src| &src.state), Some(State::Failed { .. }))
}

/// The width the pixels kept for `path` answer for.
#[cfg(test)]
pub(crate) fn kept_width(path: &str) -> Option<u32> {
    sources().by_path.get(path).and_then(|src| src.state.kept()).map(|k| k.width)
}

/// How the active document's linked images with sources stand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Loading {
    /// Images whose supplied pixels are what their placements need.
    pub loaded: usize,
    /// Images still to be supplied.
    pub loading: usize,
    /// Images whose source failed, or whose supplied pixels the linked budget leaves out: they
    /// show, and print, their previews (or fewer pixels than they need).
    pub failed: usize,
    /// Of those failed, the images whose supplied pixels do not fit the linked budget
    /// (`view.imageCache` linked) beside the others kept.
    pub over_budget: usize,
}

/// Ask for every linked image of `d` that has a source and lacks the pixels its placements need,
/// drawn or not (a host does this before printing), the requests handed out by the next
/// [`requests`]; tells how they stand. Pixels the linked budget dropped, or refused, are asked
/// for again only while they fit beside the document's own pixels kept and asked for again before
/// them (other documents' pixels give way): otherwise each would drop another, and the host is told
/// they are over budget. Without a host that fetches ([`set_fetcher`]) nothing is asked for: what
/// lacks its pixels is failed until supplied by hand.
pub fn load_all(d: &Document) -> Loading {
    let mut s = sources();
    let fetches = FETCHER.load(Ordering::Relaxed);
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
    let own: usize = seen.iter().filter_map(|k| link_pixels::peek(k)).map(|p| p.blob.bytes.len()).sum();
    let mut room = link_pixels::budget().saturating_sub(own);
    // What an earlier call asked for and nothing handed out yet, while it is still asked for: at
    // most one a path.
    let mut outbox = std::mem::take(&mut s.outbox);
    outbox.retain(|r| matches!(s.by_path.get(&r.path).map(|src| &src.state), Some(State::Pending { url, .. }) if *url == r.url));
    s.outbox = outbox;
    for im in images {
        let Some(path) = source_of(&s, &im).map(str::to_string) else { continue };
        if !d.images.get(&im.key).is_some_and(ImageBlob::is_proxy) {
            continue;
        }
        let Some(&(file, scale)) = need.get(&im.key) else { continue };
        let (mut width, mut height) = size_for(file, scale, preview_size(d, &im.key));
        let Some(src) = s.by_path.get(&path) else { continue };
        let shown = link_pixels::peek(&im.key).is_some();
        // The encoded size of pixels that answer for it but went for the budget.
        let again = match &src.state {
            // Pixels kept before a failure or a refusal that still cover what it needs print.
            State::Failed { kept: Some(k), .. } | State::Refused { kept: Some(k), .. } if covers(k, width) && shown => {
                out.loaded += 1;
                continue;
            }
            State::Failed { .. } => {
                out.failed += 1;
                continue;
            }
            State::Pending { width: w, .. } if *w >= width => {
                out.loading += 1;
                continue;
            }
            State::Kept(k) if covers(k, width) => {
                if shown {
                    out.loaded += 1;
                    continue;
                }
                // Asked for again at the width kept: what comes never has fewer pixels.
                if k.width > width {
                    (width, height) =
                        (k.width, ((u64::from(file.1) * u64::from(k.width)).div_ceil(u64::from(file.0.max(1))) as u32).clamp(1, file.1.max(1)));
                }
                Some(k.size)
            }
            State::Refused { width: w, size, .. } if w.saturating_add(1) >= width => Some(*size),
            _ => None,
        };
        if !fetches {
            // Nothing will fetch it: the host supplies it by hand, or it prints its preview.
            out.failed += 1;
            continue;
        }
        if let Some(size) = again {
            if size > room {
                out.failed += 1;
                out.over_budget += 1;
                continue;
            }
            room -= size;
        }
        if let Some(r) = ask(&mut s, &path, width, height, again.is_some()) {
            s.outbox.push(r);
        }
        out.loading += 1;
    }
    out
}

/// Run `f` on `d` with each linked image whose bytes are its preview showing the pixels a source
/// supplied for it instead, as [`for_output`] gives them; those images' blobs are put back after
/// (what `f` adds stays). For an edit that writes pixels made from them (Expand Appearance's
/// recoloured images).
pub fn with_supplied<R>(d: &mut Document, f: impl FnOnce(&mut Document) -> R) -> R {
    let swap: Vec<(String, Arc<Supplied>)> =
        d.images.iter().filter(|(_, b)| b.is_proxy()).filter_map(|(k, _)| link_pixels::peek(k).map(|p| (k.clone(), p))).collect();
    let mut kept = vec![];
    for (key, supplied) in swap {
        if let Some(old) = d.images.get(&key).cloned() {
            let shown = ImageBlob { mime: supplied.blob.mime.clone(), bytes: supplied.blob.bytes.clone(), proxy: Some(old.bytes.clone()) };
            d.images.insert(key.clone(), shown);
            kept.push((key, old));
        }
    }
    let out = f(d);
    for (key, old) in kept {
        d.images.insert(key, old);
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

    /// A picture past the decode allowance is kept as Open keeps one (the renderer draws it at
    /// reduced size), not refused by the image decoder's own default limit.
    #[test]
    fn a_picture_past_the_decode_allowance_is_kept() {
        // A PNG that says it is 23 200 × 23 200 gray pixels: more than the allowance counts as RGBA.
        let png = crate::tests::linkspanel::png_header(23_200, 23_200);
        assert_eq!(checked(png).map(|(size, _)| size), Ok((23_200, 23_200)));
    }

    #[test]
    fn a_source_s_url_takes_the_size_asked_for() {
        let s = LinkSource { url: "p.png?w={width}&h={height}&s={side}".into(), sidecar: None };
        assert_eq!(url_for(&s, 30, 40), "p.png?w=30&h=40&s=40");
    }
}
