//! Opacity-mask editing mode (click the mask thumbnail in the Transparency panel).
//!
//! The mask art moves onto a temporary isolated layer where every tool and command can edit it.
//! After each edit [`sync`] writes the layer's art back into the object's mask, so the masked
//! object updates live; the renderer doesn't paint the editing layer (the mask is seen through its
//! effect, as in the reference app). Leaving the mode removes the layer, and saving or exporting
//! never writes it ([`Document::without_edit_modes`]). Meanwhile the Transparency panel's commands
//! act on the masked object (see `opacitymask::transparency_targets`). Mask editing is not entered
//! in isolation mode or pattern editing mode ([`can_edit_mask`]), and neither mode is entered while
//! a mask is edited ([`outside_mask_editing`]).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, MaskEdit, Node, NodeId, NodeKind, Selection};

use super::{CommandSpec, EngineError, Result, Session, bad, cmd, has_doc, object_param, switch_or};

/// Name of the temporary mask-editing layer.
pub const MASK_EDIT_LAYER: &str = "Opacity Mask Editing";

/// Name of the group that holds several mask objects (or none) as one mask art node.
const MASK_GROUP: &str = "Opacity Mask";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "transparency.editOpacityMask",
            "Edit Opacity Mask",
            ["Window", "Transparency"],
            None,
            "{id?} edit the mask art of `id` (default: the first selected object with a mask) in place, as clicking the mask thumbnail in the Transparency panel does; refused in isolation mode and in pattern editing mode → {layer}",
            can_enter,
            enter
        ),
        cmd!(
            "transparency.stopEditingOpacityMask",
            "Stop Editing Opacity Mask",
            ["Window", "Transparency"],
            None,
            "{} leave mask editing (the object thumbnail) → {id}",
            has_doc,
            leave
        ),
        cmd!(
            "transparency.viewOpacityMask",
            "View Opacity Mask",
            ["Window", "Transparency"],
            None,
            "{on?: bool (default: toggle), id?} show only the opacity mask of `id` (default: the object whose mask is edited, else the first selected object with a mask) on the canvas, as greyscale coverage, and edit it, as Alt-clicking the mask thumbnail does; off shows the artwork again (still editing); refused in isolation mode and in pattern editing mode → {on, id}",
            can_edit_mask,
            view_mask
        ),
    ]
}

/// Is the document in isolation mode? Pattern editing mode and opacity mask editing isolate their
/// art too, and do not count as isolation mode here.
pub(crate) fn in_isolation_mode(st: &crate::DocState) -> bool {
    st.isolation.is_some() && st.doc.pattern_edit.is_none() && st.doc.mask_edit.is_none()
}

/// Why opacity mask editing is not entered now, if it is not: in isolation mode
/// ([`in_isolation_mode`]) and in pattern editing mode, whose tile layer the mask editing layer
/// would replace as the isolated layer.
pub(crate) fn mask_editing_refused(st: &crate::DocState) -> Option<&'static str> {
    if st.doc.pattern_edit.is_some() {
        Some("finish editing the pattern first")
    } else if in_isolation_mode(st) {
        Some("exit isolation mode first")
    } else {
        None
    }
}

/// A document is open and opacity mask editing can be entered ([`mask_editing_refused`]).
pub(crate) fn can_edit_mask(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    mask_editing_refused(st).map_or(Ok(()), |why| Err(why.into()))
}

/// Edit Opacity Mask: mask editing can be entered ([`can_edit_mask`]), and outside it no object of a
/// dynamic symbol instance's art is selected, since its instance's mask is not edited from it.
fn can_enter(s: &Session) -> std::result::Result<(), String> {
    can_edit_mask(s)?;
    if s.active().is_some_and(|st| st.doc.mask_edit.is_some()) { Ok(()) } else { super::edit::no_symbol_part(s) }
}

/// A document is open and no opacity mask is being edited: isolation mode and pattern editing mode
/// are not entered while an opacity mask is edited.
pub(crate) fn outside_mask_editing(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.doc.mask_edit.is_some() { Err("stop editing the opacity mask first".into()) } else { Ok(()) }
}

/// Mask art made of `children`: a plain group, so the mask keeps one art node. Editing spreads it
/// back out onto the editing layer.
pub(crate) fn mask_group(children: Vec<Arc<Node>>) -> Node {
    let mut g = Node::group(NodeId(u64::MAX - 1), children);
    g.name = Some(MASK_GROUP.into());
    g
}

