//! Font database: bundled OFL fonts, user fonts, the installed system fonts (cataloged once, loaded
//! on demand), outline cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use kurbo::BezPath;
use skrifa::instance::{Location, LocationRef, Size};
use skrifa::outline::OutlinePen;
use skrifa::raw::FileRef;
use skrifa::string::StringId;
use skrifa::{GlyphId, MetadataProvider, Tag};

use crate::GlyphImage;
use crate::colr::ColourContent;
use vectorcraft_doc::color::Color;

mod added;
mod alternates;
mod build;
mod cache;
mod composite;
mod db;
mod dirs;
mod face;
mod glyph_pen;
mod gsub;
mod lookup;
mod names;
mod scan;
mod stylistic_sets;
mod traits;
mod wanted;

#[cfg(test)]
pub(crate) use added::recently_added_of;
pub use added::{RECENTLY_ADDED_WITHIN, RecentlyAdded};
pub use alternates::GlyphAlternate;
pub(crate) use alternates::listed;
pub use build::style_weight;
#[cfg(test)]
use cache::GlyphCache;
use cache::{CanonicalNames, Catalog, GlyphColours, GlyphImages, PostScriptNames};
pub use composite::{CompositeCoverage, LentCompositeFonts};
#[cfg(all(not(target_arch = "wasm32"), test))]
pub(crate) use dirs::fonts_outside;
#[cfg(test)]
pub(crate) use dirs::sfnt_of;
pub use dirs::{
    PlatformFontFiles, app_font_dir, set_app_font_dir, set_platform_font_files, set_user_font_dirs, set_user_font_dirs_confined, system_font_dirs,
    user_font_dirs,
};
use face::Instances;
#[cfg(test)]
pub(crate) use face::MAX_LIVE_INSTANCES;
pub(crate) use face::PathAlignHeights;
pub use face::{FALLBACK_FAMILY, FontFace, IcfMargins, OutlineFormat};
pub(crate) use glyph_pen::FlipPen;
pub use gsub::{GlyphCategory, GlyphCategoryKind, GlyphOrigin, JoiningPosition, Substitution, SubstitutionKind};
#[cfg(not(target_arch = "wasm32"))]
use names::SysFallback;
#[cfg(test)]
pub use names::THAI_FAMILIES;
#[cfg(test)]
use names::{native_family_name, norm, pick_native_label};
#[cfg(all(not(target_arch = "wasm32"), test))]
pub(crate) use scan::file_face_names_within;
#[cfg(test)]
use scan::{MAX_FACES, enumerate_faces};
pub use scan::{is_font_file, is_suitcase};
pub use stylistic_sets::StylisticSet;
use traits::Alias;
#[cfg(test)]
use traits::{FaceKeys, face_names};
pub use traits::{FontClass, FontLikeness, FontMatch, FontTraits, GENERIC_FAMILIES, is_generic_family};
#[cfg(test)]
pub(crate) use traits::{alias_lists, face_styles, on_faces_published};
pub use wanted::{WantedFont, WantedFonts};

