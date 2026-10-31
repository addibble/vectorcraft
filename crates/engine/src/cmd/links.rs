//! Linked images (File → Place with Link): finding their files, telling when they changed, and
//! reading them again.
//!
//! A linked image keeps its file's absolute path, the path relative to the document (written on
//! save), the file's size, modification time and hash ([`LinkInfo`]), and a low-resolution
//! preview that the document saves in place of the pixels ([`ImageBlob::proxy`]). A file is looked
//! for at its path, then at its relative path and by name in the document's folder. When a
//! document opens ([`resolve`]), the files found unchanged are read again, modified ones follow
//! Preferences → File Handling → Update Links, and missing ones show their preview.
//!
//! `links.check` reports, `links.update` reads modified files again, `links.relink` points images
//! at other files. A file read again takes the image's place as its Placement Options say
//! ([`PlacementOptions`]: keep the bounds or the transforms, fit or fill the bounds…); setting
//! them applies them at once to the file each linked image already shows.
//!
//! The Links panel lists every image (`links.list`) with its Link Info (`links.info`); Go To
//! selects one (`links.goTo`), Embed keeps a linked file's pixels in the document
//! (`links.embed`) and Unembed writes an embedded image to a file it then links to
//! (`links.unembed`).
//!
//! Placed documents ([`PlacedDocument`], VectorCraft documents and PDF or `.ai` pages placed with
//! Link) link to their files the same way: checking, updating, relinking and finding them on open
//! work alike, and they are listed. Embed (Break Link) turns one into an editable copy of its art.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use serde_json::{Value, json};
use vectorcraft_doc::links::{Align, Preserve, hash_bytes};
use vectorcraft_doc::{Appearance, Document, ImageBlob, ImageObject, LinkInfo, Node, NodeId, NodeKind, PageBox, PlacedDocument, PlacementOptions};
use vectorcraft_geom::{Affine, Rect, shapes};

use super::fileio::{self, RasterImage, absolute_path, file_created, file_stamp, read_file, write_file};
use super::{CommandSpec, EngineError, Result, Session, always, bad, bool_or, cmd, has_doc, id_param, ids_param, str_param};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "links.check",
            "Check Links",
            [],
            None,
            "{ids?: [image or placed document ids] (default: every linked image and placed document)} where each linked file is and whether it changed since it was read → {links: [{name, path, status: ok|modified|missing, ids, found?: path (found away from its path: relative to the document, or by name in its folder), preview: true (showing the low-resolution preview, not the file), refused?: why the path is not followed (another computer, a device or a URL: it counts as missing)}], missing, modified}",
            has_doc,
            check
        ),
        cmd!(
            "links.update",
            "Update Links",
            [],
            None,
            "{ids?: [image or placed document ids] (default: the linked images whose file was modified or is showing its preview, and placed documents whose file was modified)} read their linked files again; each keeps its bounds. One undo step → {updated: [ids], missing: [ids]}",
            has_doc,
            update
        ),
        cmd!(
            "links.updateChanged",
            "Update Changed Links",
            [],
            None,
            "{} read again the active document's linked files that changed on disk since they were last seen (size or modification time), whatever Preferences → File Handling → Update Links says; the desktop app looks every two seconds and follows that preference. Files seen for the first time are only remembered. One undo step → {updated: [ids], ask: [], modified: []}",
            has_doc,
            crate::link_watch::update_changed
        ),
        cmd!(
            "links.relink",
            "Relink",
            [],
            None,
            "{ids?: [image or placed document ids] (default: the selected ones; with folder and nothing selected, every missing link), path | folder, copy?: false (with path, for images of a saved document: copy the file into the document's folder and link to the copy; the names Name.png, Name 2.png … Name 99.png are tried in turn until one is free, which gets the copy, or is a file with the same contents, which is used), preserve?: transforms|bounds|fileDimensions|fit|fill (how the new file takes each image's place this time, as links.placementOptions describes; default: each image's Placement Options; fit keeps the new file's aspect ratio inside the old bounds: Replace)} link images (or placed documents, each showing the same artboard or page of the new file) to the file at path, or each to the file of its link's name in folder; embedded images become linked; each keeps its bounds. One undo step → {relinked: [ids], notFound: [file names], copied?: [{from, to}], kept?: [{from, to}] (with copy)}",
            has_doc,
            relink
        ),
        cmd!(
            query "links.list",
            "Links",
            [],
            None,
            "{show?: all (default)|missing|modified|embedded, sort?: name|kind (file format)|status (missing, modified, ok, embedded); default: stacking order; top first} every image object and placed document in the layers, as the Links panel lists them → {links: [{id, name, linked, status: ok|modified|missing|embedded, format, pixelWidth, pixelHeight, path?, found?: path (found away from its path), page?, preview?: true (showing the saved preview), note?: said of an image past the decode allowance (shown at reduced size, or too large to draw), document?: true (a placed document: pageWidth and pageHeight in pt instead of pixels)}], missing, modified, embedded}",
            has_doc,
            list
        ),
        cmd!(
            "links.goTo",
            "Go To Link",
            [],
            None,
            "{id?} select image or placed document `id` (default: the first selected one); the Links panel scrolls it into view → {id, bounds: [x0, y0, x1, y1]}",
            has_doc,
            go_to
        ),
        cmd!(
            "links.embed",
            "Embed Image",
            [],
            None,
            "{ids?: [image or placed document ids] (default: the selected linked ones)} break the links (the Links panel's Break Link): images keep the linked files' pixels in the document; a placed document becomes an editable copy of its art, as placing its file without link gives (its symbols, patterns, swatches and images joining the document). An image whose file can't be found and that shows the saved preview, or a placed document whose file can't be read or changed, stays linked: relink or update it first. One undo step → {embedded: [ids], missing: [ids]}",
            has_doc,
            embed
        ),
        cmd!(
            "links.unembed",
            "Unembed",
            [],
            None,
            "{id?: an embedded image (default: the one selected), path?} write the image's pixels to the file at path (PNG, JPEG, GIF or WebP as stored, other images as PNG; a path ending in .png/.jpg/.jpeg/.gif/.webp/.tif/.tiff/.bmp converts them) and link the image to it, keeping its bounds. One undo step → {id, path}; no path → {name, dataBase64} (nothing changes: there is no file to link to)",
            has_doc,
            unembed
        ),
        cmd!(
            query "links.info",
            "Link Info",
            [],
            None,
            "{id?} an image's or placed document's Link Info (default: the one selected) → image.info's fields (id, name, linked, link, colorMode, pixelWidth, pixelHeight, width, height) and status: ok|modified|missing|embedded, format, ppi: [x, y] (the file's), effectivePpi: [x, y] (at its placed size), scale: [x%, y%] (of its 100% size), rotation (degrees, counter-clockwise), placement: {preserve, align, clip}; linked images also fileName, location (its folder), page?, fileSize? (bytes), modified?, created? (ms since the Unix epoch), refused? (why the path is not followed); a placed document has document: true, pageWidth and pageHeight (pt) instead of pixels, ppi and colorMode, and its scale is of its artboard's or page's 100% size",
            has_doc,
            info
        ),
        cmd!(
            "links.placementOptions",
            "Placement Options",
            [],
            None,
            "{ids?: [image or placed document ids] (default: the selected ones), preserve?: transforms|bounds (default)|fileDimensions|fit|fill, align?: topLeft|top|topRight|left|center (default)|right|bottomLeft|bottom|bottomRight (where the new art sits; not for bounds), clip?: false (clip it to the old bounds where it is larger)} how a file read again (links.relink, links.update) takes each image's place: transforms keeps its scale (relative to each file's 100% size), rotation and position; bounds stretches it into the old bounds; fileDimensions puts it at 100%, unrotated; fit and fill scale it proportionally to fit inside or cover the old bounds. Setting them also applies them at once to each linked image and placed document, from the file it already shows (nothing is read): fit and fill put a distorted image back in proportion inside or over its bounds, as align says, fileDimensions puts it at its file's 100% size, unrotated; bounds and transforms move nothing; with clip, art now larger than its bounds is clipped to them in a clip group of its own (a clip group already holding it is never reshaped); embedded images keep their place. Without options → {placement} of the first image; else one undo step → {ids, placement, applied: [ids that moved, changed size or were clipped], embedded?: [embedded image ids, left in place]}",
            has_doc,
            placement_options
        ),
        cmd!(
            query "links.addSources",
            "Add Link Sources",
            [],
            None,
            "{sources: [{path: the path links name (their relative path, else their path), url: where the host serves the file at a size, `{width}`, `{height}` and `{side}` (the longer) replaced by the pixels asked for (for the web build, relative to the page)}] (1 to 100000), resolution?: 300 (ppi, 1 to 2400: the most a placement is asked for at, the host's output resolution)} linked files a host can supply where VectorCraft reads no files (the web build): an image linking one shows its saved preview until it is drawn magnified past it, then the file is fetched at the size its largest placement needs at that resolution (never more than the file has) and drawn over the preview, and used for PDF export; resized larger, it is fetched again. What arrives is kept outside the document, the least recently drawn dropped first past view.imageCache's linked budget. A link with a source counts as found (status ok in links.list, links.check and the Links panel) until its fetch fails. A path already a source takes the new URL (its file is asked for again from it); one that failed is asked for again. Not an edit: nothing is saved, journaled or undone → {added, sources (how many there are now), resolution}",
            always,
            add_sources
        ),
        cmd!(
            query "links.supplySource",
            "Supply Link Source",
            [],
            None,
            "{path, dataBase64: the file's pixels (PNG, JPEG, GIF, WebP, TIFF or BMP, at most 256 MB; there may be fewer than the file has)} what a link source's fetch brought (the web build calls this itself), or a host's own copy: kept for every linked image of the open documents that links path, drawn over its preview and used for PDF export. Not an edit → {images (how many images, by content, it is kept for), pixelWidth, pixelHeight}",
            always,
            supply_source
        ),
        cmd!(
            query "links.loadSources",
            "Load Link Sources",
            [],
            None,
            "{} ask for every linked image of the active document that has a source and lacks the pixels its placements need at the sources' resolution, drawn or not (before printing or exporting: call again until loading is 0) → {loaded, loading, failed} (images, by content)",
            has_doc,
            load_sources
        ),
        cmd!(
            query "view.imageCache",
            "Image Cache",
            [],
            None,
            "{decoded?: bytes (the decoded images all views share; default an eighth of memory up to 1 GB, 256 MB on the web), linked?: bytes (the encoded pixels link sources supplied; default 512 MB, 128 MB on the web)} set the budgets (at least 16 MB each; past one, the least recently drawn go and are decoded or fetched again when next drawn) → {decoded: {budget, bytes}, linked: {budget, bytes}} (the bytes held now)",
            always,
            image_cache
        ),
    ]
}

