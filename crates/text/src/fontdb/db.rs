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
use super::traits::{Alias, FaceStyle, regular_rank};
use super::{
    Arc, AtomicU64, FaceKey, FaceState, Fit, FontDb, FontPaths, FontSource, FontsAdded, HashMap, HostIndex, Listed, Mutex, Ordering, Origin, Path,
    PathBuf, RwLock, Settled, SourceRequests, StringId, StyleRow,
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
            custom_classes: Mutex::new(HashMap::new()),
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
        let sources = self.sources.read().unwrap_or_else(|e| e.into_inner());
        v.extend(sources.by_family.keys().filter_map(|k| sources.family(k)).map(|(_, f)| f.family.clone()));
        drop(sources);
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

    /// Style names available for `family`: upright styles by weight, then italics, each once by its
    /// letters and digits: its loaded, installed and bundled styles, and those of the host's index
    /// that owns it (as [`FontDb::listed_style`] matches them).
    pub fn styles(&self, family: &str) -> Vec<String> {
        self.style_rows(family).into_iter().map(|r| r.name).collect()
    }

    /// The styles `family` lists, each once by its letters and digits: those of its loaded and
    /// installed faces (each style as its first loaded version describes it, as versions can
    /// disagree on weight), then those of the host's index that owns the family, which every
    /// resolve to the file that lists them (`craft_font_store::Family::face`). A style both have
    /// is the installed face's (nothing to fetch) under the index's spelling, so its name doesn't
    /// change when the index's face loads. Upright styles by weight, then italics.
    pub(crate) fn style_rows(&self, family: &str) -> Vec<StyleRow> {
        #[cfg(test)]
        ROW_BUILDS.with(|n| n.set(n.get() + 1));
        let (family, _) = self.canonical(family, "");
        let family = family.as_str();
        let mut v: Vec<StyleRow> = Vec::new();
        let installed = |v: &mut Vec<StyleRow>, name: &str, weight: f32, italic: bool| {
            // Once by its letters and digits: "Semi Bold" and "Semibold" are one style.
            if !v.iter().any(|r| norm(&r.name) == norm(name)) {
                v.push(StyleRow { name: name.to_string(), weight, italic, origin: Origin::Installed, file: None });
            }
        };
        for f in self.read_faces().iter().filter(|f| f.family.eq_ignore_ascii_case(family)) {
            installed(&mut v, &f.style, f.weight, f.italic);
        }
        if let Some(c) = self.read_catalog().get(&family.to_ascii_lowercase()) {
            for cf in &c.faces {
                installed(&mut v, &cf.style, cf.weight, cf.italic);
            }
        }
        for row in self.index_rows(family) {
            match v.iter_mut().find(|r| norm(&r.name) == norm(&row.name)) {
                Some(r) => {
                    r.name = row.name;
                    r.file = row.file;
                }
                None => v.push(row),
            }
        }
        v.sort_by(|a, b| a.italic.cmp(&b.italic).then(a.weight.total_cmp(&b.weight)).then(a.name.cmp(&b.name)));
        v
    }

    /// The styles of the host's index that owns `family`, each a name its family's `face` finds the
    /// very file by: a file's own style names as its name table gives them (`Weight 450`,
    /// `Lumberboy`: what type set in it stores), a static file's only, by the first of them that
    /// finds it (one face, one row); then its weight names at normal width (each hundred a
    /// variable file's `wght` axis reaches, upright or italic as it is), but not a static file's
    /// once one of its own names finds it (its one weight is named: no `450` beside
    /// `Weight 450`); else its PostScript styles. A variable file's own names aren't read: it is
    /// listed as an index without them lists it. A name that finds another file is left out, so
    /// every listed style resolves to the file that listed it.
    fn index_rows(&self, family: &str) -> Vec<StyleRow> {
        let sources = self.sources.read().unwrap_or_else(|e| e.into_inner());
        let Some((_, f)) = sources.family(family) else { return vec![] };
        let mut rows: Vec<StyleRow> = vec![];
        for file in &f.files {
            let key = FaceKey::new(&f.family, &file.sha256);
            let finds_it = |name: &str| f.face(name).is_ok_and(|found| found.sha256 == file.sha256 && found.path == file.path);
            let (by_name, by_weight, by_postscript) = file_style_candidates(file);
            let mut named: Vec<_> = by_name.into_iter().filter(|(n, ..)| finds_it(n)).take(1).collect();
            if named.is_empty() {
                named.extend(by_weight.into_iter().filter(|(n, ..)| finds_it(n)));
            }
            // Its PostScript styles only when no other name finds it ("W6" isn't also "SemiBold").
            let named = if named.is_empty() { by_postscript.into_iter().filter(|(n, ..)| finds_it(n)).collect() } else { named };
            for (name, weight, italic) in named {
                if !rows.iter().any(|r| norm(&r.name) == norm(&name)) {
                    rows.push(StyleRow { name, weight, italic, origin: Origin::Host(key.clone()), file: Some(key.clone()) });
                }
            }
        }
        rows
    }

    /// The style of `family` that `style` is, as it is stored: the one way every caller matches a
    /// style ([`Fit`]). A composite font of that name comes first and keeps `style` as it is (its
    /// parts name their own styles; no index is asked). Then the styles [`FontDb::styles`] lists,
    /// in that order: the family's loaded, installed and bundled styles, and the owning index's,
    /// each a name the index's `Family::face` finds its very file by (a style both have is the
    /// installed face's, under the index's spelling). [`Fit::Exact`] gives `style` itself: by its
    /// letters and digits, by a name a loaded face answers to, or as the index's `Family::face`
    /// finds a file of one listed style, or the listed style at that weight of a variable file
    /// (`700`; `550` names none). [`Fit::Closest`] gives that, else the nearest style by weight
    /// and slant (`build::weight_distance`), a tie going to the first in that listed order.
    /// `None` when the family lists no style, or none matches exactly.
    ///
    /// A known gap: a CSS weight with no name (`580`) is read here as that weight, as the font
    /// store reads it, but [`FontDb::face`] draws a style it has no name for nearest Regular (400),
    /// so type fitted to `580` may store SemiBold while a face drawn for `580` itself is the
    /// family's nearest Regular. Only an edit fitting a style reads it here. Another: a CSS weight
    /// that no static file of a host family names (`550`, `600 Italic`) is neither fetched nor
    /// offered for Activate; it draws as find() draws it.
    pub fn listed_style(&self, family: &str, style: &str, fit: Fit) -> Option<Listed> {
        if let Some(c) = self.composite_font(family) {
            return Some(Listed {
                family: c.name.clone(),
                style: style.to_string(),
                weight: style_weight(style),
                italic: style_italic(style),
                origin: Origin::Composite,
            });
        }
        let rows = self.style_rows(family);
        if rows.is_empty() {
            return None;
        }
        let name = self.canonical(family, "").0;
        let name = if self.has_installed_family(&name) { name } else { self.source_family(&name).unwrap_or(name) };
        let listed =
            |r: &StyleRow| Listed { family: name.clone(), style: r.name.clone(), weight: r.weight, italic: r.italic, origin: r.origin.clone() };
        let ns = norm(style);
        if let Some(r) = rows.iter().find(|r| norm(&r.name) == ns) {
            return Some(listed(r));
        }
        // A name a loaded face answers to (`Book` for its Regular).
        let alias = self.read_faces().iter().find(|f| f.family.eq_ignore_ascii_case(&name) && f.keys.styles.contains(&ns)).map(|f| norm(&f.style));
        if let Some(r) = alias.and_then(|a| rows.iter().find(|r| norm(&r.name) == a)) {
            return Some(listed(r));
        }
        // As the index finds a style: the row of that file at the weight asked; a file of one row
        // (a static file, `W6` naming its SemiBold) has only that one. A variable file has a row
        // per weight, and none for a weight it reaches without a name (`550`): no row.
        let found = {
            let sources = self.sources.read().unwrap_or_else(|e| e.into_inner());
            sources.family(&name).and_then(|(_, f)| f.face(style).ok().map(|file| FaceKey::new(&f.family, &file.sha256)))
        };
        if let Some(key) = found {
            let of_file: Vec<&StyleRow> = rows.iter().filter(|r| r.file.as_ref() == Some(&key)).collect();
            let wanted = craft_font_store::parse_style(style);
            let at = of_file.iter().find(|r| wanted.is_some_and(|(w, i)| f32::from(w) == r.weight && i == r.italic));
            let at = at.or(if of_file.len() == 1 { of_file.first() } else { None });
            if let Some(r) = at {
                return Some(listed(r));
            }
        }
        if fit == Fit::Exact {
            return None;
        }
        // A CSS weight (`580`) as the font store reads it; else the weight a style name implies.
        let (weight, italic) =
            craft_font_store::parse_style(style).map(|(w, i)| (f32::from(w), i)).unwrap_or_else(|| (style_weight(style), style_italic(style)));
        let distance = |r: &StyleRow| super::build::weight_distance(r.weight, r.italic, weight, italic);
        // Ties go to the first in the listed order (upright before italic, the lighter first), so
        // what an edit stores doesn't depend on which faces loaded first. find() takes the first
        // loaded of equals instead, so on a tie the style stored may differ from the face drawn
        // until the stored one loads: accepted.
        rows.iter().min_by(|a, b| distance(a).total_cmp(&distance(b))).map(listed)
    }

    /// The style a style-less request gets for `family`: its style nearest Regular upright
    /// ([`FontDb::listed_style`], [`Fit::Closest`]), by the catalog's, the loaded faces' and the
    /// host's index's metadata. Unlike [`FontDb::face`] it reads no font files, so the closest
    /// styles of many families cost little.
    pub fn closest_style(&self, family: &str) -> Option<String> {
        if self.composite_font(family).is_some() {
            return None;
        }
        self.listed_style(family, "Regular", Fit::Closest).map(|l| l.style)
    }

    /// The most font files the indexes a host gives may list in all ([`FontDb::set_host_indexes`]).
    pub const MAX_SOURCES: usize = 20_000;

    /// The font indexes a host gives (RFC-0034), in place of every index it gave before: each
    /// `(id, bytes)` an index in craft-font-index format (JSON or gzipped JSON), read as the source
    /// `host:<id>` and checked by the font store's rules (`craft_font_store::Index::from_host_bytes`).
    /// A host adds an index by giving its whole set again with it (section 1.3), and takes one away
    /// by leaving it out. Their families are listed in the font menus at once; a face is asked for
    /// only when type set in it is drawn ([`FontDb::take_source_requests`]). All or nothing: an
    /// index that fails its checks, or a set that would list more than [`FontDb::MAX_SOURCES`]
    /// files, changes nothing. A face is the same face while the new set still lists it, whatever
    /// index, place or path lists it ([`FaceKey`]): one queued, being fetched or loaded is kept, and
    /// one that failed is asked for again when type next draws it (section 5.3); what was asked of
    /// a face the new set doesn't list is forgotten, and its late arrival is [`Settled::Stale`].
    /// Not an edit. Returns how many families the indexes list.
    pub fn set_host_indexes(&self, list: Vec<(String, Vec<u8>)>) -> Result<usize, String> {
        self.put_host_indexes(list, true)
    }

    /// Font indexes added beside the host's others, each replacing only the one of its ID: for
    /// tests that give the process's database an index of their own while others' stay.
    #[cfg(any(test, feature = "test-fonts"))]
    pub fn add_host_indexes(&self, list: Vec<(String, Vec<u8>)>) -> Result<usize, String> {
        self.put_host_indexes(list, false)
    }

    /// [`FontDb::set_host_indexes`] when `all`, else the indexes of `list` put in place of those of
    /// their IDs, the others kept.
    fn put_host_indexes(&self, list: Vec<(String, Vec<u8>)>, all: bool) -> Result<usize, String> {
        let mut read = Vec::with_capacity(list.len());
        for (id, bytes) in &list {
            let index = craft_font_store::Index::from_host_bytes(bytes, id).map_err(|e| format!("font index {id}: {e}"))?;
            read.push(HostIndex { source: format!("host:{id}"), index });
        }
        let families = read.iter().map(|h| h.index.families().len()).sum();
        let mut sources = self.sources.write().unwrap_or_else(|e| e.into_inner());
        let replaced: Vec<String> = read.iter().map(|h| h.source.clone()).collect();
        let kept = |h: &&HostIndex| !all && !replaced.contains(&h.source);
        let kept_files: usize = sources.indexes.iter().filter(kept).map(|h| files_of(&h.index)).sum();
        let new_files: usize = read.iter().map(|h| files_of(&h.index)).sum();
        if kept_files.saturating_add(new_files) > Self::MAX_SOURCES {
            return Err(format!("the font indexes would list more than {} font files", Self::MAX_SOURCES));
        }
        if all {
            sources.indexes = read;
        } else {
            for h in read {
                match sources.indexes.iter_mut().find(|k| k.source == h.source) {
                    Some(k) => *k = h,
                    None => sources.indexes.push(h),
                }
            }
        }
        sources.rebuild();
        self.prune_faces(&sources);
        drop(sources);
        self.changed();
        Ok(families)
    }

    /// The faces asked for, brought in line with the indexes as they are now: a face still listed
    /// keeps its state, save that one that failed is forgotten, so type drawn in it asks for it
    /// again (section 5.3); a face no index lists any longer is forgotten whatever its state (one
    /// loaded stays loaded for the session).
    fn prune_faces(&self, sources: &super::Sources) {
        let mut requests = self.source_requests.lock().unwrap_or_else(|e| e.into_inner());
        requests.faces.retain(|key, (state, _)| sources.entry(key).is_some() && *state != FaceState::Failed);
        let SourceRequests { faces, queue, .. } = &mut *requests;
        queue.retain(|key| faces.get(key).is_some_and(|(state, _)| *state == FaceState::Queued));
    }

    /// The host's index `id` taken away, and what was asked of its faces forgotten: for tests that
    /// give the process's database an index of their own and leave it as they found it.
    #[cfg(any(test, feature = "test-fonts"))]
    pub fn remove_host_index(&self, id: &str) {
        let source = format!("host:{id}");
        let mut sources = self.sources.write().unwrap_or_else(|e| e.into_inner());
        sources.indexes.retain(|h| h.source != source);
        sources.rebuild();
        self.prune_faces(&sources);
        drop(sources);
        self.changed();
    }

    /// How many font files the host's indexes list ([`FontDb::set_host_indexes`]).
    pub fn source_count(&self) -> usize {
        self.sources.read().unwrap_or_else(|e| e.into_inner()).count
    }

    /// Every face type has asked for and the host hasn't taken yet ([`FontDb::take_source_requests_up_to`]).
    pub fn take_source_requests(&self) -> Vec<FontSource> {
        self.take_source_requests_up_to(usize::MAX)
    }

    /// At most `max` of the faces type has asked for, in order, for the host to fetch and hand to
    /// [`FontDb::settle`]; each is asked for once. Each is built from the index as it is now, so its
    /// URLs are current; a face no index lists any longer is dropped from the queue. The queue is
    /// the only one: a host that has room for `max` fetches takes that many.
    pub fn take_source_requests_up_to(&self, max: usize) -> Vec<FontSource> {
        let sources = self.sources.read().unwrap_or_else(|e| e.into_inner());
        let mut requests = self.source_requests.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = vec![];
        while out.len() < max {
            let Some(key) = requests.queue.pop_front() else { break };
            let Some((index, family, file)) = sources.entry(&key) else {
                requests.faces.remove(&key);
                continue;
            };
            let Some((state, (_, style))) = requests.faces.get(&key) else { continue };
            if *state != FaceState::Queued {
                continue;
            }
            let style = style.clone();
            requests.takes += 1;
            let take = requests.takes;
            if let Some((state, _)) = requests.faces.get_mut(&key) {
                *state = FaceState::Fetching(take);
            }
            out.push(FontSource { family: family.family.clone(), style, file: file.clone(), urls: index.file_urls(family, file), take });
        }
        out
    }

    /// Whether type has asked for faces not taken yet ([`FontDb::take_source_requests`]): a host
    /// that takes them once a frame asks for another frame.
    pub fn has_source_requests(&self) -> bool {
        !self.source_requests.lock().unwrap_or_else(|e| e.into_inner()).queue.is_empty()
    }

    /// The host is done with the face `src` (one take of it): `fetched` its file's bytes, or why it couldn't be
    /// fetched. The one way a face is settled. A face no index lists any longer (the host gave a
    /// set without it), or one not being fetched (asked for again meanwhile, or loaded), is
    /// [`Settled::Stale`]: nothing changes, and it is no failure. A file is added
    /// only when it is the file the index lists now, checked by the font store
    /// (`craft_font_store::verify_face`: its size and sha256, that it is a font with outlines, and
    /// that it is the family, the style and the PostScript name the index says, RFC-0034 section
    /// 3); one that fails, or a fetch that failed, is [`Settled::Refused`], and Missing Fonts offers
    /// Activate.
    pub fn settle(&self, src: &FontSource, fetched: Result<Vec<u8>, String>) -> Settled {
        let key = &src.key();
        let checked = {
            let sources = self.sources.read().unwrap_or_else(|e| e.into_inner());
            // Only the face's latest take is settled: a fetch that ends after its face was dropped
            // and asked for again (queued again, fetched again, or loaded meanwhile) is stale, so
            // it neither fails the face nor settles the retry.
            let state = self.source_requests.lock().unwrap_or_else(|e| e.into_inner()).faces.get(key).map(|(s, _)| *s);
            let Some((_, family, file)) = sources.entry(key).filter(|_| state == Some(FaceState::Fetching(src.take))) else { return Settled::Stale };
            fetched.and_then(|bytes| craft_font_store::verify_face(family, file, &bytes).map(|()| bytes).map_err(|e| e.to_string()))
        };
        let set = |state: FaceState| {
            // Only while this take is still the face's: another may have begun meanwhile.
            if let Some((s, _)) = self.source_requests.lock().unwrap_or_else(|e| e.into_inner()).faces.get_mut(key)
                && *s == FaceState::Fetching(src.take)
            {
                *s = state;
            }
        };
        match checked {
            Ok(bytes) => {
                let file = self.add_font_reporting(bytes);
                set(FaceState::Loaded);
                Settled::Added(file)
            }
            Err(why) => {
                set(FaceState::Failed);
                self.changed();
                Settled::Refused(why)
            }
        }
    }

    /// The state of the face `key`, when type has asked for it.
    fn face_state(&self, key: &FaceKey) -> Option<FaceState> {
        self.source_requests.lock().unwrap_or_else(|e| e.into_inner()).faces.get(key).map(|(s, _)| *s)
    }

    /// The face of a host's index that type set in `family` and `style` asks for: the file the
    /// owning index's `Family::face` finds for that style, exactly (no nearer style is fetched in
    /// its place, RFC-0034 section 2.2), unless an installed face has the style
    /// ([`FontDb::listed_style`], [`Fit::Exact`]), and it is neither loaded nor failed. A style the
    /// index finds a file for but lists no name of (`550` on a variable file, a PostScript style
    /// only) is that file's: listed_style names no style for it, but its type still draws it.
    pub(super) fn pending_source(&self, family: &str, style: &str) -> Option<FaceKey> {
        // Every layout asks: first what the index alone says, one lookup, before the styles are
        // listed. A face of the index's is found by the same name its row has (Family::face reads
        // letters and digits as norm does), so a style the index's face doesn't find is none of
        // its, and one loaded or failed asks for nothing.
        let key = self.host_key(family, style)?;
        match self.face_state(&key) {
            Some(FaceState::Loaded | FaceState::Failed) => None,
            // Asked for already: still pending, and nothing more to look up.
            Some(FaceState::Queued | FaceState::Fetching(_)) => Some(key),
            None => (!self.installed_has(family, style)).then_some(key),
        }
    }

    /// The face of the owning host index that type set in `family` and `style` draws: the file
    /// its `Family::face` finds for the style (a listed name, or a weight a variable file reaches
    /// unnamed, `550`). A style the index has no file of (`Black`, or `550` of static files) has
    /// none: no nearer style is fetched in its place (RFC-0034 section 2.2). Found the same way for
    /// asking ([`FontDb::pending_source`]), failing and Activate ([`FontDb::failed_source`]).
    fn host_key(&self, family: &str, style: &str) -> Option<FaceKey> {
        let sources = self.sources.read().unwrap_or_else(|e| e.into_inner());
        let (_, f) = sources.family(family)?;
        f.face(style).ok().map(|file| FaceKey::new(&f.family, &file.sha256))
    }

    /// Whether a loaded, installed or bundled face of `family` has `style` exactly: then no host
    /// face is asked for it.
    fn installed_has(&self, family: &str, style: &str) -> bool {
        matches!(self.listed_style(family, style, Fit::Exact).map(|l| l.origin), Some(Origin::Installed | Origin::Composite))
    }

    /// Whether `family` in `style` is a face a host's index lists and that isn't loaded yet: what
    /// draws it asks for it ([`FontDb::face`]), so only type set in it asks. Each style counts
    /// apart: a family with Regular loaded may still have Bold to come.
    pub fn is_pending_source(&self, family: &str, style: &str) -> bool {
        self.pending_source(family, style).is_some()
    }

    /// Whether `family` in `style` is on its way ([`FontDb::is_pending_source`]): Missing Fonts
    /// shows the font as activating.
    pub fn is_activating(&self, family: &str, style: &str) -> bool {
        self.is_pending_source(family, style)
    }

    /// Whether the style of `family` that type in `style` would be fitted to
    /// ([`FontDb::listed_style`], [`Fit::Closest`]) is a face of a host's index not loaded yet: what
    /// shows a family (a menu's sample, the Glyphs panel, a step of the font wheel) has nothing to
    /// show for it, and fetches nothing.
    pub fn is_unloaded_host_style(&self, family: &str, style: &str) -> bool {
        self.unloaded_host_style(family, style).is_some()
    }

    /// [`FontDb::is_unloaded_host_style`], with that face's style as the family lists it (what
    /// [`FontDb::activate_source`] takes) and where it is: `None` when it is loaded or no host
    /// face; a state of `None` when type hasn't asked for it, else queued, being fetched, or failed
    /// (which only Activate asks for again).
    pub fn unloaded_host_style(&self, family: &str, style: &str) -> Option<(String, Option<FaceState>)> {
        if !self.is_source(family) {
            return None;
        }
        let listed = self.listed_style(family, style, Fit::Closest)?;
        let Origin::Host(key) = listed.origin else { return None };
        match self.face_state(&key) {
            Some(FaceState::Loaded) => None,
            state => Some((listed.style, state)),
        }
    }

    /// The face of `family` in `style` a host's index lists, when it couldn't be loaded and the
    /// style isn't loaded some other way: what Missing Fonts' Activate asks for again
    /// ([`FontDb::activate_source`]).
    pub fn failed_source(&self, family: &str, style: &str) -> Option<FaceKey> {
        // Found as type asked for it ([`FontDb::host_key`]), `550` included.
        let key = self.host_key(family, style)?;
        (self.face_state(&key) == Some(FaceState::Failed) && !self.installed_has(family, style)).then_some(key)
    }

    /// Ask again for the face of `family` in `style` that couldn't be loaded
    /// ([`FontDb::failed_source`]): Missing Fonts' Activate, a retry. Returns whether it was asked
    /// for.
    pub fn activate_source(&self, family: &str, style: &str) -> bool {
        let Some(key) = self.failed_source(family, style) else { return false };
        let mut requests = self.source_requests.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((state, _)) = requests.faces.get_mut(&key) {
            *state = FaceState::Queued;
        }
        requests.queue.push_back(key);
        drop(requests);
        self.changed();
        true
    }

    /// Whether a host's index lists `family` ([`FontDb::set_host_indexes`]).
    pub fn is_source(&self, family: &str) -> bool {
        self.sources.read().unwrap_or_else(|e| e.into_inner()).family(family).is_some()
    }

    /// The family a host's index lists, as the index names it, for `family` in any case.
    pub fn source_family(&self, family: &str) -> Option<String> {
        self.sources.read().unwrap_or_else(|e| e.into_inner()).family(family).map(|(_, f)| f.family.clone())
    }

    /// The family and style of the face a host's font index lists by exactly the PostScript name
    /// `name` (RFC-0034 section 2.2: no looser match). The indexes are looked through in order, and
    /// a hit counts only in the index that owns its family (one family is never merged across
    /// indexes). The style is the row of that file named by the PostScript name's style when it
    /// finds that file, else the row at the file's own weight and slant (a variable file's default
    /// instance); `None` when none is.
    pub fn host_postscript(&self, name: &str) -> Option<Listed> {
        let (family, key, weight, italic) = {
            let sources = self.sources.read().unwrap_or_else(|e| e.into_inner());
            let (i, face) = sources.indexes.iter().enumerate().find_map(|(i, h)| h.index.find_postscript(name).ok().map(|f| (i, f)))?;
            let owner = sources.by_family.get(&face.family.family.to_ascii_lowercase()).map(|&(at, _)| at);
            if owner != Some(i) {
                return None;
            }
            (face.family.family.clone(), FaceKey::new(&face.family.family, &face.file.sha256), f32::from(face.file.weight), face.file.is_italic())
        };
        let rows: Vec<StyleRow> = self.style_rows(&family).into_iter().filter(|r| r.file.as_ref() == Some(&key)).collect();
        let suffix = name.split_once('-').map(|(_, s)| norm(s));
        let row = suffix
            .and_then(|s| rows.iter().find(|r| norm(&r.name) == s))
            .or_else(|| rows.iter().find(|r| r.weight == weight && r.italic == italic))?;
        Some(Listed { family, style: row.name.clone(), weight: row.weight, italic: row.italic, origin: Origin::Host(key) })
    }

    /// Ask for the face of `family` in `style` ([`FontDb::pending_source`]), once.
    pub(super) fn request_source(&self, family: &str, style: &str) {
        let Some(key) = self.pending_source(family, style) else { return };
        let mut requests = self.source_requests.lock().unwrap_or_else(|e| e.into_inner());
        if let std::collections::hash_map::Entry::Vacant(e) = requests.faces.entry(key.clone()) {
            e.insert((FaceState::Queued, (family.to_string(), style.to_string())));
            requests.queue.push_back(key);
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
            // The file's creation time, once for all its faces (a collection holds several).
            let created = std::fs::metadata(&p).and_then(|m| m.created()).ok();
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
                if regular_rank(&traits.likeness) < regular_rank(&kind.likeness) {
                    kind.likeness = traits.likeness;
                }
                if entry.name.is_empty() {
                    entry.name = family;
                }
                if entry.native.is_none() {
                    entry.native = native_family;
                }
                let (weight, italic) = (weight.unwrap_or_else(|| style_weight(&style)), italic || style_italic(&style));
                entry.faces.push(CatalogFace { style, path: p.clone(), weight, italic, created });
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
thread_local! {
    /// How many times this thread listed a family's styles ([`FontDb::style_rows`]).
    static ROW_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many font files `index` lists.
fn files_of(index: &craft_font_store::Index) -> usize {
    index.families().iter().map(|f| f.files.len()).sum()
}

/// The names a file of a host's index may be listed by, with the weight and slant of each: a static
/// file's own style names (craft-font-index 1.3's `names`), each at the file's own weight and
/// slant, whatever word it is (`Regular` of a file at 500, `Italic` of an upright one; a variable
/// file's aren't read), its weight
/// names at normal width (each hundred a variable file's `wght` axis reaches, upright or italic as
/// it is, both when its `ital` axis reaches both; a static file's own weight by its number when it
/// has no name, `650`), and its PostScript styles (the part of each PostScript name after its `-`,
/// at the file's own weight and slant). [`FontDb::index_rows`] keeps a name only when the family's
/// `face` finds this very file by it, a static file's weight names only when none of its own names
/// does, and the PostScript styles only when no other name does.
type Candidates = Vec<(String, f32, bool)>;

fn file_style_candidates(file: &craft_font_store::FontFile) -> (Candidates, Candidates, Candidates) {
    let own = (f32::from(file.weight), file.is_italic());
    let by_name = if file.is_variable() { vec![] } else { file.names.iter().map(|n| (n.clone(), own.0, own.1)).collect() };
    let by_postscript = file.postscript.iter().filter_map(|p| p.split_once('-')).map(|(_, s)| (s.to_string(), own.0, own.1)).collect();
    if !file.has_normal_width() {
        return (by_name, vec![], by_postscript);
    }
    let weights: Vec<u16> =
        if file.is_variable() { (1..=9).map(|w| w * 100).filter(|&w| file.has_weight(w, file.is_italic())).collect() } else { vec![file.weight] };
    let slants: Vec<bool> = [false, true].into_iter().filter(|&i| weights.first().is_some_and(|&w| file.has_weight(w, i))).collect();
    let mut out = vec![];
    for italic in slants {
        for &w in &weights {
            let name = match w {
                100 => "Thin",
                200 => "ExtraLight",
                300 => "Light",
                400 => "Regular",
                500 => "Medium",
                600 => "SemiBold",
                700 => "Bold",
                800 => "ExtraBold",
                900 => "Black",
                _ => "",
            };
            let name = if name.is_empty() { w.to_string() } else { name.to_string() };
            let name = match (italic, name.as_str()) {
                (false, _) => name,
                (true, "Regular") => "Italic".to_string(),
                (true, _) => format!("{name} Italic"),
            };
            out.push((name, f32::from(w), italic));
        }
    }
    (by_name, out, by_postscript)
}

#[cfg(test)]
mod source_tests {
    use super::*;
    use crate::test_fonts::{IndexFile, host_index};
    use crate::tests::sysfonts::renamed_to;

    /// A database with the bundled fonts only, none installed: what a host's index adds is its own.
    fn db() -> FontDb {
        FontDb::new(FontPaths::Fixed(vec![]))
    }

    /// An index of `family` with a Regular and a Bold file (Source Sans 3's, renamed `family`).
    fn two_styles(family: &str) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let regular = renamed_to("SourceSans3-Regular.ttf", family);
        let bold = renamed_to("SourceSans3-Bold.ttf", family);
        let index = host_index(&[(
            family,
            &[
                IndexFile { path: "f/Regular.ttf", bytes: &regular, italic: false, weight: 400 },
                IndexFile { path: "f/Bold.ttf", bytes: &bold, italic: false, weight: 700 },
            ],
        )]);
        (index, regular, bold)
    }

    /// `index` with `extra` (JSON fields) added to the entry of the file at `path`.
    fn with_fields(index: Vec<u8>, path: &str, extra: &str) -> Vec<u8> {
        let s = String::from_utf8(index).unwrap();
        let at = s.find(&format!(r#""path": "{path}""#)).unwrap();
        let end = at + s[at..].find('}').unwrap();
        format!("{}, {extra}{}", &s[..end], &s[end..]).into_bytes()
    }

    /// A family a host's index lists is listed with its styles at once, but it is no installed
    /// family until a face arrives: the Glyphs and OpenType panels and a composite font of the same
    /// name treat it as missing (RFC-0034 section 5.1). Listing it asks for nothing.
    #[test]
    fn a_host_family_is_listed_not_installed() {
        let db = db();
        let (index, ..) = two_styles("Listed Fam 01");
        assert_eq!(db.set_host_indexes(vec![("page".into(), index)]), Ok(1));
        assert!(db.family_list().iter().any(|f| f == "Listed Fam 01"), "listed");
        assert!(db.is_source("listed fam 01"));
        assert_eq!(db.styles("Listed Fam 01"), ["Regular", "Bold"]);
        assert!(!db.has_installed_family("Listed Fam 01"));
        assert!(!db.has_family("Listed Fam 01"));
        assert!(db.take_source_requests().is_empty(), "listing fetches nothing");
    }

    /// Only the style asked for is fetched, exactly (RFC-0034 section 2.2): Bold type asks for the
    /// Bold file at the index's URL; a style the index doesn't list (Black Italic) asks for none.
    #[test]
    fn only_the_exact_style_is_asked_for() {
        let db = db();
        let (index, ..) = two_styles("Exact Fam 001");
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        db.face("Exact Fam 001", "Black Italic");
        assert!(db.take_source_requests().is_empty(), "no nearer style is fetched");
        assert!(!db.is_pending_source("Exact Fam 001", "Black Italic"));
        db.face("Exact Fam 001", "Bold");
        let asked = db.take_source_requests();
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert_eq!(asked[0].file.weight, 700);
        assert_eq!(asked[0].urls, ["https://fonts.test/f/Bold.ttf"]);
        assert!(asked[0].integrity().starts_with("sha256-") && asked[0].integrity().ends_with('='));
        db.face("Exact Fam 001", "700");
        assert!(db.take_source_requests().is_empty(), "the same file, asked for once");
    }

    /// A check of a host's family (resolved, its face looked up) asks for nothing; type drawn in it
    /// does (RFC-0034 section 2.3).
    #[test]
    fn a_check_asks_for_nothing() {
        let db = db();
        let (index, ..) = two_styles("Checked Fam 1");
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        assert!(db.resolve_without_request("Checked Fam 1", "Regular").is_some_and(|(_, m)| m == crate::FontMatch::Missing));
        assert!(db.face_without_request("Checked Fam 1", "Bold").is_some(), "the fallback face");
        assert!(db.stored_family("Checked Fam 1", "Regular").is_none());
        assert!(db.take_source_requests().is_empty());
        db.face("Checked Fam 1", "Regular");
        assert_eq!(db.take_source_requests().len(), 1);
    }

    /// A fetched face must be what the index says (RFC-0034 section 3): a file whose bytes match its
    /// entry but that holds another family, or another weight, is refused and adds nothing; a
    /// refused face is no longer pending, Activate asks for it again, and the right file loads.
    #[test]
    fn a_fetched_face_must_be_what_the_index_says() {
        let db = db();
        let other = renamed_to("SourceSans3-Regular.ttf", "Other Fam 001");
        let bold = renamed_to("SourceSans3-Bold.ttf", "Right Fam 001");
        let index = host_index(&[(
            "Right Fam 001",
            &[
                // Its bytes are another family's.
                IndexFile { path: "r/Regular.ttf", bytes: &other, italic: false, weight: 400 },
                // A Bold file listed as Light.
                IndexFile { path: "r/Light.ttf", bytes: &bold, italic: false, weight: 300 },
            ],
        )]);
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        for (style, bytes) in [("Regular", other.clone()), ("Light", bold.clone())] {
            db.face("Right Fam 001", style);
            let src = db.take_source_requests().remove(0);
            assert!(matches!(db.settle(&src, Ok(bytes)), Settled::Refused(_)), "{style}");
            assert!(!db.has_family("Other Fam 001") && !db.has_family("Right Fam 001"), "{style}: nothing added");
            assert!(!db.is_pending_source("Right Fam 001", style), "{style}: a refused face is not on its way");
            assert!(db.failed_source("Right Fam 001", style).is_some());
        }
        // Bytes that don't match the entry fail too: the right family's Regular, which isn't the
        // file the entry names (review round 3, item 12).
        let right = renamed_to("SourceSans3-Regular.ttf", "Right Fam 001");
        assert!(db.activate_source("Right Fam 001", "Regular"));
        let src = db.take_source_requests().remove(0);
        assert!(matches!(db.settle(&src, Ok(right)), Settled::Refused(_)), "bytes that don't match the entry");
        assert!(!db.has_family("Right Fam 001"));
        db.face("Right Fam 001", "Regular");
        assert!(db.take_source_requests().is_empty(), "not asked for again by itself");
        assert!(db.activate_source("Right Fam 001", "Regular"), "Activate asks again");
        assert_eq!(db.take_source_requests().len(), 1);
        assert!(!db.activate_source("Right Fam 001", "Regular"), "once: it is on its way");

        // The right file, under a set given again, loads.
        let regular = renamed_to("SourceSans3-Regular.ttf", "Right Fam 001");
        let index = host_index(&[("Right Fam 001", &[IndexFile { path: "r/Regular.ttf", bytes: &regular, italic: false, weight: 400 }])]);
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        db.face("Right Fam 001", "Regular");
        let src = db.take_source_requests().remove(0);
        let supplied = db.supplied_generation();
        let Settled::Added(file) = db.settle(&src, Ok(regular)) else { panic!("added") };
        assert_eq!(file.added.len(), 1, "{file:?}");
        assert_ne!(db.supplied_generation(), supplied);
        assert!(!db.is_pending_source("Right Fam 001", "Regular"));
        assert!(db.has_family("Right Fam 001"));
    }

    /// Design test 1: a face being fetched arrives as itself though the host reordered its set
    /// meanwhile (A, B became B, A): a face is its family and file, not its index's place.
    #[test]
    fn a_face_in_flight_survives_a_reordered_set() {
        let db = db();
        let ((a, ..), (b, b_regular, _)) = (two_styles("Order Fam 0A1"), two_styles("Order Fam 0B1"));
        db.set_host_indexes(vec![("page".into(), a.clone()), ("page-2".into(), b.clone())]).unwrap();
        db.face("Order Fam 0B1", "Regular");
        let src = db.take_source_requests().remove(0);
        db.set_host_indexes(vec![("page".into(), b), ("page-2".into(), a)]).unwrap();
        assert!(matches!(db.settle(&src, Ok(b_regular)), Settled::Added(_)));
        assert!(db.has_family("Order Fam 0B1"));
    }

    /// Design test 2: a face no index lists any longer when it arrives is stale: dropped quietly,
    /// neither added nor failed (review round 1, item 8).
    #[test]
    fn a_late_face_of_a_replaced_index_is_dropped() {
        let db = db();
        let (index, regular, _) = two_styles("Late Face 001");
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        db.face("Late Face 001", "Regular");
        let src = db.take_source_requests().remove(0);
        let other = renamed_to("SourceSans3-Regular.ttf", "Late Face 002");
        let replaced = host_index(&[("Late Face 001", &[IndexFile { path: "f/Regular.ttf", bytes: &other, italic: false, weight: 400 }])]);
        db.set_host_indexes(vec![("page".into(), replaced)]).unwrap();
        assert_eq!(db.settle(&src, Ok(regular)), Settled::Stale);
        assert!(!db.has_family("Late Face 001"), "nothing added");
        assert!(db.failed_source("Late Face 001", "Regular").is_none(), "and nothing failed");
        assert_eq!(db.settle(&src, Err("HTTP 404".into())), Settled::Stale, "a failed fetch of it too");
    }

    /// A fetch that ends after its face was dropped, listed again and queued again is stale: its
    /// late failure neither fails the face nor keeps the queued retry from being taken (review
    /// round 3, item 3).
    #[test]
    fn a_late_failure_of_a_face_asked_for_again_is_stale() {
        let db = db();
        let ((a, ..), (b, b_regular, _)) = (two_styles("Retry Fam 0A1"), two_styles("Retry Fam 0B1"));
        db.set_host_indexes(vec![("page".into(), a.clone()), ("page-2".into(), b.clone())]).unwrap();
        db.face("Retry Fam 0B1", "Regular");
        let old = db.take_source_requests().remove(0);
        db.set_host_indexes(vec![("page".into(), a.clone())]).unwrap();
        db.set_host_indexes(vec![("page".into(), a), ("page-2".into(), b)]).unwrap();
        db.face("Retry Fam 0B1", "Regular");
        // The retry is taken, and is being fetched, before the old fetch's failure arrives
        // (review round 4, item A).
        let retry = db.take_source_requests().remove(0);
        assert_eq!(retry.key(), old.key(), "the same face");
        assert_eq!(db.settle(&old, Err("HTTP 503".into())), Settled::Stale);
        assert!(db.failed_source("Retry Fam 0B1", "Regular").is_none(), "not failed");
        assert!(matches!(db.settle(&retry, Ok(b_regular)), Settled::Added(_)), "the retry's bytes are added");
        assert!(db.has_family("Retry Fam 0B1"));
    }

    /// Type drawn in a host family asks, at every layout, whether its face is to be fetched: a style
    /// the index doesn't list, or a face loaded already, is answered without listing the family's
    /// styles (review round 3, item 7).
    #[test]
    fn a_layout_asks_cheaply_whether_a_face_is_to_be_fetched() {
        let db = db();
        let (index, regular, _) = two_styles("Cheap Ask 001");
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        db.face("Cheap Ask 001", "Regular");
        let src = db.take_source_requests().remove(0);
        assert!(matches!(db.settle(&src, Ok(regular)), Settled::Added(_)));
        let builds = || ROW_BUILDS.with(std::cell::Cell::get);
        let before = builds();
        assert!(!db.is_pending_source("Cheap Ask 001", "Black"), "a style the index doesn't list");
        assert!(!db.is_pending_source("Cheap Ask 001", "Regular"), "a face loaded");
        assert_eq!(builds(), before, "no styles listed for a style unlisted or loaded");
        db.face("Cheap Ask 001", "Bold");
        let before_queued = builds();
        assert!(db.is_pending_source("Cheap Ask 001", "Bold"), "a face queued (review round 4, item F)");
        db.take_source_requests();
        assert!(db.is_pending_source("Cheap Ask 001", "Bold"), "a face being fetched");
        assert_eq!(builds(), before_queued, "nor for a face asked for already");
    }

    /// Design test 3: a new set prunes the queue: only faces it still lists are taken, built from it
    /// as it is now (a file moved to another path is fetched from there).
    #[test]
    fn a_new_set_prunes_the_queue() {
        let db = db();
        let ((a, a_regular, _), (b, ..)) = (two_styles("Queue Fam 0A1"), two_styles("Queue Fam 0B1"));
        db.set_host_indexes(vec![("page".into(), a), ("page-2".into(), b)]).unwrap();
        db.face("Queue Fam 0A1", "Regular");
        db.face("Queue Fam 0B1", "Regular");
        let moved = host_index(&[("Queue Fam 0A1", &[IndexFile { path: "moved/Regular.ttf", bytes: &a_regular, italic: false, weight: 400 }])]);
        db.set_host_indexes(vec![("page".into(), moved)]).unwrap();
        let taken = db.take_source_requests();
        assert_eq!(
            taken.iter().map(|s| (s.family.as_str(), s.urls[0].as_str())).collect::<Vec<_>>(),
            [("Queue Fam 0A1", "https://fonts.test/moved/Regular.ttf")]
        );
    }

    /// Design test 4: a face that failed is asked for again when the host's set lists it again and
    /// type draws it (RFC-0034 section 5.3); once a set leaves it out it has no failed source.
    #[test]
    fn a_failed_face_is_asked_again_in_a_set_given_again() {
        let db = db();
        let (index, ..) = two_styles("Again Fam 001");
        db.set_host_indexes(vec![("page".into(), index.clone())]).unwrap();
        db.face("Again Fam 001", "Regular");
        let src = db.take_source_requests().remove(0);
        assert!(matches!(db.settle(&src, Err("HTTP 503".into())), Settled::Refused(_)));
        assert!(db.failed_source("Again Fam 001", "Regular").is_some());
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        assert!(db.failed_source("Again Fam 001", "Regular").is_none(), "no longer failed");
        db.face("Again Fam 001", "Regular");
        assert_eq!(db.take_source_requests().len(), 1, "asked for again");
        let (other, ..) = two_styles("Again Fam 002");
        db.set_host_indexes(vec![("page".into(), other)]).unwrap();
        assert!(db.failed_source("Again Fam 001", "Regular").is_none());
        assert_eq!(db.source_count(), 2, "replaced, not added beside");
    }

    /// Each set the host gives replaces the one before (RFC-0034 section 1.3, the hosting
    /// contract's `fonts` message): an index left out is gone, with what was asked of its families.
    #[test]
    fn a_set_of_indexes_replaces_the_one_before() {
        let db = db();
        let ((one, ..), (two, ..)) = (two_styles("Set One Fam01"), two_styles("Set Two Fam01"));
        assert_eq!(db.set_host_indexes(vec![("page".into(), one.clone()), ("page-2".into(), two)]), Ok(2));
        db.face("Set Two Fam01", "Regular");
        assert_eq!(db.take_source_requests().len(), 1);
        assert_eq!(db.set_host_indexes(vec![("page".into(), one)]), Ok(1));
        assert!(db.is_source("Set One Fam01"));
        assert!(!db.is_source("Set Two Fam01"), "left out of the set: gone");
        assert!(!db.family_list().iter().any(|f| f == "Set Two Fam01"));
        assert_eq!(db.source_count(), 2, "one index's two files");
        db.face("Set Two Fam01", "Regular");
        assert!(db.take_source_requests().is_empty(), "no index lists it now");
    }

    /// A set is taken whole or not at all (RFC-0034 section 1.3): one index that fails its checks
    /// (an http URL template) adds none of the others.
    #[test]
    fn a_list_with_a_bad_index_adds_nothing() {
        let db = db();
        let (good, ..) = two_styles("Whole Fam 001");
        let bad = String::from_utf8(two_styles("Whole Fam 002").0).unwrap().replace("https://", "http://").into_bytes();
        assert!(db.set_host_indexes(vec![("a".into(), good.clone()), ("b".into(), bad)]).is_err());
        assert!(db.set_host_indexes(vec![("bad id/".into(), good)]).is_err(), "an ID the font store refuses");
        assert!(!db.is_source("Whole Fam 001") && db.source_count() == 0);
        assert!(!db.family_list().iter().any(|f| f.starts_with("Whole Fam")));
    }

    /// A test's index taken away leaves nothing behind: not listed, not asked for.
    #[test]
    fn a_removed_index_leaves_nothing_behind() {
        let db = db();
        let (index, ..) = two_styles("Gone Fam 0001");
        db.add_host_indexes(vec![("gone".into(), index)]).unwrap();
        db.face("Gone Fam 0001", "Regular");
        db.remove_host_index("gone");
        assert!(!db.is_source("Gone Fam 0001") && db.source_count() == 0);
        assert!(db.take_source_requests().is_empty());
        assert!(!db.family_list().iter().any(|f| f == "Gone Fam 0001"));
    }

    /// The indexes have a ceiling in files: a set that would pass it changes nothing.
    #[test]
    fn indexes_have_a_ceiling_in_files() {
        let db = db();
        // Spread over families of at most 2000 files: the font store caps a family's list.
        let many = |n: usize| {
            let bytes = renamed_to("SourceSans3-Regular.ttf", "Many Fam 0001");
            let paths: Vec<String> = (0..n).map(|i| format!("m{}/{i}.ttf", i / 2000)).collect();
            let files: Vec<IndexFile<'_>> = paths.iter().map(|p| IndexFile { path: p, bytes: &bytes, italic: false, weight: 400 }).collect();
            let names: Vec<String> = (0..n.div_ceil(2000)).map(|i| format!("Many Fam {i}")).collect();
            let families: Vec<(&str, &[IndexFile<'_>])> = names.iter().zip(files.chunks(2000)).map(|(n, f)| (n.as_str(), f)).collect();
            host_index(&families)
        };
        assert!(db.set_host_indexes(vec![("a".into(), many(FontDb::MAX_SOURCES))]).is_ok());
        assert!(db.add_host_indexes(vec![("b".into(), many(1))]).is_err(), "one more file is refused");
        assert!(db.set_host_indexes(vec![("a".into(), many(FontDb::MAX_SOURCES + 1))]).is_err());
        assert_eq!(db.source_count(), FontDb::MAX_SOURCES, "unchanged");
    }

    /// Design test 5: every style a family lists is listed once by its letters and digits, and
    /// matching it exactly gives back that name and the file that lists it.
    #[test]
    fn every_listed_style_finds_itself_and_its_file() {
        let db = db();
        let bytes = renamed_to("SourceSans3-Regular.ttf", "Every Fam 001");
        let index = host_index(&[(
            "Every Fam 001",
            &[
                IndexFile { path: "e/A-Var.ttf", bytes: b"variable", italic: false, weight: 400 },
                IndexFile { path: "e/Regular.ttf", bytes: b"regular", italic: false, weight: 400 },
                IndexFile { path: "e/Bold.ttf", bytes: b"bold", italic: false, weight: 700 },
                IndexFile { path: "e/Italic.ttf", bytes: b"italic", italic: true, weight: 400 },
                IndexFile { path: "e/W6.ttf", bytes: &bytes, italic: false, weight: 650 },
            ],
        )]);
        let index = with_fields(index, "e/A-Var.ttf", r#""axes": [{"tag": "wght", "min": 300, "max": 500}]"#);
        let index = with_fields(index, "e/W6.ttf", r#""postscript": ["EveryFam001-W6"]"#);
        // The font store's own reading of the index: what each name finds, whatever this code lists.
        let store = craft_font_store::Index::from_host_bytes(&index, "page").unwrap();
        let family = store.family("Every Fam 001").unwrap().clone();
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        let rows = db.style_rows("Every Fam 001");
        let names: Vec<String> = rows.iter().map(|r| norm(&r.name)).collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "{:?}", db.styles("Every Fam 001"));
        for r in &rows {
            let l = db.listed_style("Every Fam 001", &r.name, Fit::Exact).unwrap_or_else(|| panic!("{}", r.name));
            assert_eq!(l.style, r.name);
            assert_eq!(Some(l.origin), r.file.clone().map(Origin::Host), "{}: the file that lists it", r.name);
            let found = family.face(&r.name).unwrap_or_else(|e| panic!("{}: {e}", r.name));
            assert_eq!(r.file.as_ref().map(|k| k.sha256.as_str()), Some(found.sha256.as_str()), "{}: the file its name finds", r.name);
        }
        assert!(names.contains(&"650".to_string()), "a static file's own weight by its number: {names:?}");
    }

    /// Design test 6: a file found only by its PostScript style keeps that name, beside a real
    /// Regular of the same weight, rather than being renamed Regular.
    #[test]
    fn a_postscript_style_beside_a_regular_keeps_its_name() {
        let db = db();
        let index = host_index(&[(
            "Caption Fam 1",
            &[
                IndexFile { path: "a/Regular.ttf", bytes: b"regular", italic: false, weight: 400 },
                IndexFile { path: "b/Caption.ttf", bytes: b"caption", italic: false, weight: 400 },
            ],
        )]);
        let index = with_fields(index, "b/Caption.ttf", r#""postscript": ["CaptionFam1-Caption"]"#);
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        assert_eq!(db.styles("Caption Fam 1"), ["Caption", "Regular"]);
        let l = db.listed_style("Caption Fam 1", "Caption", Fit::Exact).unwrap();
        assert_eq!((l.style.as_str(), l.weight), ("Caption", 400.0));
        assert_ne!(Some(l.origin), db.listed_style("Caption Fam 1", "Regular", Fit::Exact).map(|l| l.origin), "two files");
    }

    /// Design test 7: a variable file's bare PostScript name (no style after a `-`) names its
    /// default instance, not its lightest weight.
    #[test]
    fn a_bare_postscript_name_names_the_default_instance() {
        let db = db();
        let index = host_index(&[("Var Ps Fam 01", &[IndexFile { path: "v/Var.ttf", bytes: b"variable", italic: false, weight: 400 }])]);
        let index = with_fields(index, "v/Var.ttf", r#""axes": [{"tag": "wght", "min": 100, "max": 900}], "postscript": ["VarPsFam01"]"#);
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        assert_eq!(db.host_postscript("VarPsFam01").map(|l| l.style).as_deref(), Some("Regular"));
    }

    /// Design test 8: a PostScript name found in a second index, for a family the first index
    /// owns, names nothing: one family is never merged across indexes.
    #[test]
    fn a_postscript_hit_in_an_index_that_doesn_t_own_the_family_names_nothing() {
        let db = db();
        // Both list the same Bold file; only the second names it by a PostScript name.
        let first = host_index(&[(
            "Owned Fam 01",
            &[
                IndexFile { path: "o/Regular.ttf", bytes: b"regular", italic: false, weight: 400 },
                IndexFile { path: "o/Bold.ttf", bytes: b"bold", italic: false, weight: 700 },
            ],
        )]);
        let second = host_index(&[("Owned Fam 01", &[IndexFile { path: "o/Bold.ttf", bytes: b"bold", italic: false, weight: 700 }])]);
        let second = with_fields(second, "o/Bold.ttf", r#""postscript": ["OwnedFam01-Bold"]"#);
        db.set_host_indexes(vec![("page".into(), first), ("page-2".into(), second)]).unwrap();
        assert_eq!(db.host_postscript("OwnedFam01-Bold"), None);
    }

    /// A weight a variable file reaches without a row of its own (`550`), or a name its family's
    /// `face` finds it by that names no row (`Caption`), is no style exactly: not the file's
    /// lightest row (review round 3, item 2). A static file's one row is still found by a name
    /// that finds the file (`W6`).
    #[test]
    fn an_unnamed_weight_of_a_variable_file_is_no_style_exactly() {
        let db = db();
        let index = host_index(&[
            ("Var Exact 001", &[IndexFile { path: "v/Var.ttf", bytes: b"variable", italic: false, weight: 400 }]),
            ("Static W6 001", &[IndexFile { path: "s/W6.ttf", bytes: b"w6", italic: false, weight: 600 }]),
        ]);
        let index = with_fields(index, "v/Var.ttf", r#""axes": [{"tag": "wght", "min": 300, "max": 700}], "postscript": ["VarExact001-Caption"]"#);
        let index = with_fields(index, "s/W6.ttf", r#""postscript": ["StaticW6001-W6"]"#);
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        assert_eq!(db.listed_style("Var Exact 001", "550", Fit::Exact), None);
        assert_eq!(db.listed_style("Var Exact 001", "Caption", Fit::Exact), None);
        assert_eq!(db.listed_style("Var Exact 001", "600", Fit::Exact).map(|l| l.style).as_deref(), Some("SemiBold"));
        assert_eq!(db.listed_style("Static W6 001", "W6", Fit::Exact).map(|l| l.style).as_deref(), Some("SemiBold"));
    }

    /// The closest style to a CSS weight is by that weight: `580` is nearest SemiBold, not read as
    /// Regular (review round 3, item 6).
    #[test]
    fn the_closest_style_to_a_css_weight_is_by_that_weight() {
        let db = db();
        let index = host_index(&[("Css Weight 01", &[IndexFile { path: "v/Var.ttf", bytes: b"variable", italic: false, weight: 400 }])]);
        let index = with_fields(index, "v/Var.ttf", r#""axes": [{"tag": "wght", "min": 300, "max": 700}]"#);
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        assert_eq!(db.listed_style("Css Weight 01", "580", Fit::Closest).map(|l| l.style).as_deref(), Some("SemiBold"));
    }

    /// Loaded faces whose style names differ only in spacing and case are one style, listed once,
    /// even with another style sorting between them (review round 3, item 8; design test 5).
    #[test]
    fn styles_spelled_alike_are_listed_once() {
        let db = db();
        for (style, ps) in [("Semi Bold", "DedupFam01-SemiBold"), ("Semi Bolder", "DedupFam01-SemiBolder"), ("Semibold", "DedupFam01-Semibold2")] {
            let copy = crate::test_fonts::named_copy(false, ("Dedup Fam 01", style, "Version 1.0", ps)).unwrap();
            assert!(db.add_font(copy) > 0, "{style}");
        }
        let styles = db.styles("Dedup Fam 01");
        assert_eq!(styles.iter().filter(|s| norm(s) == "semibold").count(), 1, "{styles:?}");
    }

    /// The closest style breaks a tie by the fixed order of the listed styles (by weight, the
    /// lighter first), whatever loaded first, so what an edit stores doesn't depend on load order:
    /// Medium asked of a family with Semibold and Regular is its Regular, though find() may draw
    /// its Semibold when that loaded first (review round 4, reverting round 3 item 9).
    #[test]
    fn the_closest_style_breaks_ties_in_the_listed_order() {
        let db = db();
        assert!(db.add_font(renamed_to("SourceSans3-Semibold.ttf", "Tie Order 001")) > 0);
        assert!(db.add_font(renamed_to("SourceSans3-Regular.ttf", "Tie Order 001")) > 0);
        assert_eq!(db.listed_style("Tie Order 001", "Medium", Fit::Closest).map(|l| l.style).as_deref(), Some("Regular"));
    }

    /// Type at a weight a variable file reaches without a named row (`550`) asks for that file, as
    /// the index's `Family::face` finds it, though no style is named so; listing and checking the
    /// family fetch nothing, only type drawn in it does (review round 4, item B).
    #[test]
    fn type_at_an_unnamed_weight_asks_for_the_file_that_has_it() {
        let db = db();
        let index = host_index(&[("Unnamed W 001", &[IndexFile { path: "v/Var.ttf", bytes: b"variable", italic: false, weight: 400 }])]);
        let index = with_fields(index, "v/Var.ttf", r#""axes": [{"tag": "wght", "min": 300, "max": 700}]"#);
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        assert_eq!(db.listed_style("Unnamed W 001", "550", Fit::Exact), None, "no style named so");
        db.styles("Unnamed W 001");
        db.face_without_request("Unnamed W 001", "550");
        assert!(db.take_source_requests().is_empty(), "listing and checking fetch nothing");
        db.face("Unnamed W 001", "550");
        assert_eq!(db.take_source_requests().iter().map(|s| s.file.path.as_str()).collect::<Vec<_>>(), ["v/Var.ttf"]);
    }

    /// A static file's own style names are at the file's own weight and slant, whatever word they
    /// are (review item 1): MedievalSharp is 500 and calls itself `Regular`, with a PostScript name
    /// with no `-`, so a document naming it by PostScript finds its file, at 500, and asks for it;
    /// an upright file that calls itself `Italic` stays upright.
    #[test]
    fn a_static_file_s_own_names_take_its_weight_and_slant() {
        let db = db();
        const SHARP: &str = "Sharp Medi 01";
        const UPRIGHT: &str = "Ital Upri 01";
        let sharp = host_index(&[(SHARP, &[IndexFile { path: "m/Sharp.ttf", bytes: b"a face", italic: false, weight: 500 }])]);
        let sharp = with_fields(sharp, "m/Sharp.ttf", r#""names": ["Regular"], "postscript": ["SharpMedi01"]"#);
        let upright = host_index(&[(UPRIGHT, &[IndexFile { path: "u/Italic.ttf", bytes: b"another face", italic: false, weight: 400 }])]);
        let upright = with_fields(upright, "u/Italic.ttf", r#""names": ["Italic"]"#);
        db.set_host_indexes(vec![("page".into(), sharp), ("page-2".into(), upright)]).unwrap();
        assert_eq!(db.styles(SHARP), ["Regular"]);
        let listed = db.host_postscript("SharpMedi01").expect("the PostScript name finds the file");
        assert_eq!((listed.style.as_str(), listed.weight, listed.italic), ("Regular", 500.0, false));
        assert!(db.find_postscript("SharpMedi01").is_none(), "not loaded yet");
        assert_eq!(db.take_source_requests().iter().map(|s| s.file.path.as_str()).collect::<Vec<_>>(), ["m/Sharp.ttf"], "asked for");
        assert_eq!(db.listed_style(SHARP, "Regular", Fit::Exact).map(|l| l.weight), Some(500.0));
        let italic = db.listed_style(UPRIGHT, "Italic", Fit::Exact).expect("listed");
        assert_eq!((italic.style.as_str(), italic.weight, italic.italic), ("Italic", 400.0, false), "upright, as the file is");
    }

    /// A variable file's own style names aren't read: its index lists it exactly as an index
    /// without them does, by the weight names its `wght` axis reaches (review round 2: instance
    /// names that aren't weight words, `Condensed Bold`, are a known gap).
    #[test]
    fn a_variable_file_lists_as_if_it_had_no_names() {
        const FAMILY: &str = "Inst Var 01";
        let index = |extra: &str| {
            let var = host_index(&[(FAMILY, &[IndexFile { path: "v/Var.ttf", bytes: b"variable", italic: false, weight: 400 }])]);
            with_fields(var, "v/Var.ttf", &format!(r#""axes": [{{"tag": "wght", "min": 300, "max": 700}}], "postscript": ["InstVar01"]{extra}"#))
        };
        let (plain, named) = (db(), db());
        plain.set_host_indexes(vec![("page".into(), index(""))]).unwrap();
        named
            .set_host_indexes(vec![(
                "page".into(),
                index(
                    r#", "names": ["Regular", "Casual", "Condensed Bold"],
                       "instances": [{"name": "Regular", "coordinates": {"wght": 400}}, {"name": "Casual", "coordinates": {"wght": 400}},
                                     {"name": "Condensed Bold", "coordinates": {"wght": 700}}]"#,
                ),
            )])
            .unwrap();
        assert_eq!(named.styles(FAMILY), plain.styles(FAMILY));
        assert!(!named.styles(FAMILY).iter().any(|s| s == "Casual" || s == "Condensed Bold"), "{:?}", named.styles(FAMILY));
        for (style, fit) in [("Regular", Fit::Exact), ("400", Fit::Exact), ("Plain", Fit::Closest), ("Casual", Fit::Closest), ("Bold", Fit::Exact)] {
            let fitted = |db: &FontDb| db.listed_style(FAMILY, style, fit).map(|l| (l.style, l.weight, l.italic));
            assert_eq!(fitted(&named), fitted(&plain), "{style}");
        }
        assert_eq!(named.host_postscript("InstVar01").map(|l| l.style), plain.host_postscript("InstVar01").map(|l| l.style));
        assert!(named.take_source_requests().is_empty() && plain.take_source_requests().is_empty(), "fitting fetches nothing");
    }

    /// A static file with several own style names is listed by the first that finds it, once
    /// (review item 6): one face, one row.
    #[test]
    fn a_static_file_is_listed_by_its_first_own_name() {
        let db = db();
        const FAMILY: &str = "Two Names 01";
        let index = host_index(&[(FAMILY, &[IndexFile { path: "t/Bold.ttf", bytes: b"a face", italic: false, weight: 700 }])]);
        let index = with_fields(index, "t/Bold.ttf", r#""names": ["Weight 700", "Bold"]"#);
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        assert_eq!(db.styles(FAMILY), ["Weight 700"]);
        assert_eq!(db.listed_style(FAMILY, "Bold", Fit::Exact).map(|l| l.style).as_deref(), Some("Weight 700"), "the other name fits to it");
    }

    /// A file whose entry lists its own style names (craft-font-index 1.3) is listed by them alone
    /// and fetched by them: Roboto Slab's `Weight 450`, whose PostScript name is `RobotoSlab-450`,
    /// and Braeside's `Lumberboy`, which type set in them stores as the name table spells it. No
    /// `450` (its weight's number) or PostScript style is listed beside it, before or after the
    /// face loads, and listing and fitting fetch nothing.
    #[test]
    fn a_style_a_file_names_is_listed_and_fetched_by_that_name() {
        for (family, style, weight, postscript) in
            [("Slab Names 01", "Weight 450", 450, "SlabNames01-450"), ("Brae Names 01", "Lumberboy", 400, "BraeNames01-Regular")]
        {
            let db = db();
            let bytes = crate::test_fonts::styled_font(family, style, weight, postscript).unwrap();
            let index = host_index(&[(family, &[IndexFile { path: "s/Face.ttf", bytes: &bytes, italic: false, weight }])]);
            let index = with_fields(index, "s/Face.ttf", &format!(r#""names": ["{style}"], "postscript": ["{postscript}"]"#));
            db.set_host_indexes(vec![("page".into(), index)]).unwrap();
            assert_eq!(db.styles(family), [style], "{family}: listed once, by its own name, before it loads");
            assert_eq!(db.listed_style(family, style, Fit::Exact).map(|l| l.style).as_deref(), Some(style), "{family}: stored as named");
            assert!(db.take_source_requests().is_empty(), "{family}: listing and fitting fetch nothing");
            db.face(family, style);
            let asked = db.take_source_requests();
            assert_eq!(asked.iter().map(|s| s.file.path.as_str()).collect::<Vec<_>>(), ["s/Face.ttf"], "{family}: type in {style} fetches its file");
            assert!(matches!(db.settle(&asked[0], Ok(bytes)), Settled::Added(_)), "{family}: the face checks out");
            assert_eq!(db.styles(family), [style], "{family}: listed once, by its own name, once it loads");
            assert_eq!(db.face(family, style).map(|f| f.style.clone()).as_deref(), Some(style), "{family}: drawn in it");
        }
    }

    /// An index without style names lists and fetches as before: a static file at an unnamed
    /// weight by that weight's number, not its PostScript style (`Book`), and type set in it
    /// fetches it; nothing is fetched by a name its name table may give the style, which the index
    /// doesn't know.
    #[test]
    fn a_file_without_style_names_is_listed_by_its_weight() {
        let db = db();
        const FAMILY: &str = "Plain Names 01";
        let index = host_index(&[(FAMILY, &[IndexFile { path: "p/Face.ttf", bytes: b"a face", italic: false, weight: 450 }])]);
        let index = with_fields(index, "p/Face.ttf", r#""postscript": ["PlainNames01-Book"]"#);
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        assert_eq!(db.styles(FAMILY), ["450"]);
        db.face(FAMILY, "Weight 450");
        assert!(db.take_source_requests().is_empty(), "a name the index doesn't list fetches nothing");
        db.face(FAMILY, "450");
        assert_eq!(db.take_source_requests().iter().map(|s| s.file.path.as_str()).collect::<Vec<_>>(), ["p/Face.ttf"]);
    }

    /// A face fetched for a weight a variable file reaches unnamed (`550`) that fails is offered
    /// for Activate under that weight, and Activate asks for the same file again. In a family of
    /// static Regular and Bold files, `550` names no file: nothing is fetched (a known gap; it
    /// draws as find() draws it), as for a named style it lacks (review round 5, item 2).
    #[test]
    fn a_failed_unnamed_weight_is_offered_for_activate() {
        let db = db();
        let (stat, ..) = two_styles("Unnamed S 001");
        let var = host_index(&[("Unnamed V 001", &[IndexFile { path: "v/Var.ttf", bytes: b"variable", italic: false, weight: 400 }])]);
        let var = with_fields(var, "v/Var.ttf", r#""axes": [{"tag": "wght", "min": 300, "max": 700}]"#);
        db.set_host_indexes(vec![("page".into(), var), ("page-2".into(), stat)]).unwrap();
        db.face("Unnamed V 001", "550");
        let src = db.take_source_requests().into_iter().find(|s| s.family == "Unnamed V 001").expect("asked for");
        assert_eq!(src.file.path, "v/Var.ttf");
        assert!(matches!(db.settle(&src, Err("HTTP 503".into())), Settled::Refused(_)));
        assert!(db.failed_source("Unnamed V 001", "550").is_some(), "offered for Activate");
        assert!(db.activate_source("Unnamed V 001", "550"), "activated");
        assert_eq!(db.take_source_requests().iter().map(|s| s.file.path.as_str()).collect::<Vec<_>>(), ["v/Var.ttf"], "the same file again");
        for style in ["550", "Black"] {
            db.face("Unnamed S 001", style);
            assert!(db.take_source_requests().is_empty(), "{style} of static Regular and Bold fetches nothing");
        }
    }

    /// Design test 9: the closest style weighs slant as build::weight_distance does (#2491): Bold
    /// Italic asked of a family with Bold and Italic gives Bold, as find() draws it.
    #[test]
    fn the_closest_style_weighs_slant_as_find_does() {
        let db = db();
        let index = host_index(&[(
            "Closest Fam 1",
            &[
                IndexFile { path: "c/Bold.ttf", bytes: b"bold", italic: false, weight: 700 },
                IndexFile { path: "c/Italic.ttf", bytes: b"italic", italic: true, weight: 400 },
            ],
        )]);
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        assert_eq!(db.listed_style("Closest Fam 1", "Bold Italic", Fit::Closest).map(|l| l.style).as_deref(), Some("Bold"));
        assert_eq!(db.listed_style("Closest Fam 1", "Bold Italic", Fit::Exact), None);
    }

    /// Design test 10: a style a loaded face has is that face's, so it asks for nothing; and a
    /// style's name is the index's spelling before and after its face loads.
    #[test]
    fn an_installed_style_asks_for_nothing_and_keeps_its_spelling() {
        let db = db();
        let regular = renamed_to("SourceSans3-Regular.ttf", "Spell Fam 001");
        let semibold = renamed_to("SourceSans3-Semibold.ttf", "Spell Fam 001");
        let index = host_index(&[(
            "Spell Fam 001",
            &[
                IndexFile { path: "s/Regular.ttf", bytes: &regular, italic: false, weight: 400 },
                IndexFile { path: "s/SemiBold.ttf", bytes: &semibold, italic: false, weight: 600 },
            ],
        )]);
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        assert!(db.add_font(regular) > 0);
        assert_eq!(db.listed_style("Spell Fam 001", "Regular", Fit::Exact).map(|l| l.origin), Some(Origin::Installed));
        db.face("Spell Fam 001", "Regular");
        assert!(db.take_source_requests().is_empty(), "nothing to fetch");
        assert!(db.styles("Spell Fam 001").contains(&"SemiBold".to_string()), "the index's spelling");
        db.face("Spell Fam 001", "SemiBold");
        let src = db.take_source_requests().remove(0);
        assert!(matches!(db.settle(&src, Ok(semibold)), Settled::Added(_)));
        let styles = db.styles("Spell Fam 001");
        assert_eq!(styles.iter().filter(|s| norm(s) == "semibold").collect::<Vec<_>>(), ["SemiBold"], "the same spelling once loaded: {styles:?}");
        assert_eq!(db.listed_style("Spell Fam 001", "Semibold", Fit::Exact).map(|l| l.origin), Some(Origin::Installed));
    }

    /// A PostScript name no installed font has, which a host's index lists exactly, names that face
    /// and asks for it; a near name doesn't (RFC-0034 section 2.2).
    #[test]
    fn a_postscript_name_an_index_lists_names_and_asks_for_its_face() {
        let db = db();
        let (index, ..) = two_styles("Postscr Fam01");
        let index = with_fields(index, "f/Bold.ttf", r#""postscript": ["PostscrFam01-Bold"]"#);
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        assert_eq!(db.by_postscript_name("PostscrFam01-Bold"), Some(("Postscr Fam01".into(), "Bold".into())));
        assert!(db.find_postscript_without_request("PostscrFam01-Bold").is_none());
        assert!(db.take_source_requests().is_empty(), "a check asks for nothing");
        assert!(db.find_postscript("PostscrFam01-Bold").is_none(), "not loaded yet");
        let asked = db.take_source_requests();
        assert_eq!(asked.iter().map(|r| r.file.weight).collect::<Vec<_>>(), [700], "{asked:?}");
        assert_eq!(db.host_postscript("postscrfam01-bold"), None, "exactly, case and all");
        assert_eq!(db.host_postscript("PostscrFam01"), None);
    }

    /// A file is listed by the names its family's `face` finds it by: its weights and slant.
    #[test]
    fn a_file_is_named_by_its_weights_and_slant() {
        let db = db();
        let index = host_index(&[(
            "Named Fam 001",
            &[
                IndexFile { path: "n/Italic.ttf", bytes: b"italic", italic: true, weight: 400 },
                IndexFile { path: "n/SemiBoldItalic.ttf", bytes: b"sbi", italic: true, weight: 600 },
                IndexFile { path: "n/Var.ttf", bytes: b"variable", italic: false, weight: 400 },
            ],
        )]);
        let index = with_fields(index, "n/Var.ttf", r#""axes": [{"tag": "wght", "min": 300, "max": 500}]"#);
        db.set_host_indexes(vec![("page".into(), index)]).unwrap();
        assert_eq!(db.styles("Named Fam 001"), ["Light", "Regular", "Medium", "Italic", "SemiBold Italic"]);
    }
}
