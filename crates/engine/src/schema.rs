//! `document.schema`: the document model's JSON Schema, read one type at a time.
//!
//! The schema is generated from the model's own types and their doc comments
//! (`schemars` on `vectorcraft_doc::Document` and everything it holds), so it never lags the
//! format. A whole schema of the model is far too large to hand an agent at once, so this
//! module answers a question at a time: [`describe`] one named type (its properties, each with
//! its type, default and description, and the names of the types they refer to, to describe
//! next), [`at`] the type of the value at a JSONPath into a live document (the variant of an
//! enum chosen by the value there), and [`index`] every type with a line each.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde_json::{Map, Value, json};
use serde_json_path::JsonPath;
use serde_json_path::PathElement;

/// The root type: where a description of the model starts.
pub const ROOT: &str = "Document";

const DEFINITIONS: &str = "#/definitions/";

/// Every type's schema by name, the root among them as [`ROOT`].
fn definitions() -> &'static Map<String, Value> {
    static DEFINITIONS: OnceLock<Map<String, Value>> = OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        let root = schemars::schema_for!(vectorcraft_doc::Document);
        let Ok(Value::Object(mut root)) = serde_json::to_value(root) else { return Map::new() };
        let mut defs = match root.remove("definitions") {
            Some(Value::Object(defs)) => defs,
            _ => Map::new(),
        };
        root.remove("$schema");
        root.remove("title");
        defs.insert(ROOT.into(), Value::Object(root));
        defs
    })
}

/// `{start, types: [{name, description}]}`: every type, with the first line of its description.
pub fn index() -> Value {
    let types: Vec<Value> = definitions()
        .iter()
        .map(|(name, schema)| {
            let line = description(schema).and_then(|d| d.split_terminator(['.', '\n']).next().map(str::trim)).unwrap_or("");
            json!({"name": name, "description": line})
        })
        .collect();
    json!({"start": ROOT, "types": types})
}

/// The type named `name`, rendered as [`render`] does.
pub fn describe(name: &str) -> Result<Value, String> {
    match definitions().get(name) {
        Some(schema) => Ok(render(name, schema)),
        None => {
            let needle = name.to_lowercase();
            let near: Vec<&str> = definitions().keys().filter(|k| k.to_lowercase().contains(&needle)).take(5).map(String::as_str).collect();
            Err(match near.is_empty() {
                true => format!("no type `{name}`; start from `{ROOT}`, or `index: true` lists every type"),
                false => format!("no type `{name}`; did you mean {}?", near.join(", ")),
            })
        }
    }
}

/// The type of the value at `path` (its first match) in `document`, the document model's JSON:
/// the schema is walked beside the value, so an enum is the variant the value is.
pub fn at(document: &Value, path: &str) -> Result<Value, String> {
    let parsed = JsonPath::parse(path).map_err(|e| format!("`at` is not a JSONPath (RFC 9535): {e}"))?;
    let located = parsed.query_located(document);
    let Some(first) = located.iter().next() else { return Err(format!("nothing in the document matches `{path}`")) };
    let mut value = document;
    let mut schema = definitions().get(ROOT).ok_or("the model has no schema")?;
    let mut name = Some(ROOT.to_string());
    let mut walked = String::from("$");
    for element in first.location().iter() {
        let (resolved, resolved_name) = resolve(schema, value);
        let here = name.clone().or(resolved_name.clone()).unwrap_or_else(|| walked.clone());
        match element {
            PathElement::Name(key) => {
                walked.push_str(&format!("['{key}']"));
                value = value.get(key).ok_or_else(|| format!("nothing at {walked}"))?;
                schema = resolved
                    .get("properties")
                    .and_then(|p| p.get(*key))
                    .or_else(|| resolved.get("additionalProperties").filter(|a| a.is_object()))
                    .ok_or_else(|| format!("`{key}` is not a property of {here}"))?;
            }
            PathElement::Index(i) => {
                walked.push_str(&format!("[{i}]"));
                value = value.get(i).ok_or_else(|| format!("nothing at {walked}"))?;
                schema = match resolved.get("items") {
                    Some(Value::Array(items)) => items.get(*i).ok_or_else(|| format!("{here} has no item {i}"))?,
                    Some(items) => items,
                    None => return Err(format!("{here} is not an array")),
                };
            }
        }
        name = None;
    }
    let (resolved, resolved_name) = resolve(schema, value);
    let mut out = render(&name.or(resolved_name).unwrap_or_else(|| walked.clone()), resolved);
    if let Value::Object(o) = &mut out {
        o.insert("path".into(), Value::String(first.location().to_string()));
        o.insert("matches".into(), json!(located.len()));
        o.insert("valueType".into(), Value::String(json_type(value).into()));
    }
    Ok(out)
}

