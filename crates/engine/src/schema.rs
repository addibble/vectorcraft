//! `document.schema`: the document model's JSON Schema, read one type at a time by
//! `craft_jsonquery::Schema`.
//!
//! The schema is generated from the model's own types and their doc comments
//! (`schemars` on `vectorcraft_doc::Document` and everything it holds), so it never lags the
//! format. A whole schema of the model is far too large to hand an agent at once, so it is read a
//! question at a time: one named type (its properties, each with its type, default and
//! description, and the names of the types they refer to, to describe next), the type of the value
//! at a JSONPath into a live document (the variant of an enum chosen by the value there), or every
//! type with a line each.

use std::sync::OnceLock;

use craft_jsonquery::Schema;
use serde_json::Value;

/// The root type: where a description of the model starts.
pub const ROOT: &str = "Document";

/// The keys besides `type` the model's internally tagged enums take their tag in (a paint's
/// `kind`, a color's `model`, a live shape's `shape`).
const TAGS: [&str; 3] = ["kind", "model", "shape"];

/// The model's JSON Schema, as schemars derives it.
fn derived() -> Value {
    // Derived with nothing left out, so each field keeps its default (`describing`).
    serde_json::to_value(vectorcraft_doc::describing(|| schemars::schema_for!(vectorcraft_doc::Document))).unwrap_or_default()
}

/// The document model's schema, made once.
pub fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| Schema::new(derived(), ROOT))
}

/// [`schema`] with each variant tagged in one of [`TAGS`] given a `type` property like its tag:
/// `craft_jsonquery::Schema::at` picks an enum's variant by a value's `type` only, so [`at`]
/// walks this one beside a value given `type` where it has such a tag.
fn tagged_schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        fn walk(v: &mut Value) {
            match v {
                Value::Object(o) => {
                    for key in ["oneOf", "anyOf"] {
                        for branch in o.get_mut(key).and_then(Value::as_array_mut).into_iter().flatten() {
                            let Some(props) = branch.get_mut("properties").and_then(Value::as_object_mut) else { continue };
                            if props.contains_key("type") {
                                continue;
                            }
                            let one = |p: &Value| p.get("enum").and_then(Value::as_array).is_some_and(|e| e.len() == 1) || p.get("const").is_some();
                            if let Some(tag) = TAGS.iter().find_map(|t| props.get(*t).filter(|p| one(p)).cloned()) {
                                props.insert("type".into(), tag);
                            }
                        }
                    }
                    o.values_mut().for_each(walk);
                }
                Value::Array(a) => a.iter_mut().for_each(walk),
                _ => {}
            }
        }
        let mut root = derived();
        walk(&mut root);
        Schema::new(root, ROOT)
    })
}

/// `document.schema`'s answer to `p` (`{type?, at?, index?}`); `document` gives the document's
/// JSON, read only for `at`.
pub fn answer(p: &Value, document: impl FnOnce() -> Result<Value, String>) -> Result<Value, String> {
    let given = |key: &str| p.get(key).is_some_and(|v| !v.is_null());
    match p.get("at").and_then(Value::as_str) {
        Some(path) if !given("type") && p.get("index").and_then(Value::as_bool) != Some(true) => at(&document()?, path),
        _ => schema().answer(p, document),
    }
}

