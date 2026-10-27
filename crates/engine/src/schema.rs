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

/// The root type: where a description of the model starts.
pub const ROOT: &str = "Document";

/// The document model's schema, made once.
pub fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| Schema::new(serde_json::to_value(schemars::schema_for!(vectorcraft_doc::Document)).unwrap_or_default(), ROOT))
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
