//! Extrude & Bevel, Inflate and Rotate as live, shaded vector art, shared by the canvas and every
//! exporter. Like Revolve, the source path stays editable and the effect draws its 3D faces.
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData, Rect};
use vectorcraft_three_d::{Bevel, BevelShape, Extrude, Facet, Inflate, View};

#[cfg(test)]
use vectorcraft_doc::color::Color;

use crate::util::{flag, num, text};
use crate::{GeomContext, apply_geometry_with, is_geometry};

pub const EXTRUDE: &str = "threeD.extrude";
pub const INFLATE: &str = "threeD.inflate";
pub const ROTATE: &str = "threeD.rotate";
/// The 3D effects this module draws (Revolve has its own).
pub const SOLIDS: [&str; 3] = [EXTRUDE, INFLATE, ROTATE];

/// Is `id` one of the 3D and Materials effects (Revolve included)?
pub fn is_three_d(id: &str) -> bool {
    id == crate::REVOLVE || SOLIDS.contains(&id)
}

fn solid(fx: &[Effect]) -> Option<(usize, &Effect)> {
    fx.iter().enumerate().find(|(_, e)| e.visible && SOLIDS.contains(&e.id.as_str()))
}

/// Does `n` carry a visible Extrude & Bevel, Inflate or Rotate effect?
pub fn has_solid(n: &Node) -> bool {
    solid(&n.appearance.effects).is_some() || n.appearance.items.iter().any(|i| i.visible() && solid(i.effects()).is_some())
}

/// The Position presets: (id, [rotation X, Y, Z]). Front, Back, Left, Right, Top and Bottom face
/// the art that way; Off-Axis tilts each 20° down and 25° aside (showing the top and right);
/// Isometric lays the art on the left, right, top or bottom plane of an isometric view (the
/// visible axes foreshortened equally: atan(1/√2) ≈ 35.26° and 45°). Our own values, from the
/// geometry of each view.
pub const ROTATION_PRESETS: [(&str, [f64; 3]); 16] = [
    ("front", [0.0, 0.0, 0.0]),
    ("back", [0.0, 180.0, 0.0]),
    ("left", [0.0, 90.0, 0.0]),
    ("right", [0.0, -90.0, 0.0]),
    ("top", [-90.0, 0.0, 0.0]),
    ("bottom", [90.0, 0.0, 0.0]),
    ("offAxisFront", [-21.88, -23.4, 9.06]),
    ("offAxisBack", [-158.12, 23.4, -170.94]),
    ("offAxisLeft", [-40.74, 58.39, -36.26]),
    ("offAxisRight", [-139.26, -58.39, 143.74]),
    ("offAxisTop", [-111.88, -23.4, 9.06]),
    ("offAxisBottom", [68.12, -23.4, 9.06]),
    ("isometricLeft", [-45.0, -35.26, 30.0]),
    ("isometricRight", [-45.0, 35.26, -30.0]),
    ("isometricTop", [45.0, -35.26, 30.0]),
    ("isometricBottom", [-45.0, -35.26, -30.0]),
];

/// The view parameters every 3D effect shares, named as Revolve's.
pub fn view_options(p: &Value) -> View {
    View {
        rotation_x: num(p, "rotationX", ROTATION_PRESETS[6].1[0]),
        rotation_y: num(p, "rotationY", ROTATION_PRESETS[6].1[1]),
        rotation_z: num(p, "rotationZ", ROTATION_PRESETS[6].1[2]),
        perspective: num(p, "perspectiveAngle", 0.0),
        light_rotation: num(p, "lightRotation", vectorcraft_three_d::DEFAULT_LIGHT.0),
        light_height: num(p, "lightHeight", vectorcraft_three_d::DEFAULT_LIGHT.1),
        light_behind: flag(p, "lightBehind", false),
        light_color: crate::lighting::light_color(p),
        lights: crate::lighting::lights(p),
        light_intensity: num(p, "lightIntensity", 80.0) / 100.0,
        light_softness: num(p, "lightSoftness", 0.0) / 100.0,
        ambient: num(p, "ambient", 25.0) / 100.0,
        shade: flag(p, "shade", true),
    }
}