/// `schema` with references followed and, for a choice of schemas, the one `value` is: a null
/// takes the null branch, a tagged object the variant with its tag, anything else the first
/// branch of its JSON type. Answers the schema and the name of the definition it came from.
fn resolve<'a>(schema: &'a Value, value: &Value) -> (&'a Value, Option<String>) {
    let (schema, name) = deref(schema);
    let branches = schema.get("oneOf").or_else(|| schema.get("anyOf")).and_then(Value::as_array);
    let Some(branches) = branches else { return (schema, name) };
    let tag = value.get("type").and_then(Value::as_str);
    let pick = branches.iter().find(|b| {
        let (b, _) = deref(b);
        match (value, tag) {
            (Value::Null, _) => b.get("type") == Some(&json!("null")),
            (Value::Object(_), Some(tag)) if b.get("properties").and_then(|p| p.get("type")).is_some() => b
                .get("properties")
                .and_then(|p| p.get("type"))
                .and_then(|t| t.get("enum"))
                .and_then(Value::as_array)
                .is_some_and(|e| e.iter().any(|v| v == tag)),
            _ => types_of(b).iter().any(|t| t == json_type(value)),
        }
    });
    match pick {
        Some(b) => {
            let (b, branch_name) = deref(b);
            (b, branch_name.or(name))
        }
        None => (schema, name),
    }
}

/// `schema` with a reference followed to its definition, and its name. A described field refers
/// to its type as `allOf: [{$ref}]` (a reference takes no description beside it), so a lone
/// `allOf` is looked through too.
fn deref(schema: &Value) -> (&Value, Option<String>) {
    if let Some(name) = schema.get("$ref").and_then(Value::as_str).and_then(|r| r.strip_prefix(DEFINITIONS)) {
        return match definitions().get(name) {
            Some(def) => (def, Some(name.to_string())),
            None => (schema, None),
        };
    }
    match schema.get("allOf").and_then(Value::as_array) {
        Some(parts) if parts.len() == 1 && schema.get("properties").is_none() => deref(&parts[0]),
        _ => (schema, None),
    }
}

/// The JSON types a schema admits, as its `type` names them.
fn types_of(schema: &Value) -> Vec<String> {
    match schema.get("type") {
        Some(Value::String(t)) => vec![t.clone()],
        Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).map(String::from).collect(),
        _ => vec![],
    }
}

fn json_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn description(schema: &Value) -> Option<&str> {
    schema.get("description").and_then(Value::as_str)
}

/// One type for an agent: `{type, description, properties: [{name, type, required?, default?,
/// description}], values?: [...] (a string enum), variants?: [...] (a choice of shapes), types:
/// [the named types the properties refer to]}`.
pub fn render(name: &str, schema: &Value) -> Value {
    let mut refs = BTreeSet::new();
    let mut out = Map::new();
    out.insert("type".into(), Value::String(name.into()));
    if let Some(d) = description(schema) {
        out.insert("description".into(), Value::String(d.into()));
    }
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        // An internally tagged variant: its tag as `tag`, not as a property with one value.
        let tag = props
            .get("type")
            .and_then(|t| t.get("enum"))
            .and_then(Value::as_array)
            .filter(|e| e.len() == 1)
            .and_then(|e| e.first())
            .and_then(Value::as_str);
        if let Some(tag) = tag {
            out.insert("tag".into(), Value::String(tag.into()));
        }
        out.insert("properties".into(), properties(props, schema, &mut refs, tag.map(|_| "type")));
    }
    if let Some(values) = schema.get("enum") {
        out.insert("values".into(), values.clone());
    }
    if let Some(branches) = schema.get("oneOf").or_else(|| schema.get("anyOf")).and_then(Value::as_array) {
        let variants: Vec<Value> = branches.iter().map(|b| variant(b, &mut refs)).collect();
        out.insert("variants".into(), Value::Array(variants));
    }
    if schema.get("properties").is_none() && schema.get("enum").is_none() && schema.get("oneOf").is_none() && schema.get("anyOf").is_none() {
        out.insert("shape".into(), Value::String(type_expr(schema, &mut refs)));
    }
    refs.remove(name);
    out.insert("types".into(), Value::Array(refs.into_iter().map(Value::String).collect()));
    Value::Object(out)
}