// ---------- link sources ----------

/// The most sources one `links.addSources` call may give.
const MAX_LINK_SOURCES: usize = 100_000;

/// `links.addSources`: the files a host can supply, checked and registered with
/// [`crate::link_sources`].
fn add_sources(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.addSources";
    let list = p.get("sources").and_then(Value::as_array).ok_or_else(|| bad(C, "give sources, a list of sources each with a path and a url"))?;
    if list.is_empty() || list.len() > MAX_LINK_SOURCES {
        return Err(bad(C, format!("give 1 to {MAX_LINK_SOURCES} sources")));
    }
    let resolution = match p.get("resolution") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_f64().filter(|r| r.is_finite() && (1.0..=2400.0).contains(r)).ok_or_else(|| bad(C, "resolution is 1 to 2400 ppi"))?),
    };
    let mut sources = Vec::with_capacity(list.len());
    for (i, src) in list.iter().enumerate() {
        let text = |key: &str, max: usize| -> Result<String> {
            let v = src.get(key).and_then(Value::as_str).map(str::trim).unwrap_or("");
            if v.is_empty() || v.len() > max {
                return Err(bad(C, format!("sources[{i}].{key} is 1 to {max} characters")));
            }
            Ok(v.to_string())
        };
        sources.push((text("path", 4096)?, crate::link_sources::LinkSource { url: text("url", 4096)? }));
    }
    let added = crate::link_sources::add(sources, resolution);
    // Links supplied now count as found: the Links panel and the canvas look again (not an edit).
    for st in &mut s.docs {
        st.revision += 1;
    }
    let (count, resolution) = crate::link_sources::summary();
    Ok(json!({ "added": added, "sources": count, "resolution": resolution }))
}

/// `links.supplySource`: a source's file, as base64, kept for the images linking it.
fn supply_source(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.supplySource";
    let path = str_param(p, "path").filter(|p| !p.is_empty()).ok_or_else(|| bad(C, "give the path links name"))?;
    let data = str_param(p, "dataBase64").ok_or_else(|| bad(C, "give the file as dataBase64"))?;
    // Base64 is four characters for three bytes: refuse an oversized file before decoding it.
    if data.len() / 4 * 3 > crate::link_sources::MAX_SUPPLIED {
        return Err(bad(C, format!("{path} is larger than {} MB", crate::link_sources::MAX_SUPPLIED >> 20)));
    }
    let bytes = vectorcraft_format::base64_decode(data).ok_or_else(|| bad(C, "dataBase64 is not base64"))?;
    let (images, (w, h)) = crate::link_sources::supply(s, path, bytes).map_err(|e| bad(C, e))?;
    Ok(json!({ "images": images, "pixelWidth": w, "pixelHeight": h }))
}

/// `links.loadSources`: every linked image of the active document with a source asked for.
fn load_sources(s: &mut Session, _: &Value) -> Result<Value> {
    let l = crate::link_sources::load_all(&s.doc()?.doc);
    Ok(json!({ "loaded": l.loaded, "loading": l.loading, "failed": l.failed }))
}

/// The least an image cache's budget may be set to.
const MIN_CACHE: u64 = 16 << 20;

/// `view.imageCache`: the decoded images' and link sources' budgets, set and told.
fn image_cache(_: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "view.imageCache";
    let budget = |key: &str| -> Result<Option<usize>> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => {
                let b = v
                    .as_u64()
                    .filter(|b| *b >= MIN_CACHE)
                    .ok_or_else(|| bad(C, format!("{key} is a whole number of bytes, at least {MIN_CACHE}")))?;
                Ok(Some(usize::try_from(b).unwrap_or(usize::MAX)))
            }
        }
    };
    let (decoded, linked) = (budget("decoded")?, budget("linked")?);
    if let Some(b) = decoded {
        vectorcraft_render::set_image_cache_budget(b);
    }
    if let Some(b) = linked {
        vectorcraft_render::link_pixels::set_budget(b);
    }
    Ok(json!({
        "decoded": { "budget": vectorcraft_render::image_cache_budget(), "bytes": vectorcraft_render::image_cache_bytes() },
        "linked": { "budget": vectorcraft_render::link_pixels::budget(), "bytes": vectorcraft_render::link_pixels::bytes() },
    }))
}

// ---------- finding files ----------

/// What became of a linked file since it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Modified,
    Missing,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Modified => "modified",
            Status::Missing => "missing",
        }
    }
}

/// The image objects that show one linked file as one content (same path, same image), or the
/// placed documents that show one artboard or page of it.
struct Group {
    link: LinkInfo,
    key: String,
    /// The objects in the layers (images in symbols and patterns share the content, not an id).
    ids: Vec<NodeId>,
    /// Placed documents: what they show.
    document: Option<Shown>,
}

/// What a placed document shows: whether the art's bounds (not the artboard or page) are shown,
/// and a PDF page's box.
type Shown = (bool, PageBox);

/// The linked object `n` as (link, content key, what a placed document shows).
fn linked(n: &Node) -> Option<(&LinkInfo, &str, Option<Shown>)> {
    match &n.kind {
        NodeKind::Image(im) => Some((im.link.as_ref()?, &im.key, None)),
        NodeKind::PlacedDocument(p) => Some((&p.link, &p.key, Some((p.bounding, p.page_box)))),
        _ => None,
    }
}

/// Every linked file of `d` with the ids of the objects showing it (for [`crate::link_watch`]).
pub(crate) fn linked_groups(d: &Document) -> Vec<(String, Vec<u64>)> {
    groups(d, None).into_iter().filter(|g| !g.ids.is_empty()).map(|g| (g.link.path, ids_json(&g.ids))).collect()
}

/// The linked images and placed documents of `d` by file and content: those in the layers (only `ids` when
/// given), then, without `ids`, the ones only symbols and patterns show.
fn groups(d: &Document, ids: Option<&[NodeId]>) -> Vec<Group> {
    let mut out: Vec<Group> = vec![];
    let mut index: BTreeMap<(String, String, bool), usize> = BTreeMap::new();
    // Looked up for every linked node: a set, not the list (which may repeat an id many times).
    let wanted: Option<std::collections::HashSet<NodeId>> = ids.map(|ids| ids.iter().copied().collect());
    let mut add = |link: &LinkInfo, key: &str, document: Option<Shown>, id: Option<NodeId>| {
        let i = *index.entry((link.path.clone(), key.to_string(), document.is_some())).or_insert_with(|| {
            out.push(Group { link: link.clone(), key: key.to_string(), ids: vec![], document });
            out.len() - 1
        });
        if let (Some(id), Some(g)) = (id, out.get_mut(i)) {
            g.ids.push(id);
        }
    };
    d.walk(|n| {
        if let Some((l, key, document)) = linked(n)
            && wanted.as_ref().is_none_or(|ids| ids.contains(&n.id))
        {
            add(l, key, document, Some(n.id));
        }
    });
    if ids.is_none() {
        d.visit_images(|_, im| {
            if let Some(l) = &im.link {
                add(l, &im.key, None, None);
            }
        });
        d.visit_placed(|_, p| add(&p.link, &p.key, Some((p.bounding, p.page_box)), None));
    }
    out
}

/// A linked file where it was found.
struct Found {
    path: String,
    size: u64,
    modified: Option<u64>,
    /// Read when telling whether it changed needed them.
    bytes: Option<Vec<u8>>,
}

impl Found {
    /// The file's bytes (read now unless already read).
    fn read(&mut self) -> Result<Vec<u8>> {
        match self.bytes.take() {
            Some(b) => Ok(b),
            None => read_file(&self.path),
        }
    }

    /// `base` pointing at this file as read now (`bytes`).
    fn link(&self, bytes: &[u8], base: &LinkInfo) -> LinkInfo {
        self.link_hashed(hash_bytes(bytes), base)
    }

    /// [`Self::link`] with the bytes' hash already taken.
    fn link_hashed(&self, hash: String, base: &LinkInfo) -> LinkInfo {
        LinkInfo { path: self.path.clone(), size: Some(self.size), modified: self.modified, hash: Some(hash), ..base.clone() }
    }
}

/// The link to the file at `path` whose bytes were just read.
pub(crate) fn link_info(path: &str, bytes: &[u8]) -> LinkInfo {
    let path = absolute_path(path);
    LinkInfo {
        size: Some(bytes.len() as u64),
        modified: file_stamp(&path).and_then(|(_, m)| m),
        hash: Some(hash_bytes(bytes)),
        ..LinkInfo::new(path)
    }
}

/// The file at `path` when there is one.
fn found_at(path: String) -> Option<Found> {
    let (size, modified) = file_stamp(&path)?;
    Some(Found { path, size, modified, bytes: None })
}

/// Where the file of `link` (stored in the document saved at `doc_path`) is: at its path, else at
/// its relative path or by name in the document's folder. Only paths
/// [`fileio::document_path`] follows are looked at.
fn locate(link: &LinkInfo, doc_path: Option<&str>) -> Option<Found> {
    let stored = [Some(link.path.as_str()), link.relative.as_deref(), Some(link.name())];
    stored.into_iter().flatten().filter_map(|p| fileio::document_path(p, doc_path).ok()).find_map(found_at)
}

/// Why the path of `link` (stored in the document saved at `doc_path`) isn't followed, when it
/// names another computer, a device or a URL.
fn refused(link: &LinkInfo, doc_path: Option<&str>) -> Option<craft_docpath::Refused> {
    fileio::document_path(&link.path, doc_path).err().filter(|r| *r != craft_docpath::Refused::Invalid)
}

/// Where the file of group `g` (of the document saved at `doc_path`) is and whether it still has
/// the content shown. Its bytes are read only when its size or modification time changed. A file
/// not found that a host supplies ([`crate::link_sources`]) is OK, with nothing found.
fn probe(g: &Group, doc_path: Option<&str>) -> (Status, Option<Found>) {
    // Not found here, but supplied by the host (a link source): found all the same, as a file a
    // font provider serves counts as installed.
    let unread = || if crate::link_sources::sourced(&g.link) { Status::Ok } else { Status::Missing };
    let Some(mut f) = locate(&g.link, doc_path) else { return (unread(), None) };
    let l = &g.link;
    if l.size == Some(f.size) && l.modified.is_some() && l.modified == f.modified {
        return (Status::Ok, Some(f));
    }
    let Ok(bytes) = f.read() else { return (unread(), None) };
    let same = match &l.hash {
        Some(h) => *h == hash_bytes(&bytes),
        // Linked before links kept a hash: the image the file decodes to.
        None => g.document.is_none() && fileio::raster_image(&bytes).is_ok_and(|img| img.key == g.key),
    };
    f.bytes = Some(bytes);
    (if same { Status::Ok } else { Status::Modified }, Some(f))
}

