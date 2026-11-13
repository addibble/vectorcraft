//! Format v5: a colour linked to a global swatch is saved by the swatch's name; a file for a v4
//! reader leaves out defaults as v4 does but writes every colour whole.

use serde_json::{Value, json};
use vectorcraft_doc::color::{Color, Paint, Swatch};
use vectorcraft_doc::{AppearanceItem, Document, FillLayer, Node, NodeKind};
use vectorcraft_format::{SaveOptions, load, save_with};

/// A document with global swatch `Accent` and a rectangle filled with it.
fn linked_doc() -> Document {
    let accent = Color::Rgb { r: 0.6, g: 0.1, b: 0.1 };
    let mut d = Document::new(100.0, 100.0);
    d.swatches.push(Swatch { name: "Accent".into(), paint: Paint::solid(accent), global: true, spot: false });
    let mut n = Node::new(
        d.alloc_id(),
        NodeKind::Path {
            path: vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0)),
            rule: Default::default(),
            live: None,
            clipping: false,
            guide: false,
        },
    );
    n.appearance.items = vec![AppearanceItem::Fill(FillLayer::new(Paint::Solid { color: accent, swatch: Some("Accent".into()), tint: 1.0 }))];
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, n).unwrap();
    d
}

fn fill(bytes: &[u8]) -> Value {
    let v: Value = serde_json::from_slice(bytes).unwrap();
    v["document"]["layers"][0]["kind"]["children"][0]["appearance"]["items"][0]["paint"].clone()
}

#[test]
fn v5_saves_a_linked_colour_by_name_and_v4_whole() {
    let d = linked_doc();
    let v5 = save_with(&d, &SaveOptions::default()).unwrap();
    assert_eq!(fill(&v5), json!("Accent"));
    let v4 = save_with(&d, &SaveOptions { version: 4, ..SaveOptions::default() }).unwrap();
    let whole = fill(&v4);
    assert_eq!((whole["swatch"].as_str(), whole["color"]["model"].as_str()), (Some("Accent"), Some("rgb")), "{whole}");
    // A v4 file leaves out the same defaults (its readers fill them in): only the colour differs.
    let keys = |b: &[u8]| serde_json::from_slice::<Value>(b).unwrap()["document"].as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(&v4), keys(&v5));
    for bytes in [v5, v4] {
        assert_eq!(load(&bytes).unwrap(), d, "both read back as saved");
    }
}

/// `bytes`, a saved file, with its document's JSON changed by `f`.
fn edited(bytes: &[u8], f: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut v: Value = serde_json::from_slice(bytes).unwrap();
    f(&mut v["document"]);
    serde_json::to_vec(&v).unwrap()
}

/// A file whose colours are linked to a swatch it lacks opens with the swatch recreated black and
/// a note; the recreated swatch is the document's own from then on, so a save keeps it and the
/// paints linked to it by name, and the file reopens the same without a note.
#[test]
fn a_recreated_swatch_survives_save_and_reopen() {
    let bytes = edited(&save_with(&linked_doc(), &SaveOptions::default()).unwrap(), |d| {
        d["swatches"].as_array_mut().unwrap().retain(|w| w["name"] != "Accent");
    });
    let opened = vectorcraft_format::load_file(&bytes).unwrap();
    assert!(opened.depth_notes.iter().any(|n| n.contains("“Accent”") && n.contains("recreated")), "{:?}", opened.depth_notes);
    let w = opened.doc.swatch("Accent").expect("recreated");
    assert_eq!((&w.paint, w.global), (&Paint::solid(Color::BLACK), true));
    let saved = save_with(&opened.doc, &SaveOptions::default()).unwrap();
    assert_eq!(fill(&saved), json!("Accent"), "still linked by name");
    let reopened = vectorcraft_format::load_file(&saved).unwrap();
    assert_eq!(reopened.depth_notes, Vec::<String>::new());
    assert_eq!(reopened.doc, opened.doc);
}

/// A name whose first swatch isn't a global colour, though a global colour of the name follows it,
/// opens black with a note: a file never written by VectorCraft, whose saves name only global
/// colours.
#[test]
fn a_name_whose_first_swatch_is_not_global_opens_black_with_a_note() {
    let bytes = edited(&save_with(&linked_doc(), &SaveOptions::default()).unwrap(), |d| {
        let swatches = d["swatches"].as_array_mut().unwrap();
        let mut process = swatches.iter().find(|w| w["name"] == "Accent").unwrap().clone();
        process["global"] = json!(false);
        swatches.insert(0, process);
    });
    let opened = vectorcraft_format::load_file(&bytes).unwrap();
    assert!(opened.depth_notes.iter().any(|n| n.contains("aren't global colours show black: “Accent”")), "{:?}", opened.depth_notes);
    assert_eq!(
        opened.doc.layers[0].children().unwrap()[0].appearance.fill_paint(),
        Paint::Solid { color: Color::BLACK, swatch: Some("Accent".into()), tint: 1.0 }
    );
}

/// A document's JSON read other than as a file opened (a JSON Patch over the open document)
/// refuses a link to a swatch it lacks, naming it and the fix, and recreates nothing.
#[test]
fn reading_a_documents_json_refuses_a_link_to_a_missing_swatch() {
    let bytes = save_with(&linked_doc(), &SaveOptions::default()).unwrap();
    let mut v: Value = serde_json::from_slice(&bytes).unwrap();
    v["document"]["swatches"].as_array_mut().unwrap().retain(|w| w["name"] != "Accent");
    let err = vectorcraft_format::read_document_json(v["document"].take()).unwrap_err().to_string();
    assert!(err.contains("linked by name to \"Accent\", a swatch the document lacks: add \"Accent\" as a global colour swatch"), "{err}");
}
