//! `.vectorcraft` format: round trips, robustness against garbage, versioning, unknown fields.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use serde_json::{Value, json};
use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeId, NodeKind};
use vectorcraft_format::{SaveOptions, base64_decode, base64_encode, load, load_file, pdf_content, preview, save, save_with, sniff};
use vectorcraft_geom::{Affine, Anchor, AnchorKind, PathData, Point, SubPath};
use vectorcraft_testkit::fixtures;
use vectorcraft_testkit::invariants::{check_document, check_native_roundtrip, check_native_roundtrip_exact, doc_json, json_approx_eq};
use vectorcraft_testkit::strategies::arb_ops;

fn rich_doc() -> Document {
    (*fixtures::rich_session().doc().unwrap().doc).clone()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn base64_roundtrip(data in prop::collection::vec(any::<u8>(), 0..300)) {
        let s = base64_encode(&data);
        prop_assert_eq!(s.len() % 4, 0);
        prop_assert_eq!(base64_decode(&s), Some(data.clone()));
        // Whitespace is ignored.
        let spaced: String = s.chars().flat_map(|c| [c, '\n']).collect();
        prop_assert_eq!(base64_decode(&spaced), Some(data));
    }

    #[test]
    fn base64_decode_never_panics(s in ".{0,64}") {
        let _ = base64_decode(&s);
    }

    /// Arbitrary bytes are rejected (or, if they happen to parse, yield a valid document) — never a panic.
    #[test]
    fn load_garbage_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        if let Ok(d) = load(&bytes) {
            check_document(&d).map_err(TestCaseError::fail)?;
        }
    }

    /// Mutating a valid file (byte flips / truncation) never panics the loader.
    #[test]
    fn load_mutated_file_never_panics(cut in 0usize..4000, flips in prop::collection::vec((0usize..4000, any::<u8>()), 0..8)) {
        let mut bytes = save(&Document::new(100.0, 100.0), false);
        for (i, b) in flips {
            let n = bytes.len();
            bytes[i % n] = b;
        }
        bytes.truncate(cut.min(bytes.len()));
        let _ = load(&bytes);
    }

    /// Documents produced by random editing survive save → load (numbers to 1e-12).
    #[test]
    fn random_documents_roundtrip(ops in arb_ops(5..40)) {
        let mut s = fixtures::session();
        for op in &ops {
            let _ = op.apply(&mut s);
        }
        let d = s.doc().unwrap().doc.clone();
        check_native_roundtrip(&d).map_err(TestCaseError::fail)?;
        // Pretty and compact encodings load to the same document.
        let a = doc_json(&load(&save(&d, true)).unwrap());
        let b = doc_json(&load(&save(&d, false)).unwrap());
        prop_assert_eq!(a, b);
    }

    /// Embedded image bytes survive the base64 round trip exactly (saves keep the images in use).
    #[test]
    fn images_roundtrip(data in prop::collection::vec(any::<u8>(), 0..2000), key in "[a-z \"\\\\]{1,8}") {
        let mut d = Document::new(10.0, 10.0);
        d.images.insert(key.clone(), ImageBlob::new("image/png", data.clone()));
        let layer = d.layers[0].id;
        let id = d.alloc_id();
        let im = ImageObject { key: key.clone(), width: 1, height: 1, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
        d.insert(Some(layer), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
        for pretty in [false, true] {
            let back = load(&save(&d, pretty)).unwrap();
            prop_assert_eq!(back.images.get(&key).map(|b| b.bytes.as_ref().clone()), Some(data.clone()));
        }
    }

    /// Area type's rows and columns from a file: a count up to `u64::MAX` reads clamped to 1..=100,
    /// and what loads saves and loads again unchanged.
    #[test]
    fn area_type_rows_and_columns_read_as_1_to_100(rows in prop_oneof![0u64..=200, any::<u64>()], columns in prop_oneof![0u64..=200, any::<u64>()]) {
        use vectorcraft_doc::{AreaOptions, TextKind, TextObject};
        use vectorcraft_geom::{Point, Rect, shapes};
        let count = |n: u64| usize::try_from(n).unwrap_or(usize::MAX);
        let mut t = TextObject::point(Point::ZERO, "Rows and columns", Default::default());
        t.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 300.0, 120.0)) };
        t.area.rows = count(rows);
        t.area.columns = count(columns);
        let mut d = Document::new(400.0, 300.0);
        let layer = d.layers[0].id;
        let id = d.alloc_id();
        d.insert(Some(layer), 0, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
        let read = |d: &Document| match d.node(id).map(|n| &n.kind) {
            Some(NodeKind::Text(t)) => Some((t.area.rows, t.area.columns)),
            _ => None,
        };
        let back = load(&save(&d, false)).unwrap();
        let max = AreaOptions::MAX_ROWS_COLUMNS;
        prop_assert_eq!(read(&back), Some((count(rows).clamp(1, max), count(columns).clamp(1, max))));
        prop_assert_eq!(read(&load(&save(&back, false)).unwrap()), read(&back));
    }

    /// Compressed and older-version saves of random documents load to the same document.
    #[test]
    fn compressed_and_older_saves_roundtrip(ops in arb_ops(5..30)) {
        let mut s = fixtures::session();
        for op in &ops {
            let _ = op.apply(&mut s);
        }
        let d = s.doc().unwrap().doc.clone();
        let want = doc_json(&load(&save(&d, false)).unwrap());
        for o in [SaveOptions { compress: true, ..SaveOptions::default() }, SaveOptions { version: 2, ..SaveOptions::default() }, SaveOptions { version: 1, pretty: true, ..SaveOptions::default() }] {
            let back = load(&save_with(&d, &o).unwrap()).unwrap();
            prop_assert!(json_approx_eq(&doc_json(&back), &want, 1e-12), "{:?} differs", o);
        }
    }

    /// Mutating a compressed file (byte flips / truncation) never panics the loader or the sniffer.
    #[test]
    fn load_mutated_compressed_file_never_panics(cut in 0usize..2000, flips in prop::collection::vec((0usize..2000, any::<u8>()), 0..8)) {
        let mut bytes = save_with(&Document::new(100.0, 100.0), &SaveOptions { compress: true, ..SaveOptions::default() }).unwrap();
        for (i, b) in flips {
            let n = bytes.len();
            bytes[i % n] = b;
        }
        bytes.truncate(cut.min(bytes.len()));
        let _ = sniff(&bytes);
        let _ = preview(&bytes);
        if let Ok(d) = load(&bytes) {
            check_document(&d).map_err(TestCaseError::fail)?;
        }
    }

    /// Mutating a file carrying ICC profiles and a PDF (save options) never panics the readers.
    #[test]
    fn load_mutated_file_with_profiles_never_panics(cut in 0usize..3000, flips in prop::collection::vec((0usize..3000, any::<u8>()), 0..8)) {
        let o = SaveOptions {
            profiles: [("Studio RGB".to_string(), vec![1, 2, 3]), ("Press".to_string(), vec![])].into(),
            pdf: Some(b"%PDF-1.7".to_vec()),
            ..SaveOptions::default()
        };
        let mut bytes = save_with(&Document::new(100.0, 100.0), &o).unwrap();
        for (i, b) in flips {
            let n = bytes.len();
            bytes[i % n] = b;
        }
        bytes.truncate(cut.min(bytes.len()));
        let _ = pdf_content(&bytes);
        if let Ok(f) = load_file(&bytes) {
            check_document(&f.doc).map_err(TestCaseError::fail)?;
        }
    }
}