/// Is image `key` of `d` showing its preview (its file not read)?
fn previewing(d: &Document, key: &str) -> bool {
    d.images.get(key).is_none_or(ImageBlob::is_proxy)
}

/// A linked raster image whose file is an EPS (Preferences › File Handling › Use Low Resolution
/// Proxy for Linked EPS).
fn linked_eps(link: &LinkInfo) -> bool {
    fileio::format_for_name(link.name()).is_some_and(|f| f.id == "eps")
}

/// With `low_res_proxy_eps`, a linked EPS image that already shows its saved preview keeps it on
/// open instead of reading the file again (Update Links still reads modified files).
fn keep_eps_preview(low_res_proxy_eps: bool, g: &Group, d: &Document) -> bool {
    low_res_proxy_eps && g.document.is_none() && linked_eps(&g.link) && previewing(d, &g.key)
}

/// Does group `g` of `d` show something other than its file: an image its preview, a placed
/// document nothing (its file's bytes are gone)?
fn stale(d: &Document, g: &Group) -> bool {
    match g.document {
        None => previewing(d, &g.key),
        Some(_) => !d.images.contains_key(&g.key),
    }
}

/// A linked file of a document (File → Package): its path, name and bytes.
pub(crate) struct LinkedFile {
    pub path: String,
    pub name: String,
    /// The file actually read, which may be next to the current document after a move.
    /// Nested links must be resolved against this folder, not the obsolete original path.
    pub found_path: Option<String>,
    /// Read from where the file is found, else the pixels the document holds when they are the
    /// file's own; `None` when neither.
    pub bytes: Option<Vec<u8>>,
}

/// Every file the linked images of `d` (saved at `doc_path`) show, in the layers, symbols and
/// patterns, once each.
pub(crate) fn linked_files(d: &Document, doc_path: Option<&str>) -> Vec<LinkedFile> {
    let mut seen = std::collections::BTreeSet::new();
    groups(d, None)
        .into_iter()
        .filter(|g| seen.insert(g.link.path.clone()))
        .map(|g| {
            let own = |b: &&ImageBlob| !b.is_proxy() && fileio::format_for_name(g.link.name()).is_some_and(|f| f.mime == b.mime);
            let found = locate(&g.link, doc_path);
            let found_path = found.as_ref().map(|f| f.path.clone());
            let bytes = found.and_then(|mut f| f.read().ok()).or_else(|| d.images.get(&g.key).filter(own).map(|b| b.bytes.to_vec()));
            LinkedFile { path: g.link.path.clone(), name: g.link.name().to_string(), found_path, bytes }
        })
        .collect()
}

/// `d` for output: linked raster images that still show only their saved preview read their files
/// again (Preferences › Use Low Resolution Proxy for Linked EPS keeps the preview on screen only).
pub(crate) fn full_linked_images<'a>(d: &'a Document, doc_path: Option<&str>) -> (Cow<'a, Document>, Vec<String>) {
    let mut keys = BTreeSet::new();
    d.visit_images(|_, im| {
        if im.link.is_some() && d.images.get(&im.key).is_some_and(ImageBlob::is_proxy) {
            keys.insert(im.key.clone());
        }
    });
    if keys.is_empty() {
        return (Cow::Borrowed(d), vec![]);
    }
    let mut out = d.clone();
    let mut previews = BTreeSet::new();
    let all = groups(d, None);
    for key in keys {
        let Some(g) = all.iter().find(|g| g.document.is_none() && g.key == key) else { continue };
        let Some(mut f) = locate(&g.link, doc_path) else {
            previews.insert(key);
            continue;
        };
        let Some(bytes) = f.read().ok() else {
            previews.insert(key);
            continue;
        };
        let Ok(img) = fileio::raster_image(&bytes) else {
            previews.insert(key);
            continue;
        };
        let preview = d.images.get(&key).map(|b| b.bytes.clone());
        // A file read as it is keeps its own type; one turned upright (EXIF orientation) is the PNG it became.
        let mime = match fileio::format_for_name(g.link.name()) {
            Some(f) if img.blob.bytes.as_slice() == bytes.as_slice() => f.mime.to_string(),
            _ => img.blob.mime.clone(),
        };
        out.images.insert(key, ImageBlob { mime, bytes: img.blob.bytes, proxy: preview });
    }
    let mut warnings = vec![];
    if !previews.is_empty() {
        warnings
            .push(format!("{} linked image(s) output as their previews: their files can't be found or read (relink or update them)", previews.len()));
    }
    (Cow::Owned(out), warnings)
}

/// The linked files of the open document `st` that `links.findFiles` looks for, each with the
/// objects in the layers that show it: the files of objects `ids` whatever their status, else the
/// files that can't be found.
pub(crate) fn files_to_find(st: &crate::DocState, ids: Option<&[NodeId]>) -> Vec<(LinkInfo, Vec<NodeId>)> {
    groups(&st.doc, ids)
        .into_iter()
        .filter(|g| !g.ids.is_empty() && (ids.is_some() || probe(g, st.path.as_deref()).0 == Status::Missing))
        .map(|g| (g.link, g.ids))
        .collect()
}

// ---------- reading files into the document ----------

/// Keep `full` (a file's pixels) as image `key` of `d`, unless pixels are there already; a
/// preview there is kept, and a linked image gets the one `full` carries ([`readable`] makes it
/// when the file is read). Nothing is decoded here.
pub(crate) fn store_image(d: &mut Document, key: &str, full: ImageBlob, linked: bool) {
    let made = full.proxy.clone().filter(|_| linked);
    let blob = match d.images.get(key) {
        Some(b) if !b.is_proxy() => ImageBlob { proxy: b.proxy.clone().or(made), ..b.clone() },
        old => ImageBlob { proxy: old.and_then(|b| b.proxy.clone()).or(made), ..full },
    };
    d.images.insert(key.to_string(), blob);
}

/// Whether `bytes` are a PNG, JPEG, GIF or WebP file: the formats [`fileio::raster_image`] keeps
/// as they are after reading only their header.
pub(crate) fn kept_unread(bytes: &[u8]) -> bool {
    use image::ImageFormat as F;
    matches!(image::guess_format(bytes), Ok(F::Png | F::Jpeg | F::Gif | F::WebP))
}

/// `blob`, an image [`fileio::raster_image`] read from a file, ready to keep. When `unread` (the
/// file is one [`kept_unread`] names), its pixels are decoded once to check that they can be read,
/// and a `linked` image's preview is made from that decode ([`ImageBlob::checked_with_proxy`]);
/// otherwise a `linked` image gets its preview ([`ImageBlob::with_proxy`]). An error when the
/// pixels can't be read.
pub(crate) fn readable(blob: ImageBlob, unread: bool, linked: bool) -> Result<ImageBlob> {
    if !unread {
        return Ok(if linked { blob.with_proxy() } else { blob });
    }
    blob.checked_with_proxy(linked).ok_or_else(|| EngineError::Other("the image's pixels can't be read".into()))
}

/// [`readable`], and the pixels it decoded are kept in the image store the renderers share
/// ([`vectorcraft_render::keep_decoded_image`]), so the first frame that draws the image finds
/// them there and does not decode the file again.
pub(crate) fn readable_and_kept(blob: ImageBlob, unread: bool, linked: bool) -> Result<ImageBlob> {
    if !unread {
        return readable(blob, unread, linked);
    }
    let (blob, pixels) = blob.checked_with_pixels(linked).ok_or_else(|| EngineError::Other("the image's pixels can't be read".into()))?;
    if let Some(pixels) = pixels {
        vectorcraft_render::keep_decoded_image(&blob, pixels);
    }
    Ok(blob)
}

/// The linked image file `bytes`, read ([`fileio::raster_image`]) with its preview; an error when
/// its pixels can't be read ([`readable`]).
fn linked_image(bytes: &[u8]) -> Result<RasterImage> {
    let mut img = fileio::raster_image(bytes)?;
    img.blob = readable(img.blob, kept_unread(bytes), true)?;
    Ok(img)
}

/// The file read for a link: the link to it and its image.
type FileRead = (LinkInfo, RasterImage);

/// The file at `f` read for a link like `base` (its image decoded).
fn read_image(f: &mut Found, base: &LinkInfo) -> Result<FileRead> {
    let bytes = f.read()?;
    Ok((f.link(&bytes, base), linked_image(&bytes)?))
}

/// A linked file read for the objects that show it.
enum Read {
    Image(FileRead),
    /// A VectorCraft document or a PDF or `.ai` file read for placed documents: the link to it and
    /// what they show.
    Document(LinkInfo, super::place::document::Source),
}

/// The file at `f` read for group `g`: its image, or the artboard or page `g` shows.
fn read_for(f: &mut Found, g: &Group, cmd: &str) -> Result<Read> {
    let Some((bounding, page_box)) = g.document else { return read_image(f, &g.link).map(Read::Image) };
    let bytes = f.read()?;
    let page = g.link.page.unwrap_or(1);
    let src = super::place::document::source(&f.path, &bytes, Some(&f.path), page, bounding, page_box, cmd)?;
    Ok(Read::Document(f.link(&bytes, &g.link), src))
}

/// Objects `ids` show what was read (see [`install`], [`install_document`]).
fn install_read(d: &mut Document, ids: &[NodeId], read: Read, force: Option<Preserve>) -> Result<()> {
    match read {
        Read::Image(r) => install(d, ids, r, force),
        Read::Document(link, src) => install_document(d, ids, &link, &src, force),
    }
}

/// Placed documents `ids` show `src`, read from the file `link` points at, each placed as its
/// Placement Options say (`force`: as that Preserve choice says, whatever they say).
fn install_document(d: &mut Document, ids: &[NodeId], link: &LinkInfo, src: &super::place::document::Source, force: Option<Preserve>) -> Result<()> {
    let mut clips = vec![];
    let (w, h) = (src.frame.width(), src.frame.height());
    for id in ids {
        let Some(NodeKind::PlacedDocument(p)) = d.node(*id).map(|n| &n.kind) else { continue };
        let o = PlacementOptions { preserve: force.unwrap_or(p.placement.preserve), ..p.placement };
        let (xf, clip) = placed_xf(p.xf, (p.width, p.height), (1.0, 1.0), (w, h), (1.0, 1.0), o, clip_frame(d, *id));
        if let Some(n) = d.node_mut(*id)
            && let NodeKind::PlacedDocument(p) = &mut n.kind
        {
            let p: &mut PlacedDocument = p;
            (p.key, p.width, p.height, p.xf, p.link) = (src.key.clone(), w, h, xf, link.clone());
        }
        clips.extend(clip.map(|c| (*id, c)));
    }
    super::place::document::store(d, src);
    clips.into_iter().try_for_each(|(id, clip)| clip_image(d, id, clip))
}

