//! Extrude & Bevel, Inflate and Rotate on a symbol instance: applied to it as to a group of its
//! symbol's art, expanded to that art's solids, exported as their meshes.
#![allow(clippy::unwrap_used, clippy::panic)]
use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use crate::Session;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Two filled rectangles made a symbol: the instance that replaced them, selected.
fn instance(s: &mut Session) -> NodeId {
    run(s, "file.new", json!({"width": 400, "height": 400}));
    let a = run(s, "path.create", json!({"d": "M100 100L160 100L160 200L100 200Z"}))["id"].as_u64().unwrap();
    run(s, "paint.setFill", json!({"color": "#338ad6"}));
    let b = run(s, "path.create", json!({"d": "M170 120L240 120L240 180L170 180Z"}))["id"].as_u64().unwrap();
    run(s, "paint.setFill", json!({"color": "#d68a33"}));
    run(s, "select.set", json!({"ids": [a, b]}));
    let id = NodeId(run(s, "symbol.new", json!({"name": "Mark"}))["id"].as_u64().unwrap());
    run(s, "select.set", json!({"ids": [id.0]}));
    id
}

fn render(s: &Session) -> Vec<u8> {
    let doc = &s.doc().unwrap().doc;
    vectorcraft_render::Renderer::new().render(doc, 400, 400, vectorcraft_geom::Affine::IDENTITY, &Default::default()).pixels
}

#[test]
fn a_symbol_instance_takes_extrude_inflate_and_rotate() {
    for effect in ["threeD.extrude", "threeD.inflate", "threeD.rotate"] {
        let mut s = Session::new();
        let id = instance(&mut s);
        let flat = render(&s);
        run(&mut s, "effect.apply", json!({"effect": effect, "item": null, "params": {"rotationX": 30.0, "rotationY": -20.0}}));
        let n = s.doc().unwrap().doc.node(id).unwrap();
        assert!(matches!(n.kind, NodeKind::SymbolInstance { .. }), "{effect}: it stays an instance");
        assert_eq!(n.appearance.effects[0].id, effect);
        assert_ne!(render(&s), flat, "{effect}: drawn as a solid");
        run(&mut s, "effect.setParams", json!({"index": 0, "item": null, "params": {"rotationX": 60.0}}));
    }
}

#[test]
fn a_symbol_whose_art_holds_what_cannot_be_a_solid_is_refused_with_why() {
    let mut s = Session::new();
    let inner = instance(&mut s);
    let c = run(&mut s, "path.create", json!({"d": "M100 250L200 250L200 300L100 300Z"}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [inner.0, c]}));
    let outer = run(&mut s, "symbol.new", json!({"name": "Outer"}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [outer]}));
    let e = s.execute("effect.apply", &json!({"effect": "threeD.extrude", "item": null, "params": {}})).unwrap_err().to_string();
    assert!(e.contains("symbol instance") && e.contains("edit the symbol") && e.contains("symbol instances"), "{e}");
}

#[test]
fn a_3d_effect_on_an_instances_fill_is_refused_with_why() {
    let mut s = Session::new();
    instance(&mut s);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    let e = s.execute("effect.apply", &json!({"effect": "threeD.extrude", "item": 0, "params": {}})).unwrap_err().to_string();
    assert!(e.contains("whole instance"), "{e}");
}

/// What exports bake (SVG, PDF, print) draws a stained instance's solids as the canvas draws them,
/// with a color adjustment after the 3D effect too, which recolors the shaded faces.
#[test]
fn exports_bake_an_instances_stained_solids_as_drawn() {
    for adjusted in [false, true] {
        let mut s = Session::new();
        instance(&mut s);
        run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
        run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}}));
        if adjusted {
            run(&mut s, "effect.apply", json!({"effect": "adjust.hueSaturation", "item": null, "params": {"hue": 90}}));
        }
        let doc = &s.doc().unwrap().doc;
        let baked = vectorcraft_render::effects::bake_document(doc).unwrap();
        // Baked into the solids' faces: no instance is left for an export to write flat.
        let mut live = 0;
        for layer in &baked.layers {
            layer.walk(&mut |n| live += usize::from(matches!(n.kind, NodeKind::SymbolInstance { .. })));
        }
        assert_eq!(live, 0, "adjusted {adjusted}: the instance baked into its solids");
        let draw = |d: &vectorcraft_doc::Document| {
            vectorcraft_render::Renderer::new().render(d, 400, 400, vectorcraft_geom::Affine::IDENTITY, &Default::default()).pixels
        };
        let (drawn, exported) = (draw(doc), draw(&baked));
        let differ = drawn.iter().zip(&exported).filter(|(a, b)| a.abs_diff(**b) > 8).count();
        assert!(differ < drawn.len() / 400, "adjusted {adjusted}: {differ} channels differ");
    }
}

