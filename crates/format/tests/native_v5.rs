//! Format v5: a colour linked to a global swatch is saved by the swatch's name; a file for a v4
//! reader leaves out defaults as v4 does but writes every colour whole.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