/// A group [`mask_group`] made (named so, with nothing of its own that its members would lose).
fn is_mask_group(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. })
        && n.name.as_deref() == Some(MASK_GROUP)
        && n.has_default_transparency()
        && n.appearance.items.is_empty()
        && n.appearance.effects.is_empty()
}

/// The objects mask art is made of: the members of a [`mask_group`], else the art itself.
pub(crate) fn mask_parts(art: &Arc<Node>) -> Vec<Arc<Node>> {
    match art.children() {
        Some(ch) if is_mask_group(art) => ch.clone(),
        _ => vec![art.clone()],
    }
}

fn enter(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "transparency.editOpacityMask";
    let st = s.doc()?;
    if st.doc.mask_edit.is_some() {
        return Err(bad(C, "already editing an opacity mask"));
    }
    let given = object_param(p, "id", C)?;
    if given.is_none() {
        super::edit::refuse_symbol_part(s)?;
    }
    let id = given.or_else(|| first_masked(st)).ok_or_else(|| bad(C, "select an object with an opacity mask"))?;
    let layer = start(s, id)?;
    Ok(json!({ "layer": layer.0 }))
}

/// The first selected object with an opacity mask. Callers refuse first while an object of a
/// dynamic symbol instance's art is selected ([`super::edit::refuse_symbol_part`]).
fn first_masked(st: &crate::DocState) -> Option<NodeId> {
    st.selection.subjects().iter().copied().find(|i| st.doc.node(*i).is_some_and(|n| n.extras().mask.is_some()))
}

/// Enter mask editing for `id` (one undo step) and isolate the editing layer. Returns the layer.
/// A saved document stays saved: the editing layer is never saved, and the mask keeps its art
/// until an edit on the layer changes it ([`sync`]).
fn start(s: &mut Session, id: NodeId) -> Result<NodeId> {
    super::symboledit::keep_typing(s)?;
    let back = super::symboledit::Return::of(s.doc()?);
    let layer = s.edit("Edit Opacity Mask", |d, sel| begin(d, sel, id))?;
    show_editing(s, Some(layer))?;
    let st = s.doc_mut()?;
    if let Some(before) = back.before() {
        st.saved_content_kept(before);
    }
    st.mask_edit_return = back.after_entering(st);
    Ok(layer)
}

fn view_mask(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "transparency.viewOpacityMask";
    let given = object_param(p, "id", C)?;
    let st = s.doc()?;
    let editing = st.doc.mask_edit.map(|m| m.object);
    if !switch_or(p, "on", st.shown_mask().is_none(), C)? {
        let st = s.doc_mut()?;
        let id = st.mask_view.take();
        st.revision += 1;
        return Ok(json!({ "on": false, "id": id.map(|i| i.0) }));
    }
    if given.is_none() && editing.is_none() {
        super::edit::refuse_symbol_part(s)?;
    }
    let id = given.or(editing).or_else(|| first_masked(st)).ok_or_else(|| bad(C, "select an object with an opacity mask"))?;
    match editing {
        Some(e) if e != id => return Err(bad(C, "already editing another object's opacity mask")),
        Some(_) => {}
        None => {
            start(s, id)?;
        }
    }
    let st = s.doc_mut()?;
    st.mask_view = Some(id);
    st.revision += 1;
    Ok(json!({ "on": true, "id": id.0 }))
}

/// Enter mask editing for `id` inside an edit: copy its mask art onto a new editing layer and
/// select it. Returns the layer.
pub(crate) fn begin(d: &mut Document, sel: &mut Selection, id: NodeId) -> Result<NodeId> {
    let art = d
        .node(id)
        .and_then(|n| n.extras().mask.as_ref())
        .map(|m| m.art.clone())
        .ok_or_else(|| EngineError::Other("the object has no opacity mask".into()))?;
    let layer = d.add_layer(Some(MASK_EDIT_LAYER));
    let parts = mask_parts(&art);
    let mut ids = Vec::with_capacity(parts.len());
    for part in &parts {
        // Fresh ids: copies of a masked object share the ids inside their mask art.
        let copy = d.reid(part);
        ids.push(d.insert(Some(layer), usize::MAX, copy)?);
    }
    // Copied type whose layout caches `reid` dropped has them again.
    super::typecmd::refresh_bounds_of(d, &ids);
    d.mask_edit = Some(MaskEdit { object: id, layer });
    sel.set(ids);
    Ok(layer)
}