#[test]
fn expand_appearance_makes_an_instances_solids_its_art() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}}));
    let drawn = render(&s);
    run(&mut s, "effect.expandAppearance", json!({}));
    let d = &s.doc().unwrap().doc;
    let mut instances = 0;
    let mut paths = 0;
    for layer in &d.layers {
        layer.walk(&mut |n| match n.kind {
            NodeKind::SymbolInstance { .. } => instances += 1,
            NodeKind::Path { .. } => paths += 1,
            _ => {}
        });
    }
    assert_eq!(instances, 0, "the instance became its solids");
    assert!(paths > 10, "the faces of both solids: {paths}");
    assert!(d.node(id).is_some_and(|n| n.appearance.effects.is_empty()));
    let after = render(&s);
    let differ = drawn.iter().zip(&after).filter(|(a, b)| a.abs_diff(**b) > 8).count();
    assert!(differ < drawn.len() / 200, "drawn as before: {differ} channels differ");
}

#[test]
fn an_instances_solids_export_as_meshes() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}}));
    run(&mut s, "assets.add3D", json!({"ids": [id.0]}));
    let r = run(&mut s, "assets.export3D", json!({}));
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "{r}");
}

/// A symbol changed after its instance took a 3D effect, so that its art can't become solids, is
/// refused by 3D export as applying the effect would refuse it, naming the symbol.
#[test]
fn a_symbol_changed_past_3d_is_refused_by_3d_export_with_why() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}}));
    run(&mut s, "assets.add3D", json!({"ids": [id.0]}));
    // The symbol's art now holds an instance of another symbol.
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let held = vectorcraft_doc::Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Mark".into(), xf: vectorcraft_geom::Affine::IDENTITY });
    let art = vectorcraft_doc::Node::group(d.alloc_id(), vec![std::sync::Arc::new(held)]);
    d.symbols.iter_mut().find(|m| m.name == "Mark").unwrap().art = std::sync::Arc::new(art);
    for (cmd, p) in [("assets.add3D", json!({"ids": [id.0]})), ("assets.export3D", json!({}))] {
        let e = s.execute(cmd, &p).unwrap_err().to_string();
        assert!(e.contains("symbol instance") && e.contains("edit the symbol"), "{cmd}: {e}");
    }
}

/// Canvas, exports and Expand Appearance draw an instance's solids alike with a geometry effect
/// after its 3D effect and a color adjustment: the effects after the 3D effect act on the solids.
#[test]
fn an_instances_solids_under_later_effects_export_and_expand_as_drawn() {
    let mut s = Session::new();
    instance(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}}));
    run(&mut s, "effect.apply", json!({"effect": "distort.transform", "item": null, "params": {"scaleH": 50.0, "scaleV": 50.0, "rotate": 30.0}}));
    run(&mut s, "effect.apply", json!({"effect": "adjust.hueSaturation", "item": null, "params": {"hue": 1}}));
    let draw = |d: &vectorcraft_doc::Document| {
        vectorcraft_render::Renderer::new().render(d, 400, 400, vectorcraft_geom::Affine::IDENTITY, &Default::default()).pixels
    };
    let drawn = draw(&s.doc().unwrap().doc);
    let differ = |other: &[u8]| drawn.iter().zip(other).filter(|(a, b)| a.abs_diff(**b) > 8).count();
    let baked = vectorcraft_render::effects::bake_document(&s.doc().unwrap().doc).unwrap();
    assert!(differ(&draw(&baked)) < drawn.len() / 400, "exported: {}", differ(&draw(&baked)));
    run(&mut s, "effect.expandAppearance", json!({}));
    let expanded = draw(&s.doc().unwrap().doc);
    assert!(differ(&expanded) < drawn.len() / 400, "expanded: {}", differ(&expanded));
}

