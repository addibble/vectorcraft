//! `document.patch`: JSON Patch ([RFC 6902](https://www.rfc-editor.org/rfc/rfc6902)) with JSON
//! Pointer paths ([RFC 6901](https://www.rfc-editor.org/rfc/rfc6901)) over the document model's
//! JSON as a saved file holds it, the way `document.query` names it (each match's `pointer`),
//! each operation applied by `craft_jsonquery::Patch`: in order, all or none. The result is read
//! the way a file's document is (`vectorcraft_format::read_document_json`, `fix_read`), which
//! refuses what the model cannot be, and the command records one undo step and one journal
//! entry, so the patch is undone and replayed like any other edit.
//!
//! [`apply`] keeps what a patch may do within bounds as it goes: its operations ([`MAX_OPS`]),
//! the values the document holds and how deep they nest ([`MAX_VALUES`], [`MAX_NESTING`]), so
//! copies cannot double the tree until memory runs out; and a field left out at its default,
//! which `replace` and `test` take as holding it. (A session confined to read roots, which is
//! shown linked files outside them by file name only, is not offered `document.patch`.) `test`
//! compares numbers by value. It also notes where the patch put values ([`Applied::written`]), and
//! which are new, so a copy, not the object it copies, takes a fresh id
//! (`Document::give_fresh_ids`).

use std::collections::BTreeMap;

use craft_jsonquery::Patch;
use serde_json::{Value, json};

/// Most JSON values (objects, arrays, strings, numbers…) a patched document may hold, as many as
/// a file's reader buffers ([`vectorcraft_doc::node::MAX_BUFFERED_VALUES`]).
pub const MAX_VALUES: usize = vectorcraft_doc::node::MAX_BUFFERED_VALUES;

/// Most levels of JSON a patched document's values nest to. A file's reader stops at 128, and a
/// saved file holds the document inside its own top-level object, one level more.
pub const MAX_NESTING: usize = 126;

/// Most operations one patch applies.
pub const MAX_OPS: usize = 1 << 16;

/// A patch applied ([`apply`]).
pub struct Applied {
    /// The document's saved form, patched.
    pub value: Value,
    /// How many operations the patch has.
    pub ops: usize,
    /// Where in `value` the patch put values, and which of them are new.
    marks: Mark,
}

impl Applied {
    /// Whether the value at `pointer` in the patched document is one the patch added, copied or
    /// replaced, or lies inside one.
    pub fn added(&self, pointer: &str) -> bool {
        self.marks.added(pointer)
    }

    /// The pointers in the patched document of the values the patch wrote (added, copied,
    /// replaced or moved), outermost only.
    pub fn written(&self) -> Vec<String> {
        let mut out = vec![];
        let mut stack = vec![(&self.marks, String::new())];
        while let Some((m, at)) = stack.pop() {
            if m.written {
                out.push(at);
                continue;
            }
            match &m.members {
                Members::None => {}
                Members::Object(o) => stack.extend(o.iter().map(|(k, c)| (c, format!("{at}/{k}")))),
                Members::Array(a) => stack.extend(a.iter().enumerate().map(|(i, c)| (c, format!("{at}/{i}")))),
            }
        }
        out
    }
}

/// The document's forms the patch is applied to: `saved` (its saved form, what is patched) and
/// `full` (every field written, in step with `saved`; `None` when no operation needs a field's
/// default).
pub struct Forms {
    pub saved: Value,
    pub full: Option<Value>,
}

/// Whether any operation of `p`'s patch is a `replace` or a `test`, which may need a field's
/// default ([`Forms::full`]).
pub fn needs_defaults(p: &Value) -> bool {
    p.get("patch")
        .and_then(Value::as_array)
        .is_some_and(|ops| ops.iter().any(|op| matches!(op.get("op").and_then(Value::as_str), Some("replace" | "test"))))
}