/// Process-wide font database.
pub struct FontDb {
    faces: RwLock<Vec<Arc<FontFace>>>,
    outlines: Mutex<HashMap<(u32, u32), Arc<BezPath>>>,
    /// [`FontDb::glyph_image`]'s answers per (face, glyph), misses too.
    images: Mutex<GlyphImages>,
    colours: Mutex<GlyphColours>,
    catalog: RwLock<Catalog>,
    /// The installed faces by PostScript name (documents name fonts so: PDF, EPS, .ai).
    postscript: RwLock<PostScriptNames>,
    /// Where the system font scan looks.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    font_paths: FontPaths,
    /// The installed fonts' other names (localized, legacy, full, PostScript) by normalized name.
    aliases: RwLock<HashMap<String, Vec<Alias>>>,
    /// Keeps [`FontDb::canonical`]'s answers for names other than the loaded and installed
    /// families until fonts are added or rescanned.
    canonical_cache: Mutex<CanonicalNames>,
    /// Set once the font folders have been scanned. Lookups by family name wait for the first
    /// scan, so what they find never depends on what ran before them.
    #[cfg(not(target_arch = "wasm32"))]
    cataloged: std::sync::OnceLock<()>,
    /// Families (lowercased) whose installed files were all loaded to find other versions of their
    /// faces (see [`FontDb::versions`]); a rescan forgets them.
    #[cfg(not(target_arch = "wasm32"))]
    version_scanned: RwLock<std::collections::HashSet<String>>,
    /// [`FontDb::family_list`], built on demand and dropped when fonts are added or rescanned.
    family_cache: Mutex<Option<Arc<[String]>>>,
    menu_cache: Mutex<Option<MenuFamilies>>,
    generation: AtomicU64,
    /// This database's [`FontDb::id`].
    id: u64,
    /// The font indexes a host gave ([`FontDb::set_host_indexes`]), and the faces of them type
    /// asked for, each with its state ([`FontDb::take_source_requests`], [`FontDb::settle`]).
    sources: RwLock<Sources>,
    source_requests: Mutex<SourceRequests>,
    /// [`FontDb::supplied_generation`].
    supplied_generation: AtomicU64,
    /// System fallback state: characters no system font covers.
    #[cfg(not(target_arch = "wasm32"))]
    sys: Mutex<SysFallback>,
    /// What the last scan read, to tell when fonts were installed or removed since.
    #[cfg(not(target_arch = "wasm32"))]
    stamp: Mutex<Option<ScanStamp>>,
    /// Faces instanced at the axis settings of type's Variable Font sliders ([`FontDb::instance`]).
    instances: Mutex<Instances>,
    /// Composite fonts (Type › Composite Fonts…) by lowercased name ([`FontDb::set_composite_font`]):
    /// type naming one as its family is set in its parts' fonts. A thread's guards can lend others
    /// in their place ([`FontDb::lend_composite_fonts`]).
    composites: RwLock<HashMap<String, Arc<vectorcraft_doc::CompositeFont>>>,
    /// The lookups of the custom character sets of the composite fonts held or lent, each built
    /// when first used.
    custom_classes: Mutex<composite::CustomClasses>,
}

/// Where a database's system font scan looks: folders (read with their subfolders) and font files.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
enum FontPaths {
    Fixed(Vec<PathBuf>),
    /// The platform's ([`system_font_dirs`]), looked up again by each scan: fonts registered and
    /// font folders added since count too.
    System,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
impl FontPaths {
    fn get(&self) -> Vec<PathBuf> {
        match self {
            FontPaths::Fixed(paths) => paths.clone(),
            FontPaths::System => system_font_dirs(),
        }
    }
}

/// What a system font scan read: the paths it started from, and when each folder it listed (and
/// font file named by itself) last changed. Installing or removing a font changes its folder's.
#[cfg(not(target_arch = "wasm32"))]
struct ScanStamp {
    paths: Vec<PathBuf>,
    modified: Vec<(PathBuf, Option<std::time::SystemTime>)>,
}

#[cfg(not(target_arch = "wasm32"))]
fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);
const OUTLINE_CACHE_MAX: usize = 50_000;
/// The most glyph images [`FontDb::glyph_image`] keeps (each a PNG of some 10–60 KB).
const IMAGE_CACHE_MAX: usize = 4_096;
/// The most PNG bytes it keeps: a font can make each one up to [`crate::bitmap::MAX_PNG_BYTES`].
const IMAGE_CACHE_BYTES: usize = 64 << 20;
/// How many answers [`FontDb::canonical`] keeps at most. When it has kept that many, it clears
/// them before it keeps the next one.
const CANONICAL_CACHE_MAX: usize = 4_096;

/// [`FontDb::menu_family_list`], with the [`FontDb::family_list`] it was built from.
type MenuFamilies = (Arc<[String]>, Arc<[String]>);

/// A face of a host's font index (RFC-0034), as the host fetches it: built from the index as it is
/// when the host takes the request ([`FontDb::take_source_requests`]), so its URLs are current. The
/// host fetches it from one of `urls` (the best first) and hands the bytes to [`FontDb::settle`],
/// which checks them against the index before adding them. Until then the type draws in the
/// fallback face, as Illustrator draws a font Adobe Fonts is still activating.
#[derive(Clone, Debug, PartialEq)]
pub struct FontSource {
    /// The family, as the index names it.
    pub family: String,
    /// The style type asked for it in.
    pub style: String,
    /// The index's entry for the file: its path, size, sha256, weight, slant and PostScript names.
    pub file: craft_font_store::FontFile,
    /// Where the file is, the best first (`craft_font_store::Index::file_urls`).
    pub urls: Vec<String>,
    /// Which take of the face this is: [`FontDb::settle`] settles only the latest one, so an
    /// older fetch of a face asked for again ends quietly.
    pub take: u64,
}