/// Type across a 9-slice guide is cut into clipping groups when a stretched instance places the
/// symbol's art: the refusal says so, not that the symbol holds clipping groups.
#[test]
fn a_9_slice_symbol_whose_guides_cut_type_is_refused_with_why() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 400}));
    let t = run(&mut s, "text.create", json!({"x": 100, "y": 200, "text": "WWWW", "size": 60}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [t]}));
    let id = run(&mut s, "symbol.new", json!({"name": "Sliced", "nineSlice": true}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [id]}));
    // Stretched, the guides cut the art into cells.
    run(&mut s, "object.transformEach", json!({"scaleH": 250.0, "scaleV": 60.0}));
    let e = s.execute("effect.apply", &json!({"effect": "threeD.extrude", "item": null, "params": {}})).unwrap_err().to_string();
    assert!(e.contains("9-slice") && !e.contains("its clipping groups"), "{e}");
}

/// Expand Appearance on an instance whose symbol changed, after its 3D effect was applied, into art
/// that can't become solids leaves out the 3D effect, which draws nothing: the instance stays, as
/// the canvas draws it.
#[test]
fn expand_appearance_expands_an_instance_whose_3d_effect_cant_build() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}}));
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let held = vectorcraft_doc::Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Other".into(), xf: vectorcraft_geom::Affine::IDENTITY });
    let art = vectorcraft_doc::Node::group(d.alloc_id(), vec![std::sync::Arc::new(held)]);
    d.symbols.iter_mut().find(|m| m.name == "Mark").unwrap().art = std::sync::Arc::new(art);
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = s.doc().unwrap().doc.node(id).cloned().unwrap();
    assert!(n.appearance.effects.is_empty(), "the 3D effect left out");
}

/// With a path and an instance selected and the path's fill the active row, a 3D effect applies:
/// the instance, which has no fill row there, takes it on the object.
#[test]
fn a_3d_effect_with_a_fill_row_active_applies_to_an_instance_on_the_object() {
    let mut s = Session::new();
    let inst = instance(&mut s);
    // Stained: the instance has a fill row too, which 3D never fits.
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    let p = run(&mut s, "path.create", json!({"d": "M300 300L350 300L350 350L300 350Z"}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"color": "#33aa55"}));
    run(&mut s, "select.set", json!({"ids": [p, inst.0]}));
    run(&mut s, "appearance.setActiveItem", json!({"index": 0}));
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "params": {"depth": 20.0}}));
    let d = &s.doc().unwrap().doc;
    let n = d.node(inst).unwrap();
    assert!(n.appearance.effects.iter().any(|e| e.id == "threeD.extrude"), "on the instance");
    assert!(n.appearance.items.iter().all(|i| i.effects().is_empty()), "not on its fill");
}

/// A reflected, turned and stretched instance's solids export as the canvas draws them.
#[test]
fn a_transformed_instances_solids_export_as_drawn() {
    let mut s = Session::new();
    instance(&mut s);
    run(&mut s, "object.transformEach", json!({"scaleH": 140.0, "scaleV": 70.0, "rotate": 25.0, "reflectX": true}));
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}}));
    let doc = &s.doc().unwrap().doc;
    let draw = |d: &vectorcraft_doc::Document| {
        vectorcraft_render::Renderer::new().render(d, 400, 400, vectorcraft_geom::Affine::IDENTITY, &Default::default()).pixels
    };
    let (drawn, exported) = (draw(doc), draw(&vectorcraft_render::effects::bake_document(doc).unwrap()));
    let differ = drawn.iter().zip(&exported).filter(|(a, b)| a.abs_diff(**b) > 8).count();
    assert!(differ < drawn.len() / 400, "{differ} channels differ");
}

