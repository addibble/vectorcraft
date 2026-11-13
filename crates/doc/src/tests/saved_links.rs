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

/// The document's first object.
fn first(d: &Document) -> &Node {
    &d.layers[0].children().unwrap()[0]
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

/// A link to a swatch the document lacks opens, as Illustrator's Deleted Global Color: a black
/// global swatch of the name is made, the paints stay linked to it, and a note names it.
#[test]
fn a_fill_linked_by_name_to_a_swatch_the_document_lacks_recreates_it_black() {
    for (tint, black) in [(1.0, Color::BLACK), (0.4, Color::BLACK.tinted(0.4))] {
        let mut v = saved(&doc(tint));
        v["swatches"] = serde_json::json!([]);
        let (back, note) = crate::swatches::noting_recreated_swatches(|| serde_json::from_value::<Document>(v));
        let back = back.unwrap();
        let w = back.swatch("Accent").expect("recreated");
        assert_eq!((&w.paint, w.global, w.spot), (&Paint::solid(Color::BLACK), true, false));
        let fill = first(&back).appearance.fill_paint();
        assert_eq!(fill, Paint::Solid { color: black, swatch: Some("Accent".into()), tint }, "tint {tint}");
        let note = note.expect("a note");
        assert!(note.contains("“Accent”") && note.starts_with("1 swatch "), "{note}");
    }
    // A CMYK document's black is process black.
    let mut v = saved(&doc(1.0));
    v["swatches"] = serde_json::json!([]);
    v["color_mode"] = serde_json::json!("Cmyk");
    let back: Document = crate::swatches::noting_recreated_swatches(|| serde_json::from_value(v)).0.unwrap();
    assert_eq!(back.swatch("Accent").unwrap().paint, Paint::solid(Color::Cmyk { c: 0.0, m: 0.0, y: 0.0, k: 1.0 }));
}

/// The note lists at most ten names, and none when nothing was recreated.
#[test]
fn the_note_of_recreated_swatches_is_capped() {
    let mut v = saved(&doc(1.0));
    v["swatches"] = serde_json::json!([]);
    let names: Vec<String> = (0..13).map(|i| format!("S{i:02}")).collect();
    v["char_styles"] = serde_json::json!(names.iter().map(|n| serde_json::json!({"name": n, "attrs": {"fill": n}})).collect::<Vec<_>>());
    let (back, note) = crate::swatches::noting_recreated_swatches(|| serde_json::from_value::<Document>(v));
    assert_eq!(back.unwrap().swatches.len(), 14);
    let note = note.unwrap();
    assert!(note.starts_with("14 swatches ") && note.contains("“Accent”") && note.contains("“S08”") && !note.contains("“S09”"), "{note}");
    assert!(note.contains(" and 4 more."), "{note}");
    let (_, none) = crate::swatches::noting_recreated_swatches(|| serde_json::from_value::<Document>(saved(&doc(1.0))));
    assert_eq!(none, None);
}

/// A link by name takes the colour linking gives: a Lab spot colour through the colour settings
/// (which a saved file never writes by name, but a hand-made one may), and Registration.
#[test]
fn a_name_reads_as_linking_gives_its_colour() {
    let lab = Color::Lab { l: 50.0, a: 60.0, b: 30.0 };
    let mut d = doc(1.0);
    d.swatches[0] = Swatch { name: "Accent".into(), paint: Paint::solid(lab), global: true, spot: true };
    d.spot_use_lab = false;
    let shown = d.global_color("Accent").unwrap();
    let mut v = saved(&d);
    v["layers"][0]["kind"]["children"][0]["appearance"]["items"][0]["paint"] = serde_json::json!("Accent");
    let back: Document = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(first(&back).appearance.fill_paint(), Paint::Solid { color: shown, swatch: Some("Accent".into()), tint: 1.0 });
    let registration = vectorcraft_color::swatch::REGISTRATION;
    v["layers"][0]["kind"]["children"][0]["appearance"]["items"][0]["paint"] = serde_json::json!(registration);
    let back: Document = serde_json::from_value(v).unwrap();
    assert_eq!(first(&back).appearance.fill_paint().color(), back.global_color(registration));
    assert!(!back.swatches_iter().any(|w| w.name == registration), "the built-in one, not recreated");
}

/// A character style's fill or stroke given by a swatch's name is filled in as the document is
/// read, though nothing else in it is, so no colour without its value reaches type.
#[test]
fn a_character_styles_paint_by_name_is_filled_in() {
    let mut v = saved(&doc(1.0));
    v["char_styles"] =
        serde_json::json!([{"name": "Red", "attrs": {"fill": "Accent", "stroke": {"type": "solid", "swatch": "Accent", "tint": 0.5}}}]);
    // The art's own paint written whole: only the style leaves its colour to the swatch.
    v["layers"][0]["kind"]["children"][0]["appearance"]["items"][0]["paint"] = serde_json::to_value(Paint::solid(accent())).unwrap();
    let back: Document = serde_json::from_value(v).unwrap();
    let attrs = &back.char_styles[0].attrs;
    let fill: Paint = serde_json::from_value(attrs["fill"].clone()).unwrap();
    assert_eq!(fill, Paint::Solid { color: accent(), swatch: Some("Accent".into()), tint: 1.0 });
    let stroke: Paint = serde_json::from_value(attrs["stroke"].clone()).unwrap();
    assert_eq!(stroke, Paint::Solid { color: accent().tinted(0.5), swatch: Some("Accent".into()), tint: 0.5 });
}

/// Paints by name everywhere a document holds them, not only a top-level fill: a stroke at a tint,
/// a symbol's art and a tint swatch's own paint are written by name and read back the same.
#[test]
fn linked_paints_inside_the_document_are_saved_by_name_and_read_back() {
    let mut d = doc(1.0);
    let linked = |tint: f32| Paint::Solid { color: accent().tinted(tint), swatch: Some("Accent".into()), tint };
    let mut art = first(&d).clone();
    art.appearance.items.push(AppearanceItem::Stroke(crate::StrokeLayer::new(linked(0.5), 1.0)));
    let id = first(&d).id;
    *d.node_mut(id).unwrap() = art.clone();
    art.id = d.alloc_id();
    d.symbols.push(crate::Symbol { name: "Mark".into(), art: std::sync::Arc::new(art), dynamic: false });
    d.swatches.push(Swatch { name: "Accent 30%".into(), paint: linked(0.3), global: false, spot: false });
    let v = saved(&d);
    let text = v.to_string();
    assert_eq!(text.matches(r#""paint":"Accent""#).count(), 2, "the fill and the symbol's fill: {text}");
    assert_eq!(
        text.matches(r#""swatch":"Accent","tint":0.5,"type":"solid""#).count()
            + text.matches(r#""type":"solid","swatch":"Accent","tint":0.5"#).count(),
        2,
        "{text}"
    );
    let tint = v["swatches"].as_array().unwrap().iter().find(|w| w["name"] == "Accent 30%").unwrap();
    assert!(tint["paint"].get("color").is_none(), "the tint swatch's own paint: {tint}");
    let back: Document = serde_json::from_value(v).unwrap();
    assert_eq!(back, d);
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

/// Only a file being opened recreates a missing swatch. Any other read of a document's JSON (a
/// JSON Patch over the open document, a host writing one of its parts back) refuses a link to a
/// swatch the document lacks, naming it and saying how to fix it, and makes no swatch: a string
/// that names none ("none", "#336699") is refused as it always was.
#[test]
fn outside_a_file_opened_a_link_to_a_missing_swatch_is_refused() {
    for tint in [1.0, 0.4] {
        let mut v = saved(&doc(tint));
        v["swatches"] = serde_json::json!([]);
        let err = serde_json::from_value::<Document>(v).unwrap_err().to_string();
        assert!(err.contains("linked by name to \"Accent\", a swatch the document lacks"), "{err}");
        assert!(err.contains("add \"Accent\" as a global colour swatch, or give the paint its colour"), "{err}");
    }
    for name in ["none", "#336699"] {
        let mut v = saved(&doc(1.0));
        v["layers"][0]["kind"]["children"][0]["appearance"]["items"][0]["paint"] = serde_json::json!(name);
        let err = serde_json::from_value::<Document>(v).unwrap_err().to_string();
        assert!(err.contains(&format!("\"{name}\", a swatch the document lacks")), "{err}");
        assert_eq!(err.contains("{\"type\": \"none\"}"), name == "none", "{err}");
    }
    // Many names: the first few, and how many more.
    let mut v = saved(&doc(1.0));
    v["swatches"] = serde_json::json!([]);
    let names: Vec<String> = (0..13).map(|i| format!("S{i:02}")).collect();
    v["char_styles"] = serde_json::json!(names.iter().map(|n| serde_json::json!({"name": n, "attrs": {"fill": n}})).collect::<Vec<_>>());
    let err = serde_json::from_value::<Document>(v).unwrap_err().to_string();
    assert!(err.contains("\"Accent\"") && err.contains("\"S08\"") && !err.contains("\"S09\"") && err.contains(" and 4 more"), "{err}");
}

/// A name that is a swatch's but not a global colour's (a process swatch, a gradient) gives no
/// colour: refused outside a file opened; a file opened shows it black and its note says so. The
/// first swatch of the name decides, as [`Document::swatch`] finds it, though a global colour of
/// the name comes after it.
#[test]
fn a_link_by_name_to_a_swatch_that_is_not_a_global_colour() {
    let gradient = Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default())));
    for paint in [Paint::solid(accent()), gradient] {
        let global = matches!(paint, Paint::Gradient(_));
        let mut d = doc(1.0);
        // The first swatch of the name, before the global colour "Accent" that `doc` adds.
        d.swatches[0] = Swatch { name: "Accent".into(), paint: paint.clone(), global, spot: false };
        let mut v = saved(&d);
        v["layers"][0]["kind"]["children"][0]["appearance"]["items"][0]["paint"] = serde_json::json!("Accent");
        let err = serde_json::from_value::<Document>(v.clone()).unwrap_err().to_string();
        assert!(err.contains("linked by name to \"Accent\", a swatch that isn't a global colour"), "{err}");
        assert!(err.contains("make \"Accent\" a global colour swatch, or give the paint its colour"), "{err}");
        let (back, note) = crate::swatches::noting_recreated_swatches(|| serde_json::from_value::<Document>(v));
        let back = back.unwrap();
        assert_eq!(back.swatches.len(), d.swatches.len(), "none recreated");
        assert_eq!(back.swatch("Accent").unwrap().paint, paint);
        assert_eq!(first(&back).appearance.fill_paint(), Paint::Solid { color: Color::BLACK, swatch: Some("Accent".into()), tint: 1.0 });
        let note = note.expect("a note");
        assert!(note.contains("swatches that aren't global colours show black: “Accent”"), "{note}");
        assert!(!note.contains("recreated"), "{note}");
    }
}