/// Extrude & Bevel's parameters. `bevel` is `"none"` or a [`BevelShape`] id.
pub fn extrude_options(p: &Value) -> Extrude {
    Extrude {
        depth: num(p, "depth", 50.0),
        twist: num(p, "twist", 0.0),
        taper: num(p, "taper", 100.0) / 100.0,
        cap: flag(p, "cap", true),
        bevel: BevelShape::parse(text(p, "bevel", "none")).map(|shape| Bevel {
            shape,
            width: num(p, "bevelWidth", 4.0),
            height: num(p, "bevelHeight", 4.0),
            repeat: num(p, "bevelRepeat", 1.0).clamp(1.0, 32.0) as usize,
            space: num(p, "bevelSpace", 0.0) / 100.0,
            inside: flag(p, "bevelInside", false),
            both_sides: flag(p, "bevelBothSides", false),
        }),
    }
}

pub fn inflate_options(p: &Value) -> Inflate {
    Inflate {
        depth: num(p, "depth", 0.0),
        volume: num(p, "volume", 50.0),
        both_sides: flag(p, "bothSides", false),
        twist: num(p, "twist", 0.0),
        taper: num(p, "taper", 100.0) / 100.0,
        cap: flag(p, "cap", true),
    }
}

/// The shape a solid is made from: a path's or a compound path's outline.
fn profile(n: &Node) -> Option<(PathData, FillRule)> {
    match &n.kind {
        NodeKind::Path { path, rule, guide: false, clipping: false, .. } => Some((path.clone(), *rule)),
        NodeKind::Compound { children, rule } => {
            Some((PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect()), *rule))
        }
        _ => None,
    }
}

/// One shape of a solid: an outline and the fills and strokes that paint it, bottom first.
struct Shape {
    path: PathData,
    rule: FillRule,
    items: Vec<AppearanceItem>,
}

/// Can a 3D effect on `n` itself be built from its art ([`shapes`]): type, from its glyph
/// outlines, and a symbol instance, from its symbol's art?
fn through_art(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Text(_) | NodeKind::SymbolInstance { .. })
}

/// The shapes a 3D effect on `n` itself is made from, after `before` (the geometry effects ahead of
/// it): a path or compound path as one shape painted by its own fills and strokes; type and a
/// symbol instance through their art ([`crate::outline_art`]: type's glyph outlines, each run in its
/// own paint, and the type's own fills and strokes; the symbol's art, `symbol_art`, placed by the
/// instance), reshaped as one piece by `before`, each visible path or compound path in it a shape
/// painted as it is, after its own geometry effects. Clipping paths and guides are left out, and so
/// are the images and symbol instances inside the art (they have no outline to build from). `None`
/// for other objects, or when nothing in the art paints.
fn shapes(doc: Option<&Document>, n: &Node, symbol_art: Option<&Node>, before: &[Effect]) -> Option<Vec<Shape>> {
    let ctx = GeomContext::of(n);
    if !through_art(n) {
        let (path, rule) = profile(n)?;
        let items = n.appearance.items.iter().filter(|i| crate::paints(i)).cloned().collect();
        return Some(vec![Shape { path: geometry(&path, before, &ctx), rule, items }]);
    }
    let mut art = crate::outline_art(doc, n, symbol_art)?;
    let fx: Vec<&Effect> = before.iter().filter(|e| e.visible && is_geometry(&e.id)).collect();
    crate::reshape::reshape_leaves(&mut art, &fx);
    let mut out = vec![];
    collect_shapes(&art, &mut out);
    (!out.is_empty()).then_some(out)
}