/// The pixels of `s`'s document: as drawn, as exports bake it, and after Expand Appearance of
/// the selection.
fn drawn_baked_expanded(s: &mut Session) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let draw = |d: &vectorcraft_doc::Document| {
        vectorcraft_render::Renderer::new().render(d, 400, 400, vectorcraft_geom::Affine::IDENTITY, &Default::default()).pixels
    };
    let drawn = draw(&s.doc().unwrap().doc);
    let baked = vectorcraft_render::effects::bake_document(&s.doc().unwrap().doc).unwrap();
    run(s, "effect.expandAppearance", json!({}));
    // A color adjustment left in the art would be applied again when it is drawn.
    for (what, d) in [("exported", &baked), ("expanded", &s.doc().unwrap().doc)] {
        let mut left = 0;
        for layer in &d.layers {
            layer.walk(&mut |n| {
                let items = n.appearance.items.iter().flat_map(|i| i.effects());
                left += n.appearance.effects.iter().chain(items).filter(|e| vectorcraft_render::effects::is_adjustment(&e.id)).count();
            });
        }
        assert_eq!(left, 0, "{what}: no color adjustment left");
    }
    (drawn, draw(&baked), draw(&s.doc().unwrap().doc))
}

fn render_doc(d: &vectorcraft_doc::Document) -> Vec<u8> {
    vectorcraft_render::Renderer::new().render(d, 400, 400, vectorcraft_geom::Affine::IDENTITY, &Default::default()).pixels
}

fn differ(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x.abs_diff(**y) > 8).count()
}

/// A color adjustment after a 3D effect recolors the shaded solids: exports and Expand Appearance
/// as on the canvas, for an instance and for a group, with an adjustment that shading doesn't
/// commute with.
#[test]
fn an_adjustment_after_3d_recolors_the_shaded_solids_everywhere() {
    for grouped in [false, true] {
        let mut s = Session::new();
        instance(&mut s);
        if grouped {
            run(&mut s, "symbol.breakLink", json!({}));
        }
        run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}}));
        run(&mut s, "effect.apply", json!({"effect": "adjust.brightnessContrast", "item": null, "params": {"brightness": 60, "contrast": 60}}));
        let (drawn, baked, expanded) = drawn_baked_expanded(&mut s);
        assert!(differ(&drawn, &baked) < drawn.len() / 400, "grouped {grouped}: exported {}", differ(&drawn, &baked));
        assert!(differ(&drawn, &expanded) < drawn.len() / 400, "grouped {grouped}: expanded {}", differ(&drawn, &expanded));
    }
}

/// An instance with a color adjustment whose 3D effect can't build any more (its symbol changed)
/// expands and exports flat, as the canvas draws it.
#[test]
fn an_adjusted_instance_whose_3d_effect_cant_build_expands_flat() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}}));
    run(&mut s, "effect.apply", json!({"effect": "distort.roughen", "item": null, "params": {"size": 2, "detail": 2}}));
    run(&mut s, "effect.apply", json!({"effect": "adjust.hueSaturation", "item": null, "params": {"hue": 90}}));
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let held = vectorcraft_doc::Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Other".into(), xf: vectorcraft_geom::Affine::IDENTITY });
    let art = d.symbols.iter().find(|m| m.name == "Mark").unwrap().art.clone();
    let mixed = vectorcraft_doc::Node::group(d.alloc_id(), vec![art, std::sync::Arc::new(held)]);
    d.symbols.iter_mut().find(|m| m.name == "Mark").unwrap().art = std::sync::Arc::new(mixed);
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let (drawn, baked, expanded) = drawn_baked_expanded(&mut s);
    let mut three_d = 0;
    for layer in &s.doc().unwrap().doc.layers {
        layer.walk(&mut |n| three_d += usize::from(n.appearance.effects.iter().any(|e| e.id.starts_with("threeD."))));
    }
    assert_eq!(three_d, 0, "no 3D effect left");
    assert!(differ(&drawn, &baked) < drawn.len() / 400, "exported {}", differ(&drawn, &baked));
    assert!(differ(&drawn, &expanded) < drawn.len() / 400, "expanded {}", differ(&drawn, &expanded));
}