/// Points per pixel of the file image `key` of `d` shows, at the resolution it declares (72 ppi
/// when it declares none, or when only its preview is loaded).
fn file_pt(d: &Document, key: &str) -> (f64, f64) {
    super::place::pt_per_px(file_ppi(d, key))
}

/// The resolution the file image `key` of `d` shows declares (none while only its preview is
/// loaded).
fn file_ppi(d: &Document, key: &str) -> Option<(f64, f64)> {
    d.images.get(key).filter(|b| !b.is_proxy()).and_then(|b| fileio::ppi::resolution(&b.bytes))
}

/// Where the clip of an image goes: the old bounds' frame (bounds space to the document) and size.
type Clip = (Affine, (f64, f64));

/// The scale across and down of transform `xf`.
fn scales(xf: Affine) -> (f64, f64) {
    let [a, b, c, d, ..] = xf.as_coeffs();
    (a.hypot(b), c.hypot(d))
}

/// The transform an image placed with `xf` showing `old` pixels (`old_pt`: points per pixel at
/// 100%) gives a new file of `new` pixels (`new_pt`), as Placement Options `o` say; and the clip
/// when `o.clip` and the new art overflows the old bounds. `within`: the bounds to place it in
/// when they aren't the art's own (the rectangle Clip to Bounding Box clips it to, see
/// [`clip_frame`]). (A placed document's "pixels" are points, one point each.)
fn placed_xf(
    xf: Affine,
    old: (f64, f64),
    old_pt: (f64, f64),
    new: (f64, f64),
    new_pt: (f64, f64),
    o: PlacementOptions,
    within: Option<Clip>,
) -> (Affine, Option<Clip>) {
    let (ow, oh, nw, nh) = (old.0, old.1, new.0.max(1e-9), new.1.max(1e-9));
    // The bounds' frame: the transform without its scale (rotation, reflection and position).
    let (su, sv) = scales(xf);
    if o.preserve == Preserve::Bounds || su < 1e-12 || sv < 1e-12 {
        return (xf * Affine::scale_non_uniform(ow / nw, oh / nh), None);
    }
    let (frame, room) = match (o.preserve, within) {
        (Preserve::FileDimensions, _) => {
            let r = match within {
                Some((f, room)) => f.transform_rect_bbox(Rect::new(0.0, 0.0, room.0, room.1)),
                None => xf.transform_rect_bbox(Rect::new(0.0, 0.0, ow, oh)),
            };
            (Affine::translate(r.origin().to_vec2()), (r.width(), r.height()))
        }
        (_, Some(clip)) => clip,
        _ => (xf * Affine::scale_non_uniform(1.0 / su, 1.0 / sv), (ow * su, oh * sv)),
    };
    // The new file's size at 100% (pt), then as placed.
    let natural = (nw * new_pt.0, nh * new_pt.1);
    let size = match o.preserve {
        Preserve::Transforms => (natural.0 * su / old_pt.0, natural.1 * sv / old_pt.1),
        Preserve::Fit | Preserve::Fill => {
            let (kx, ky) = (room.0 / natural.0, room.1 / natural.1);
            let k = if o.preserve == Preserve::Fit { kx.min(ky) } else { kx.max(ky) };
            (natural.0 * k, natural.1 * k)
        }
        _ => natural,
    };
    let (fx, fy) = o.align.fractions();
    let at = Affine::translate(((room.0 - size.0) * fx, (room.1 - size.1) * fy));
    let over = size.0 > room.0 + 1e-6 || size.1 > room.1 + 1e-6;
    (frame * at * Affine::scale_non_uniform(size.0 / nw, size.1 / nh), (o.clip && over).then_some((frame, room)))
}

/// Clip image `id` of `d` to `room` in `frame`: the clipping path of the clip group holding just
/// it changes, else the image goes into a new clip group.
fn clip_image(d: &mut Document, id: NodeId, (frame, room): Clip) -> Result<()> {
    let path = shapes::rectangle(Rect::new(0.0, 0.0, room.0, room.1)).transformed(frame);
    if let Some(c) = own_clip_path(d, id) {
        if let Some(Node { kind: NodeKind::Path { path: p, .. }, .. }) = d.node_mut(c) {
            *p = path;
        }
        return Ok(());
    }
    wrap_in_clip(d, id, path)
}

/// The rectangle Clip to Bounding Box keeps image (or placed document) `id` of `d` in: with Clip
/// on, the clipping path of the clip group holding just it, when that is a rectangle with its
/// sides along the art's. Fit, Fill and File Dimensions place the art in it rather than in the
/// art's own bounds, which a Fill has made bigger.
fn clip_frame(d: &Document, id: NodeId) -> Option<Clip> {
    let xf = match &d.node(id)?.kind {
        NodeKind::Image(im) if im.placement.clip => im.xf,
        NodeKind::PlacedDocument(p) if p.placement.clip => p.xf,
        _ => return None,
    };
    let NodeKind::Path { path, .. } = &d.node(own_clip_path(d, id)?)?.kind else { return None };
    let (su, sv) = scales(xf);
    if su < 1e-12 || sv < 1e-12 {
        return None;
    }
    let frame = xf * Affine::scale_non_uniform(1.0 / su, 1.0 / sv);
    let local = path.transformed(frame.inverse());
    let r = local.bounds()?;
    same_rectangle(&local, &shapes::rectangle(r)).then(|| (frame * Affine::translate(r.origin().to_vec2()), (r.width(), r.height())))
}

/// The clipping path of the clip group holding just image (or placed document) `id` of `d`.
fn own_clip_path(d: &Document, id: NodeId) -> Option<NodeId> {
    let parent = d.parent_of(id)?;
    match &d.node(parent)?.kind {
        NodeKind::Group { children, clip: true } => match &children[..] {
            [c, im] if im.id == id && matches!(c.kind, NodeKind::Path { clipping: true, .. }) => Some(c.id),
            _ => None,
        },
        _ => None,
    }
}

/// Put object `id` of `d` into a new clip group of its own, clipped by `path`, where it was.
fn wrap_in_clip(d: &mut Document, id: NodeId, path: vectorcraft_geom::PathData) -> Result<()> {
    let (par, index, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
    let image = d.remove(id)?;
    let none = vectorcraft_color::Paint::None;
    let mut clip = Node::path(d.alloc_id(), path, Appearance::basic(none.clone(), none, 0.0));
    if let NodeKind::Path { clipping, .. } = &mut clip.kind {
        *clipping = true;
    }
    let group = Node::new(d.alloc_id(), NodeKind::Group { children: vec![std::sync::Arc::new(clip), image], clip: true });
    d.insert(par, index, group)?;
    Ok(())
}

/// Replace image `id` with the raster at `path`, keeping the object's bounds (what an image
/// variable does on `dataset.select`). The file is linked; a missing or unreadable path is an
/// error so the dataset application can skip that binding.
pub(crate) fn set_image_from_path(d: &mut Document, id: NodeId, path: &str, doc_path: Option<&str>) -> Result<()> {
    // A path stored in the document (a data set's value) is followed as its other stored paths
    // are: on this computer or the document's own share only, so choosing a data set can't make
    // the app connect to another computer.
    let path = fileio::document_path(path, doc_path).map_err(|r| EngineError::Other(format!("the image `{path}` isn't followed: {r}")))?;
    let bytes = read_file(&path)?;
    let read = (link_info(&path, &bytes), linked_image(&bytes)?);
    install(d, &[id], read, Some(Preserve::Bounds))
}

/// Image objects `ids` show the file read for `link` (`img`), each placed as its Placement
/// Options say (`force`: as that Preserve choice says, whatever they say).
fn install(d: &mut Document, ids: &[NodeId], (link, img): FileRead, force: Option<Preserve>) -> Result<()> {
    let new_pt = super::place::pt_per_px(img.ppi);
    let mut clips = vec![];
    for id in ids {
        let Some(NodeKind::Image(im)) = d.node(*id).map(|n| &n.kind) else { continue };
        let o = PlacementOptions { preserve: force.unwrap_or(im.placement.preserve), ..im.placement };
        let size = |w: u32, h: u32| (w as f64, h as f64);
        let (xf, clip) = placed_xf(im.xf, size(im.width, im.height), file_pt(d, &im.key), size(img.width, img.height), new_pt, o, clip_frame(d, *id));
        if let Some(n) = d.node_mut(*id)
            && let NodeKind::Image(im) = &mut n.kind
        {
            (im.key, im.width, im.height, im.xf, im.link) = (img.key.clone(), img.width, img.height, xf, Some(link.clone()));
        }
        clips.extend(clip.map(|c| (*id, c)));
    }
    store_image(d, &img.key, img.blob, true);
    clips.into_iter().try_for_each(|(id, clip)| clip_image(d, id, clip))
}

/// Groups (of the document saved at `doc_path`) as `{name, path, ids}` rows (those with objects in
/// the layers), with `refused`: why the path isn't followed, when it isn't.
fn rows<'a>(gs: impl Iterator<Item = &'a Group>, doc_path: Option<&str>) -> Vec<Value> {
    gs.filter(|g| !g.ids.is_empty())
        .map(|g| {
            let mut row = json!({ "name": g.link.name(), "path": g.link.path, "ids": ids_json(&g.ids) });
            note_refused(&mut row, &g.link, doc_path);
            row
        })
        .collect()
}

/// `row["refused"]`: why the path of `link` isn't followed, when it isn't ([`refused`]).
fn note_refused(row: &mut Value, link: &LinkInfo, doc_path: Option<&str>) {
    if let Some(r) = refused(link, doc_path) {
        row["refused"] = json!(r.to_string());
    }
}

fn ids_json(ids: &[NodeId]) -> Vec<u64> {
    ids.iter().map(|id| id.0).collect()
}