impl FontSource {
    /// The file's Subresource Integrity value, `sha256-<base64>`, for a fetch that the browser
    /// checks too.
    pub fn integrity(&self) -> String {
        let digest: Vec<u8> =
            (0..self.file.sha256.len() / 2).filter_map(|i| u8::from_str_radix(self.file.sha256.get(2 * i..2 * i + 2)?, 16).ok()).collect();
        format!("sha256-{}", base64_std(&digest))
    }

    /// The face's identity: its family and its file.
    pub fn key(&self) -> FaceKey {
        FaceKey::new(&self.family, &self.file.sha256)
    }
}

/// Standard base64 with padding (RFC 4648 section 4), as Subresource Integrity writes a digest.
fn base64_std(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= c.len() { char::from(A[((n >> shift) & 63) as usize]) } else { '=' });
        }
    }
    out
}

/// A face of a host's font index: its family, in lowercase, and its file's sha256. The family is
/// part of it because the face is checked to be that family, and one file can be listed under two.
/// Which index lists it, and where, is no part of it: a face still listed after the host gives a
/// new set is the same face.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FaceKey {
    pub family_lc: String,
    pub sha256: String,
}

impl FaceKey {
    pub fn new(family: &str, sha256: &str) -> Self {
        FaceKey { family_lc: family.to_ascii_lowercase(), sha256: sha256.to_string() }
    }
}

/// One font index a host gave, under its source ID (`host:<id>`): a label, which only the tests'
/// index-by-index form matches.
#[derive(Debug)]
struct HostIndex {
    source: String,
    index: craft_font_store::Index,
}

/// The font indexes a host gave ([`FontDb::set_host_indexes`]), in the order given, and where
/// each family is: the first index that lists a family owns it, for every lookup.
#[derive(Debug, Default)]
struct Sources {
    indexes: Vec<HostIndex>,
    /// Each family, keyed in lowercase: its index and its place in that index's families.
    by_family: HashMap<String, (usize, usize)>,
    /// How many font files the indexes list in all: at most [`FontDb::MAX_SOURCES`].
    count: usize,
}

impl Sources {
    /// The family `family` (any case) and the index owning it.
    fn family(&self, family: &str) -> Option<(&craft_font_store::Index, &craft_font_store::Family)> {
        let &(i, f) = self.by_family.get(&family.to_ascii_lowercase())?;
        let index = &self.indexes.get(i)?.index;
        Some((index, index.families().get(f)?))
    }

    /// The owning family's entry for the face `key`, whatever index, place or path lists it.
    fn entry(&self, key: &FaceKey) -> Option<(&craft_font_store::Index, &craft_font_store::Family, &craft_font_store::FontFile)> {
        let (index, family) = self.family(&key.family_lc)?;
        let file = family.files.iter().find(|f| f.sha256 == key.sha256)?;
        Some((index, family, file))
    }

    /// Where each family is, after the indexes changed.
    fn rebuild(&mut self) {
        self.by_family.clear();
        self.count = 0;
        for (i, h) in self.indexes.iter().enumerate() {
            for (f, family) in h.index.families().iter().enumerate() {
                self.count += family.files.len();
                self.by_family.entry(family.family.to_ascii_lowercase()).or_insert((i, f));
            }
        }
    }
}

/// Where a face of a host's index is, once type has asked for it ([`FontDb::unloaded_host_style`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceState {
    /// Type asked for it; the host hasn't taken it yet.
    Queued,
    /// The host took it and is fetching it, in the take numbered so ([`FontSource::take`]): only
    /// that take's result settles it.
    Fetching(u64),
    /// Its file arrived, passed its checks and was added.
    Loaded,
    /// It couldn't be fetched, or failed its checks: Activate asks for it again.
    Failed,
}

/// The faces of the host's indexes type asked for: each face's state and the family and style type
/// asked for it in, and the faces queued for the host, in order. The queue is the only one: the
/// host takes from it as it has room ([`FontDb::take_source_requests`]).
#[derive(Debug, Default)]
struct SourceRequests {
    faces: HashMap<FaceKey, (FaceState, (String, String))>,
    /// The last take's number ([`FontSource::take`]).
    takes: u64,
    queue: std::collections::VecDeque<FaceKey>,
}

/// How [`FontDb::settle`] ended a face the host fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settled {
    /// Its file passed its checks and was added: what it added.
    Added(FontsAdded),
    /// It couldn't be fetched, or its file failed its checks: why. Missing Fonts offers Activate.
    Refused(String),
    /// No index the host gives lists it any longer: dropped quietly, no failure.
    Stale,
}