/// Leave mask editing (`transparency.stopEditingOpacityMask`). `object.exitIsolation` runs it too
/// while a mask is edited. When nothing was edited since entering, the entering step is taken
/// back, as leaving symbol editing mode takes its step back ([`super::symboledit::leave`]): the
/// document, its undo and redo steps and whether it is saved are as before entering. Otherwise
/// leaving is one undo step, which changes nothing that is saved.
pub(crate) fn leave(s: &mut Session, _: &Value) -> Result<Value> {
    let Some(me) = s.doc()?.doc.mask_edit else { return Err(bad("transparency.stopEditingOpacityMask", "not editing an opacity mask")) };
    super::symboledit::keep_typing(s)?;
    s.end_text_edit();
    let st = s.doc_mut()?;
    let back = std::mem::take(&mut st.mask_edit_return);
    // The mask's art as entering found it: no edit wrote to it since ([`sync`]).
    let art = |d: &Document| d.node(me.object).and_then(|n| n.extras().mask.as_ref()).map(|m| m.art.clone());
    let unchanged = back.before().is_some_and(|b| matches!((art(b), art(&st.doc)), (Some(a), Some(b)) if Arc::ptr_eq(&a, &b)));
    // Saved while editing: taking the entering step back would undo what the save changed. The
    // saved document holds this editing layer itself, not one that took the same id after Undo.
    let saved_here = st.saved_doc.mask_edit.is_some_and(|e| e.layer == me.layer)
        && matches!((st.saved_doc.node(me.layer), st.doc.node(me.layer)), (Some(a), Some(b)) if std::ptr::eq(a, b));
    let entered = back.entered().cloned();
    if !(unchanged && !saved_here && back.take_back(st)) {
        let before = st.doc.clone();
        s.edit("Stop Editing Opacity Mask", |d, sel| {
            finish(d, sel, entered.as_deref());
            Ok(())
        })?;
        s.doc_mut()?.saved_content_kept(&before);
    }
    show_editing(s, None)?;
    Ok(json!({ "id": me.object.0 }))
}

/// Leave mask editing inside an edit (no-op when not editing): drop the editing layer and select
/// the masked object. Every edit wrote the layer's art into the mask ([`sync`]); art changed on the
/// layer since (an add-on shape's own data, written after its edit) is written too, unless the
/// layer is as entering (`entered`, the document entering made) left it.
pub(crate) fn finish(d: &mut Document, sel: &mut Selection, entered: Option<&Document>) {
    let Some(me) = d.mask_edit else { return };
    if let Some(art) = layer_art(d, me.layer)
        && !entered.filter(|e| e.mask_edit == Some(me)).and_then(|e| layer_art(e, me.layer)).is_some_and(|old| same_art(&old, &art))
    {
        write_art(d, me.object, art);
    }
    d.drop_edit_modes();
    if d.node(me.object).is_some() {
        sel.set_target(d, me.object);
    } else {
        sel.clear();
    }
}

/// Isolate the editing layer and draw into it (`Some`), or return to the document (`None`, which
/// also ends View Opacity Mask).
pub(crate) fn show_editing(s: &mut Session, layer: Option<NodeId>) -> Result<()> {
    let st = s.doc_mut()?;
    st.isolation = layer;
    st.active_layer = layer.or_else(|| st.doc.default_layer());
    if layer.is_none() {
        st.mask_view = None;
    }
    st.revision += 1;
    Ok(())
}

/// After undo or redo, isolation follows the restored document into or out of mask editing
/// (`was` is the editing layer before, if any), so it never names a layer that is gone (whose id
/// a later object could reuse). Out of mask editing, View Opacity Mask ends.
pub(crate) fn follow_history(st: &mut crate::DocState, was: Option<NodeId>) {
    match st.doc.mask_edit {
        Some(me) => {
            st.isolation = Some(me.layer);
            st.active_layer = Some(me.layer);
        }
        None => {
            st.mask_view = None;
            if was.is_some() && st.isolation == was {
                st.isolation = None;
                st.active_layer = st.doc.default_layer();
            }
        }
    }
}

