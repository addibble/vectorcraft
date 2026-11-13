use super::build::make_face;
use super::build::{style_italic, style_weight};
#[cfg(not(target_arch = "wasm32"))]
use super::cache::CatalogFace;
use super::cache::{CanonicalNames, Catalog, GlyphCache, PostScriptNames};
#[cfg(not(target_arch = "wasm32"))]
use super::dirs::confined_font_dirs;
use super::face::{BUNDLED, FontBytes, FontFace, Instances};
#[cfg(not(target_arch = "wasm32"))]
use super::names::SysFallback;
use super::names::{name, norm};
use super::scan::enumerate_faces;
#[cfg(not(target_arch = "wasm32"))]
use super::scan::{file_face_names, is_font_file};
#[cfg(test)]
use super::traits::faces_published;
#[cfg(not(target_arch = "wasm32"))]
use super::traits::{Alias, FaceStyle};
use super::{
    Arc, AtomicU64, FontDb, FontPaths, FontSource, FontsAdded, HashMap, Mutex, Ordering, Path, PathBuf, RwLock, SourceRequests, StringId, source_key,
};
#[cfg(not(target_arch = "wasm32"))]
use super::{ScanStamp, modified};

/// The next [`FontDb::id`].
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

impl FontDb {
    /// A database holding the bundled fonts, whose system font scan reads `font_dirs` (folders,
    /// read with their subfolders, and font files).
    pub fn with_font_dirs(font_dirs: Vec<PathBuf>) -> Self {
        Self::new(FontPaths::Fixed(font_dirs))
    }

    /// Same scan paths as [`FontDb::global`] (platform lister and font folders), for tests.
    #[cfg(test)]
    pub(crate) fn for_system_scan_test() -> Self {
        Self::new(FontPaths::System)
    }

    fn new(font_paths: FontPaths) -> Self {
        let mut faces = Vec::new();
        // Bundled fonts first, then explicitly supplied craft-fonts faces: Japanese (Mincho
        // first), Arabic, Thai, then remaining faces in manifest order. Existing script fallback
        // priorities stay ahead of the additional families users can now pick by name.
        let craft = crate::craft_fonts::document_fonts().into_iter().map(|f| f.bytes);
        for data in BUNDLED.iter().copied().chain(craft) {
            for (i, spec) in enumerate_faces(data) {
                if let Some(f) = make_face(FontBytes::Static(data), i, spec, None) {
                    faces.push(Arc::new(f));
                }
            }
        }
        Self {
            faces: RwLock::new(faces),
            outlines: Mutex::new(HashMap::new()),
            images: Mutex::new(GlyphCache::default()),
            colours: Mutex::new(GlyphCache::default()),
            catalog: RwLock::new(Catalog::new()),
            postscript: RwLock::new(PostScriptNames::new()),
            font_paths,
            aliases: RwLock::new(HashMap::new()),
            canonical_cache: Mutex::new(CanonicalNames::new()),
            #[cfg(not(target_arch = "wasm32"))]
            cataloged: std::sync::OnceLock::new(),
            #[cfg(not(target_arch = "wasm32"))]
            version_scanned: RwLock::new(std::collections::HashSet::new()),
            family_cache: Mutex::new(None),
            menu_cache: Mutex::new(None),
            sources: RwLock::new(super::Sources::default()),
            source_requests: Mutex::new(SourceRequests::default()),
            supplied_generation: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            #[cfg(not(target_arch = "wasm32"))]
            sys: Mutex::new(SysFallback { enabled: true, ..Default::default() }),
            #[cfg(not(target_arch = "wasm32"))]
            stamp: Mutex::new(None),
            instances: Mutex::new(Instances::default()),
            composites: RwLock::new(HashMap::new()),
        }
    }