fn properties(props: &Map<String, Value>, schema: &Value, refs: &mut BTreeSet<String>, skip: Option<&str>) -> Value {
    let required: BTreeSet<&str> = schema.get("required").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    let list: Vec<Value> = props
        .iter()
        .filter(|(k, _)| Some(k.as_str()) != skip)
        .map(|(k, p)| {
            let mut o = Map::new();
            o.insert("name".into(), Value::String(k.clone()));
            o.insert("type".into(), Value::String(type_expr(p, refs)));
            if required.contains(k.as_str()) {
                o.insert("required".into(), Value::Bool(true));
            }
            if let Some(d) = p.get("default") {
                o.insert("default".into(), d.clone());
            }
            if let Some(d) = description(p) {
                o.insert("description".into(), Value::String(d.into()));
            }
            Value::Object(o)
        })
        .collect();
    Value::Array(list)
}

/// One branch of a choice: a tagged object as `{tag, description?, properties}`, a string
/// enum as `{values}`, anything else as `{type}`.
fn variant(branch: &Value, refs: &mut BTreeSet<String>) -> Value {
    let (b, name) = deref(branch);
    let tag = b
        .get("properties")
        .and_then(|p| p.get("type"))
        .and_then(|t| t.get("enum"))
        .and_then(Value::as_array)
        .and_then(|e| e.first())
        .and_then(Value::as_str);
    let mut o = Map::new();
    match (tag, b.get("properties").and_then(Value::as_object), b.get("enum")) {
        (Some(tag), Some(props), _) => {
            o.insert("tag".into(), Value::String(tag.into()));
            o.insert("properties".into(), properties(props, b, refs, Some("type")));
        }
        (None, Some(props), _) => {
            o.insert("properties".into(), properties(props, b, refs, None));
        }
        (_, None, Some(values)) => {
            o.insert("values".into(), values.clone());
        }
        _ => {
            o.insert("type".into(), Value::String(type_expr(branch, refs)));
        }
    }
    if let Some(name) = name {
        o.insert("type".into(), Value::String(name));
    }
    if let Some(d) = description(b) {
        o.insert("description".into(), Value::String(d.into()));
    }
    Value::Object(o)
}