/// The patch in `p` (`{patch}`) applied to `forms`, operation by operation; an error names the
/// operation that failed and why, and nothing is kept.
pub fn apply(p: &Value, mut forms: Forms) -> Result<Applied, String> {
    // The whole patch is checked first, so a malformed operation late in it is refused up front.
    let ops = Patch::from_params(p)?.len();
    if ops > MAX_OPS {
        return Err(format!("the patch has {ops} operations; at most {MAX_OPS} are applied at once"));
    }
    let raw = p.get("patch").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut values = count(&forms.saved, usize::MAX);
    if values > MAX_VALUES || nesting(&forms.saved) > MAX_NESTING {
        return Err(format!("the document is too large to patch (more than {MAX_VALUES} values or {MAX_NESTING} levels)"));
    }
    let mut marks = Mark::default();
    for (i, op) in raw.iter().enumerate() {
        let failed = |why: String| format!("operation {i} ({}): {why}", op.get("op").and_then(Value::as_str).unwrap_or("?"));
        let name = op.get("op").and_then(Value::as_str).unwrap_or_default();
        let path = op.get("path").and_then(Value::as_str).unwrap_or_default();
        let from = op.get("from").and_then(Value::as_str).unwrap_or_default();
        // A field left out at its default holds it, for `replace` and `test`.
        let default = match name {
            "replace" | "test"
                if forms.saved.pointer(path).is_none() && parent(path).is_some_and(|q| forms.saved.pointer(q).is_some_and(Value::is_object)) =>
            {
                forms.full.as_ref().and_then(|f| f.pointer(path)).cloned()
            }
            _ => None,
        };
        if name == "test" {
            // Numbers are equal when their values are (RFC 6902 §4.6: 1 and 1.0).
            let held = forms.saved.pointer(path).or(default.as_ref());
            match (held, &default) {
                (Some(h), _) if op.get("value").is_some_and(|v| same(h, v)) => continue,
                (Some(d), Some(_)) => return Err(failed(format!("`{path}` is left out at its default, {d}, not the value tested"))),
                // The patch says why it fails.
                _ => {}
            }
        }
        let saved_op = match (&default, name) {
            (Some(_), "replace") => json!({"op": "add", "path": path, "value": op.get("value").cloned().unwrap_or(Value::Null)}),
            _ => op.clone(),
        };
        // The values and levels the document holds after the operation.
        let source = match name {
            "add" | "replace" => op.get("value"),
            "copy" | "move" => forms.saved.pointer(from),
            _ => None,
        };
        let replaced = match name {
            "replace" | "remove" => forms.saved.pointer(path),
            "add" | "copy" | "move" if parent(path).is_some_and(|q| forms.saved.pointer(q).is_some_and(Value::is_object)) => {
                forms.saved.pointer(path)
            }
            _ => None,
        };
        if let Some(source) = source {
            let depth = path.split('/').count().saturating_sub(1) + nesting(source);
            if depth > MAX_NESTING {
                return Err(failed(format!("the document's values would nest {depth} levels deep, past the {MAX_NESTING} a file holds")));
            }
            if name != "move" {
                let room = MAX_VALUES.saturating_sub(values).saturating_add(replaced.map_or(0, |r| count(r, usize::MAX)));
                let new = count(source, room);
                if new > room {
                    return Err(failed(format!("the document would hold more than {MAX_VALUES} values")));
                }
                values = values + new - replaced.map_or(0, |r| count(r, usize::MAX));
            }
        } else if name == "remove" {
            values -= replaced.map_or(0, |r| count(r, usize::MAX)).min(values);
        }
        marks.before(name, path, from, &forms.saved);
        let one = |op: &Value| Patch::parse(&Value::Array(vec![op.clone()]));
        one(&saved_op)?.apply(&mut forms.saved).map_err(|e| failed(e.replacen("'/0'", &format!("'/{i}'"), 1)))?;
        // The full form follows along; where it cannot (a field it holds that the saved form
        // was given whole), it stops giving defaults.
        if let Some(full) = &mut forms.full
            && one(op)?.apply(full).is_err()
        {
            forms.full = None;
        }
    }
    give_missing_ids(&mut forms.saved);
    Ok(Applied { value: forms.saved, ops, marks })
}