/// What [`resolve`] found when a document opened.
#[derive(Default)]
pub struct Resolved {
    /// `{name, path, ids}` of files that weren't found: their images show the preview.
    pub missing: Vec<Value>,
    /// Modified files left as they were (Update Links isn't Automatically).
    pub modified: Vec<Value>,
    /// Modified files read again (Update Links: Automatically).
    pub updated: Vec<Value>,
}

impl Resolved {
    /// The `document.open` result fields.
    pub fn to_json(&self) -> Value {
        json!({ "missingLinks": self.missing, "modifiedLinks": self.modified, "updatedLinks": self.updated })
    }
}

/// The linked files of document `d`, just read from `doc_path`: those found unchanged are read
/// again (their images had the preview), modified ones are read again when `update`, links found
/// away from their path take the new path. A link nothing is known of whose image shows a preview
/// (a file another app wrote kept only that) was never read: its file is read now, into the
/// image's bounds, which isn't a change to report. Nothing here is an undo step.
pub fn resolve(d: &mut Document, doc_path: Option<&str>, update: bool, low_res_proxy_eps: bool) -> Resolved {
    let mut out = Resolved::default();
    // The image (with its preview) and hash of each file read for a preview here, by path: images
    // of other keys linking to the same file share them, rather than each reading, decoding and
    // keeping a copy of its own.
    let mut read_images: BTreeMap<String, (ImageBlob, String)> = BTreeMap::new();
    for g in groups(d, None) {
        let row = rows(std::iter::once(&g), doc_path);
        let unread = g.document.is_none() && g.link.hash.is_none() && g.link.size.is_none() && previewing(d, &g.key);
        let eps_preview = keep_eps_preview(low_res_proxy_eps, &g, d);
        match probe(&g, doc_path) {
            (Status::Missing, _) => out.missing.extend(row),
            (Status::Modified, Some(mut f)) if update || unread => {
                match read_for(&mut f, &g, "document.open").and_then(|read| install_read(d, &g.ids, read, unread.then_some(Preserve::Bounds))) {
                    Ok(()) if unread => {}
                    Ok(()) => out.updated.extend(row),
                    Err(_) => out.missing.extend(row),
                }
            }
            (Status::Modified, _) => out.modified.extend(row),
            // A placed document keeps its file's bytes: only lost ones are read again.
            (Status::Ok, Some(mut f)) if g.document.is_some() => {
                let read = if stale(d, &g) { Some(read_for(&mut f, &g, "document.open")) } else { None };
                match read {
                    Some(Ok(read)) => {
                        if install_read(d, &g.ids, read, Some(Preserve::Bounds)).is_err() {
                            out.missing.extend(row);
                        }
                    }
                    Some(Err(_)) => out.missing.extend(row),
                    None => {
                        let link = match f.bytes.take() {
                            Some(b) => f.link(&b, &g.link),
                            None => LinkInfo { path: f.path.clone(), ..g.link.clone() },
                        };
                        if link != g.link {
                            set_links(d, &g.ids, &link);
                        }
                    }
                }
            }
            (Status::Ok, Some(mut f)) => {
                let preview = previewing(d, &g.key);
                if let Some((blob, hash)) = read_images.get(&f.path).filter(|_| preview && !eps_preview).cloned() {
                    store_image(d, &g.key, blob, true);
                    let link = f.link_hashed(hash, &g.link);
                    if link != g.link {
                        set_links(d, &g.ids, &link);
                    }
                    continue;
                }
                // Read when the images show the preview, or when the file's details changed.
                let bytes = if (preview && !eps_preview) || f.bytes.is_some() { f.read().ok() } else { None };
                if preview && !eps_preview {
                    let Some((b, img)) = bytes.as_deref().and_then(|b| Some((b, linked_image(b).ok()?))) else {
                        out.missing.extend(row);
                        continue;
                    };
                    let blob = img.blob;
                    read_images.insert(f.path.clone(), (blob.clone(), hash_bytes(b)));
                    store_image(d, &g.key, blob, true);
                } else if let Some(b) = d.images.get(&g.key).filter(|b| b.proxy.is_none()) {
                    // Linked before links kept a preview.
                    let b = b.clone().with_proxy();
                    d.images.insert(g.key.clone(), b);
                }
                // Found away from its path, or read again: the link follows.
                let link = match &bytes {
                    Some(b) => f.link(b, &g.link),
                    None => LinkInfo { path: f.path.clone(), ..g.link.clone() },
                };
                if link != g.link {
                    set_links(d, &g.ids, &link);
                }
            }
            (Status::Ok, None) => {}
        }
    }
    out
}

/// The link of linked object `n` (a linked image or a placed document) to change.
fn link_mut(n: &mut Node) -> Option<&mut LinkInfo> {
    match &mut n.kind {
        NodeKind::Image(im) => im.link.as_mut(),
        NodeKind::PlacedDocument(p) => Some(&mut p.link),
        _ => None,
    }
}

/// Linked image objects (or placed documents) `ids` link to `link`.
fn set_links(d: &mut Document, ids: &[NodeId], link: &LinkInfo) {
    for id in ids {
        if let Some(l) = d.node_mut(*id).and_then(link_mut) {
            *l = link.clone();
        }
    }
}

/// Placed documents of the document saved at `path` (document `saved` of `s`) in the other open
/// documents: read again from the new file, each as one Update Links step of its document.
pub(crate) fn refresh_placed(s: &mut Session, saved: u64, path: &str) {
    let path = absolute_path(path);
    let targets: Vec<(usize, Vec<u64>)> = s
        .documents()
        .iter()
        .enumerate()
        .filter(|(_, st)| st.uid != saved)
        .filter_map(|(i, st)| {
            let mut ids = vec![];
            st.doc.walk(|n| {
                if let NodeKind::PlacedDocument(p) = &n.kind
                    && p.link.path == path
                {
                    ids.push(n.id.0);
                }
            });
            (!ids.is_empty()).then_some((i, ids))
        })
        .collect();
    if targets.is_empty() {
        return;
    }
    // Each document is updated as the active one, without switching tools.
    let back = s.active;
    for (i, ids) in targets {
        s.active = Some(i);
        // A file that can't be read leaves what its objects show: links.check reports it.
        let _ = update(s, &json!({ "ids": ids }));
    }
    s.active = back;
}

// ---------- relative paths ----------

/// `target` relative to folder `base` with `/` separators; `None` when they share no root (another
/// drive, or either is relative).
fn relative_path(target: &Path, base: &Path) -> Option<String> {
    let (t, b): (Vec<Component>, Vec<Component>) = (target.components().collect(), base.components().collect());
    if !matches!(t.first(), Some(Component::Prefix(_) | Component::RootDir)) {
        return None;
    }
    // Windows paths compare without case.
    let same = |x: &Component, y: &Component| if cfg!(windows) { x.as_os_str().eq_ignore_ascii_case(y.as_os_str()) } else { x == y };
    let common = t.iter().zip(&b).take_while(|(x, y)| same(x, y)).count();
    if common == 0 {
        return None;
    }
    let ups = std::iter::repeat_n(std::borrow::Cow::Borrowed(".."), b.len().saturating_sub(common));
    let rest = t.iter().skip(common).map(|c| c.as_os_str().to_string_lossy());
    Some(ups.chain(rest).collect::<Vec<std::borrow::Cow<str>>>().join("/"))
}

/// `d` as saved to `dest`: the links' paths relative to its folder written in; `None` when they
/// are already.
pub fn with_relative_paths(d: &Document, dest: &str) -> Option<Document> {
    let dest = absolute_path(dest);
    let dir = Path::new(&dest).parent()?;
    let mut changes = vec![];
    d.walk(|n| {
        if let Some((l, _, _)) = linked(n) {
            let r = relative_path(Path::new(&l.path), dir);
            if r != l.relative {
                changes.push((n.id, r));
            }
        }
    });
    if changes.is_empty() {
        return None;
    }
    let mut d = d.clone();
    for (id, r) in changes {
        if let Some(l) = d.node_mut(id).and_then(link_mut) {
            l.relative = r;
        }
    }
    Some(d)
}

// ---------- commands ----------

fn check(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let ids = ids_param(p, "ids");
    let (mut missing, mut modified) = (0, 0);
    let mut links = vec![];
    for g in groups(&st.doc, ids.as_deref()).iter().filter(|g| !g.ids.is_empty()) {
        let (status, found) = probe(g, st.path.as_deref());
        missing += usize::from(status == Status::Missing);
        modified += usize::from(status == Status::Modified);
        let mut row = json!({ "name": g.link.name(), "path": g.link.path, "status": status.as_str(), "ids": ids_json(&g.ids) });
        note_refused(&mut row, &g.link, st.path.as_deref());
        if let Some(f) = found.filter(|f| f.path != g.link.path) {
            row["found"] = json!(f.path);
        }
        if stale(&st.doc, g) {
            row["preview"] = json!(true);
        }
        links.push(row);
    }
    Ok(json!({ "links": links, "missing": missing, "modified": modified }))
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let ids = ids_param(p, "ids");
    let (mut reads, mut missing) = (vec![], vec![]);
    for g in groups(&st.doc, ids.as_deref()).into_iter().filter(|g| !g.ids.is_empty()) {
        match probe(&g, st.path.as_deref()) {
            (Status::Missing, _) | (_, None) => missing.extend(ids_json(&g.ids)),
            // Unchanged and showing the file: nothing to read unless asked for by id.
            (Status::Ok, Some(_)) if ids.is_none() && !stale(&st.doc, &g) => {}
            // Unreadable as an image (or a document) counts as missing.
            (_, Some(mut f)) => match read_for(&mut f, &g, "links.update") {
                Ok(read) => reads.push((g.ids, read)),
                Err(_) => missing.extend(ids_json(&g.ids)),
            },
        }
    }
    let updated: Vec<u64> = reads.iter().flat_map(|(ids, _)| ids_json(ids)).collect();
    if !reads.is_empty() {
        s.edit("Update Links", |d, _| reads.into_iter().try_for_each(|(ids, read)| install_read(d, &ids, read, None)))?;
    }
    Ok(json!({ "updated": updated, "missing": missing }))
}