/// A schema as one readable type expression: `Artboard`, `array of Node`, `map of ImageBlob`,
/// `number | null`, `one of: left | right`, `object {x, y}`.
fn type_expr(schema: &Value, refs: &mut BTreeSet<String>) -> String {
    if let Some(name) = schema.get("$ref").and_then(Value::as_str).and_then(|r| r.strip_prefix(DEFINITIONS)) {
        refs.insert(name.to_string());
        return name.to_string();
    }
    if let Some(branches) = schema.get("oneOf").or_else(|| schema.get("anyOf")).and_then(Value::as_array) {
        let parts: Vec<String> = branches.iter().map(|b| type_expr(b, refs)).collect();
        return if parts.len() > 8 { format!("one of {} kinds", parts.len()) } else { parts.join(" | ") };
    }
    if let Some(parts) = schema.get("allOf").and_then(Value::as_array) {
        return parts.iter().map(|p| type_expr(p, refs)).collect::<Vec<_>>().join(" & ");
    }
    let types = types_of(schema);
    let nullable = types.iter().any(|t| t == "null");
    let base = match types.iter().find(|t| *t != "null").map(String::as_str) {
        Some("array") => match schema.get("items") {
            Some(Value::Array(items)) => format!("[{}]", items.iter().map(|i| type_expr(i, refs)).collect::<Vec<_>>().join(", ")),
            Some(items) => format!("array of {}", type_expr(items, refs)),
            None => "array".into(),
        },
        Some("object") => match (schema.get("properties").and_then(Value::as_object), schema.get("additionalProperties")) {
            (Some(props), _) => format!("object {{{}}}", props.keys().cloned().collect::<Vec<_>>().join(", ")),
            (None, Some(ap)) if ap.is_object() => format!("map of {}", type_expr(ap, refs)),
            _ => "object".into(),
        },
        Some("string") => match schema.get("enum").and_then(Value::as_array) {
            Some(values) => {
                format!("one of: {}", values.iter().map(|v| v.as_str().map_or_else(|| v.to_string(), String::from)).collect::<Vec<_>>().join(" | "))
            }
            None => "string".into(),
        },
        Some(t) => t.to_string(),
        None => match schema.get("enum").and_then(Value::as_array) {
            Some(values) => format!("one of: {}", values.iter().map(Value::to_string).collect::<Vec<_>>().join(" | ")),
            None => "any".into(),
        },
    };
    if nullable && !types.iter().all(|t| t == "null") { format!("{base} | null") } else { base }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property<'a>(rendered: &'a Value, name: &str) -> &'a Value {
        rendered["properties"].as_array().unwrap().iter().find(|p| p["name"] == name).unwrap_or_else(|| panic!("{name} in {rendered}"))
    }

    #[test]
    fn the_root_describes_the_document_and_names_the_types_to_read_next() {
        let root = describe(ROOT).unwrap();
        assert_eq!(root["type"], ROOT);
        assert_eq!(property(&root, "artboards")["type"], "array of Artboard");
        assert_eq!(property(&root, "layers")["type"], "array of Node");
        assert_eq!(property(&root, "images")["type"], "map of ImageBlob");
        let types: Vec<&str> = root["types"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
        assert!(types.contains(&"Artboard") && types.contains(&"Node") && types.contains(&"Swatch"), "{types:?}");
        let artboard = describe("Artboard").unwrap();
        assert!(property(&artboard, "rect")["description"].as_str().unwrap().contains("points"), "{artboard}");
        assert_eq!(property(&artboard, "rect")["type"], "Rect");
    }

    #[test]
    fn an_unknown_type_is_refused_with_the_nearest_names() {
        let err = describe("artboard").unwrap_err();
        assert!(err.contains("Artboard"), "{err}");
        assert!(describe("Nonesuch").unwrap_err().contains("index: true"));
    }

    #[test]
    fn the_index_lists_every_type_once_with_a_line_each() {
        let index = index();
        let names: Vec<&str> = index["types"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&ROOT) && names.contains(&"Appearance"));
        assert_eq!(names.len(), definitions().len());
        let doc = index["types"].as_array().unwrap().iter().find(|t| t["name"] == "Node").unwrap();
        assert!(!doc["description"].as_str().unwrap().contains('\n'));
    }

    #[test]
    fn every_reference_in_the_schema_resolves() {
        fn walk(v: &Value, missing: &mut Vec<String>) {
            match v {
                Value::Object(o) => {
                    if let Some(r) = o.get("$ref").and_then(Value::as_str)
                        && r.strip_prefix(DEFINITIONS).is_none_or(|name| !definitions().contains_key(name))
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
        definitions().values().for_each(|v| walk(v, &mut missing));
        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn the_type_at_a_path_follows_the_value_into_an_enum_variant() {
        let doc = serde_json::to_value(vectorcraft_doc::Document::new(100.0, 100.0)).unwrap();
        let kind = at(&doc, "$.layers[0].kind").unwrap();
        assert_eq!(kind["path"], "$['layers'][0]['kind']");
        assert_eq!(kind["valueType"], "object");
        assert_eq!(property(&kind, "children")["type"], "array of Node", "{kind}");
        assert_eq!(kind["tag"], "layer");
        assert!(kind["properties"].as_array().unwrap().iter().all(|p| p["name"] != "type"), "{kind}");
        assert!(property(&kind, "clip")["description"].as_str().is_some_and(|d| d.contains("clip")), "{kind}");
        let rect = at(&doc, "$.artboards[0].rect").unwrap();
        assert_eq!(rect["type"], "Rect");
        assert_eq!(at(&doc, "$.artboards[*].name").unwrap()["matches"], 1);
        assert!(at(&doc, "$.artboards[9]").unwrap_err().contains("nothing"));
        assert!(at(&doc, "$.artboards[0].nonesuch").unwrap_err().contains("nothing"));
        assert!(at(&doc, "layers[").unwrap_err().contains("RFC 9535"));
    }
}