/// How [`FontDb::listed_style`] matches a style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// That style only, by its name (any spelling of the same letters and digits) or, for a host's
    /// index, as `craft_font_store::Family::face` finds it (`700`, `Semi Bold`).
    Exact,
    /// That style, else the family's nearest (`build::weight_distance`: weight, and a slant
    /// penalty less than Regular to Bold). Only an edit fitting type to a family uses it.
    Closest,
}

/// Where a listed style comes from ([`FontDb::listed_style`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A composite font of that name: the style is the type's own, and no index is asked.
    Composite,
    /// A loaded, installed or bundled face: nothing to fetch.
    Installed,
    /// A face of a host's index, fetched when type set in it is drawn.
    Host(FaceKey),
}

/// A family's style as [`FontDb::listed_style`] gives it: the family and style to store, their
/// weight and slant, and where the face comes from.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub family: String,
    pub style: String,
    pub weight: f32,
    pub italic: bool,
    pub origin: Origin,
}

/// One style a family lists ([`FontDb::styles`]): its name, weight, slant and origin, and the face
/// of the host's index that has it, when one does (also for a style an installed face has).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StyleRow {
    pub(crate) name: String,
    pub(crate) weight: f32,
    pub(crate) italic: bool,
    pub(crate) origin: Origin,
    pub(crate) file: Option<FaceKey>,
}

/// What [`FontDb::add_font_reporting`] did with a font file's bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontsAdded {
    /// The faces the file holds: 0 when it is no TrueType, OpenType or collection font.
    pub found: usize,
    /// The faces the file holds, as (family, style), whether or not they were loaded already.
    pub held: Vec<(String, String)>,
    /// The faces it added, as (family, style); a face already loaded (the same family, style and
    /// version) is not added again.
    pub added: Vec<(String, String)>,
}

#[cfg(test)]
mod alias_tests {
    use super::*;