fn relink(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.relink";
    // How the new file takes the image's place this time (default: its Placement Options).
    let preserve = match str_param(p, "preserve") {
        None => None,
        Some(v) => Some(Preserve::from_id(v).ok_or_else(|| bad(C, format!("preserve `{v}` is not transforms|bounds|fileDimensions|fit|fill")))?),
    };
    let st = s.doc()?;
    let folder_param = str_param(p, "folder");
    // With copy: the saved document's folder, which the file is copied into.
    let copy_to = match (bool_or(p, "copy", false), str_param(p, "path")) {
        (false, _) => None,
        (true, None) => return Err(bad(C, "copy needs path: the file to copy into the document's folder")),
        (true, Some(_)) => {
            let doc = st.path.as_deref().map(absolute_path).ok_or_else(|| bad(C, "save the document first: copy puts the file in its folder"))?;
            Some(Path::new(&doc).parent().map(Path::to_path_buf).ok_or_else(|| bad(C, format!("{doc} has no folder")))?)
        }
    };
    // An image or a placed document, with its link.
    let image = |id: &NodeId| {
        st.doc.node(*id).and_then(|n| match &n.kind {
            NodeKind::Image(im) => Some(im.link.as_ref()),
            NodeKind::PlacedDocument(p) => Some(Some(&p.link)),
            _ => None,
        })
    };
    let ids: Vec<NodeId> = match ids_param(p, "ids") {
        Some(ids) => ids,
        None => {
            let selected: Vec<NodeId> = st.selection.objects.iter().copied().filter(|id| image(id).is_some()).collect();
            match folder_param {
                Some(_) if selected.is_empty() => {
                    groups(&st.doc, None).into_iter().filter(|g| probe(g, st.path.as_deref()).0 == Status::Missing).flat_map(|g| g.ids).collect()
                }
                _ => selected,
            }
        }
    };
    if let Some(id) = ids.iter().find(|id| image(id).is_none()) {
        return Err(bad(C, format!("object {} is not an image or a linked file", id.0)));
    }
    if ids.is_empty() {
        return Err(bad(C, "select the images to relink, or pass ids"));
    }
    if copy_to.is_some() && ids.iter().any(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::PlacedDocument(_)))) {
        return Err(bad(C, "copy takes images only: relink placed documents where their files are"));
    }
    // (ids, the file) per file to read.
    let mut files: Vec<(Vec<NodeId>, String)> = vec![];
    let mut not_found = vec![];
    match (str_param(p, "path"), folder_param) {
        (Some(path), _) => files.push((ids, absolute_path(path))),
        (None, Some(dir)) => {
            for id in ids {
                let Some(link) = image(&id).flatten() else { continue };
                let path = absolute_path(&Path::new(dir).join(link.name()).to_string_lossy());
                match files.iter_mut().find(|(_, p)| *p == path) {
                    Some((ids, _)) => ids.push(id),
                    None if file_stamp(&path).is_some() => files.push((vec![id], path)),
                    None => not_found.push(link.name().to_string()),
                }
            }
        }
        (None, None) => return Err(bad(C, "give path (a file) or folder")),
    }
    let mut reads = vec![];
    let (mut copied, mut kept) = (vec![], vec![]);
    for (ids, path) in files {
        let bytes = read_file(&path)?;
        // Images show the file; placed documents each show their artboard or page of it.
        let mut images = vec![];
        let mut documents: Vec<((u32, bool, PageBox), Vec<NodeId>)> = vec![];
        for id in ids {
            match st.doc.node(id).map(|n| &n.kind) {
                Some(NodeKind::PlacedDocument(pl)) => {
                    let k = (pl.page(), pl.bounding, pl.page_box);
                    match documents.iter_mut().find(|(e, _)| *e == k) {
                        Some((_, ids)) => ids.push(id),
                        None => documents.push((k, vec![id])),
                    }
                }
                _ => images.push(id),
            }
        }
        // Read before it is copied: a file that can't be shown isn't copied.
        let img = if images.is_empty() { None } else { Some(linked_image(&bytes).map_err(|e| bad(C, format!("{path}: {e}")))?) };
        let mut sources = vec![];
        for ((page, bounding, page_box), ids) in documents {
            sources.push((page, ids, super::place::document::source(&path, &bytes, Some(&path), page, bounding, page_box, C)?));
        }
        let to = match &copy_to {
            Some(dir) => {
                let (to, new) = copy_into_folder(&path, &bytes, dir)?;
                let pair = json!({ "from": path, "to": to });
                if new {
                    copied.push(pair)
                } else {
                    kept.push(pair)
                }
                to
            }
            None => path,
        };
        let link = link_info(&to, &bytes);
        if let Some(img) = img {
            reads.push((images, Read::Image((link.clone(), img))));
        }
        for (page, ids, src) in sources {
            reads.push((ids, Read::Document(LinkInfo { page: Some(page), ..link.clone() }, src)));
        }
    }
    let relinked: Vec<u64> = reads.iter().flat_map(|(ids, _)| ids_json(ids)).collect();
    if !reads.is_empty() {
        s.edit("Relink", |d, _| reads.into_iter().try_for_each(|(ids, read)| install_read(d, &ids, read, preserve)))?;
    }
    not_found.sort();
    not_found.dedup();
    let mut out = json!({ "relinked": relinked, "notFound": not_found });
    if copy_to.is_some() {
        out["copied"] = json!(copied);
        out["kept"] = json!(kept);
    }
    Ok(out)
}

/// `links.relink {copy}`: put the file at `from` (`bytes`, as just read) into the folder `dir`,
/// never replacing a file there ([`fileio::copy_new`]) → where it is now, and whether it was
/// copied (`false`: a file there with the same contents, of its name or a numbered one, is used).
#[cfg(not(target_arch = "wasm32"))]
fn copy_into_folder(from: &str, bytes: &[u8], dir: &Path) -> Result<(String, bool)> {
    use std::io::Write as _;
    let source = Path::new(from);
    let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // Each name it takes is checked against the automation roots in force.
    let create = |to: &Path| {
        crate::file_access::check_write(&to.to_string_lossy()).map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e))?;
        vectorcraft_format::write_new_with(to, |f| f.write_all(bytes))
    };
    // A file there that automation may not read isn't taken for the same file.
    let same = |to: &Path, m: &std::fs::Metadata| {
        m.len() == bytes.len() as u64 && crate::file_access::check_read(&to.to_string_lossy()).is_ok() && std::fs::read(to).is_ok_and(|b| b == bytes)
    };
    match fileio::copy_new(source, dir, create, same) {
        Ok(Some(fileio::Placed::Copied(to))) => Ok((to.to_string_lossy().into_owned(), true)),
        Ok(Some(fileio::Placed::Kept(to))) => Ok((to.to_string_lossy().into_owned(), false)),
        Ok(None) => Err(EngineError::Other(format!("every numbered name for {name} is taken in the document's folder"))),
        Err(e) => Err(EngineError::Other(format!("{}: {e}", dir.display()))),
    }
}

/// The web has no file system: nothing is copied.
#[cfg(target_arch = "wasm32")]
fn copy_into_folder(_: &str, _: &[u8], _: &Path) -> Result<(String, bool)> {
    Err(bad("links.relink", "copying a file isn't available on the web"))
}

// ---------- the Links panel ----------

/// Image object `id` of `d`.
fn image_of(d: &Document, id: NodeId) -> Option<&ImageObject> {
    match &d.node(id)?.kind {
        NodeKind::Image(im) => Some(im),
        _ => None,
    }
}

/// The selected image objects of `st` that `keep` accepts.
fn selected_images(st: &crate::DocState, keep: impl Fn(&ImageObject) -> bool) -> Vec<NodeId> {
    st.selection.objects.iter().copied().filter(|id| image_of(&st.doc, *id).is_some_and(&keep)).collect()
}

/// `id`, else the first selected image.
fn image_param(st: &crate::DocState, p: &Value, cmd: &str) -> Result<NodeId> {
    let id = id_param(p, "id").or_else(|| selected_images(st, |_| true).first().copied()).ok_or_else(|| bad(cmd, "select an image, or pass id"))?;
    match image_of(&st.doc, id) {
        Some(_) => Ok(id),
        None if st.doc.node(id).is_none() => Err(crate::EngineError::NoNode(id)),
        None => Err(bad(cmd, format!("object {} is not an image", id.0))),
    }
}

/// The Placement Options of image or placed document `n`.
fn placement_mut(n: &mut Node) -> Option<&mut PlacementOptions> {
    match &mut n.kind {
        NodeKind::Image(im) => Some(&mut im.placement),
        NodeKind::PlacedDocument(p) => Some(&mut p.placement),
        _ => None,
    }
}

/// The Placement Options of image or placed document `id` of `d`.
fn placement_of(d: &Document, id: NodeId) -> Option<PlacementOptions> {
    match &d.node(id)?.kind {
        NodeKind::Image(im) => Some(im.placement),
        NodeKind::PlacedDocument(p) => Some(p.placement),
        _ => None,
    }
}

/// `id`, else the first selected image or placed document.
fn linked_param(st: &crate::DocState, p: &Value, cmd: &str) -> Result<NodeId> {
    let first = || st.selection.objects.iter().copied().find(|id| placement_of(&st.doc, *id).is_some());
    let id = id_param(p, "id").or_else(first).ok_or_else(|| bad(cmd, "select an image, or pass id"))?;
    match placement_of(&st.doc, id) {
        Some(_) => Ok(id),
        None if st.doc.node(id).is_none() => Err(crate::EngineError::NoNode(id)),
        None => Err(bad(cmd, format!("object {} is not an image or a linked file", id.0))),
    }
}

/// Per linked image in the layers of `st`: its file's status, where it was found when away from
/// its path, and whether the image shows its preview (each file probed once).
fn statuses(st: &crate::DocState) -> BTreeMap<NodeId, (Status, Option<String>, bool)> {
    let mut out = BTreeMap::new();
    for g in groups(&st.doc, None).iter().filter(|g| !g.ids.is_empty()) {
        let (status, found) = probe(g, st.path.as_deref());
        let found = found.map(|f| f.path).filter(|p| *p != g.link.path);
        let preview = stale(&st.doc, g);
        out.extend(g.ids.iter().map(|id| (*id, (status, found.clone(), preview))));
    }
    out
}

/// The format of the file image `im` of `d` shows: its link's extension, else its pixels' type.
fn format_label(d: &Document, im: &ImageObject) -> &'static str {
    im.link
        .as_ref()
        .and_then(|l| fileio::format_for_name(l.name()))
        .or_else(|| d.images.get(&im.key).and_then(|b| fileio::FORMATS.iter().find(|f| f.raster && f.mime == b.mime)))
        .map_or("Image", |f| f.label)
}

/// A placed document's `links.list` fields: `format` (`VectorCraft`, or the PDF or `.ai` file's
/// format), `document: true` and its artboard's or page's size (`pageWidth`, `pageHeight`, pt)
/// instead of pixels.
fn document_row(p: &PlacedDocument) -> Value {
    let pdf = fileio::format_for_name(p.link.name()).filter(|f| matches!(f.id, "pdf" | "ai" | "ait"));
    json!({ "format": pdf.map_or("VectorCraft", |f| f.label), "document": true, "pageWidth": p.width, "pageHeight": p.height })
}