/// Write the editing layer's art into the object's mask after an edit that changed it (`before`
/// is the document before the edit). Entering, and edits that left the layer's art alone, write
/// nothing, so the mask keeps its art as it is (its objects' ids and links) until it is edited.
/// Leaves the mode when the object or the layer is gone (e.g. deleted).
pub(crate) fn sync(before: &Document, d: &mut Document) {
    let Some(me) = d.mask_edit else { return };
    let (Some(art), true) = (layer_art(d, me.layer), d.node(me.object).is_some_and(|n| n.extras().mask.is_some())) else {
        d.mask_edit = None;
        let _ = d.remove_edit_layer(me.layer);
        return;
    };
    if before.mask_edit.is_none_or(|b| b.layer != me.layer) || layer_art(before, me.layer).is_some_and(|old| same_art(&old, &art)) {
        return;
    }
    write_art(d, me.object, art);
}

/// The art on mask editing layer `layer`: ruler guides on it are not part of the mask.
fn layer_art(d: &Document, layer: NodeId) -> Option<Vec<Arc<Node>>> {
    d.node(layer).and_then(|l| l.children()).map(|ch| ch.iter().filter(|c| !c.is_ruler_guide()).cloned().collect())
}

/// Whether `a` and `b` are the same objects.
fn same_art(a: &[Arc<Node>], b: &[Arc<Node>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| Arc::ptr_eq(a, b))
}