    /// A one-table font holding a name table from (platform, language, name id, string) records:
    /// Windows Unicode (UTF-16) or Mac Roman (ASCII here).
    fn name_font(records: &[(u16, u16, u16, &str)]) -> Vec<u8> {
        let (mut head, mut strings) = (vec![], vec![]);
        let n = records.len() as u16;
        for v in [0, n, 6 + 12 * n] {
            head.extend_from_slice(&v.to_be_bytes());
        }
        for &(platform, lang, id, s) in records {
            let bytes: Vec<u8> = if platform == 3 { s.encode_utf16().flat_map(u16::to_be_bytes).collect() } else { s.as_bytes().to_vec() };
            let encoding = if platform == 3 { 1 } else { 0 };
            for v in [platform, encoding, lang, id, bytes.len() as u16, strings.len() as u16] {
                head.extend_from_slice(&v.to_be_bytes());
            }
            strings.extend(bytes);
        }
        head.extend(strings);
        let mut font = vec![0, 1, 0, 0, 0, 1, 0, 16, 0, 0, 0, 0];
        font.extend_from_slice(b"name");
        for v in [0u32, 28, head.len() as u32] {
            font.extend_from_slice(&v.to_be_bytes());
        }
        font.extend(head);
        font
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_file_search_for_uncovered_characters_stops_at_its_budget() {
        let db = FontDb::with_font_dirs(vec![]);
        // The first search scans the font folders, which starts the count again.
        db.system_fallback('\u{E001}');
        db.sys.lock().unwrap().file_searches = names::MAX_FILE_SEARCHES;
        assert!(!db.system_fallback('\u{E000}'));
        let sys = db.sys.lock().unwrap();
        assert_eq!(sys.file_searches, names::MAX_FILE_SEARCHES);
        assert!(!sys.misses.contains(&'\u{E000}'), "an unsearched character is not a remembered miss");
    }

    #[test]
    fn a_collection_lists_at_most_max_faces() {
        let mut font = name_font(&[(3, 0x0409, 1, "Fam"), (3, 0x0409, 2, "Regular")]);
        let n = 1000u32;
        let base = 12 + 4 * n;
        let table = u32::from_be_bytes(font[20..24].try_into().unwrap()) + base;
        font[20..24].copy_from_slice(&table.to_be_bytes());
        let mut ttc = b"ttcf".to_vec();
        for v in [0x0001_0000, n] {
            ttc.extend_from_slice(&u32::to_be_bytes(v));
        }
        for _ in 0..n {
            ttc.extend_from_slice(&base.to_be_bytes());
        }
        ttc.extend(font);
        let faces = enumerate_faces(&ttc);
        assert!(!faces.is_empty());
        assert!(faces.len() <= MAX_FACES as usize, "{} faces", faces.len());
    }

    fn keys(records: &[(u16, u16, u16, &str)]) -> (String, String, FaceKeys) {
        let font = name_font(records);
        let f = skrifa::FontRef::new(&font).unwrap();
        let (family, style) = face_names(&f).unwrap();
        let k = FaceKeys::of(&f, &family, &style);
        (family, style, k)
    }

    const EN: u16 = 0x0409;
    const JA: u16 = 0x0411;

    /// Laid out like the name table of Hiragino Sans W3.
    const HIRAGINO_W3: &[(u16, u16, u16, &str)] = &[
        (1, 0, 1, "Hiragino Sans"),
        (1, 0, 2, "W3"),
        (3, EN, 1, "Hiragino Sans W3"),
        (3, EN, 2, "Regular"),
        (3, JA, 1, "ヒラギノ角ゴシック W3"),
        (3, JA, 2, "Regular"),
        (3, EN, 6, "HiraginoSans-W3"),
        (3, EN, 16, "Hiragino Sans"),
        (3, JA, 16, "ヒラギノ角ゴシック"),
        (3, EN, 17, "W3"),
        (3, JA, 17, "W3"),
    ];

    #[test]
    fn a_face_answers_to_its_names_in_every_language() {
        let (family, style, k) = keys(HIRAGINO_W3);
        assert_eq!((family.as_str(), style.as_str()), ("Hiragino Sans", "W3"));
        let aliases = Alias::of(&family, &style, &k);
        let find = |name: &str| aliases.iter().filter(|(n, _)| *n == norm(name)).map(|(_, a)| a.clone()).collect::<Vec<_>>();
        assert!(find("ヒラギノ 角ゴシック").iter().any(|a| a.family == "Hiragino Sans" && a.style.is_none()), "a family name in Japanese");
        let legacy = find("Hiragino Sans W3");
        assert!(legacy.iter().any(|a| a.style.as_deref() == Some("W3") && a.paired.as_deref() == Some("regular")), "{legacy:?}");
        assert!(!legacy.iter().any(|a| a.paired.as_deref() == Some("w3")), "Mac family + Windows style is no pair");
        assert!(find("HiraginoSans-W3").iter().any(|a| a.style.as_deref() == Some("W3") && a.paired.is_none()), "the PostScript name");
    }

    /// #394 Preferences › Type › Show Font Names in English: pick the non-English name-table label.
    #[test]
    fn pick_native_label_prefers_a_non_english_name_record() {
        assert_eq!(
            pick_native_label("Hiragino Sans", &[(Some("en".into()), "Hiragino Sans".into()), (Some("ja".into()), "ヒラギノ角ゴシック".into())])
                .as_deref(),
            Some("ヒラギノ角ゴシック")
        );
        assert!(pick_native_label("Source Sans 3", &[(Some("en".into()), "Source Sans 3".into())]).is_none());
        let font = name_font(HIRAGINO_W3);
        let f = skrifa::FontRef::new(&font).unwrap();
        let (family, _) = face_names(&f).unwrap();
        assert_eq!(family, "Hiragino Sans");
        assert_eq!(native_family_name(&f, &family).as_deref(), Some("ヒラギノ角ゴシック"));
    }

    #[test]
    fn without_typographic_names_the_legacy_ones_are_the_familys() {
        let (family, style, k) = keys(&[(3, EN, 1, "Example Serif"), (3, EN, 2, "Bold Italic"), (3, JA, 1, "例セリフ"), (3, JA, 2, "太字斜体")]);
        assert_eq!((family.as_str(), style.as_str()), ("Example Serif", "Bold Italic"));
        assert!(k.families.contains(&norm("例セリフ")) && k.styles.contains(&norm("太字斜体")) && k.legacy.is_empty());
    }

    #[test]
    fn resolve_says_how_a_font_matched() {
        let db = FontDb::with_font_dirs(vec![]);
        let m = |f: &str, s: &str| db.resolve(f, s).map(|(face, m)| (face.family.clone(), face.style.clone(), m)).unwrap();
        assert_eq!(m("Source Sans 3", "Semibold"), ("Source Sans 3".into(), "Semibold".into(), FontMatch::Exact));
        assert_eq!(m("Source Sans 3", "Black").2, FontMatch::Style);
        assert_eq!(m("No Such Family", "Regular"), (FALLBACK_FAMILY.into(), "Regular".into(), FontMatch::Missing));
        // A PostScript name names its face, whatever style is asked for (text opened from a PDF).
        assert_eq!(m("SourceSans3-Semibold", "Regular"), ("Source Sans 3".into(), "Semibold".into(), FontMatch::Exact));
        let semi = db.find_postscript("sourcesans3-semibold").unwrap();
        assert_eq!(semi.style, "Semibold");
        assert!(db.find_postscript("NoSuchFont-Regular").is_none());
        assert!(db.has_family("SourceSans3-Regular"));
    }

    /// A PDF names a font by PostScript name; when it isn't installed the importer splits the name
    /// (`Honoka-Mincho` → Honoka, Mincho). Once a font of that PostScript name is installed, under
    /// a family name of its own (ほのか明朝), the split pair finds it.
    #[test]
    fn a_family_and_style_split_from_a_postscript_name_find_its_face() {
        let db = FontDb::with_font_dirs(vec![]);
        db.ensure_catalog();
        // An installed face whose family name shares nothing with its PostScript name.
        let alias = Alias { family: "Source Sans 3".into(), style: Some("Semibold".into()), paired: None };
        db.aliases.write().unwrap().insert(norm("Honoka-Mincho"), vec![alias]);
        db.changed();
        let m = db.resolve("Honoka", "Mincho").map(|(face, m)| (face.family.clone(), face.style.clone(), m)).unwrap();
        assert_eq!(m, ("Source Sans 3".into(), "Semibold".into(), FontMatch::Exact));
        // Unknown names still fall back.
        assert_eq!(db.resolve("Honoka", "Gothic").unwrap().1, FontMatch::Missing);
    }

    /// The database keeps at most [`CANONICAL_CACHE_MAX`] answers, and names asked for past that
    /// many still get the same answers.
    #[test]
    fn the_answers_kept_for_names_are_capped() {
        let db = FontDb::with_font_dirs(vec![]);
        for i in 0..=CANONICAL_CACHE_MAX {
            let name = format!("No Such Family {i}");
            assert_eq!(db.canonical(&name, "Regular"), (name.clone(), "Regular".into()));
        }
        assert!(db.canonical_cache.lock().unwrap().len() <= CANONICAL_CACHE_MAX);
        assert_eq!(db.canonical("SourceSans3-Semibold", "Regular"), ("Source Sans 3".into(), "Semibold".into()));
    }
}

#[cfg(test)]
mod race_tests {
    use super::*;