/// The visible paths and compound paths under `n` as [`Shape`]s, in paint order.
fn collect_shapes(n: &Node, out: &mut Vec<Shape>) {
    if !n.visible {
        return;
    }
    match &n.kind {
        NodeKind::Path { guide: false, clipping: false, .. } | NodeKind::Compound { .. } => {
            let items: Vec<AppearanceItem> = n.appearance.items.iter().filter(|i| crate::paints(i)).cloned().collect();
            if let Some((path, rule)) = profile(n)
                && !items.is_empty()
            {
                out.push(Shape { path: geometry(&path, &n.appearance.effects, &GeomContext::of(n)), rule, items });
            }
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => children.iter().for_each(|c| collect_shapes(c, out)),
        _ => {}
    }
}

fn facets(id: &str, path: &PathData, rule: FillRule, bounds: Rect, p: &Value) -> Result<Vec<Facet>, String> {
    let view = view_options(p);
    match id {
        EXTRUDE => vectorcraft_three_d::extrude(path, rule, bounds, extrude_options(p), view),
        INFLATE => vectorcraft_three_d::inflate(path, rule, bounds, inflate_options(p), view),
        _ => vectorcraft_three_d::plane(path, rule, bounds, view),
    }
}

/// Validate before a command changes the document; the evaluator bounds loaded files too.
/// A symbol instance is checked against its symbol's art in `doc`.
pub fn validate_solid(doc: Option<&Document>, n: &Node, id: &str, p: &Value) -> Result<(), String> {
    crate::lighting::validate(p)?;
    let label = crate::effect_info(id).map_or("This effect", |e| e.label.trim_end_matches('…'));
    if let Some(v) = p.get("bevel")
        && !v.as_str().is_some_and(|s| s == "none" || BevelShape::parse(s).is_some())
    {
        let names: Vec<_> = BevelShape::ALL.iter().map(|b| b.id()).collect();
        return Err(format!("bevel must be none or one of {}", names.join(", ")));
    }
    let art = symbol_art(doc, n);
    let shapes = shapes(doc, n, art, &[]).ok_or_else(|| {
        format!("{label} currently requires a path, compound path, type or symbol instance. Convert images and groups to paths first.")
    })?;
    let bounds = shapes.iter().filter_map(|s| s.path.bounds()).reduce(|a, b| a.union(b));
    let b = bounds.ok_or_else(|| format!("{label} requires a shape with at least two anchors"))?;
    shapes.iter().try_for_each(|s| facets(id, &s.path, s.rule, b, p).map(|_| ()))
}

/// The art of the symbol `n` is an instance of, in `doc`.
fn symbol_art<'a>(doc: Option<&'a Document>, n: &Node) -> Option<&'a Node> {
    let NodeKind::SymbolInstance { symbol, .. } = &n.kind else { return None };
    doc?.symbols.iter().find(|s| s.name == *symbol).map(|s| &*s.art)
}

/// Can 3D effect `id` (Revolve included, [`is_three_d`]) with parameters `p` draw `n`? Checked
/// before a command changes the document: Revolve by [`crate::validate_revolve`], the others by
/// [`validate_solid`].
pub fn validate_three_d(doc: Option<&Document>, n: &Node, id: &str, p: &Value) -> Result<(), String> {
    if id == crate::REVOLVE { crate::validate_revolve(n, p) } else { validate_solid(doc, n, id, p) }
}

/// One surface: each facet a path shaded from `paint`, as a group with the paint's opacity and
/// blend mode (so overlapping facets don't add up their transparency).
fn surface(n: &Node, facets: Vec<Facet>, paint: &Paint, opacity: f32, blend: BlendMode, colored: bool) -> Node {
    if colored && paint.color().is_some() && facets.iter().any(|face| face.rgb_gradient.is_some()) {
        let mut group = crate::lighting::solid_surface(n.id, &facets, paint);
        group.opacity = opacity;
        group.blend = blend;
        return group;
    }
    // Preserve existing white-light paints, alpha composition and seam joins exactly.
    let (opaque, mask) = if colored { crate::lighting::opaque_paint(paint) } else { (paint.clone(), None) };
    let paint = &opaque;
    let children = facets
        .into_iter()
        .map(|face| {
            // Adjacent facets, antialiased on their own, would leave hairline seams: a narrow
            // stroke of the same paint closes them. No wider than the facet is thick, though: seen
            // at a grazing angle a facet can be thinner than the stroke, and its stroke would then
            // paint over its neighbours.
            let seam = (face.thickness() / 2.0).clamp(0.05, 0.75);
            if let Some(g) = face.rgb_gradient
                && (paint.color().is_some() || matches!(paint, Paint::Gradient(_)))
            {
                return Arc::new(crate::lighting::facet_art(n.id, &face.path(), paint, g, seam));
            }
            let items = crate::lighting::fills(paint, face.illumination, face.brightness, colored);
            let items = items
                .into_iter()
                .flat_map(|item| {
                    let AppearanceItem::Fill(fill) = &item else { return vec![item] };
                    let mut edge = StrokeLayer::new(fill.paint.clone(), seam);
                    if colored {
                        edge.join = vectorcraft_doc::LineJoin::Round;
                    }
                    edge.opacity = fill.opacity;
                    vec![item, AppearanceItem::Stroke(edge)]
                })
                .collect();
            Arc::new(Node::path(n.id, face.path(), Appearance { items, ..Default::default() }))
        })
        .collect();
    let mut group = Node::group(n.id, children);
    group.opacity = opacity;
    group.blend = blend;
    if let Some(mask) = mask {
        let bounds = group.visual_bounds().unwrap_or(Rect::ZERO).inflate(1.0, 1.0);
        let coverage = Node::path(n.id, vectorcraft_geom::shapes::rectangle(bounds), Appearance::basic(mask, Paint::None, 0.0));
        group.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(coverage, true)));
    }
    group
}