/// A clipping group the user drew in a 9-slice symbol's art is refused as one, not as the guides'
/// cutting.
#[test]
fn a_drawn_clipping_group_in_a_9_slice_symbol_is_refused_as_one() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 400}));
    let a = run(&mut s, "path.create", json!({"d": "M100 100L200 100L200 200L100 200Z"}))["id"].as_u64().unwrap();
    let b = run(&mut s, "path.create", json!({"d": "M120 120L180 120L180 180L120 180Z"}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [a, b]}));
    run(&mut s, "object.clippingMask.make", json!({}));
    let id = run(&mut s, "symbol.new", json!({"name": "Clipped", "nineSlice": true}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [id]}));
    run(&mut s, "object.transformEach", json!({"scaleH": 250.0, "scaleV": 60.0}));
    let e = s.execute("effect.apply", &json!({"effect": "threeD.extrude", "item": null, "params": {}})).unwrap_err().to_string();
    assert!(e.contains("clipping groups") && !e.contains("9-slice"), "{e}");
}

/// Expanding an instance's solids leaves none of its own parts on what it makes: a dynamic
/// symbol's appearance changes are the instance's, not its art's.
#[test]
fn expanding_an_instances_solids_keeps_none_of_its_overrides() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}}));
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    d.node_mut(id).unwrap().extras_mut().symbol_overrides =
        vec![vectorcraft_doc::symbol_overrides::SymbolOverride { path: vec![0], opacity: Some(0.5), ..Default::default() }];
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let mut overrides = 0;
    for layer in &s.doc().unwrap().doc.layers {
        layer.walk(&mut |n| overrides += n.extras().symbol_overrides.len());
    }
    assert_eq!(overrides, 0);
}

/// A 3D effect on a symbol instance's fill (as a graphic style might set it) builds nothing: the
/// instance exports and expands flat, as the canvas draws it. One on a group's own fill builds the
/// fill's solid, exported and expanded as drawn.
#[test]
fn a_3d_effect_on_a_fill_exports_and_expands_as_drawn() {
    for grouped in [false, true] {
        let mut s = Session::new();
        let id = instance(&mut s);
        let fx = vectorcraft_doc::Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0}), visible: true };
        if grouped {
            run(&mut s, "symbol.breakLink", json!({}));
            run(&mut s, "appearance.addFill", json!({}));
            run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": 0, "params": {"depth": 20.0}}));
        } else {
            run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
            let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
            d.node_mut(id).unwrap().appearance.effects_mut(Some(0)).unwrap().push(fx);
        }
        let (drawn, baked, expanded) = drawn_baked_expanded(&mut s);
        assert!(differ(&drawn, &baked) < drawn.len() / 400, "exported {}", differ(&drawn, &baked));
        assert!(differ(&drawn, &expanded) < drawn.len() / 400, "expanded {}", differ(&drawn, &expanded));
    }
}

/// An instance's fill stains its art and draws none of its effects: with a 3D effect and a Drop
/// Shadow there (a graphic style, the Eyedropper or Appearance › Transfer can put them there),
/// exports and Expand Appearance draw no shadow either, and Expand Appearance makes no image for
/// it.
#[test]
fn an_instances_fill_shadow_is_drawn_nowhere() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let fx = |id: &str, params: Value| vectorcraft_doc::Effect { id: id.into(), params, visible: true };
    let item = d.node_mut(id).unwrap().appearance.effects_mut(Some(0)).unwrap();
    item.push(fx("threeD.extrude", json!({"depth": 20.0})));
    item.push(fx("stylize.dropShadow", json!({"x": 12, "y": 12, "blur": 0, "opacity": 100})));
    let (drawn, baked, expanded) = drawn_baked_expanded(&mut s);
    assert!(differ(&drawn, &baked) < drawn.len() / 400, "exported {}", differ(&drawn, &baked));
    assert!(differ(&drawn, &expanded) < drawn.len() / 400, "expanded {}", differ(&drawn, &expanded));
    // Expanded, it makes no image of a shadow it never draws.
    let mut images = 0;
    for l in &s.doc().unwrap().doc.layers {
        l.walk(&mut |n| images += usize::from(matches!(n.kind, vectorcraft_doc::NodeKind::Image(_))));
    }
    assert_eq!(images, 0);
}