/// A coordinate: small integers, zeros of both signs, decimals and any finite f64.
fn arb_coord() -> impl Strategy<Value = f64> {
    prop_oneof![(-50i32..50).prop_map(f64::from), Just(0.0), Just(-0.0), -1e4f64..1e4, any::<f64>().prop_filter("finite", |x| x.is_finite()),]
}

fn arb_point() -> impl Strategy<Value = Point> {
    (arb_coord(), arb_coord()).prop_map(|(x, y)| Point::new(x, y))
}

/// An anchor whose handles are each at the point (no handle) or anywhere, of either kind.
fn arb_anchor() -> impl Strategy<Value = Anchor> {
    (arb_point(), prop::option::of(arb_point()), prop::option::of(arb_point()), any::<bool>()).prop_map(|(p, h_in, h_out, smooth)| Anchor {
        p,
        h_in: h_in.unwrap_or(p),
        h_out: h_out.unwrap_or(p),
        kind: if smooth { AnchorKind::Smooth } else { AnchorKind::Corner },
    })
}

/// Subpaths open or closed, empty or not, some ending with an anchor on their first point.
fn arb_path() -> impl Strategy<Value = PathData> {
    let sub = (prop::collection::vec(arb_anchor(), 0..6), any::<bool>(), any::<bool>(), any::<bool>()).prop_map(
        |(mut anchors, closed, back, bare_ends)| {
            if back && let Some(first) = anchors.first().copied() {
                anchors.push(Anchor { p: first.p, ..first });
            }
            if bare_ends && !closed {
                if let Some(a) = anchors.first_mut() {
                    a.h_in = a.p;
                }
                if let Some(a) = anchors.last_mut() {
                    a.h_out = a.p;
                }
            }
            SubPath::new(anchors, closed)
        },
    );
    prop::collection::vec(sub, 0..4).prop_map(PathData::new)
}