fn paint_of(item: &AppearanceItem) -> (Paint, f32, BlendMode) {
    match item {
        AppearanceItem::Fill(f) => (f.paint.clone(), f.opacity, f.blend),
        AppearanceItem::Stroke(s) => (s.paint.clone(), s.opacity, s.blend),
    }
}

/// The area `item` paints on `path`, and the rule that fills it: the path itself for a fill, the
/// stroke's outline (whose contours nest as its holes do) for a stroke.
fn region(path: &PathData, rule: FillRule, item: &AppearanceItem) -> (PathData, FillRule) {
    match item {
        AppearanceItem::Stroke(s) if s.width > 0.0 && s.width.is_finite() => (crate::stroke::outline_region(path, rule, s), FillRule::EvenOdd),
        _ => (path.clone(), rule),
    }
}

/// The 3D art of one effect `fx` on `shapes` (already through the effects before it), which turn,
/// twist and taper together about the center of their bounds.
/// Rotate turns every fill and stroke, each in its own paint. Extrude & Bevel and Inflate build a
/// solid from each shape for its top visible fill and one from the outline of each visible stroke,
/// each in its own paint. When every one is opaque with normal blending and no colored light makes
/// a surface one composite or masks it, their faces are drawn in one back-to-front order (where
/// depths tie, as for the caps turned toward the viewer, in the order of the appearance);
/// otherwise each solid is a surface of its own, in that order.
fn art_for(n: &Node, fx: &Effect, shapes: &[Shape]) -> Option<Vec<Arc<Node>>> {
    let b = shapes.iter().filter_map(|s| s.path.bounds()).reduce(|a, c| a.union(c))?;
    let p = crate::merged_params(&fx.id, &fx.params);
    let colored = view_options(&p).has_colored_lights();
    if fx.id == ROTATE {
        // Every part turns about the centre of the object's own bounds, so they stay together.
        let mut out = vec![];
        for shape in shapes {
            for item in &shape.items {
                let (paint, opacity, blend) = paint_of(item);
                let (area, rule) = region(&shape.path, shape.rule, item);
                let f = facets(ROTATE, &area, rule, b, &p).ok()?;
                out.push(Arc::new(surface(n, f, &paint, opacity, blend, colored)));
            }
        }
        return Some(out);
    }
    let mut parts: Vec<(&AppearanceItem, PathData, FillRule)> = vec![];
    for shape in shapes {
        let top_fill = shape.items.iter().rposition(|i| matches!(i, AppearanceItem::Fill(_)));
        parts.extend(shape.items.iter().enumerate().filter(|(k, i)| Some(*k) == top_fill || matches!(i, AppearanceItem::Stroke(_))).map(|(_, i)| {
            let (area, rule) = region(&shape.path, shape.rule, i);
            (i, area, rule)
        }));
    }
    // The solids share the bounds of all their shapes: one center to turn, twist and taper about,
    // and Inflate's grid reaches over the strokes' outlines.
    let bounds = parts.iter().filter_map(|(_, area, _)| area.bounds()).reduce(|a, c| a.union(c)).unwrap_or(b);
    let built = if fx.id == INFLATE {
        // One eye for every puff, as far away as the highest needs.
        let shapes: Vec<(&PathData, FillRule)> = parts.iter().map(|(_, area, rule)| (area, *rule)).collect();
        vectorcraft_three_d::inflate_together(&shapes, bounds, inflate_options(&p), view_options(&p))
    } else {
        parts.iter().map(|(_, area, rule)| facets(&fx.id, area, *rule, bounds, &p)).collect()
    };
    let mut solids = vec![];
    for ((item, _, _), f) in parts.iter().zip(built) {
        let item: &AppearanceItem = item;
        match f {
            Ok(f) => solids.push((item, f)),
            // A stroke's outline past the limits is left out; the fill's solid still draws.
            Err(_) if matches!(item, AppearanceItem::Stroke(_)) => {}
            // A loaded file or a shape edited past the limits keeps drawing its source path
            // instead of failing the frame; commands validate the same evaluator and report its
            // errors.
            Err(_) => return None,
        }
    }
    if solids.is_empty() {
        return None;
    }
    let opaque = solids.iter().all(|(item, _)| {
        let (_, opacity, blend) = paint_of(item);
        opacity >= 1.0 && blend == BlendMode::Normal
    });
    let surfaces: Vec<(Vec<f64>, Node)> = solids
        .into_iter()
        .map(|(item, f)| {
            let depths: Vec<f64> = f.iter().map(|f| f.depth).collect();
            let (paint, opacity, blend) = paint_of(item);
            (depths, surface(n, f, &paint, opacity, blend, colored))
        })
        .collect();
    // One order takes a plain group of faces from each solid, one face for each facet. With
    // colored lights a surface can instead be one clipped composite of all its facets, or carry an
    // opacity mask, and then it stays whole.
    let plain = surfaces.iter().all(|(depths, s)| {
        s.extras().mask.is_none() && matches!(&s.kind, NodeKind::Group { children, clip: false } if children.len() == depths.len())
    });
    if !opaque || !plain {
        return Some(surfaces.into_iter().map(|(_, s)| Arc::new(s)).collect());
    }
    let mut faces = vec![];
    for (depths, s) in surfaces {
        if let NodeKind::Group { children, .. } = s.kind {
            faces.extend(depths.into_iter().zip(children));
        }
    }
    // A stable sort: faces at the same depth keep the order of the appearance.
    faces.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(vec![Arc::new(Node::group(n.id, faces.into_iter().map(|(_, face)| face).collect()))])
}