/// Make `art` (the editing layer's) the mask art of `object`, unless it already is.
fn write_art(d: &mut Document, object: NodeId, art: Vec<Arc<Node>>) {
    let Some(m) = d.node(object).and_then(|n| n.extras().mask.as_ref()) else { return };
    let held = match art.as_slice() {
        [one] => Arc::ptr_eq(&m.art, one),
        many => is_mask_group(&m.art) && m.art.children().is_some_and(|ch| same_art(ch, many)),
    };
    if held {
        return;
    }
    let new_art = match art.as_slice() {
        [one] => one.clone(),
        // Several objects (or none): a group, so the mask keeps one art node.
        _ => Arc::new(mask_group(art)),
    };
    if let Some(m) = d.node_mut(object).and_then(|n| n.extras_if_any().and_then(|e| e.mask.as_mut())) {
        m.art = new_art;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use crate::{NodeId, Session};

    /// A saved document whose object has an opacity mask of its own art (a white rectangle over
    /// the masked one), and the masked object's id.
    fn saved_with_a_mask() -> (Session, u64) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
        let obj = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap()["id"].as_u64().unwrap();
        let m = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 200})).unwrap()["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({"color": "#ffffff", "ids": [m]})).unwrap();
        s.execute("select.set", &json!({"ids": [obj, m]})).unwrap();
        s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        assert!(s.doc().unwrap().doc.mask_edit.is_none());
        s.doc_mut().unwrap().mark_saved();
        (s, obj)
    }

    fn dirty(s: &Session) -> bool {
        s.doc().unwrap().is_dirty()
    }

    fn mask_art(s: &Session, obj: u64) -> Arc<vectorcraft_doc::Node> {
        s.doc().unwrap().doc.node(NodeId(obj)).unwrap().extras().mask.as_ref().unwrap().art.clone()
    }

    /// Entering and leaving mask editing changes nothing that is saved: a saved document stays
    /// saved, the mask keeps its art as it was (no copy of the editing layer's is written back),
    /// and leaving without an edit takes the entering step back. Art drawn meanwhile makes it
    /// modified, also after.
    #[test]
    fn a_saved_document_stays_saved_while_a_mask_is_only_looked_at() {
        let (mut s, obj) = saved_with_a_mask();
        let art = mask_art(&s, obj);
        let steps = s.doc().unwrap().history.undo.len();
        s.execute("transparency.editOpacityMask", &json!({"id": obj})).unwrap();
        assert!(!dirty(&s), "entering mask editing leaves a saved document saved");
        assert!(Arc::ptr_eq(&art, &mask_art(&s, obj)), "entering leaves the mask's art as it is");
        s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        assert!(!dirty(&s), "and so does leaving it");
        assert!(Arc::ptr_eq(&art, &mask_art(&s, obj)), "and leaving too");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps, "entering and leaving without an edit leave no step");
        s.execute("transparency.editOpacityMask", &json!({"id": obj})).unwrap();
        s.execute("shape.ellipse", &json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
        assert!(dirty(&s), "art drawn in the mask is an edit to save");
        assert!(!Arc::ptr_eq(&art, &mask_art(&s, obj)), "and the mask takes it");
        s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        assert!(dirty(&s), "and leaving keeps it modified");
        // Saved while editing, leaving keeps it saved.
        s.execute("transparency.editOpacityMask", &json!({"id": obj})).unwrap();
        s.doc_mut().unwrap().mark_saved();
        s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        assert!(!dirty(&s), "saved while editing");
    }

    /// Undo after entering and leaving takes back the edit before them, and the redo steps
    /// entering cleared come back on leaving.
    #[test]
    fn entering_and_leaving_keep_undo_and_redo() {
        let (mut s, obj) = saved_with_a_mask();
        s.execute("shape.ellipse", &json!({"x": 220, "y": 220, "width": 40, "height": 40})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(!dirty(&s));
        let redo = s.doc().unwrap().history.redo.len();
        assert_eq!(redo, 1);
        s.execute("transparency.editOpacityMask", &json!({"id": obj})).unwrap();
        s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().history.redo.len(), redo, "leaving gives the redo step back");
        assert!(!dirty(&s));
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(dirty(&s), "the ellipse is back");
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(!dirty(&s), "and gone again: the saved document");
        let r = s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(r["undone"], "Make Opacity Mask", "{r}");
        assert!(s.doc().unwrap().doc.mask_edit.is_none());
    }

    /// Typing in progress when mask editing is entered or left is kept as an edit to save, not
    /// taken for part of entering or leaving.
    #[test]
    fn typing_in_progress_is_an_edit_to_save() {
        for leaving in [false, true] {
            let (mut s, obj) = saved_with_a_mask();
            if leaving {
                s.execute("transparency.editOpacityMask", &json!({"id": obj})).unwrap();
                assert!(!dirty(&s));
            }
            s.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": false})).unwrap();
            s.doc_mut().unwrap().mark_saved();
            s.begin_interaction("Typing").unwrap();
            let typed = json!({"x": 220, "y": 220, "width": 40, "height": 40});
            s.doc_mut().unwrap().interaction.as_mut().unwrap().preview = Some(("shape.rectangle".into(), typed));
            let method = if leaving { "transparency.stopEditingOpacityMask" } else { "transparency.editOpacityMask" };
            let params = if leaving { json!({}) } else { json!({"id": obj}) };
            s.execute(method, &params).unwrap();
            assert!(dirty(&s), "leaving {leaving}: the typing is kept as an edit");
        }
    }

    /// An edit made in an undo group while editing a mask is part of the group's one step, not a
    /// step of its own: leaving keeps it, though the entering step is still the last one.
    #[test]
    fn leaving_keeps_an_edit_made_in_an_undo_group() {
        let (mut s, obj) = saved_with_a_mask();
        s.execute("edit.undoGroup.begin", &json!({})).unwrap();
        s.execute("transparency.editOpacityMask", &json!({"id": obj})).unwrap();
        s.execute("transparency.set", &json!({"ids": [obj], "opacity": 40})).unwrap();
        s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        s.execute("edit.undoGroup.end", &json!({})).unwrap();
        let opacity = s.doc().unwrap().doc.node(NodeId(obj)).unwrap().opacity;
        assert!((opacity - 0.4).abs() < 1e-6, "the opacity set while editing is kept: {opacity}");
        assert!(dirty(&s));
    }

    /// Entering and leaving mask editing in a batch (`command.batch`) keep the batch one undo step.
    #[test]
    fn a_batch_that_edits_a_mask_is_one_step() {
        let (mut s, obj) = saved_with_a_mask();
        let steps = s.doc().unwrap().history.undo.len();
        let commands = json!([
            {"command": "transparency.editOpacityMask", "params": {"id": obj}},
            {"command": "shape.ellipse", "params": {"x": 20, "y": 20, "width": 40, "height": 40}},
            {"command": "transparency.stopEditingOpacityMask", "params": {}},
            {"command": "transparency.set", "params": {"ids": [obj], "opacity": 40}},
        ]);
        s.execute("command.batch", &json!({"commands": commands})).unwrap();
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "one step for the batch");
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(!dirty(&s), "and Undo takes all of it back");
    }

    /// What a command writes on the editing layer after its edit (an add-on shape's own data) is in
    /// the mask once editing ends.
    #[test]
    fn leaving_writes_what_was_written_after_an_edit() {
        let (mut s, obj) = saved_with_a_mask();
        s.execute("transparency.editOpacityMask", &json!({"id": obj})).unwrap();
        let star = s.execute("addon.shapes.star", &json!({"cx": 60, "cy": 60, "radius1": 30, "rounded": 2})).unwrap()["id"].as_u64().unwrap();
        let drawn = s.doc().unwrap().doc.node(NodeId(star)).unwrap().clone();
        s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        let parts = super::mask_parts(&mask_art(&s, obj));
        assert!(parts.iter().any(|p| **p == drawn), "the star as drawn, with its own data, is in the mask");
    }

    /// Entering again after Undo took entering back makes an editing layer with the same id as the
    /// one a save held: leaving it without an edit still takes its entering step back.
    #[test]
    fn a_layer_saved_before_undo_is_not_the_one_left() {
        let (mut s, obj) = saved_with_a_mask();
        s.execute("transparency.editOpacityMask", &json!({"id": obj})).unwrap();
        let first = s.doc().unwrap().doc.mask_edit.unwrap().layer;
        s.doc_mut().unwrap().mark_saved();
        s.execute("edit.undo", &json!({})).unwrap();
        let steps = s.doc().unwrap().history.undo.len();
        s.execute("transparency.editOpacityMask", &json!({"id": obj})).unwrap();
        assert_eq!(s.doc().unwrap().doc.mask_edit.unwrap().layer, first, "the same id again");
        s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().history.undo.len(), steps, "leaving takes entering back");
        assert_eq!(s.doc().unwrap().history.redo.len(), 1, "and gives the redo step back");
    }

    #[test]
    fn edit_mask_art_in_place() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
        let obj = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap()["id"].as_u64().unwrap();
        let m = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 200})).unwrap()["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({"color": "#ffffff", "ids": [m]})).unwrap();
        s.execute("select.set", &json!({"ids": [obj, m]})).unwrap();
        s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        let layer = s.execute("transparency.editOpacityMask", &json!({})).unwrap()["layer"].as_u64().unwrap();
        assert_eq!(s.doc().unwrap().isolation, Some(NodeId(layer)));
        // Edit the mask art with ordinary commands: widen it; the object's mask follows live.
        s.execute("object.move", &json!({"dx": 50, "dy": 0})).unwrap();
        let mask_bounds =
            |s: &Session| s.doc().unwrap().doc.node(NodeId(obj)).unwrap().extras().mask.as_ref().unwrap().art.geometric_bounds().unwrap();
        assert!((mask_bounds(&s).x0 - 50.0).abs() < 1e-6);
        // Drawing while editing adds to the mask.
        s.execute("shape.ellipse", &json!({"x": 180, "y": 150, "width": 40, "height": 40})).unwrap();
        assert!(mask_bounds(&s).x1 > 185.0);
        // Rendered: the object shows through the moved mask; the mask art itself isn't painted
        // (the ellipse pokes out of the object at x = 210).
        let doc = s.doc().unwrap().doc.clone();
        let img = vectorcraft_render::Renderer::new().render(&doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default());
        assert!(img.pixel(100, 100)[3] > 200);
        assert_eq!(img.pixel(25, 100)[3], 0);
        assert_eq!(img.pixel(212, 170)[3], 0);
        s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        let st = s.doc().unwrap();
        assert!(st.doc.mask_edit.is_none() && st.isolation.is_none());
        assert!(!st.doc.layers.iter().any(|l| l.name.as_deref() == Some(super::MASK_EDIT_LAYER)));
        assert_eq!(st.selection.objects, vec![NodeId(obj)]);
        // Two mask objects went back as one group; editing again spreads them out on the layer.
        let layer = s.execute("transparency.editOpacityMask", &json!({"id": obj})).unwrap()["layer"].as_u64().unwrap();
        assert_eq!(s.doc().unwrap().doc.node(NodeId(layer)).unwrap().children().unwrap().len(), 2);
        assert_eq!(s.doc().unwrap().selection.len(), 2);
        s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        // Undo returns to editing mode's last state, then to before.
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.doc().unwrap().doc.mask_edit.is_some());
    }
}