/// The type of the value at `path` (its first match) in `document`: [`Schema::at`], with each
/// object on the way to it that is tagged in one of [`TAGS`] taken as the variant its tag names.
pub fn at(document: &Value, path: &str) -> Result<Value, String> {
    let plain = schema().at(document, path)?;
    let opts = craft_jsonquery::QueryOpts {
        limit: Some(1),
        depth: Some(0),
        ..craft_jsonquery::QueryOpts::new(crate::query::MAX_BYTES, craft_jsonquery::Layout::Compact)
    };
    let found = craft_jsonquery::query::run(document, path, opts)?;
    let Some(pointer) = found["matches"][0]["pointer"].as_str() else { return Ok(plain) };
    let mut tagged = document.clone();
    let mut here = Some(&mut tagged);
    let mut tokens = pointer.split('/').skip(1).map(|t| t.replace("~1", "/").replace("~0", "~"));
    while let Some(v) = here {
        if let Value::Object(o) = &mut *v
            && !o.contains_key("type")
            && let Some(tag) = TAGS.iter().find_map(|t| o.get(*t).and_then(Value::as_str))
        {
            o.insert("type".into(), Value::String(tag.to_string()));
        }
        let Some(token) = tokens.next() else { break };
        here = match v {
            Value::Object(o) => o.get_mut(&token),
            Value::Array(a) => token.parse::<usize>().ok().and_then(|i| a.get_mut(i)),
            _ => None,
        };
    }
    let out = tagged_schema().at(&tagged, path)?;
    // The `type` given may not change which value matches; were it to, the plain answer stands.
    Ok(if out["path"] == plain["path"] { out } else { plain })
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{ROOT, schema};

    fn property<'a>(rendered: &'a Value, name: &str) -> &'a Value {
        rendered["properties"].as_array().unwrap().iter().find(|p| p["name"] == name).unwrap_or_else(|| panic!("{name} in {rendered}"))
    }

    #[test]
    fn the_root_describes_the_document_and_names_the_types_to_read_next() {
        let root = schema().describe(ROOT).unwrap();
        assert_eq!(root["type"], ROOT);
        assert_eq!(property(&root, "artboards")["type"], "array of Artboard");
        assert_eq!(property(&root, "layers")["type"], "array of Node");
        assert_eq!(property(&root, "images")["type"], "map of ImageBlob");
        let types: Vec<&str> = root["types"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
        assert!(types.contains(&"Artboard") && types.contains(&"Node") && types.contains(&"Swatch"), "{types:?}");
        let artboard = schema().describe("Artboard").unwrap();
        assert!(property(&artboard, "rect")["description"].as_str().unwrap().contains("points"), "{artboard}");
        assert_eq!(property(&artboard, "rect")["type"], "Rect");
    }

    #[test]
    fn an_unknown_type_is_refused_with_the_nearest_names() {
        let err = schema().describe("artboard").unwrap_err();
        assert!(err.contains("Artboard"), "{err}");
        assert!(schema().describe("Nonesuch").unwrap_err().contains("index: true"));
    }

    #[test]
    fn the_index_lists_every_type_once_with_a_line_each() {
        let index = schema().index();
        let names: Vec<&str> = index["types"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&ROOT) && names.contains(&"Appearance"));
        assert_eq!(names.len(), schema().types().len());
        let doc = index["types"].as_array().unwrap().iter().find(|t| t["name"] == "Node").unwrap();
        assert!(!doc["description"].as_str().unwrap().contains('\n'));
    }

    #[test]
    fn every_reference_in_the_schema_resolves() {
        fn walk(v: &Value, missing: &mut Vec<String>) {
            match v {
                Value::Object(o) => {
                    if let Some(r) = o.get("$ref").and_then(Value::as_str)
                        && r.strip_prefix("#/definitions/").is_none_or(|name| !schema().types().contains_key(name))
                    {
                        missing.push(r.into());
                    }
                    o.values().for_each(|v| walk(v, missing));
                }
                Value::Array(a) => a.iter().for_each(|v| walk(v, missing)),
                _ => {}
            }
        }
        let mut missing = vec![];
        schema().types().values().for_each(|v| walk(v, &mut missing));
        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn the_type_at_a_path_follows_the_value_into_an_enum_variant() {
        let doc = serde_json::to_value(vectorcraft_doc::Document::new(100.0, 100.0)).unwrap();
        let kind = schema().at(&doc, "$.layers[0].kind").unwrap();
        assert_eq!(kind["path"], "$['layers'][0]['kind']");
        assert_eq!(kind["valueType"], "object");
        assert_eq!(property(&kind, "children")["type"], "array of Node", "{kind}");
        assert_eq!(kind["tag"], "layer");
        assert!(kind["properties"].as_array().unwrap().iter().all(|p| p["name"] != "type"), "{kind}");
        assert!(property(&kind, "clip")["description"].as_str().is_some_and(|d| d.contains("clip")), "{kind}");
        let rect = schema().at(&doc, "$.artboards[0].rect").unwrap();
        assert_eq!(rect["type"], "Rect");
        assert_eq!(schema().at(&doc, "$.artboards[*].name").unwrap()["matches"], 1);
        assert!(schema().at(&doc, "$.artboards[9]").unwrap_err().contains("nothing"));
        assert!(schema().at(&doc, "$.artboards[0].nonesuch").unwrap_err().contains("nothing"));
        assert!(schema().at(&doc, "layers[").unwrap_err().contains("RFC 9535"));
    }
}