/// Whether `a` and `b` are equal as RFC 6902's `test` compares them: numbers by their values,
/// arrays item by item, objects member by member whatever their order.
fn same(a: &Value, b: &Value) -> bool {
    let mut stack = vec![(a, b)];
    while let Some((a, b)) = stack.pop() {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => {
                let numerically = (x.is_f64() || y.is_f64()) && x.as_f64() == y.as_f64();
                if x != y && !numerically {
                    return false;
                }
            }
            (Value::Array(x), Value::Array(y)) if x.len() == y.len() => stack.extend(x.iter().zip(y)),
            (Value::Object(x), Value::Object(y)) if x.len() == y.len() => {
                for (k, v) in x {
                    let Some(w) = y.get(k) else { return false };
                    stack.push((v, w));
                }
            }
            _ if a == b => {}
            _ => return false,
        }
    }
    true
}

/// The pointer of the value holding `path`'s; `None` for the whole document.
fn parent(path: &str) -> Option<&str> {
    path.rfind('/').map(|i| &path[..i])
}

/// How many JSON values `v` holds, itself included, counted up to just past `cap`.
fn count(v: &Value, cap: usize) -> usize {
    let (mut n, mut stack) = (0usize, vec![v]);
    while let Some(v) = stack.pop() {
        n += 1;
        if n > cap {
            break;
        }
        match v {
            Value::Array(a) => stack.extend(a),
            Value::Object(o) => stack.extend(o.values()),
            _ => {}
        }
    }
    n
}

/// How many levels of arrays and objects `v` nests to (0 for a number, string, boolean or null).
fn nesting(v: &Value) -> usize {
    let (mut deepest, mut stack) = (0, vec![(v, 0)]);
    while let Some((v, level)) = stack.pop() {
        let children: Box<dyn Iterator<Item = &Value>> = match v {
            Value::Array(a) => Box::new(a.iter()),
            Value::Object(o) => Box::new(o.values()),
            _ => continue,
        };
        deepest = deepest.max(level + 1);
        stack.extend(children.map(|c| (c, level + 1)));
    }
    deepest
}

/// A JSON Pointer's reference tokens, unescaped (RFC 6901 §4).
fn unescaped(token: &str) -> std::borrow::Cow<'_, str> {
    if token.contains('~') { token.replace("~1", "/").replace("~0", "~").into() } else { token.into() }
}

/// Where a patch put values, as a tree beside the document's JSON: what the operations wrote
/// (`written`) and which of those values are new (`added`: added, copied or replaced; a moved
/// value keeps what it was). An array's marks move with its items, so each operation costs what
/// it costs the JSON.
#[derive(Default)]
struct Mark {
    added: bool,
    written: bool,
    members: Members,
}

/// A value inside a new one ([`Mark::container`]).
struct Inside;

#[derive(Default)]
enum Members {
    #[default]
    None,
    Object(BTreeMap<String, Mark>),
    Array(Vec<Mark>),
}

impl Mark {
    fn new(added: bool) -> Mark {
        Mark { added, written: true, members: Members::None }
    }

    /// Whether the value at `pointer` is new, or lies inside one.
    fn added(&self, pointer: &str) -> bool {
        let mut m = self;
        for t in pointer.split('/').skip(1) {
            if m.added {
                return true;
            }
            let next = match &m.members {
                Members::Object(o) => o.get(t),
                Members::Array(a) => t.parse::<usize>().ok().and_then(|i| a.get(i)),
                Members::None => None,
            };
            let Some(next) = next else { return false };
            m = next;
        }
        m.added
    }

