//! `document.patch`: JSON Patch ([RFC 6902](https://www.rfc-editor.org/rfc/rfc6902)) with JSON
//! Pointer paths ([RFC 6901](https://www.rfc-editor.org/rfc/rfc6901)) over the document model's
//! JSON, the way `document.json` gives it and `document.query` names it (each match's `pointer`),
//! applied by `craft_jsonquery::Patch`: in order, all or none. The result loads through the
//! document model, which refuses what the model cannot be, and the command records one undo step
//! and one journal entry, so the patch is undone and replayed like any other edit.
//!
//! Ids are the model's: an object added without one, or a copy whose id another object has,
//! gets a fresh id from the document's counter ([`assign_ids`]), the way paste and duplicate do.

use serde_json::{Value, json};

/// Give every node of the patched model without an `id`, or with one another node has, a fresh
/// id from the model's counter (`next_id`), and answer how many. A node is an object with an
/// object `kind` that is a layer (`layers`), a child (`children`), or a symbol's or mask's `art`.
pub fn assign_ids(value: &mut Value) -> usize {
    let mut seen = std::collections::BTreeSet::new();
    let mut next = value.get("next_id").and_then(Value::as_u64).unwrap_or(1);
    let mut fresh = 0;
    fn walk(v: &mut Value, node: bool, seen: &mut std::collections::BTreeSet<u64>, next: &mut u64, fresh: &mut usize) {
        match v {
            Value::Object(o) => {
                if node && o.get("kind").is_some_and(Value::is_object) {
                    let id = o.get("id").and_then(Value::as_u64).filter(|id| !seen.contains(id));
                    let id = id.unwrap_or_else(|| {
                        while seen.contains(next) {
                            *next += 1;
                        }
                        o.insert("id".into(), json!(*next));
                        *fresh += 1;
                        *next
                    });
                    seen.insert(id);
                    *next = (*next).max(id.saturating_add(1));
                }
                for (key, child) in o.iter_mut() {
                    let holds_nodes = matches!(key.as_str(), "layers" | "children" | "art");
                    match child {
                        Value::Array(items) => items.iter_mut().for_each(|item| walk(item, holds_nodes, seen, next, fresh)),
                        other => walk(other, holds_nodes, seen, next, fresh),
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| walk(item, false, seen, next, fresh)),
            _ => {}
        }
    }
    walk(value, false, &mut seen, &mut next, &mut fresh);
    if let Value::Object(o) = value {
        o.insert("next_id".into(), json!(next));
    }
    fresh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_copied_nodes_get_fresh_ids_and_the_counter_follows() {
        let mut doc = json!({"next_id": 5, "layers": [{"id": 1, "kind": {"type": "layer", "children": [
            {"id": 2, "kind": {"type": "path"}},
            {"id": 2, "kind": {"type": "path"}},
            {"kind": {"type": "group", "children": [{"id": 9, "kind": {"type": "path"}}]}},
            {"id": 3, "appearance": {"items": [{"kind": "fill"}]}, "kind": {"type": "path"}}
        ]}}], "symbols": [{"name": "pip", "art": {"kind": {"type": "path"}}}]});
        assert_eq!(assign_ids(&mut doc), 3);
        let children = doc["layers"][0]["kind"]["children"].as_array().unwrap();
        let ids: Vec<u64> = children.iter().map(|c| c["id"].as_u64().unwrap()).collect();
        assert_eq!(ids, [2, 5, 6, 3]);
        assert_eq!(doc["symbols"][0]["art"]["id"], 10);
        assert_eq!(doc["next_id"], 11);
        assert!(doc["layers"][0]["kind"]["children"][3]["appearance"]["items"][0].get("id").is_none(), "an appearance item is not a node");
    }
}