fn arb_affine() -> impl Strategy<Value = Affine> {
    let entry = prop_oneof![Just(0.0), Just(-0.0), Just(1.0), Just(-1.0), arb_coord()];
    prop::array::uniform6(entry).prop_map(Affine::new)
}

/// Every coordinate's bits, every kind and closed flag: equal only when exactly the same.
fn path_bits(p: &PathData) -> Vec<(Vec<[u64; 6]>, Vec<AnchorKind>, bool)> {
    p.subpaths
        .iter()
        .map(|s| {
            let pts = s.anchors.iter().map(|a| [a.p.x, a.p.y, a.h_in.x, a.h_in.y, a.h_out.x, a.h_out.y].map(f64::to_bits)).collect();
            (pts, s.anchors.iter().map(|a| a.kind).collect(), s.closed)
        })
        .collect()
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Geometry {
    path: PathData,
    #[serde(with = "vectorcraft_geom::notation::affine")]
    xf: Affine,
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, failure_persistence: None, ..ProptestConfig::default() })]

    /// Saved as SVG notation (or, where it can't say a path exactly, structured) and as a command
    /// answer, a path and a transform read back bit for bit.
    #[test]
    fn svg_geometry_reads_back_exactly(path in arb_path(), xf in arb_affine()) {
        let g = Geometry { path, xf };
        let saved = vectorcraft_doc::as_saved(|| serde_json::to_string(&g).unwrap());
        let answer = serde_json::to_string(&g).unwrap();
        prop_assert!(!answer.contains("\"d\"") && !answer.contains('('), "answers stay structured: {}", answer);
        let notation = vectorcraft_geom::notation::path_data(&g.path);
        if let Some((d, _)) = &notation {
            // Only what the grammar writes: absolute M, L, C, Z, numbers without `+`, one space.
            prop_assert!(d.chars().all(|c| "MLCZ0123456789.-e ".contains(c)) && !d.contains("  "), "{}", d);
            prop_assert!(saved.contains("\"d\""));
            // Path data reads back bit for bit (-0 stays -0, also in a handle).
            let back: Geometry = serde_json::from_str(&saved).unwrap();
            prop_assert_eq!(path_bits(&back.path), path_bits(&g.path), "{}", saved);
        }
        for text in [&saved, &answer] {
            let back: Geometry = serde_json::from_str(text).unwrap();
            // The structured anchors leave out a handle equal (==) to its point, as they always have.
            prop_assert_eq!(&back.path, &g.path, "{}", text);
            prop_assert_eq!(back.xf.as_coeffs().map(f64::to_bits), g.xf.as_coeffs().map(f64::to_bits), "{}", text);
        }
    }

    /// Arbitrary text is refused or read, never a panic.
    #[test]
    fn svg_geometry_parsers_never_panic(d in "[MmLlHhVvCcZzQA0-9 ,.eE+-]{0,48}", kinds in "[cs x]{0,8}", t in "[a-zA-Z(),. 0-9eE+-]{0,40}") {
        let _ = vectorcraft_geom::notation::parse_path_data(&d, None);
        let _ = vectorcraft_geom::notation::parse_path_data(&d, Some(&kinds));
        let _ = vectorcraft_geom::notation::parse_transform(&t);
    }
}

#[test]
fn rich_document_roundtrips() {
    let d = rich_doc();
    check_native_roundtrip(&d).unwrap();
    let bytes = save(&d, true);
    assert!(sniff(&bytes));
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["format"], "vectorcraft");
    assert_eq!(v["version"], vectorcraft_format::VERSION);
}

