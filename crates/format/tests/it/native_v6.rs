//! `.vectorcraft` v6: geometry in SVG's notation (paths as path data, transforms as SVG
//! transforms, live shapes without the path they rebuild), read back exactly; older versions keep
//! the structured geometry.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, LiveShape, Node, NodeKind};
use vectorcraft_format::{SaveOptions, VERSION, load, save, save_with};
use vectorcraft_geom::{Affine, Anchor, AnchorKind, FillRule, PathData, Point, SubPath};
use vectorcraft_testkit::fixtures;
use vectorcraft_testkit::invariants::{check_native_roundtrip_exact, doc_json};

fn path_node(d: &mut Document, path: PathData, live: Option<LiveShape>) {
    let layer = d.layers[0].id;
    let id = d.alloc_id();
    let kind = NodeKind::Path { path, rule: FillRule::NonZero, live: live.map(Box::new), clipping: false, guide: false };
    d.insert(Some(layer), 0, Node::new(id, kind)).unwrap();
}

fn geometry_doc() -> Document {
    let mut d = Document::new(200.0, 200.0);
    let curve = SubPath::new(
        vec![
            Anchor { p: Point::new(0.0, 0.0), h_in: Point::new(-5.0, 5.0), h_out: Point::new(5.0, -5.0), kind: AnchorKind::Smooth },
            Anchor { p: Point::new(20.0, 0.25), h_in: Point::new(15.0, -5.0), h_out: Point::new(20.0, 0.25), kind: AnchorKind::Corner },
        ],
        true,
    );
    path_node(&mut d, PathData::single(curve), None);
    path_node(&mut d, PathData::single(SubPath::polyline(&[Point::new(1.0, 1.0), Point::new(9.0, 1.0)], false)), None);
    let rect = LiveShape::Rectangle {
        w: 40.0,
        h: 30.0,
        radii: [0.0; 4],
        kinds: Default::default(),
        radii_relative: false,
        xf: Affine::translate((10.0, 20.0)),
    };
    path_node(&mut d, rect.to_path(), Some(rect));
    d
}

fn document_of(bytes: &[u8]) -> Value {
    let v: Value = serde_json::from_slice(bytes).unwrap();
    v["document"].clone()
}

fn kinds(document: &Value) -> Vec<Value> {
    document["layers"][0]["kind"]["children"].as_array().unwrap().iter().map(|n| n["kind"].clone()).collect()
}

#[test]
fn saved_geometry_is_svg_notation_and_reads_back_exactly() {
    let d = geometry_doc();
    let bytes = save(&d, false);
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["version"], VERSION);
    assert_eq!(VERSION, 6);
    let kinds = kinds(&v["document"]);
    // Inserted at 0: the live rectangle first, the curve last.
    assert_eq!(kinds[0]["live"]["xf"], json!("translate(10 20)"));
    assert!(kinds[0].get("path").is_none(), "a live shape is saved without the path it rebuilds: {}", kinds[0]);
    assert_eq!(kinds[1]["path"], json!({"d": "M1 1L9 1"}));
    assert_eq!(kinds[2]["path"], json!({"d": "M0 0C5 -5 15 -5 20 0.25C20 0.25 -5 5 0 0Z", "kinds": "sc"}));
    let back = load(&bytes).unwrap();
    assert_eq!(back.layers, d.layers, "anchors, kinds and the live shape's path come back exactly");
    check_native_roundtrip_exact(&d).unwrap();
}

#[test]
fn older_versions_keep_the_structured_geometry() {
    let d = geometry_doc();
    for version in 1..VERSION {
        let bytes = save_with(&d, &SaveOptions { version, ..SaveOptions::default() }).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.contains("\"d\":"), "v{version} has no path data");
        assert!(text.contains("\"subpaths\":"), "v{version} keeps anchors");
        assert!(!text.contains("translate("), "v{version} keeps transforms as numbers");
        let kinds = kinds(&document_of(&bytes));
        assert!(kinds[0].get("path").is_some(), "v{version} keeps a live shape's path");
        assert_eq!(load(&bytes).unwrap().layers, d.layers, "v{version}");
    }
}

#[test]
fn a_rich_document_saves_smaller_and_reads_back_the_same() {
    let d = (*fixtures::rich_session().doc().unwrap().doc).clone();
    let v5 = save(&d, false);
    let v4 = save_with(&d, &SaveOptions { version: 4, ..SaveOptions::default() }).unwrap();
    assert!(v5.len() < v4.len(), "{} < {}", v5.len(), v4.len());
    assert_eq!(doc_json(&load(&v5).unwrap()), doc_json(&load(&v4).unwrap()));
}

#[test]
fn a_live_shape_whose_path_differs_keeps_it_and_a_missing_one_is_rebuilt() {
    let mut d = Document::new(100.0, 100.0);
    let ellipse = LiveShape::Ellipse { w: 10.0, h: 20.0, pie: (0.0, 360.0), xf: Affine::IDENTITY };
    let mut moved = ellipse.to_path();
    moved.subpaths[0].anchors[0].p.x += 1.0;
    path_node(&mut d, moved.clone(), Some(ellipse.clone()));
    let bytes = save(&d, false);
    assert!(kinds(&document_of(&bytes))[0].get("path").is_some());
    assert_eq!(load(&bytes).unwrap().layers, d.layers);
    // Written by hand without a path: the shape draws it.
    let mut v: Value = serde_json::from_slice(&bytes).unwrap();
    v["document"]["layers"][0]["kind"]["children"][0]["kind"].as_object_mut().unwrap().remove("path");
    let back = load(&serde_json::to_vec(&v).unwrap()).unwrap();
    let NodeKind::Path { path, .. } = &back.layers[0].children().unwrap()[0].kind else { panic!("a path") };
    assert_eq!(*path, ellipse.to_path());
}

#[test]
fn files_written_by_hand_read_svg_path_data_and_transform_lists() {
    let mut d = Document::new(100.0, 100.0);
    path_node(&mut d, PathData::default(), None);
    let mut v: Value = serde_json::from_slice(&save(&d, false)).unwrap();
    v["document"]["layers"][0]["kind"]["children"][0]["kind"]["path"] = json!({"d": "m10 10 h20 v20 h-20 z"});
    let back = load(&serde_json::to_vec(&v).unwrap()).unwrap();
    let NodeKind::Path { path, .. } = &back.layers[0].children().unwrap()[0].kind else { panic!("a path") };
    let pts: Vec<Point> = path.subpaths[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(pts, [Point::new(10.0, 10.0), Point::new(30.0, 10.0), Point::new(30.0, 30.0), Point::new(10.0, 30.0)]);
    assert!(path.subpaths[0].closed);
    v["document"]["layers"][0]["kind"]["children"][0]["kind"]["path"] = json!({"d": "M0 0A5 5 0 0 1 10 0"});
    let err = load(&serde_json::to_vec(&v).unwrap()).unwrap_err().to_string();
    assert!(err.contains("`A` is not read"), "{err}");
}