    /// Enable or disable the lazy system-font fallback for characters the loaded fonts lack
    /// (native only; on by default).
    pub fn set_system_fallback(&self, on: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.sys.lock().unwrap_or_else(|e| e.into_inner()).enabled = on;
        }
        #[cfg(target_arch = "wasm32")]
        let _ = on;
    }

    /// Is the lazy system-font fallback on ([`Self::set_system_fallback`])?
    pub(crate) fn system_fallback_on(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.sys.lock().unwrap_or_else(|e| e.into_inner()).enabled
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    /// Process-wide database: the bundled fonts, then the installed system fonts, cataloged the
    /// first time a lookup by family name needs them (or ahead of time by
    /// [`FontDb::scan_in_background`]).
    pub fn global() -> &'static FontDb {
        static DB: std::sync::OnceLock<FontDb> = std::sync::OnceLock::new();
        DB.get_or_init(|| FontDb::new(FontPaths::System))
    }

    pub(super) fn read_faces(&self) -> std::sync::RwLockReadGuard<'_, Vec<Arc<FontFace>>> {
        self.faces.read().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn read_catalog(&self) -> std::sync::RwLockReadGuard<'_, Catalog> {
        self.ensure_catalog();
        self.catalog.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Fonts were added or the catalog changed: cached family lists and [`FontDb::canonical`]'s
    /// answers are stale.
    pub(super) fn changed(&self) {
        *self.family_cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.canonical_cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// A number that changes whenever the available fonts do (fonts added, system fonts
    /// rescanned): lists built from [`FontDb::family_list`] are current while it doesn't. While a
    /// guard of this thread lends composite fonts other than the ones the database holds
    /// ([`FontDb::lend_composite_fonts`]), it is a number of its own for those definitions.
    pub fn generation(&self) -> u64 {
        self.lent_generation(self.generation.load(Ordering::Relaxed))
    }

    /// A number no other database made in this process has: with [`FontDb::generation`], it tells
    /// what a list built from a database was built from.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Family names available (loaded plus installed system fonts), sorted and deduplicated.
    pub fn families(&self) -> Vec<String> {
        self.family_list().to_vec()
    }

    /// [`FontDb::families`], shared: cheap to call every frame.
    pub fn family_list(&self) -> Arc<[String]> {
        let catalog = self.read_catalog();
        let mut cache = self.family_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(list) = cache.as_ref() {
            return list.clone();
        }
        let mut v: Vec<String> = self.read_faces().iter().map(|f| f.family.clone()).collect();
        v.extend(catalog.values().map(|c| c.name.clone()));
        v.extend(self.sources.read().unwrap_or_else(|e| e.into_inner()).by_family.values().filter_map(|of| of.first()).map(|s| s.family.clone()));
        v.sort_by_key(|a| a.to_lowercase());
        v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let list: Arc<[String]> = v.into();
        *cache = Some(list.clone());
        list
    }

    /// The families font menus and lists show: [`FontDb::family_list`] without the system's hidden
    /// families, whose names start with "." (macOS keeps ".SF NS", ".LastResort" and others for its
    /// own interface, and Mac apps don't list them). Those still resolve by name, for documents and
    /// fallbacks that name them. The composite fonts ([`FontDb::composite_font_names`]) come first.
    /// Cheap to call every frame.
    pub fn menu_family_list(&self) -> Arc<[String]> {
        let all = self.family_list();
        let mut cache = self.menu_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((from, list)) = cache.as_ref()
            && Arc::ptr_eq(from, &all)
        {
            return list.clone();
        }
        let composites = self.composite_font_names();
        let list: Arc<[String]> = composites.into_iter().chain(all.iter().filter(|f| !f.starts_with('.')).cloned()).collect();
        *cache = Some((all, list.clone()));
        list
    }

    /// Whether the installed fonts are cataloged, so lookups by family name no longer wait for
    /// the scan. Always true on wasm, which has no installed fonts.
    pub fn installed_fonts_ready(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.cataloged.get().is_some()
        }
        #[cfg(target_arch = "wasm32")]
        true
    }

    /// [`FontDb::menu_family_list`] without waiting for the installed fonts' scan: until it ends,
    /// the families already loaded (the bundled fonts and any loaded meanwhile), sorted, without
    /// the hidden ones. For what is built every frame, such as the menu bar's Type › Font, so the
    /// window's first frame doesn't wait for the scan of a large font library (#5483).
    /// [`FontDb::generation`] changes when the scan ends.
    pub fn menu_family_list_now(&self) -> Arc<[String]> {
        if self.installed_fonts_ready() {
            return self.menu_family_list();
        }
        let mut v: Vec<String> = self.read_faces().iter().map(|f| f.family.clone()).filter(|f| !f.starts_with('.')).collect();
        v.sort_by_key(|a| a.to_lowercase());
        v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        self.composite_font_names().into_iter().chain(v).collect()
    }

    /// How a font menu labels `family`: the English name when `english` is on (Preferences › Type ›
    /// Show Font Names in English, the default), else the font's native-language name when it has
    /// one. The document and commands still use the English family ([`Self::canonical`]).
    pub fn family_display_name(&self, family: &str, english: bool) -> String {
        let (family, _) = self.canonical(family, "");
        if english {
            return family;
        }
        if let Some(native) = self.read_faces().iter().find(|f| f.family.eq_ignore_ascii_case(&family)).and_then(|f| f.native_family.clone()) {
            return native;
        }
        if let Some(native) = self.read_catalog().get(&family.to_ascii_lowercase()).and_then(|c| c.native.clone()) {
            return native;
        }
        family
    }

    /// Style names available for `family`: upright styles by weight, then italics.
    pub fn styles(&self, family: &str) -> Vec<String> {
        let (family, _) = self.canonical(family, "");
        let family = family.as_str();
        let mut v: Vec<(bool, f32, String)> = Vec::new();
        // Each style once, as its first loaded version describes it (versions can disagree on weight).
        for f in self.read_faces().iter().filter(|f| f.family.eq_ignore_ascii_case(family)) {
            if !v.iter().any(|(_, _, s)| s.eq_ignore_ascii_case(&f.style)) {
                v.push((f.italic, f.weight, f.style.clone()));
            }
        }
        if let Some(c) = self.read_catalog().get(&family.to_ascii_lowercase()) {
            for cf in &c.faces {
                if !v.iter().any(|(_, _, s)| s.eq_ignore_ascii_case(&cf.style)) {
                    v.push((cf.italic, cf.weight, cf.style.clone()));
                }
            }
        }
        for src in self.sources.read().unwrap_or_else(|e| e.into_inner()).by_family.get(&family.to_ascii_lowercase()).into_iter().flatten() {
            if !v.iter().any(|(_, _, s)| s.eq_ignore_ascii_case(&src.style)) {
                v.push((style_italic(&src.style), style_weight(&src.style), src.style.clone()));
            }
        }
        v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
        v.dedup_by(|a, b| a.2 == b.2);
        v.into_iter().map(|t| t.2).collect()
    }

    /// The style a style-less request gets for `family`: its face nearest Regular upright, by the
    /// catalog's or the loaded faces' metadata. Unlike [`FontDb::face`] it reads no font files, so
    /// the closest styles of many families cost little.
    pub fn closest_style(&self, family: &str) -> Option<String> {
        let (family, _) = self.canonical(family, "");
        let family = family.as_str();
        let mut best: Option<(f32, String)> = None;
        let consider = |best: &mut Option<(f32, String)>, weight: f32, italic: bool, style: &str| {
            let d = super::build::weight_distance(weight, italic, 400.0, false);
            if best.as_ref().is_none_or(|(b, _)| d < *b) {
                *best = Some((d, style.to_string()));
            }
        };
        for f in self.read_faces().iter().filter(|f| f.family.eq_ignore_ascii_case(family)) {
            consider(&mut best, f.weight, f.italic, &f.style);
        }
        if let Some(c) = self.read_catalog().get(&family.to_ascii_lowercase()) {
            for cf in &c.faces {
                consider(&mut best, cf.weight, cf.italic, &cf.style);
            }
        }
        best.map(|(_, style)| style)
    }

    /// The most font sources a database keeps ([`FontDb::add_sources`]).
    pub const MAX_SOURCES: usize = 20_000;

    /// Fonts a host can supply ([`FontSource`]): listed in the font menus from now on and
    /// requested when type names their family. A family and style already a source is kept as it
    /// was, unless it failed to load ([`FontDb::source_settled`]): given again, it replaces the
    /// failed one and, when type had asked for it, is asked for again at once. Returns how many
    /// were added; `None`, adding none, when the new ones would pass [`FontDb::MAX_SOURCES`] in
    /// all.
    pub fn add_sources(&self, list: Vec<FontSource>) -> Option<usize> {
        let mut sources = self.sources.write().unwrap_or_else(|e| e.into_inner());
        let known = |sources: &super::Sources, src: &FontSource| {
            sources.by_family.get(&src.family.to_ascii_lowercase()).is_some_and(|of| of.iter().any(|s| s.style.eq_ignore_ascii_case(&src.style)))
        };
        let mut new: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
        for src in list.iter().filter(|src| !known(&sources, src)) {
            new.insert(source_key(src));
            if sources.count.saturating_add(new.len()) > Self::MAX_SOURCES {
                return None;
            }
        }
        let mut requests = self.source_requests.lock().unwrap_or_else(|e| e.into_inner());
        let mut added = 0;
        for src in list {
            let key = source_key(&src);
            let at = sources.by_family.get(&key.0).and_then(|of| of.iter().position(|s| s.style.eq_ignore_ascii_case(&src.style)));
            if let Some(at) = at {
                if requests.settled.get(&key) != Some(&false) {
                    continue;
                }
                // Activate again: a source type asked for is asked for again now.
                requests.settled.remove(&key);
                if requests.asked.contains(&key) {
                    requests.queue.push(src.clone());
                }
                if let Some(s) = sources.by_family.get_mut(&key.0).and_then(|of| of.get_mut(at)) {
                    *s = src;
                }
                added += 1;
                continue;
            }
            sources.by_family.entry(key.0).or_default().push(src);
            sources.count += 1;
            added += 1;
        }
        drop(requests);
        drop(sources);
        if added > 0 {
            self.changed();
        }
        Some(added)
    }

    /// The font sources of `family` taken away, and what was asked of them forgotten: for tests
    /// that give the process's database sources of their own and leave it as they found it.
    #[cfg(any(test, feature = "test-fonts"))]
    pub fn remove_sources(&self, family: &str) {
        let mut sources = self.sources.write().unwrap_or_else(|e| e.into_inner());
        let Some(gone) = sources.by_family.remove(&family.to_ascii_lowercase()) else { return };
        sources.count = sources.count.saturating_sub(gone.len());
        let mut requests = self.source_requests.lock().unwrap_or_else(|e| e.into_inner());
        for src in &gone {
            let key = source_key(src);
            requests.asked.remove(&key);
            requests.settled.remove(&key);
        }
        requests.queue.retain(|q| !q.family.eq_ignore_ascii_case(family));
        drop(requests);
        drop(sources);
        self.changed();
    }

    /// How many fonts a host can supply ([`FontDb::add_sources`]).
    pub fn source_count(&self) -> usize {
        self.sources.read().unwrap_or_else(|e| e.into_inner()).count
    }

    /// The sources type has asked for since the last call, for the host to fetch and add
    /// ([`FontDb::add_font`]); each is asked for once.
    pub fn take_source_requests(&self) -> Vec<FontSource> {
        std::mem::take(&mut self.source_requests.lock().unwrap_or_else(|e| e.into_inner()).queue)
    }

    /// Whether type has asked for sources not taken yet ([`FontDb::take_source_requests`]): a host
    /// that takes them once a frame asks for another frame.
    pub fn has_source_requests(&self) -> bool {
        !self.source_requests.lock().unwrap_or_else(|e| e.into_inner()).queue.is_empty()
    }

    /// The host is done with `src`: `file` is what adding its file did
    /// ([`FontDb::add_font_reporting`]), `None` when it couldn't be fetched. Either way it is no
    /// longer pending. The file counts only when it holds a face of the source's family (by any of
    /// the family's names; its style may be named otherwise, "Book" for a "Regular" source): a file
    /// of another family leaves the type in the fallback face, so it counts as a failure. A failed
    /// source is asked for again only once the host gives it again ([`FontDb::add_sources`]).
    /// Returns whether the source loaded.
    pub fn source_settled(&self, src: &FontSource, file: Option<&FontsAdded>) -> bool {
        let loaded = file.is_some_and(|f| self.holds_family(src, &f.held));
        self.source_requests.lock().unwrap_or_else(|e| e.into_inner()).settled.insert(source_key(src), loaded);
        loaded
    }

    /// Whether a face of `family` in `style` is loaded, however it came, by any of its style's
    /// names (as [`FontDb::find`] takes them).
    fn style_loaded(&self, family: &str, style: &str) -> bool {
        let ns = norm(style);
        self.read_faces().iter().any(|f| f.family.eq_ignore_ascii_case(family) && (norm(&f.style) == ns || f.keys.styles.contains(&ns)))
    }

    /// The source of `family` nearest `style`: the one of that style, else the nearest in slant,
    /// then weight (as [`FontDb::face`] takes the nearest loaded style), the first given of equals.
    /// Whether it is loaded or settled doesn't matter here.
    fn nearest_source(&self, family: &str, style: &str) -> Option<FontSource> {
        let sources = self.sources.read().unwrap_or_else(|e| e.into_inner());
        let of_family = sources.by_family.get(&family.to_ascii_lowercase())?;
        let ns = norm(style);
        if let Some(s) = of_family.iter().find(|s| norm(&s.style) == ns) {
            return Some(s.clone());
        }
        let (italic, weight) = (style_italic(style), style_weight(style));
        let distance = |s: &&FontSource| (style_italic(&s.style) != italic, (style_weight(&s.style) - weight).abs());
        of_family.iter().min_by(|a, b| distance(a).0.cmp(&distance(b).0).then(distance(a).1.total_cmp(&distance(b).1))).cloned()
    }

    /// The source that type set in `family` and `style` asks for: the family's source nearest
    /// that style ([`FontDb::nearest_source`]); `None` when that style is loaded, when no source
    /// supplies the family, or when the source's own face is loaded or the host is done with it
    /// ([`FontDb::source_settled`]).
    pub(super) fn pending_source(&self, family: &str, style: &str) -> Option<FontSource> {
        // Most families are no source's: one lookup, before the loaded faces are looked through.
        let src = self.nearest_source(family, style)?;
        if self.style_loaded(family, style) {
            return None;
        }
        let settled = self.source_requests.lock().unwrap_or_else(|e| e.into_inner()).settled.contains_key(&source_key(&src));
        (!settled && !self.style_loaded(family, &src.style)).then_some(src)
    }

    /// The style a source supplies for type set in `family` and `style` while that style isn't
    /// loaded: the nearest style of the family's sources ([`FontDb::nearest_source`]). `None` when
    /// the style is loaded or no source supplies the family. Type set in a source's family before
    /// its file arrives takes this style, not the fallback face's.
    pub fn source_style(&self, family: &str, style: &str) -> Option<String> {
        let src = self.nearest_source(family, style)?;
        (!self.style_loaded(family, style)).then_some(src.style)
    }

    /// The source of `family` nearest `style` ([`FontDb::nearest_source`]) when it couldn't be
    /// loaded ([`FontDb::source_settled`]) and the style isn't loaded some other way: what a Missing
    /// Fonts dialog's Activate gives again ([`FontDb::add_sources`]).
    pub fn failed_source(&self, family: &str, style: &str) -> Option<FontSource> {
        let src = self.nearest_source(family, style)?;
        let failed = self.source_requests.lock().unwrap_or_else(|e| e.into_inner()).settled.get(&source_key(&src)) == Some(&false);
        (failed && !self.style_loaded(family, style)).then_some(src)
    }

    /// The family a source supplies, as the host named it, for `family` in any case.
    pub fn source_family(&self, family: &str) -> Option<String> {
        let sources = self.sources.read().unwrap_or_else(|e| e.into_inner());
        sources.by_family.get(&family.to_ascii_lowercase()).and_then(|of| of.first()).map(|s| s.family.clone())
    }

    /// Ask for the source of `family` in `style` ([`FontDb::pending_source`]), once.
    pub(super) fn request_source(&self, family: &str, style: &str) {
        let Some(src) = self.pending_source(family, style) else { return };
        let key = source_key(&src);
        let mut requests = self.source_requests.lock().unwrap_or_else(|e| e.into_inner());
        if requests.asked.insert(key) {
            requests.queue.push(src);
        }
    }

    /// Add a user font (TTF/OTF/TTC bytes). Returns the number of faces added (0 if unparseable or
    /// every face was already present).
    pub fn add_font(&self, bytes: Vec<u8>) -> usize {
        self.add_font_from(bytes, None)
    }

    /// [`FontDb::add_font`], telling an unreadable file (no faces found) from one whose faces are
    /// all loaded already: what the file holds and which faces it added.
    pub fn add_font_reporting(&self, bytes: Vec<u8>) -> FontsAdded {
        self.add_font_report(bytes, None)
    }

    /// [`FontDb::add_font`] for the bytes of the file at `path` (when known).
    pub(super) fn add_font_from(&self, bytes: Vec<u8>, path: Option<&Path>) -> usize {
        self.add_font_report(bytes, path).added.len()
    }

    /// [`FontDb::add_font_reporting`] for the bytes of the file at `path` (when known).
    pub(super) fn add_font_report(&self, bytes: Vec<u8>, path: Option<&Path>) -> FontsAdded {
        let report = self.add_fonts([(bytes, path.map(Path::to_path_buf))]);
        if path.is_none() && !report.added.is_empty() {
            self.supplied_generation.fetch_add(1, Ordering::Relaxed);
        }
        report
    }

    /// A number that changes whenever a font file given as bytes ([`FontDb::add_font`],
    /// [`FontDb::add_font_reporting`]: a host's, a font source's) adds a face, and not when a
    /// lookup loads an installed family's files or font sources are listed, as
    /// [`FontDb::generation`] does: what was drawn in the fallback face for a family that wasn't
    /// there is drawn again when it changes.
    pub fn supplied_generation(&self) -> u64 {
        self.supplied_generation.load(Ordering::Relaxed)
    }

    /// The file `bytes` of the font source `src`, which the host fetched: added only when it holds
    /// a face of the source's family (by any of the family's names; its style may be named
    /// otherwise, "Book" for a "Regular" source), so a file of another family joins no menu and
    /// lays out no type. The source is settled either way ([`FontDb::source_settled`]). Returns
    /// what the file holds and added, and whether the source loaded.
    pub fn add_source_file(&self, src: &FontSource, bytes: Vec<u8>) -> (FontsAdded, bool) {
        let held: Vec<(String, String)> = enumerate_faces(&bytes).into_iter().map(|(_, spec)| (spec.family, spec.style)).collect();
        let file = if self.holds_family(src, &held) { self.add_font_reporting(bytes) } else { FontsAdded { found: held.len(), held, added: vec![] } };
        let loaded = self.source_settled(src, Some(&file));
        (file, loaded)
    }

    /// Whether the faces `held` (family, style) include one of `src`'s family, by any of its names.
    fn holds_family(&self, src: &FontSource, held: &[(String, String)]) -> bool {
        let family = self.canonical(&src.family, "").0;
        held.iter().any(|(f, _)| f.eq_ignore_ascii_case(&family) || f.eq_ignore_ascii_case(&src.family))
    }

    /// Add the faces of `files` (each file's bytes, and the file when known) all at once: a lookup
    /// on another thread finds all of them or none, never a family with only some of its styles
    /// loaded, whose nearest style it would take instead (Arial Bold Italic, the first of Arial's
    /// files, for upright Regular text). Returns what the files held and which faces they added.
    pub(super) fn add_fonts(&self, files: impl IntoIterator<Item = (Vec<u8>, Option<PathBuf>)>) -> FontsAdded {
        // Another version of a family and style is kept: a document may name it (the first one
        // loaded stays the one the family and style alone resolve to).
        let same = |f: &FontFace, family: &str, style: &str, version: &str| {
            f.family.eq_ignore_ascii_case(family) && f.style.eq_ignore_ascii_case(style) && f.version == version
        };
        let mut report = FontsAdded::default();
        let mut made: Vec<FontFace> = Vec::new();
        for (bytes, path) in files {
            let data = Arc::new(bytes);
            let faces = enumerate_faces(&data);
            report.found += faces.len();
            report.held.extend(faces.iter().map(|(_, spec)| (spec.family.clone(), spec.style.clone())));
            for (i, spec) in faces {
                let version = skrifa::FontRef::from_index(&data, i).ok().and_then(|f| name(&f, &[StringId::VERSION_STRING])).unwrap_or_default();
                let present = |f: &FontFace| same(f, &spec.family, &spec.style, &version);
                if self.read_faces().iter().any(|f| present(f)) || made.iter().any(present) {
                    continue;
                }
                if let Some(f) = make_face(FontBytes::Owned(data.clone()), i, spec, path.clone()) {
                    made.push(f);
                }
            }
        }
        let mut faces = self.faces.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        for f in made {
            // Another thread may have added the same face meanwhile.
            if !faces.iter().any(|g| same(g, &f.family, &f.style, &f.version)) {
                report.added.push((f.family.clone(), f.style.clone()));
                faces.push(Arc::new(f));
            }
        }
        drop(faces);
        #[cfg(test)]
        faces_published();
        if !report.added.is_empty() {
            self.changed();
        }
        report
    }

    /// Catalog the installed fonts unless that has been done: the first caller scans the font
    /// folders, any other waits for that scan to finish. A no-op on wasm.
    pub(super) fn ensure_catalog(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.cataloged.get_or_init(|| {
            self.scan_font_dirs();
        });
    }

    /// Catalog the installed fonts on a background thread, so the first lookup by family name
    /// (opening a file, the font menus) doesn't wait for the scan. A no-op once they are
    /// cataloged, and on wasm.
    pub fn scan_in_background(&'static self) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.cataloged.get().is_none() {
            // A failed spawn leaves the scan to the first lookup that needs it.
            let _ = std::thread::Builder::new().name("font-scan".into()).spawn(move || self.ensure_catalog());
        }
    }

    /// Scan the font folders again (fonts installed or removed since), cataloging the faces
    /// found (native only). Font data is loaded when a cataloged family is first resolved. Faces
    /// loaded from files the scan no longer finds are dropped. Returns the number of faces
    /// cataloged.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_system_fonts(&self) -> usize {
        // The first scan, or a rescan once it is done (never both at once).
        let mut first = None;
        self.cataloged.get_or_init(|| first = Some(self.scan_font_dirs()));
        first.unwrap_or_else(|| self.scan_font_dirs())
    }

    /// Whether fonts were installed or removed since the last scan (a font folder or font file it
    /// read changed, or a new one is to be read), for [`FontDb::load_system_fonts`] to catalog
    /// them. Costs a look at each folder the scan read, not a scan. `false` before the first scan
    /// and on wasm.
    pub fn installed_fonts_changed(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let paths = self.font_paths.get();
            let stamp = self.stamp.lock().unwrap_or_else(|e| e.into_inner());
            stamp.as_ref().is_some_and(|s| s.paths != paths || s.modified.iter().any(|(p, t)| modified(p) != *t))
        }
        #[cfg(target_arch = "wasm32")]
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn scan_font_dirs(&self) -> usize {
        // Files may have been installed since: other versions are looked for again.
        self.version_scanned.write().unwrap_or_else(|e| e.into_inner()).clear();
        let mut catalog = Catalog::new();
        let mut aliases: HashMap<String, Vec<Alias>> = HashMap::new();
        let mut postscript = PostScriptNames::new();
        let mut n = 0;
        let paths = self.font_paths.get();
        let mut modified_at = Vec::new();
        // Each file with whether it was named by itself rather than found in a folder.
        let mut files = Vec::new();
        let mut stack = paths.clone();
        // Each folder once, however links lead back to it.
        let mut visited = std::collections::HashSet::new();
        let confined = confined_font_dirs();
        let inside_confined = |p: &Path| std::fs::canonicalize(p).is_ok_and(|r| confined.iter().any(|root| r.starts_with(root)));
        while let Some(d) = stack.pop() {
            if !visited.insert(std::fs::canonicalize(&d).unwrap_or_else(|_| d.clone())) {
                continue;
            }
            // In a confined folder, a link is followed only to somewhere inside one.
            let in_confined = !confined.is_empty() && inside_confined(&d);
            // Before listing it: a font installed meanwhile changes it after this.
            modified_at.push((d.clone(), modified(&d)));
            // Not a folder: a font file named by itself (or nothing, yet).
            let Ok(rd) = std::fs::read_dir(&d) else {
                files.push((d, true));
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if in_confined && e.file_type().is_ok_and(|t| t.is_symlink()) && !inside_confined(&p) {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                } else {
                    files.push((p, false));
                }
            }
        }
        for (p, named) in files {
            // A file named by itself is a font whatever its name (font services keep fonts in
            // files without an extension); one of a folder's only with a font's extension, or
            // a suitcase font (whose fonts are in its resource fork, whatever its name).
            if !named && !is_font_file(&p) && !crate::suitcase::is_suitcase(&p) {
                continue;
            }
            for FaceStyle { family, native_family, style, keys, weight, italic, traits, .. } in file_face_names(&p) {
                for (k, a) in Alias::of(&family, &style, &keys) {
                    aliases.entry(k).or_default().push(a);
                }
                // `keys.postscript` is normalized ([`norm`]), as `by_postscript_name` looks it up.
                for ps in &keys.postscript {
                    postscript.insert(ps.clone(), (family.clone(), style.clone()));
                }
                let entry = catalog.entry(family.to_ascii_lowercase()).or_default();
                let kind = entry.traits.get_or_insert(traits);
                kind.variable |= traits.variable;
                if entry.name.is_empty() {
                    entry.name = family;
                }
                if entry.native.is_none() {
                    entry.native = native_family;
                }
                let (weight, italic) = (weight.unwrap_or_else(|| style_weight(&style)), italic || style_italic(&style));
                entry.faces.push(CatalogFace { style, path: p.clone(), weight, italic });
                n += 1;
            }
        }
        log::debug!("cataloged {n} system font faces");
        let installed: std::collections::HashSet<PathBuf> = catalog.values().flat_map(|c| c.faces.iter().map(|cf| cf.path.clone())).collect();
        *self.stamp.lock().unwrap_or_else(|e| e.into_inner()) = Some(ScanStamp { paths, modified: modified_at });
        *self.catalog.write().unwrap_or_else(|e| e.into_inner()) = catalog;
        *self.aliases.write().unwrap_or_else(|e| e.into_inner()) = aliases;
        *self.postscript.write().unwrap_or_else(|e| e.into_inner()) = postscript;
        // Faces loaded from a file this scan did not catalog (the font was removed, or its folder
        // is no longer read) are dropped with their outlines. A family with no face left and no
        // cataloged file leaves the lists, and type set in it resolves to the fallback font. A face
        // without a file (bundled, or added as bytes) stays.
        let mut dropped = std::collections::HashSet::new();
        self.faces.write().unwrap_or_else(|e| e.into_inner()).retain(|f| {
            let keep = f.path.as_ref().is_none_or(|p| installed.contains(p));
            if !keep {
                dropped.insert(f.id);
            }
            keep
        });
        if !dropped.is_empty() {
            log::debug!("dropped {} font faces whose files are no longer installed", dropped.len());
            self.outlines.lock().unwrap_or_else(|e| e.into_inner()).retain(|(id, _), _| !dropped.contains(id));
        }
        // A font installed since may cover the characters no font did: they are looked for again.
        {
            let mut sys = self.sys.lock().unwrap_or_else(|e| e.into_inner());
            sys.scans = sys.scans.wrapping_add(1);
            sys.misses.clear();
            sys.file_searches = 0;
        }
        self.changed();
        n
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;

    /// A database with the bundled fonts only, none installed: what a font source adds is its own.
    fn db() -> FontDb {
        FontDb::new(FontPaths::Fixed(vec![]))
    }

    fn source(family: &str, style: &str) -> FontSource {
        FontSource { family: family.into(), style: style.into(), url: format!("fonts/{family} {style}.ttf"), integrity: "sha256-x".into() }
    }

    /// A font source not loaded yet is listed, but it is no installed family: the Glyphs and
    /// OpenType panels and a composite font of the same name treat it as missing until it arrives.
    #[test]
    fn a_source_not_loaded_is_no_installed_family() {
        let db = db();
        db.add_sources(vec![source("Sourced Only", "Regular")]);
        assert!(db.family_list().iter().any(|f| f == "Sourced Only"), "listed");
        assert!(db.is_source("sourced only"));
        assert!(!db.has_installed_family("Sourced Only"));
        assert!(!db.has_family("Sourced Only"));
    }

    /// Type in a style no source supplies asks for the nearest one: Bold Italic for Black Italic,
    /// not the first given (Regular); a style of its own asks for that.
    #[test]
    fn a_style_asks_for_its_nearest_source() {
        let db = db();
        let (regular, bold_italic) = (source("Near Fam", "Regular"), source("Near Fam", "Bold Italic"));
        db.add_sources(vec![regular.clone(), source("Near Fam", "Bold"), bold_italic.clone()]);
        assert_eq!(db.pending_source("Near Fam", "Black Italic"), Some(bold_italic));
        assert_eq!(db.pending_source("Near Fam", "Light"), Some(regular));
        assert_eq!(db.source_style("Near Fam", "Semibold").as_deref(), Some("Bold"));
    }

    /// Only a style's own source (or one of its slant and weight) is on its way: with only Regular
    /// supplied, Bold type is missing though Regular is pending.
    #[test]
    fn a_style_no_source_supplies_is_not_activating() {
        let db = db();
        db.add_sources(vec![source("Act Fam", "Book")]);
        assert!(db.is_activating("Act Fam", "Regular"), "Book is Regular's weight");
        assert!(db.is_pending_source("Act Fam", "Bold"), "Bold would ask for Book");
        assert!(!db.is_activating("Act Fam", "Bold"));
    }

    /// A check of a source's family (resolved, its face looked up) asks for nothing; type drawn in
    /// it does.
    #[test]
    fn a_check_asks_for_no_source() {
        let db = db();
        let src = source("Checked Fam", "Regular");
        db.add_sources(vec![src.clone()]);
        assert!(db.resolve_without_request("Checked Fam", "Regular").is_some_and(|(_, m)| m == crate::FontMatch::Missing));
        assert!(db.face_without_request("Checked Fam", "Bold").is_some(), "the fallback face");
        assert!(db.stored_family("Checked Fam", "Regular").is_none());
        assert!(db.take_source_requests().is_empty());
        db.face("Checked Fam", "Regular");
        assert_eq!(db.take_source_requests(), [src]);
    }

    /// A source's file that holds another family is not added: it joins no menu. One that holds the
    /// family is, and only a file given as bytes moves the supplied generation.
    #[test]
    fn a_source_file_of_another_family_is_not_added() {
        let db = db();
        let src = source("Right Family1", "Regular");
        db.add_sources(vec![src.clone()]);
        let supplied = db.supplied_generation();
        let (file, loaded) = db.add_source_file(&src, crate::tests::sysfonts::renamed_to("SourceSans3-Regular.ttf", "Wrong Family1"));
        assert!(!loaded && file.added.is_empty() && file.found == 1, "{file:?}");
        assert!(!db.has_family("Wrong Family1"), "the other family is not added");
        assert_eq!(db.supplied_generation(), supplied);
        let (file, loaded) = db.add_source_file(&src, crate::tests::sysfonts::renamed_to("SourceSans3-Regular.ttf", "Right Family1"));
        assert!(loaded && file.added.len() == 1, "{file:?}");
        assert_ne!(db.supplied_generation(), supplied);
        let listed = db.generation();
        db.add_sources(vec![source("Listed Only1", "Regular")]);
        assert_ne!(db.generation(), listed);
        let supplied = db.supplied_generation();
        db.add_sources(vec![source("Listed Two 1", "Regular")]);
        assert_eq!(db.supplied_generation(), supplied, "listing a source moves only the generation");
    }

    /// A test's sources taken away leave nothing behind: not listed, not asked for.
    #[test]
    fn removed_sources_leave_nothing_behind() {
        let db = db();
        let src = source("Gone Fam", "Regular");
        db.add_sources(vec![src.clone()]);
        db.face("Gone Fam", "Regular");
        db.remove_sources("gone fam");
        assert!(!db.is_source("Gone Fam") && db.source_count() == 0);
        assert!(db.take_source_requests().is_empty());
        assert!(!db.family_list().iter().any(|f| f == "Gone Fam"));
    }

    /// The sources have a ceiling in all: a list whose new sources would pass it adds none, and
    /// one of sources the database has (a failed one given again) still passes.
    #[test]
    fn sources_have_a_ceiling_in_all() {
        let db = db();
        let many: Vec<FontSource> = (0..FontDb::MAX_SOURCES - 1).map(|i| source(&format!("Fam {i}"), "Regular")).collect();
        assert_eq!(db.add_sources(many), Some(FontDb::MAX_SOURCES - 1));
        assert_eq!(db.add_sources(vec![source("Last A", "Regular"), source("Last B", "Regular")]), None);
        assert!(!db.is_source("Last A"), "refused whole");
        db.source_settled(&source("Fam 0", "Regular"), None);
        assert_eq!(db.add_sources(vec![source("Fam 0", "Regular"), source("Last A", "Regular")]), Some(2));
        assert_eq!(db.source_count(), FontDb::MAX_SOURCES);
        assert_eq!(db.add_sources(vec![source("Fam 1", "Regular")]), Some(0), "kept as it was: no room needed");
    }
}