    /// The member `t` of this mark, whose value in the document is `container`; `create` makes
    /// it (and the array's items before it) when there is none.
    fn member(&mut self, container: &Value, t: &str, create: bool) -> Option<&mut Mark> {
        match container {
            Value::Array(_) => {
                let i = t.parse::<usize>().ok()?;
                if !matches!(self.members, Members::Array(_)) {
                    if !create {
                        return None;
                    }
                    self.members = Members::Array(vec![]);
                }
                let Members::Array(a) = &mut self.members else { return None };
                if i >= a.len() {
                    if !create {
                        return None;
                    }
                    a.resize_with(i + 1, Mark::default);
                }
                a.get_mut(i)
            }
            Value::Object(_) => {
                if !matches!(self.members, Members::Object(_)) {
                    if !create {
                        return None;
                    }
                    self.members = Members::Object(BTreeMap::new());
                }
                let Members::Object(o) = &mut self.members else { return None };
                if !o.contains_key(t) && !create {
                    return None;
                }
                Some(o.entry(t.to_string()).or_default())
            }
            _ => None,
        }
    }

    /// The mark of the value holding `path`'s, with that value in `doc` and `path`'s last token;
    /// `create` makes the marks on the way. `Err(Inside)` when a value on the way is new: what
    /// lies inside it is new too.
    fn container<'m, 'd>(&'m mut self, doc: &'d Value, path: &'d str, create: bool) -> Result<Option<(&'m mut Mark, &'d Value, &'d str)>, Inside> {
        let Some((q, last)) = path.rsplit_once('/') else { return Ok(None) };
        let (mut m, mut value) = (self, doc);
        for t in q.split('/').skip(1) {
            if m.added {
                return Err(Inside);
            }
            let next = match value {
                Value::Array(a) => t.parse::<usize>().ok().and_then(|i| a.get(i)),
                Value::Object(o) => o.get(unescaped(t).as_ref()),
                _ => None,
            };
            let Some(next) = next else { return Ok(None) };
            let Some(member) = m.member(value, t, create) else { return Ok(None) };
            (m, value) = (member, next);
        }
        if m.added {
            return Err(Inside);
        }
        Ok(Some((m, value, last)))
    }

    /// Put `mark` at `path`, inserted in an array (`-` appends; `removed` items of it were taken
    /// out first, by a move) or as an object's member.
    fn put(&mut self, doc: &Value, path: &str, mark: Mark, insert: bool, removed: usize) {
        if path.is_empty() {
            *self = mark;
            return;
        }
        let Ok(Some((m, container, last))) = self.container(doc, path, true) else { return };
        match container {
            Value::Array(items) => {
                let len = items.len().saturating_sub(removed);
                let Some(i) = (if last == "-" { Some(len) } else { last.parse::<usize>().ok() }) else { return };
                if !matches!(m.members, Members::Array(_)) {
                    m.members = Members::Array(vec![]);
                }
                let Members::Array(a) = &mut m.members else { return };
                if i < a.len() {
                    if insert {
                        a.insert(i, mark);
                    } else if let Some(slot) = a.get_mut(i) {
                        *slot = mark;
                    }
                } else {
                    a.resize_with(i, Mark::default);
                    a.push(mark);
                }
            }
            Value::Object(_) => {
                if !matches!(m.members, Members::Object(_)) {
                    m.members = Members::Object(BTreeMap::new());
                }
                if let Members::Object(o) = &mut m.members {
                    o.insert(last.to_string(), mark);
                }
            }
            _ => {}
        }
    }

    /// Take out the mark at `path` (an array's items after it move down one), answering it.
    fn take(&mut self, doc: &Value, path: &str) -> Mark {
        match self.container(doc, path, false) {
            Err(Inside) => Mark::new(true),
            Ok(None) => Mark::default(),
            Ok(Some((m, container, last))) => match (&mut m.members, container) {
                (Members::Array(a), Value::Array(_)) => match last.parse::<usize>() {
                    Ok(i) if i < a.len() => a.remove(i),
                    _ => Mark::default(),
                },
                (Members::Object(o), Value::Object(_)) => o.remove(last).unwrap_or_default(),
                _ => Mark::default(),
            },
        }
    }

