//! Extrude & Bevel, Inflate and Rotate on a symbol instance build its solids from its symbol's art,
//! placed and stained as the instance shows it: they draw what the same effect draws on a group of
//! that art.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Effect, Node, NodeKind, Symbol};
use vectorcraft_geom::shapes;

use crate::*;

const W: u32 = 120;
const H: u32 = 100;

/// Two rectangles of different colors, centered on the origin: a symbol's art.
fn art(d: &mut Document, scale: f64) -> Node {
    let mut rect = |r: Rect, c: Color| {
        let r = Rect::new(r.x0 * scale, r.y0 * scale, r.x1 * scale, r.y1 * scale);
        Arc::new(Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(Paint::solid(c), Paint::None, 0.0)))
    };
    let (a, b) =
        (rect(Rect::new(-15.0, -10.0, -2.0, 10.0), Color::rgb(0.1, 0.3, 0.8)), rect(Rect::new(2.0, -6.0, 15.0, 6.0), Color::rgb(0.9, 0.6, 0.1)));
    Node::group(d.alloc_id(), vec![a, b])
}

/// Where the instance places the art.
fn place() -> Affine {
    Affine::translate((55.0, 50.0)) * Affine::rotate(0.3) * Affine::scale(1.2)
}

fn effect(id: &str, params: serde_json::Value) -> Effect {
    Effect { id: id.into(), params, visible: true }
}

/// A document with the symbol "Mark" (art at `scale`) and an instance of it with `fx`; the
/// instance's id.
fn instance_doc(fx: Vec<Effect>, scale: f64) -> (Document, NodeId) {
    let mut d = Document::new(W as f64, H as f64);
    let a = art(&mut d, scale);
    d.symbols.push(Symbol { dynamic: false, name: "Mark".into(), art: Arc::new(a) });
    let mut n = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Mark".into(), xf: place() });
    n.appearance.effects = fx;
    let l = d.layers[0].id;
    let id = d.insert(Some(l), 0, n).unwrap();
    (d, id)
}

/// The same art placed as a group with `fx`.
fn group_doc(fx: Vec<Effect>) -> Document {
    let mut d = Document::new(W as f64, H as f64);
    let mut g = art(&mut d, 1.0);
    g.transform(place(), false);
    g.appearance.effects = fx;
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    d
}

fn render(d: &Document) -> Rendered {
    Renderer::new().render(d, W, H, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

fn solids() -> [Effect; 3] {
    [
        effect("threeD.extrude", json!({"depth": 20.0, "position": "isometricTop"})),
        effect("threeD.inflate", json!({"depth": 8.0, "position": "isometricTop"})),
        effect("threeD.rotate", json!({"rotationX": 50.0, "rotationY": -30.0})),
    ]
}

#[test]
fn a_3d_effect_on_a_symbol_instance_draws_the_solids_of_its_art() {
    let flat = render(&instance_doc(vec![], 1.0).0);
    for fx in solids() {
        let instance = render(&instance_doc(vec![fx.clone()], 1.0).0);
        assert!(instance.pixels != flat.pixels, "{}: drawn as a solid, not flat", fx.id);
        assert!(instance.pixels == render(&group_doc(vec![fx.clone()])).pixels, "{}: as on a group of its art", fx.id);
    }
}

#[test]
fn an_instances_solid_is_measured_by_its_symbols_art() {
    let fx = solids()[0].clone();
    let measure = |d: &Document, id: NodeId| crate::fx::cull_bounds(&d.symbol_boxes(), d.node(id).unwrap()).unwrap();
    let (d, id) = instance_doc(vec![fx.clone()], 1.0);
    let group = group_doc(vec![fx.clone()]);
    let g = &group.layers[0].children().unwrap()[0];
    assert_eq!(measure(&d, id), crate::fx::cull_bounds(&group.symbol_boxes(), g).unwrap());
    // The symbol's art grows, the instance stays as it is: its solid is measured afresh.
    let (bigger, id) = instance_doc(vec![fx], 2.0);
    let (a, b) = (measure(&d, id), measure(&bigger, id));
    assert!(b.width() > a.width() * 1.5 && b.height() > a.height() * 1.5, "{a:?} {b:?}");
}

/// Changing a symbol's 9-slice guides places its art anew, and the solids of an instance with a 3D
/// effect follow: a renderer that drew it before draws it as a new one does.
#[test]
fn an_instances_solids_follow_its_symbols_9_slice_guides() {
    let fx = solids()[0].clone();
    let (mut d, _) = instance_doc(vec![fx], 1.0);
    let guides = |g: f64| Some(vectorcraft_doc::SymbolNineSlice { left: g, top: g, right: g, bottom: g });
    d.set_symbol_nine_slice("Mark", guides(0.1));
    // Stretched, so the guides decide where the art goes.
    if let Some(NodeKind::SymbolInstance { xf, .. }) = d.layers[0].children().map(|c| c[0].id).and_then(|id| d.node_mut(id)).map(|n| &mut n.kind) {
        *xf = Affine::translate((55.0, 50.0)) * Affine::scale_non_uniform(2.5, 0.8);
    }
    let mut r = Renderer::new();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let before = r.render(&d, W, H, Affine::IDENTITY, &opts).pixels;
    d.set_symbol_nine_slice("Mark", guides(0.4));
    let after = r.render(&d, W, H, Affine::IDENTITY, &opts).pixels;
    assert!(r.stats.three_d_built > 0, "drawn as solids, built again");
    assert!(after != before, "the guides moved the art");
    assert!(after == Renderer::new().render(&d, W, H, Affine::IDENTITY, &opts).pixels, "as a new renderer draws it");
}

/// Type in a symbol's art extrudes as type in a group does: as one shape, in its first run's paints.
#[test]
fn type_in_a_symbol_extrudes_as_in_a_group() {
    use vectorcraft_doc::text::{CharStyle, TextObject, TextRun};
    let _fonts = crate::tests_fontchange::fonts_steady();
    let type_art = |d: &mut Document| {
        let style = CharStyle { size: 30.0, fill: Paint::solid(Color::rgb(0.1, 0.3, 0.8)), ..Default::default() };
        let mut t = TextObject::point(vectorcraft_geom::Point::new(-25.0, 10.0), "K", style.clone());
        t.runs.push(TextRun::new("W", CharStyle { fill: Paint::solid(Color::rgb(0.9, 0.6, 0.1)), ..style }));
        Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)))
    };
    let fx = solids()[0].clone();
    let mut inst = Document::new(W as f64, H as f64);
    let art = type_art(&mut inst);
    let held = Node::group(inst.alloc_id(), vec![Arc::new(art)]);
    inst.symbols.push(Symbol { dynamic: false, name: "Word".into(), art: Arc::new(held) });
    let mut n = Node::new(inst.alloc_id(), NodeKind::SymbolInstance { symbol: "Word".into(), xf: place() });
    n.appearance.effects = vec![fx.clone()];
    let l = inst.layers[0].id;
    inst.insert(Some(l), 0, n).unwrap();
    let mut grouped = Document::new(W as f64, H as f64);
    let mut g = Node::group(grouped.alloc_id(), vec![Arc::new(type_art(&mut grouped))]);
    g.transform(place(), false);
    g.appearance.effects = vec![fx];
    let l = grouped.layers[0].id;
    grouped.insert(Some(l), 0, g).unwrap();
    let (a, b) = crate::tests_fontchange::with_fonts_unchanged(|| (render(&inst), render(&grouped)));
    assert!(a.pixels == b.pixels, "as in a group");
}