/// Sort rank of a `links.list` status: missing, modified, ok, embedded.
fn status_rank(status: &str) -> usize {
    ["missing", "modified", "ok", "embedded"].iter().position(|s| *s == status).unwrap_or(4)
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.list";
    let st = s.doc()?;
    let show = str_param(p, "show").unwrap_or("all");
    if !matches!(show, "all" | "missing" | "modified" | "embedded") {
        return Err(bad(C, format!("show `{show}`: all, missing, modified or embedded")));
    }
    let statuses = statuses(st);
    let mut objects = vec![];
    st.doc.walk(|n| {
        if matches!(n.kind, NodeKind::Image(_) | NodeKind::PlacedDocument(_)) {
            objects.push(n);
        }
    });
    // Top first, as the Layers panel lists them.
    objects.reverse();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut rows = vec![];
    for n in objects {
        let (status, found, preview) = match statuses.get(&n.id) {
            Some((s, f, p)) => (s.as_str(), f.as_deref(), *p),
            None => ("embedded", None, false),
        };
        *counts.entry(status).or_default() += 1;
        if show != "all" && show != status {
            continue;
        }
        let (link, mut row) = match &n.kind {
            NodeKind::Image(im) => {
                let mut row = json!({ "format": format_label(&st.doc, im), "pixelWidth": im.width, "pixelHeight": im.height });
                // Only an image that could be past the decode allowance (at 16 bytes a pixel, the
                // most a decoder gives) has its header read for this.
                let large = im.width as u64 * im.height as u64 > fileio::MAX_RASTER_ALLOC / 16;
                if let Some(note) = st.doc.images.get(&im.key).filter(|_| large).and_then(|b| fileio::size_note(&b.bytes)) {
                    row["note"] = json!(note);
                }
                (im.link.as_ref(), row)
            }
            NodeKind::PlacedDocument(p) => (Some(&p.link), document_row(p)),
            _ => continue,
        };
        let name = link.map_or_else(|| n.display_name(), |l| l.name().to_string());
        if let Some(o) = row.as_object_mut() {
            o.insert("id".into(), json!(n.id.0));
            o.insert("name".into(), json!(name));
            o.insert("linked".into(), json!(link.is_some()));
            o.insert("status".into(), json!(status));
        }
        if let Some(l) = link {
            row["path"] = json!(l.path);
            if let Some(page) = l.page {
                row["page"] = json!(page);
            }
        }
        if let Some(f) = found {
            row["found"] = json!(f);
        }
        if preview {
            row["preview"] = json!(true);
        }
        rows.push(row);
    }
    let text = |r: &Value, k: &str| r[k].as_str().unwrap_or_default().to_lowercase();
    match str_param(p, "sort") {
        None => {}
        Some("name") => rows.sort_by_cached_key(|r| text(r, "name")),
        Some("kind") => rows.sort_by_cached_key(|r| (text(r, "format"), text(r, "name"))),
        Some("status") => rows.sort_by_cached_key(|r| (status_rank(r["status"].as_str().unwrap_or_default()), text(r, "name"))),
        Some(x) => return Err(bad(C, format!("sort `{x}`: name, kind or status"))),
    }
    let count = |k: &str| counts.get(k).copied().unwrap_or(0);
    Ok(json!({ "links": rows, "missing": count("missing"), "modified": count("modified"), "embedded": count("embedded") }))
}

fn go_to(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let id = linked_param(st, p, "links.goTo")?;
    let b = st.doc.node(id).and_then(Node::geometric_bounds).unwrap_or_default();
    s.select(|_, sel| sel.set([id]))?;
    Ok(json!({ "id": id.0, "bounds": [b.x0, b.y0, b.x1, b.y1] }))
}

fn embed(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.embed";
    let st = s.doc()?;
    let kind = |id: &NodeId| st.doc.node(*id).map(|n| &n.kind);
    let linked = |id: &NodeId| match kind(id) {
        Some(NodeKind::Image(im)) => im.link.is_some(),
        Some(NodeKind::PlacedDocument(_)) => true,
        _ => false,
    };
    let ids = ids_param(p, "ids").unwrap_or_else(|| st.selection.objects.iter().copied().filter(linked).collect());
    if let Some(id) = ids.iter().find(|id| !matches!(kind(id), Some(NodeKind::Image(_) | NodeKind::PlacedDocument(_)))) {
        return Err(bad(C, format!("object {} is not an image or a placed document", id.0)));
    }
    if ids.is_empty() {
        return Err(bad(C, "select the linked images or placed documents, or pass ids"));
    }
    // Placed documents become editable copies of their art: those whose file can be read.
    let (mut documents, mut missing) = (vec![], vec![]);
    for id in ids.iter().copied() {
        let Some(NodeKind::PlacedDocument(p)) = kind(&id) else { continue };
        let readable = st.doc.images.get(&p.key).is_some_and(|b| vectorcraft_doc::placed_document::full_bytes(p, b).is_some());
        if readable { documents.push(id) } else { missing.push(id) }
    }
    let ids: Vec<NodeId> = ids.into_iter().filter(|id| matches!(kind(id), Some(NodeKind::Image(_)))).collect();
    // Images showing their preview take their file's pixels first.
    let (mut done, mut reads) = (vec![], vec![]);
    for g in groups(&st.doc, Some(&ids)) {
        if !previewing(&st.doc, &g.key) {
            done.extend(g.ids);
            continue;
        }
        match probe(&g, st.path.as_deref()) {
            (Status::Ok | Status::Modified, Some(mut f)) => match read_image(&mut f, &g.link) {
                Ok(read) => {
                    done.extend(&g.ids);
                    reads.push((g.ids, read));
                }
                Err(_) => missing.extend(g.ids),
            },
            _ => missing.extend(g.ids),
        }
    }
    if !done.is_empty() || !documents.is_empty() {
        s.edit("Embed", |d, _| {
            for (ids, read) in reads {
                install(d, &ids, read, Some(Preserve::Bounds))?;
            }
            for id in &done {
                if let Some(Node { kind: NodeKind::Image(im), .. }) = d.node_mut(*id) {
                    im.link = None;
                }
            }
            documents.iter().try_for_each(|id| super::place::document::expand(d, *id, C))
        })?;
    }
    done.extend(documents);
    Ok(json!({ "embedded": ids_json(&done), "missing": ids_json(&missing) }))
}

/// The raster formats Unembed writes, by extension.
const UNEMBED_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "tif", "tiff", "bmp"];

/// Encoded image `bytes` re-encoded in the format of extension `ext` (JPEG without alpha).
fn convert_image(bytes: &[u8], ext: &str) -> std::result::Result<Vec<u8>, String> {
    let format = image::ImageFormat::from_extension(ext).ok_or_else(|| format!("can't write .{ext} images"))?;
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let img = if format == image::ImageFormat::Jpeg { image::DynamicImage::ImageRgb8(img.to_rgb8()) } else { img };
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), format).map_err(|e| e.to_string())?;
    Ok(out)
}

fn unembed(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.unembed";
    let st = s.doc()?;
    let id = image_param(st, p, C)?;
    let n = st.doc.node(id).ok_or(crate::EngineError::NoNode(id))?;
    let im = image_of(&st.doc, id).ok_or_else(|| bad(C, format!("object {} is not an image", id.0)))?;
    if im.link.is_some() {
        return Err(bad(C, format!("image {} is linked already", id.0)));
    }
    let blob = st.doc.images.get(&im.key).filter(|b| !b.is_proxy()).ok_or_else(|| bad(C, "the image's pixels aren't in the document"))?;
    // As stored (PNG, JPEG, GIF, WebP), unless the path names another format.
    let stored = fileio::FORMATS.iter().find(|f| f.raster && f.mime == blob.mime).map_or("png", |f| f.extensions.first().copied().unwrap_or("png"));
    let path = str_param(p, "path");
    let ext = match path.map(fileio::extension) {
        Some(e) if UNEMBED_EXTS.contains(&e.as_str()) => e,
        Some(e) if !e.is_empty() => return Err(bad(C, format!("can't write .{e} images: use .png, .jpg, .gif, .webp, .tif or .bmp"))),
        _ => stored.to_string(),
    };
    let same = fileio::format(&ext).is_some_and(|f| f.mime == blob.mime);
    let bytes = if same { blob.bytes.to_vec() } else { convert_image(&blob.bytes, &ext).map_err(|e| bad(C, e))? };
    let Some(path) = path else {
        let name = n.display_name();
        let stem = Path::new(&name).file_stem().map_or_else(|| name.clone(), |s| s.to_string_lossy().into_owned());
        return Ok(json!({ "name": format!("{stem}.{ext}"), "dataBase64": vectorcraft_format::base64_encode(&bytes) }));
    };
    let path = absolute_path(&if fileio::extension(path).is_empty() { format!("{path}.{ext}") } else { path.to_string() });
    // Read before writing: pixels that can't be read leave no file behind.
    let img = linked_image(&bytes)?;
    write_file(&path, &bytes)?;
    let read = (link_info(&path, &bytes), img);
    s.edit("Unembed", |d, _| install(d, &[id], read, Some(Preserve::Bounds)))?;
    Ok(json!({ "id": id.0, "path": path }))
}

/// Placement Options as `{preserve, align, clip}`.
fn placement_json(o: PlacementOptions) -> Value {
    json!({ "preserve": o.preserve.id(), "align": o.align.id(), "clip": o.clip })
}

/// Link Info's fields of the linked file `l` (of object `id` of `st`) into `out`.
fn file_fields(st: &crate::DocState, id: NodeId, l: &LinkInfo, out: &mut Value) {
    // Within what the caller that opened the document may read, whoever asks.
    crate::file_access::confine(st.opened_by.as_ref(), || file_fields_here(st, id, l, out));
}

fn file_fields_here(st: &crate::DocState, id: NodeId, l: &LinkInfo, out: &mut Value) {
    let status = groups(&st.doc, Some(&[id])).first().map_or(Status::Missing, |g| probe(g, st.path.as_deref()).0);
    let doc_path = st.path.as_deref();
    let path = fileio::document_path(&l.path, doc_path).ok();
    let stamp = path.as_deref().and_then(file_stamp);
    let folder_len = l.path.len().saturating_sub(l.name().len());
    out["status"] = json!(status.as_str());
    note_refused(out, l, doc_path);
    out["path"] = json!(l.path);
    out["fileName"] = json!(l.name());
    out["location"] = json!(l.path.get(..folder_len).unwrap_or_default().trim_end_matches(['/', '\\']));
    for (k, v) in [
        ("page", l.page.map(u64::from)),
        ("fileSize", stamp.map(|s| s.0).or(l.size)),
        ("modified", stamp.and_then(|s| s.1).or(l.modified)),
        ("created", path.as_deref().and_then(file_created)),
    ] {
        if let Some(v) = v {
            out[k] = json!(v);
        }
    }
}