fn geometry(path: &PathData, fx: &[Effect], ctx: &GeomContext) -> PathData {
    apply_geometry_with(fx, path, path.bounds().unwrap_or(Rect::ZERO), ctx)
}

/// `n` drawn with its Extrude & Bevel, Inflate or Rotate effect: a group of shaded faces keeping
/// the object's id and transparency, the source path untouched. `None` without such an effect.
/// A symbol instance is drawn by [`solid_art_with`] from its symbol's art.
pub fn solid_art(n: &Node) -> Option<Node> {
    solid_art_with(None, n, None)
}

/// [`solid_art`] for any object the effect can be built from ([`shapes`]): a symbol instance from
/// `symbol_art`, the art of its symbol in `doc`. Type and symbol instances take the effect on the
/// object itself, not on one of their fills or strokes.
pub fn solid_art_with(doc: Option<&Document>, n: &Node, symbol_art: Option<&Node>) -> Option<Node> {
    if !has_solid(n) {
        return None;
    }
    let ctx = GeomContext::of(n);
    let painted: Vec<&AppearanceItem> = n.appearance.items.iter().filter(|i| crate::paints(i)).collect();
    let mut children = vec![];
    let remaining: Vec<Effect>;
    if let Some((index, fx)) = solid(&n.appearance.effects) {
        children.extend(art_for(n, fx, &shapes(doc, n, symbol_art, n.appearance.effects.get(..index)?)?)?);
        // Later geometry effects act on the projected art as a whole.
        remaining = n
            .appearance
            .effects
            .iter()
            .enumerate()
            .filter(|(i, e)| *i != index && (!is_geometry(&e.id) || *i > index))
            .map(|(_, e)| e.clone())
            .collect();
    } else {
        let (path, rule) = profile(n)?;
        let path = geometry(&path, &n.appearance.effects, &ctx);
        remaining = n.appearance.effects.iter().filter(|e| !is_geometry(&e.id)).cloned().collect();
        for item in &painted {
            if let Some((index, fx)) = solid(item.effects()) {
                let p = geometry(&path, item.effects().get(..index)?, &ctx.item(item));
                for mut art in art_for(n, fx, &[Shape { path: p, rule, items: vec![(*item).clone()] }])? {
                    Arc::make_mut(&mut art).appearance.effects =
                        item.effects().iter().skip(index + 1).filter(|e| !is_three_d(&e.id)).cloned().collect();
                    children.push(art);
                }
            } else {
                children.push(Arc::new(Node::path(n.id, path.clone(), Appearance { items: vec![(*item).clone()], ..Default::default() })));
            }
        }
    }
    let mut art = n.clone();
    art.kind = NodeKind::Group { children, clip: false };
    art.appearance = Appearance { effects: remaining, ..Default::default() }.into();
    Some(art)
}

#[cfg(test)]
#[path = "tests_solids.rs"]
mod tests;