/// A 3D effect beside Crop Marks draws its solid with the marks: exports and Expand Appearance as
/// the canvas draws it.
#[test]
fn a_3d_effect_with_crop_marks_exports_and_expands_as_drawn() {
    let mut s = Session::new();
    instance(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}}));
    run(&mut s, "effect.apply", json!({"effect": "cropMarks", "item": null, "params": {}}));
    let (drawn, baked, expanded) = drawn_baked_expanded(&mut s);
    assert!(differ(&drawn, &baked) < drawn.len() / 400, "exported {}", differ(&drawn, &baked));
    assert!(differ(&drawn, &expanded) < drawn.len() / 400, "expanded {}", differ(&drawn, &expanded));
}

/// A symbol instance in perspective doesn't take a 3D effect: its solids wouldn't lie on the plane.
#[test]
fn an_instance_in_perspective_is_refused_3d() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    run(&mut s, "perspective.attach", json!({"ids": [id.0], "plane": "right"}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let e = s.execute("effect.apply", &json!({"effect": "threeD.extrude", "item": null, "params": {}})).unwrap_err().to_string();
    assert!(e.contains("perspective"), "{e}");
}

/// Applied with a stained instance's fill row active, the effect lands on the instance itself, and
/// the answer says so.
#[test]
fn effect_apply_reports_the_object_level_for_an_instance() {
    let mut s = Session::new();
    instance(&mut s);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "appearance.setActiveItem", json!({"index": 0}));
    let r = run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "params": {"depth": 20.0}}));
    assert!(r["item"].is_null(), "{r}");
}

/// Expanding an instance's solids keeps its own Attributes, slice options and graphic style link,
/// as expanding a path or a group keeps them.
#[test]
fn expanding_an_instances_solids_keeps_its_own_parts() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}}));
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    d.node_mut(id).unwrap().extras_mut().slice = Some(Box::default());
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "effect.expandAppearance", json!({}));
    assert!(s.doc().unwrap().doc.node(id).is_some_and(|n| n.extras().slice.is_some()));
}

/// Attaching a symbol instance with a 3D effect to the perspective grid is refused, saying why: its
/// solids wouldn't lie on the plane.
#[test]
fn an_instance_with_3d_isnt_attached_to_the_grid() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 20.0}}));
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    let e = s.execute("perspective.attach", &json!({"ids": [id.0], "plane": "right"})).unwrap_err().to_string();
    assert!(e.contains("perspective"), "{e}");
}

/// Moving an instance's 3D effect onto its fill is refused: there it would do nothing.
#[test]
fn an_instances_3d_effect_isnt_moved_onto_its_fill() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 20.0}}));
    let e = s.execute("effect.move", &json!({"from": 0, "to": 0, "fromItem": null, "toItem": 0, "ids": [id.0]})).unwrap_err().to_string();
    assert!(e.contains("not to its fill or stroke"), "{e}");
}

/// A plug-in's records don't put a 3D effect on a symbol instance: it is built from the symbol's art,
/// which they don't come with.
#[test]
fn a_plugins_3d_effect_on_an_instance_is_refused_for_what_it_is() {
    let mut s = Session::new();
    let id = instance(&mut s);
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let fx = vectorcraft_doc::Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0}), visible: true };
    let e = crate::cmd::plugin::admit_effects_for("plugin.run", &[fx], &n).unwrap_err();
    assert!(e.contains("built from its symbol's art"), "{e}");
}