/// Scale (% of 100%, given points per unit at 100%) and counter-clockwise rotation of `xf`.
fn scale_rotation(xf: Affine, (px, py): (f64, f64)) -> (Value, Value) {
    let [a, b, c, d, ..] = xf.as_coeffs();
    // Counter-clockwise on the page (y points down). Adding 0 turns -0 into 0.
    (json!([a.hypot(b) / px * 100.0, c.hypot(d) / py * 100.0]), json!((-b).atan2(a).to_degrees() + 0.0))
}

/// Link Info of placed document `id`.
fn placed_info(st: &crate::DocState, id: NodeId) -> Result<Value> {
    let n = st.doc.node(id).ok_or(crate::EngineError::NoNode(id))?;
    let NodeKind::PlacedDocument(pl) = &n.kind else { return Err(bad("links.info", format!("object {} is not a placed document", id.0))) };
    let size = n.geometric_bounds().unwrap_or_default();
    let (scale, rotation) = scale_rotation(pl.xf, (1.0, 1.0));
    let mut out = document_row(pl);
    let fields = [
        ("id", json!(id.0)),
        ("name", json!(n.display_name())),
        ("linked", json!(true)),
        ("link", json!(pl.link.path)),
        ("width", json!(size.width())),
        ("height", json!(size.height())),
        ("scale", scale),
        ("rotation", rotation),
        ("placement", placement_json(pl.placement)),
    ];
    if let Some(o) = out.as_object_mut() {
        o.extend(fields.into_iter().map(|(k, v)| (k.to_string(), v)));
    }
    file_fields(st, id, &pl.link, &mut out);
    Ok(out)
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.info";
    let id = linked_param(s.doc()?, p, C)?;
    if image_of(&s.doc()?.doc, id).is_none() {
        return placed_info(s.doc()?, id);
    }
    let mut out = super::place::image_info(s, &json!({ "id": id.0 }))?;
    let st = s.doc()?;
    let im = image_of(&st.doc, id).ok_or_else(|| bad(C, format!("object {} is not an image", id.0)))?;
    let (px, py) = file_pt(&st.doc, &im.key);
    out["effectivePpi"] = out["ppi"].take();
    out["ppi"] = json!([72.0 / px, 72.0 / py]);
    (out["scale"], out["rotation"]) = scale_rotation(im.xf, (px, py));
    out["format"] = json!(format_label(&st.doc, im));
    out["placement"] = placement_json(im.placement);
    out["status"] = json!("embedded");
    if let Some(l) = &im.link {
        file_fields(st, id, l, &mut out);
    }
    Ok(out)
}

fn placement_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.placementOptions";
    let st = s.doc()?;
    let ids = ids_param(p, "ids").unwrap_or_else(|| st.selection.objects.iter().copied().filter(|id| placement_of(&st.doc, *id).is_some()).collect());
    if let Some(id) = ids.iter().find(|id| placement_of(&st.doc, **id).is_none()) {
        return Err(bad(C, format!("object {} is not an image or a linked file", id.0)));
    }
    if ids.is_empty() {
        return Err(bad(C, "select the images, or pass ids"));
    }
    let choice = |k: &str| -> Result<Option<&str>> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_str().map(Some).ok_or_else(|| bad(C, format!("{k} must be a string"))),
        }
    };
    let unknown = |k: &str, v: &str| bad(C, format!("{k} `{v}` is not one of the choices"));
    let preserve = choice("preserve")?.map(|v| Preserve::from_id(v).ok_or_else(|| unknown("preserve", v))).transpose()?;
    let align = choice("align")?.map(|v| Align::from_id(v).ok_or_else(|| unknown("align", v))).transpose()?;
    let clip = match p.get("clip") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| bad(C, "clip must be true or false"))?),
    };
    let first = ids.first().and_then(|id| placement_of(&st.doc, *id)).unwrap_or_default();
    if preserve.is_none() && align.is_none() && clip.is_none() {
        return Ok(json!({ "placement": placement_json(first) }));
    }
    let set = |o: &mut PlacementOptions| {
        o.preserve = preserve.unwrap_or(o.preserve);
        o.align = align.unwrap_or(o.align);
        o.clip = clip.unwrap_or(o.clip);
    };
    let embedded: Vec<NodeId> = ids.iter().copied().filter(|id| image_of(&st.doc, *id).is_some_and(|im| im.link.is_none())).collect();
    let (applied, previewing) = s.edit("Placement Options", |d, _| {
        // Where Clip to Bounding Box already keeps the art, as the options were.
        let within: Vec<Option<Clip>> = ids.iter().map(|id| clip_frame(d, *id)).collect();
        for id in &ids {
            if let Some(o) = d.node_mut(*id).and_then(placement_mut) {
                set(o);
            }
        }
        apply_placement(d, &ids, &within)
    })?;
    let mut shown = first;
    set(&mut shown);
    let mut out = json!({ "ids": ids_json(&ids), "placement": placement_json(shown), "applied": ids_json(&applied) });
    if !embedded.is_empty() {
        out["embedded"] = json!(ids_json(&embedded));
    }
    if !previewing.is_empty() {
        out["previewing"] = json!(ids_json(&previewing));
    }
    Ok(out)
}

/// Links panel › Placement Options' OK: the linked images and placed documents among `ids` of `d`
/// take their place again at once, as their Placement Options now say, from the file they already
/// show: the "old" art is each one as it is placed and the "new" art is the same file, so a
/// distorted image fitted or filled into its bounds is in proportion again, and one put at File
/// Dimensions is at the file's 100% size, unrotated. Nothing is read from disk (the web build has
/// no files), so Fit, Fill and Clip work while the file is missing too; File Dimensions waits for
/// the file.
///
/// - Bounds and Transforms move nothing: the same file in the same bounds, at the same scale.
/// - Embedded images keep their place (Placement Options are for linked files); their options are
///   still set, for when they are relinked.
/// - File Dimensions leaves a linked image showing only its preview in place: the preview doesn't
///   keep the file's resolution, so its 100% size is unknown.
/// - Clip to Bounding Box clips art that now overflows its old bounds, in a clip group of its own.
///   Art it already clips is placed in that clip's rectangle (`within`, each id's
///   [`clip_frame`] before the options changed), so a filled image changes its alignment in it.
///   A clip group already holding it is never reshaped: one whose clipping path is already that
///   rectangle is kept as it is, and inside any other (a mask the user drew, a crop) the art gets
///   a clip group of its own, so both clip it. Without Clip, an existing clip group is left alone.
///
/// → the objects that moved, changed size or were clipped, and the images File Dimensions left in
/// place for want of their file's resolution.
fn apply_placement(d: &mut Document, ids: &[NodeId], within: &[Option<Clip>]) -> Result<(Vec<NodeId>, Vec<NodeId>)> {
    let (mut applied, mut previewing) = (vec![], vec![]);
    for (i, id) in ids.iter().enumerate() {
        let Some(n) = d.node(*id) else { continue };
        let (xf, size, pt, o) = match &n.kind {
            NodeKind::Image(im) if im.link.is_some() => {
                if im.placement.preserve == Preserve::FileDimensions && self::previewing(d, &im.key) {
                    previewing.push(*id);
                    continue;
                }
                (im.xf, (im.width as f64, im.height as f64), file_pt(d, &im.key), im.placement)
            }
            NodeKind::PlacedDocument(p) => (p.xf, (p.width, p.height), (1.0, 1.0), p.placement),
            _ => continue,
        };
        let empty = !(size.0 > 0.0 && size.1 > 0.0) || !xf.as_coeffs().iter().all(|c| c.is_finite());
        if empty || matches!(o.preserve, Preserve::Bounds | Preserve::Transforms) {
            continue;
        }
        let (placed, clip) = placed_xf(xf, size, pt, size, pt, o, within.get(i).copied().flatten());
        let moved = xf.as_coeffs().iter().zip(placed.as_coeffs()).any(|(a, b)| (a - b).abs() > 1e-9 * (1.0 + a.abs()));
        if moved && let Some(n) = d.node_mut(*id) {
            match &mut n.kind {
                NodeKind::Image(im) => im.xf = placed,
                NodeKind::PlacedDocument(p) => p.xf = placed,
                _ => {}
            }
        }
        let clipped = match clip {
            Some(c) => clip_applied(d, *id, c)?,
            None => false,
        };
        if moved || clipped {
            applied.push(*id);
        }
    }
    Ok((applied, previewing))
}

/// Clip object `id` of `d` to `room` in `frame` for [`apply_placement`] → whether it was clipped
/// now: false when the clip group holding just it already clips it to that rectangle.
fn clip_applied(d: &mut Document, id: NodeId, (frame, room): Clip) -> Result<bool> {
    let path = shapes::rectangle(Rect::new(0.0, 0.0, room.0, room.1)).transformed(frame);
    let same = own_clip_path(d, id).and_then(|c| d.node(c)).is_some_and(|c| match &c.kind {
        NodeKind::Path { path: p, .. } => same_rectangle(p, &path),
        _ => false,
    });
    if same {
        return Ok(false);
    }
    wrap_in_clip(d, id, path)?;
    Ok(true)
}

/// Whether path `p` is the rectangle `rect` (four corners with no handles), from any corner and in
/// either direction.
fn same_rectangle(p: &vectorcraft_geom::PathData, rect: &vectorcraft_geom::PathData) -> bool {
    let corners = |p: &vectorcraft_geom::PathData| match &p.subpaths[..] {
        [sp] if sp.closed && sp.anchors.len() == 4 && sp.anchors.iter().all(|a| a.h_in == a.p && a.h_out == a.p) => {
            Some(sp.anchors.iter().map(|a| a.p).collect::<Vec<_>>())
        }
        _ => None,
    };
    let (Some(a), Some(b)) = (corners(p), corners(rect)) else { return false };
    let scale = b.iter().fold(1.0_f64, |m, q| m.max(q.x.abs()).max(q.y.abs()));
    b.iter().all(|q| a.iter().any(|c| (c.x - q.x).abs() <= 1e-6 * scale && (c.y - q.y).abs() <= 1e-6 * scale))
}