    /// Note operation `name` (on `path`, from `from`) before it is applied to `doc`.
    fn before(&mut self, name: &str, path: &str, from: &str, doc: &Value) {
        match name {
            "add" | "copy" => self.put(doc, path, Mark::new(true), true, 0),
            "replace" => self.put(doc, path, Mark::new(true), false, 0),
            "remove" => {
                self.take(doc, path);
            }
            "move" if from != path => {
                let mut moved = self.take(doc, from);
                moved.written = true;
                // `path` is taken in the document with the value already removed from `from`.
                let shrinks = parent(from) == parent(path) && parent(from).is_some_and(|q| doc.pointer(q).is_some_and(Value::is_array));
                self.put(doc, path, moved, true, usize::from(shrinks));
            }
            _ => {}
        }
    }
}

/// Give every artboard and every object of the patched document's art without an `id` the id 0,
/// which the patch's artboard ids and `Document::give_fresh_ids` then replace with a fresh one. Only where the model keeps its
/// objects: the layers, the art of symbols, graph designs and pattern swatches, and inside an
/// object its opacity mask's art and its members (`children`, an envelope's `content`, a
/// repeat's `source`); other JSON (an effect's parameters, foreign data) is left as it is.
fn give_missing_ids(doc: &mut Value) {
    fn node(v: &mut Value) {
        let Value::Object(o) = v else { return };
        o.entry("id").or_insert(json!(0));
        if let Some(art) = o.get_mut("mask").and_then(|m| m.get_mut("art")) {
            node(art);
        }
        if let Some(Value::Object(kind)) = o.get_mut("kind") {
            for key in ["children", "content", "source"] {
                if let Some(Value::Array(members)) = kind.get_mut(key) {
                    members.iter_mut().for_each(node);
                }
            }
        }
    }
    let Value::Object(d) = doc else { return };
    if let Some(Value::Array(artboards)) = d.get_mut("artboards") {
        for a in artboards.iter_mut().filter_map(Value::as_object_mut) {
            a.entry("id").or_insert(json!(0));
        }
    }
    if let Some(Value::Array(layers)) = d.get_mut("layers") {
        layers.iter_mut().for_each(node);
    }
    for key in ["symbols", "graph_designs"] {
        if let Some(Value::Array(defs)) = d.get_mut(key) {
            defs.iter_mut().filter_map(|def| def.get_mut("art")).for_each(node);
        }
    }
    if let Some(Value::Array(patterns)) = d.get_mut("patterns") {
        for art in patterns.iter_mut().filter_map(|p| p.get_mut("art")).filter_map(Value::as_array_mut) {
            art.iter_mut().for_each(node);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn applied(doc: Value, patch: Value) -> Result<Applied, String> {
        apply(&json!({"patch": patch}), Forms { saved: doc, full: None })
    }

    #[test]
    fn objects_added_without_an_id_get_id_0_where_the_model_keeps_objects_only() {
        let doc = json!({"layers": [{"id": 1, "kind": {"type": "layer", "children": []}}], "symbols": [{"name": "s", "art": {"id": 2, "kind": {"type": "group", "children": []}}}]});
        let a = applied(
            doc,
            json!([
                {"op": "add", "path": "/layers/0/kind/children/-", "value": {"kind": {"type": "envelope", "content": [{"kind": {"type": "path"}}]}, "mask": {"art": {"kind": {"type": "path"}}}}},
                {"op": "add", "path": "/symbols/0/art/kind/children/-", "value": {"kind": {"type": "repeat", "source": [{"kind": {"type": "path"}}]}, "appearance": {"kind": {"type": "x"}}}}
            ]),
        )
        .unwrap();
        let v = &a.value;
        let added = &v["layers"][0]["kind"]["children"][0];
        assert_eq!((&added["id"], &added["kind"]["content"][0]["id"], &added["mask"]["art"]["id"]), (&json!(0), &json!(0), &json!(0)));
        let repeat = &v["symbols"][0]["art"]["kind"]["children"][0];
        assert_eq!((&repeat["id"], &repeat["kind"]["source"][0]["id"]), (&json!(0), &json!(0)));
        assert!(repeat["appearance"].get("id").is_none(), "an appearance is not an object of the art");
        assert!(a.added("/layers/0/kind/children/0/kind/content/0") && a.added("/symbols/0/art/kind/children/0"));
        assert!(!a.added("/layers/0") && !a.added("/symbols/0/art"));
    }

    #[test]
    fn a_copy_placed_before_what_it_copies_is_the_added_one() {
        let doc = json!({"layers": [{"id": 1, "kind": {"type": "layer", "children": [{"id": 2, "kind": {"type": "path"}}, {"id": 3, "kind": {"type": "path"}}]}}]});
        let a = applied(
            doc,
            json!([
                {"op": "copy", "from": "/layers/0/kind/children/1", "path": "/layers/0/kind/children/0"},
                {"op": "add", "path": "/layers/0/kind/children/0", "value": {"id": 9, "kind": {"type": "path"}}},
                {"op": "move", "from": "/layers/0/kind/children/1", "path": "/layers/0/kind/children/-"},
                {"op": "remove", "path": "/layers/0/kind/children/0"}
            ]),
        )
        .unwrap();
        let ids: Vec<&Value> = a.value["layers"][0]["kind"]["children"].as_array().unwrap().iter().map(|c| &c["id"]).collect();
        assert_eq!(ids, [&json!(2), &json!(3), &json!(3)]);
        let added: Vec<bool> = (0..3).map(|i| a.added(&format!("/layers/0/kind/children/{i}"))).collect();
        assert_eq!(added, [false, false, true], "the copy, moved last, is still the added one");
    }

    #[test]
    fn copies_that_would_double_the_document_past_the_budget_are_refused() {
        let mut patch = vec![json!({"op": "add", "path": "/a", "value": [0, 1]})];
        for _ in 0..40 {
            patch.push(json!({"op": "copy", "from": "/a", "path": "/a/-"}));
        }
        let err = applied(json!({"layers": []}), Value::Array(patch)).err().unwrap();
        assert!(err.contains("nest") || err.contains("values"), "{err}");
        let mut patch = vec![json!({"op": "add", "path": "/a", "value": {"b": [0, 1, 2, 3]}})];
        for i in 0..40 {
            patch.push(json!({"op": "copy", "from": "/a", "path": format!("/c{i}")}));
            patch.push(json!({"op": "copy", "from": "/a", "path": "/a/b/-"}));
        }
        let err = applied(json!({"layers": []}), Value::Array(patch)).err().unwrap();
        assert!(err.contains(&format!("more than {MAX_VALUES} values")), "{err}");
    }

    #[test]
    fn nesting_past_what_a_file_holds_is_refused() {
        let mut deep = json!(0);
        for _ in 0..MAX_NESTING {
            deep = json!([deep]);
        }
        let err = applied(json!({"layers": []}), json!([{"op": "add", "path": "/x", "value": deep}])).err().unwrap();
        assert!(err.starts_with("operation 0 (add)") && err.contains("nest"), "{err}");
    }

    #[test]
    fn replace_and_test_take_a_field_left_out_as_its_default() {
        let forms = || Forms { saved: json!({"a": {"x": 1}}), full: Some(json!({"a": {"x": 1, "opacity": 1.0}})) };
        let p = json!({"patch": [{"op": "test", "path": "/a/opacity", "value": 1.0}, {"op": "replace", "path": "/a/opacity", "value": 0.5}]});
        assert_eq!(apply(&p, forms()).unwrap().value, json!({"a": {"x": 1, "opacity": 0.5}}));
        let wrong = json!({"patch": [{"op": "test", "path": "/a/opacity", "value": 0.2}]});
        assert!(apply(&wrong, forms()).err().unwrap().contains("left out at its default, 1.0"));
        let unknown = json!({"patch": [{"op": "replace", "path": "/a/nonesuch", "value": 0}]});
        assert!(apply(&unknown, forms()).is_err(), "a field the model does not have is still refused");
    }

    #[test]
    fn nesting_up_to_what_a_saved_file_holds_is_allowed() {
        let mut deep = json!(0);
        for _ in 0..MAX_NESTING - 1 {
            deep = json!([deep]);
        }
        let a = applied(json!({"layers": []}), json!([{"op": "add", "path": "/x", "value": deep}])).unwrap();
        assert_eq!(nesting(&a.value), MAX_NESTING);
        // A saved file holds the document one level down, and its reader takes 128 levels.
        let file = json!({"version": 1, "document": a.value});
        assert!(serde_json::from_str::<Value>(&file.to_string()).is_ok());
        let one_more = json!({"version": 1, "document": {"x": [file["document"]["x"].clone()]}});
        assert!(serde_json::from_str::<Value>(&one_more.to_string()).is_err(), "the reader's limit is where MAX_NESTING says");
    }

    #[test]
    fn test_compares_numbers_by_value() {
        let doc = json!({"a": {"x": 1, "y": [2.0, {"z": -3}]}});
        let ok = json!([{"op": "test", "path": "/a", "value": {"y": [2, {"z": -3.0}], "x": 1.0}}]);
        assert!(applied(doc.clone(), ok).is_ok());
        for wrong in [json!({"x": 1.5, "y": [2, {"z": -3}]}), json!({"x": 1, "y": [2]}), json!({"x": "1", "y": [2, {"z": -3}]})] {
            assert!(applied(doc.clone(), json!([{"op": "test", "path": "/a", "value": wrong}])).is_err(), "{wrong}");
        }
        assert!(applied(doc, json!([{"op": "test", "path": "/b", "value": 1}])).is_err(), "nothing to test");
    }

    #[test]
    fn a_patch_past_the_most_operations_is_refused() {
        let ops: Vec<Value> = (0..=MAX_OPS).map(|_| json!({"op": "test", "path": "/a", "value": 1})).collect();
        let err = applied(json!({"a": 1}), Value::Array(ops)).err().unwrap();
        assert!(err.contains(&format!("at most {MAX_OPS}")), "{err}");
    }

    #[test]
    fn what_a_patch_wrote_is_followed_through_moves_and_removals() {
        let doc = json!({"a": [{"id": 1}, {"id": 2}, {"id": 3}], "b": {"c": {"id": 4}}});
        let a = applied(
            doc,
            json!([
                // Something new inside a value moved: the new one only.
                {"op": "move", "from": "/b/c", "path": "/a/1"},
                {"op": "add", "path": "/a/1/kids", "value": [{"id": 5}]},
                // Out of a new value, still new.
                {"op": "add", "path": "/n", "value": {"inner": {"id": 6}}},
                {"op": "move", "from": "/n/inner", "path": "/a/0"},
                {"op": "remove", "path": "/n"},
                // The items after one removed move down.
                {"op": "remove", "path": "/a/1"}
            ]),
        )
        .unwrap();
        let ids: Vec<&Value> = a.value["a"].as_array().unwrap().iter().map(|v| &v["id"]).collect();
        assert_eq!(ids, [&json!(6), &json!(4), &json!(2), &json!(3)]);
        let added: Vec<bool> = ["/a/0", "/a/1", "/a/1/kids/0", "/a/2", "/b"].iter().map(|p| a.added(p)).collect();
        assert_eq!(added, [true, false, true, false, false]);
        let mut written = a.written();
        written.sort();
        assert_eq!(written, ["/a/0", "/a/1"]);
    }
}