#[test]
fn sniff_distinguishes_formats() {
    assert!(sniff(&save(&Document::new(1.0, 1.0), false)));
    assert!(!sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"));
    assert!(!sniff(b"%PDF-1.7"));
    assert!(!sniff(b""));
    assert!(!sniff(&[0xff, 0xfe, 0x00]));
}

#[test]
fn rejects_newer_versions_and_other_formats() {
    let mut v: Value = serde_json::from_slice(&save(&Document::new(1.0, 1.0), false)).unwrap();
    v["version"] = json!(vectorcraft_format::VERSION + 1);
    assert!(matches!(load(&serde_json::to_vec(&v).unwrap()), Err(vectorcraft_format::FormatError::TooNew(_))));
    v["version"] = json!(vectorcraft_format::VERSION);
    v["format"] = json!("other");
    assert!(matches!(load(&serde_json::to_vec(&v).unwrap()), Err(vectorcraft_format::FormatError::NotVectorcraft(_))));
    v["format"] = json!("vectorcraft");
    v["images"] = json!({"x": {"mime": "image/png", "data": "not base64!"}});
    assert!(matches!(load(&serde_json::to_vec(&v).unwrap()), Err(vectorcraft_format::FormatError::BadImage(_))));
}

#[test]
fn unknown_fields_are_preserved() {
    let mut v: Value = serde_json::from_slice(&save(&Document::new(1.0, 1.0), false)).unwrap();
    v["document"]["unknown"] = json!({"futureFeature": {"a": [1, 2, 3]}});
    let d = load(&serde_json::to_vec(&v).unwrap()).unwrap();
    let again: Value = serde_json::from_slice(&save(&d, false)).unwrap();
    assert_eq!(again["document"]["unknown"]["futureFeature"]["a"], json!([1, 2, 3]));
}

#[test]
fn load_repairs_next_id() {
    let d = rich_doc();
    let mut v: Value = serde_json::from_slice(&save(&d, false)).unwrap();
    v["document"]["next_id"] = json!(1);
    let mut back = load(&serde_json::to_vec(&v).unwrap()).unwrap();
    check_document(&back).unwrap();
    let fresh = back.alloc_id();
    assert!(back.node(fresh).is_none(), "allocated id {fresh} collides");
    let max = fixtures::all_ids(&back).into_iter().map(|NodeId(i)| i).max().unwrap();
    assert!(fresh.0 > max);
}

#[test]
fn approx_equal_documents_compare_equal() {
    let d = rich_doc();
    assert!(json_approx_eq(&doc_json(&d), &doc_json(&d), 0.0));
}

#[test]
fn bug_f64_bit_exact_roundtrip() {
    check_native_roundtrip_exact(&rich_doc()).unwrap();
    let mut d = Document::new(126.86291501015239, 214.69463130731182);
    d.raster_effects_ppi = (0.1 + 0.2) * 1000.0;
    check_native_roundtrip_exact(&d).unwrap();
}

/// Saved selections come from the file: older files have none, and a hostile one is cut down to
/// what the Select menu can list, with names that fit and ids the document has.
#[test]
fn saved_selections_from_a_file_are_tidied() {
    let d = rich_doc();
    let mut v: Value = serde_json::from_slice(&save(&d, false)).unwrap();
    assert!(v["document"].get("saved_selections").is_none(), "nothing written when there are none");
    assert!(load(&serde_json::to_vec(&v).unwrap()).unwrap().saved_selections.is_empty());
    let id = fixtures::all_ids(&d).into_iter().next().unwrap();
    let missing = fixtures::all_ids(&d).into_iter().map(|NodeId(i)| i).max().unwrap() + 1;
    let mut list = vec![
        json!({"name": format!("  {}  ", "x".repeat(1000)), "objects": [id.0, missing, id.0]}),
        json!({"name": "   ", "objects": [id.0]}),
        json!({"name": "Dup", "objects": []}),
        json!({"name": "Dup", "objects": [id.0]}),
    ];
    list.extend((0..100).map(|i| json!({"name": format!("S{i}"), "objects": [id.0]})));
    v["document"]["saved_selections"] = json!(list);
    let back = load(&serde_json::to_vec(&v).unwrap()).unwrap();
    let saved = &back.saved_selections;
    assert_eq!(saved.len(), vectorcraft_doc::SavedSelection::MAX);
    assert_eq!(saved[0].name, "x".repeat(vectorcraft_doc::SavedSelection::MAX_NAME));
    assert_eq!(saved[0].objects, vec![id], "a missing id is dropped, a repeated one kept once");
    assert_eq!((saved[1].name.as_str(), saved[1].objects.len()), ("Dup", 0), "blank names go, the first of a name stays");
    assert_eq!(saved[2].name, "S0");
    // What loads saves and loads again unchanged.
    assert_eq!(&load(&save(&back, false)).unwrap().saved_selections, saved);
    // Junk in the member is a format error, not a crash.
    v["document"]["saved_selections"] = json!([{"name": 5, "objects": "x"}]);
    assert!(load(&serde_json::to_vec(&v).unwrap()).is_err());
}
