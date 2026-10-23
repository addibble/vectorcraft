//! A colour linked to a global swatch is saved by the swatch's name and filled in from the swatch
//! when the document is read ([`vectorcraft_color::saved_links`]).

use serde_json::Value;
use vectorcraft_color::{Color, Paint, Swatch};

use crate::{AppearanceItem, Document, FillLayer, Node, NodeKind};

fn accent() -> Color {
    Color::Rgb { r: 0.6, g: 0.1, b: 0.1 }
}

/// A document with global swatch `Accent` and a rectangle whose fill links to it at `tint`.
fn doc(tint: f32) -> Document {
    let mut d = Document::new(100.0, 100.0);
    d.swatches.push(Swatch { name: "Accent".into(), paint: Paint::solid(accent()), global: true, spot: false });
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
    let fill = Paint::Solid { color: accent().tinted(tint), swatch: Some("Accent".into()), tint };
    n.appearance.items = vec![AppearanceItem::Fill(FillLayer::new(fill))];
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, n).unwrap();
    d
}

fn saved(d: &Document) -> Value {
    crate::as_saved(|| serde_json::to_value(d)).unwrap()
}

/// Every paint in `v`'s art, as written.
fn fills(v: &Value) -> Vec<Value> {
    let mut out = vec![];
    fn walk(v: &Value, out: &mut Vec<Value>) {
        match v {
            Value::Object(o) => {
                if let Some(p) = o.get("paint") {
                    out.push(p.clone());
                }
                o.values().for_each(|x| walk(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    walk(&v["layers"], &mut out);
    out
}

#[test]
fn a_linked_fill_is_saved_by_name_and_reads_back_the_same() {
    for (tint, written) in [(1.0, serde_json::json!("Accent")), (0.4, serde_json::json!({"type": "solid", "swatch": "Accent", "tint": 0.4f32}))] {
        let d = doc(tint);
        let v = saved(&d);
        assert_eq!(fills(&v), [written], "tint {tint}");
        let back: Document = serde_json::from_value(v).unwrap();
        assert_eq!(back, d, "the colour is filled in from the swatch");
    }
}

#[test]
fn outside_as_saved_a_linked_fill_keeps_its_colour() {
    let v = serde_json::to_value(doc(1.0)).unwrap();
    assert!(fills(&v)[0]["color"].is_object(), "{:?}", fills(&v));
}

#[test]
fn a_fill_linked_by_name_to_a_swatch_the_document_lacks_is_refused() {
    let mut v = saved(&doc(1.0));
    v["swatches"] = serde_json::json!([]);
    let err = serde_json::from_value::<Document>(v).unwrap_err().to_string();
    assert!(err.contains("\"Accent\""), "{err}");
}

#[test]
fn a_spot_lab_swatch_shown_through_the_colour_settings_keeps_its_colours_written() {
    let lab = Color::Lab { l: 50.0, a: 60.0, b: 30.0 };
    let mut d = doc(1.0);
    d.swatches[0] = Swatch { name: "Accent".into(), paint: Paint::solid(lab), global: true, spot: true };
    d.spot_use_lab = false;
    // Shown as its working-CMYK equivalent, which another machine's settings may make otherwise:
    // never by name.
    assert!(!d.saved_links().contains_key("Accent"));
    assert!(fills(&saved(&d))[0]["color"].is_object());
}