    #[test]
    fn a_family_loaded_by_another_thread_meanwhile_is_still_found() {
        // Two threads ask for the same installed family at once: each must get it, not the
        // fallback (the second one's load finds every face already added).
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
        let db: &'static FontDb = Box::leak(Box::new(FontDb::with_font_dirs(vec![dir])));
        // Only the catalog knows Inter (the bundled faces are dropped).
        db.faces.write().unwrap().retain(|f| f.family != "Inter");
        let threads: Vec<_> = (0..8).map(|_| std::thread::spawn(move || db.face("Inter", "Regular").map(|f| f.family.clone()))).collect();
        for t in threads {
            assert_eq!(t.join().unwrap().as_deref(), Some("Inter"));
        }
    }
}

#[cfg(test)]
mod image_cache_tests {
    use super::*;

    #[test]
    fn the_glyph_image_cache_keeps_its_bytes_within_its_budget() {
        // Images as large as a font may make them: no test font has enough of those to fill the
        // budget, so they're made here.
        let image = || {
            let blob = vectorcraft_doc::ImageBlob::png(vec![0; crate::bitmap::MAX_PNG_BYTES]);
            Some(Arc::new(GlyphImage { blob, width: 1, height: 1, xf: kurbo::Affine::IDENTITY }))
        };
        let mut cache = GlyphCache::default();
        for gid in 0..20 {
            cache.keep((1, gid), image());
            assert!(cache.bytes <= IMAGE_CACHE_BYTES, "{} bytes after {} images", cache.bytes, gid + 1);
        }
        // The same glyph again, as when two threads look it up at once, counts once.
        cache.keep((1, 19), image());
        assert_eq!(cache.bytes, cache.kept.values().flatten().map(|i: &Arc<GlyphImage>| i.blob.bytes.len()).sum::<usize>());
        // A glyph without an image costs nothing, and is still kept.
        cache.keep((1, 99), None);
        assert!(cache.kept.contains_key(&(1, 99)));
    }
}