/// The instance `id` of `s`, in perspective with a 3D effect, draws flat however they came
/// together: on the canvas as without the effect, and exported and expanded as drawn.
fn draws_flat_in_perspective(s: &mut Session, id: NodeId, how: &str) {
    let doc = &s.doc().unwrap().doc;
    let n = doc.node(id).unwrap();
    assert!(n.projection().is_some() && vectorcraft_render::effects::has_solid(n), "{how}: in perspective with a 3D effect");
    let mut flat = (**doc).clone();
    let m = flat.node_mut(id).unwrap();
    m.appearance.effects.retain(|e| !vectorcraft_render::effects::is_three_d(&e.id));
    for i in 0..m.appearance.items.len() {
        m.appearance.effects_mut(Some(i)).unwrap().retain(|e| !vectorcraft_render::effects::is_three_d(&e.id));
    }
    let drawn = render_doc(doc);
    assert!(differ(&drawn, &render_doc(&flat)) == 0, "{how}: drawn flat");
    run(s, "select.set", json!({"ids": [id.0]}));
    let (drawn, baked, expanded) = drawn_baked_expanded(s);
    assert!(differ(&drawn, &baked) < drawn.len() / 400, "{how}: exported {}", differ(&drawn, &baked));
    assert!(differ(&drawn, &expanded) < drawn.len() / 400, "{how}: expanded {}", differ(&drawn, &expanded));
}

/// An instance and a 3D effect meet in perspective by ways no command refuses: its group attached,
/// a hidden effect shown, the Eyedropper copying an extruded path's appearance onto it, a graphic
/// style with one applied to it.
#[test]
fn an_instance_in_perspective_draws_its_3d_effect_flat() {
    let extrude = json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}});
    // Its group attached.
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "effect.apply", extrude.clone());
    let other = run(&mut s, "path.create", json!({"d": "M300 300L350 300L350 350L300 350Z"}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [id.0, other]}));
    let g = run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap();
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    run(&mut s, "perspective.attach", json!({"ids": [g], "plane": "right"}));
    draws_flat_in_perspective(&mut s, id, "group attached");
    // A hidden effect shown.
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "effect.apply", extrude.clone());
    run(&mut s, "effect.setParams", json!({"index": 0, "item": null, "visible": false}));
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    run(&mut s, "perspective.attach", json!({"ids": [id.0], "plane": "right"}));
    run(&mut s, "appearance.showAllHidden", json!({"ids": [id.0]}));
    draws_flat_in_perspective(&mut s, id, "shown");
    // The Eyedropper.
    let mut s = Session::new();
    let id = instance(&mut s);
    let src = run(&mut s, "path.create", json!({"d": "M300 300L350 300L350 350L300 350Z"}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [src]}));
    run(&mut s, "effect.apply", extrude);
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    run(&mut s, "perspective.attach", json!({"ids": [id.0], "plane": "right"}));
    run(&mut s, "appearance.copyFrom", json!({"source": src, "ids": [id.0]}));
    draws_flat_in_perspective(&mut s, id, "eyedropper");
    // A graphic style.
    let mut s = Session::new();
    let id = instance(&mut s);
    let src = run(&mut s, "path.create", json!({"d": "M300 300L350 300L350 350L300 350Z"}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [src]}));
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}}));
    let style = run(&mut s, "graphicStyle.new", json!({"name": "Deep"}))["name"].as_str().unwrap().to_string();
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    run(&mut s, "perspective.attach", json!({"ids": [id.0], "plane": "right"}));
    run(&mut s, "graphicStyle.apply", json!({"name": style, "ids": [id.0]}));
    draws_flat_in_perspective(&mut s, id, "graphic style");
}

/// An instance's 3D effect on its fill, with Crop Marks, expands without it, as drawn.
#[test]
fn an_instances_fill_3d_with_crop_marks_expands_without_it() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "effect.apply", json!({"effect": "cropMarks", "item": null, "params": {}}));
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let fx = vectorcraft_doc::Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0}), visible: true };
    d.node_mut(id).unwrap().appearance.effects_mut(Some(0)).unwrap().push(fx);
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let (drawn, baked, expanded) = drawn_baked_expanded(&mut s);
    let mut three_d = 0;
    for layer in &s.doc().unwrap().doc.layers {
        layer.walk(&mut |n| {
            let items = n.appearance.items.iter().flat_map(|i| i.effects());
            three_d += n.appearance.effects.iter().chain(items).filter(|e| vectorcraft_render::effects::is_three_d(&e.id)).count();
        });
    }
    assert_eq!(three_d, 0, "no 3D effect left");
    assert!(differ(&drawn, &baked) < drawn.len() / 400, "exported {}", differ(&drawn, &baked));
    assert!(differ(&drawn, &expanded) < drawn.len() / 400, "expanded {}", differ(&drawn, &expanded));
}

/// Type whose fill revolves (as a file may hold it: applying Revolve to type is refused), with a
/// Transform effect of its own, is reshaped into pieces that each revolve: exported and expanded
/// as the canvas draws it, not flat. A regression guard for how the art drawn keeps its fills' 3D
/// effects (`keeps_item_three_d`): it passes before this change too.
#[test]
fn reshaped_type_with_a_revolving_fill_exports_and_expands_as_drawn() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 400}));
    let t = run(&mut s, "text.create", json!({"x": 100, "y": 200, "text": "Kw", "size": 60, "color": "#3366cc"}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [t]}));
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "paint.setFill", json!({"color": "#3366cc"}));
    run(&mut s, "effect.apply", json!({"effect": "distort.transform", "item": null, "params": {"moveH": 20}}));
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let fx = vectorcraft_doc::Effect { id: "threeD.revolve".into(), params: json!({}), visible: true };
    d.node_mut(NodeId(t)).unwrap().appearance.effects_mut(Some(0)).unwrap().push(fx);
    let (drawn, baked, expanded) = drawn_baked_expanded(&mut s);
    assert!(differ(&drawn, &baked) < drawn.len() / 400, "exported {}", differ(&drawn, &baked));
    assert!(differ(&drawn, &expanded) < drawn.len() / 400, "expanded {}", differ(&drawn, &expanded));
}

/// Expand Appearance of an adjusted instance whose 3D effect can't build answers its id once.
#[test]
fn expanding_an_adjusted_flat_instance_answers_its_id_once() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 30.0}}));
    run(&mut s, "effect.apply", json!({"effect": "adjust.hueSaturation", "item": null, "params": {"hue": 90}}));
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let held = vectorcraft_doc::Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Other".into(), xf: vectorcraft_geom::Affine::IDENTITY });
    let art = d.symbols.iter().find(|m| m.name == "Mark").unwrap().art.clone();
    let mixed = vectorcraft_doc::Node::group(d.alloc_id(), vec![art, std::sync::Arc::new(held)]);
    d.symbols.iter_mut().find(|m| m.name == "Mark").unwrap().art = std::sync::Arc::new(mixed);
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let ids = run(&mut s, "effect.expandAppearance", json!({}))["ids"].clone();
    let ids: Vec<u64> = ids.as_array().unwrap().iter().filter_map(|v| v.as_u64()).collect();
    let mut once = ids.clone();
    once.dedup();
    assert_eq!(ids, once, "each id once");
}

/// A hidden 3D effect on an instance's fill draws nothing either way: Expand Appearance isn't offered
/// for it. A regression guard for which effects make an instance expandable: it passes before this
/// change too.
#[test]
fn a_hidden_3d_effect_on_an_instances_fill_offers_no_expand() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let fx = vectorcraft_doc::Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0}), visible: false };
    d.node_mut(id).unwrap().appearance.effects_mut(Some(0)).unwrap().push(fx);
    assert!(!crate::cmd::effectcmd::expandable(s.doc().unwrap().doc.node(id).unwrap()));
}

/// Moving Revolve onto an instance's fill is refused, as applying it there is.
#[test]
fn revolve_isnt_moved_onto_an_instances_fill() {
    let mut s = Session::new();
    let id = instance(&mut s);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let fx = vectorcraft_doc::Effect { id: "threeD.revolve".into(), params: json!({}), visible: true };
    d.node_mut(id).unwrap().appearance.effects.push(fx);
    let e = s.execute("effect.move", &json!({"from": 0, "to": 0, "fromItem": null, "toItem": 0, "ids": [id.0]})).unwrap_err().to_string();
    assert!(e.contains("Revolve"), "{e}");
}
