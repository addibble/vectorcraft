//! `document.query`: JSONPath ([RFC 9535](https://www.rfc-editor.org/rfc/rfc9535)) over the
//! document model's JSON, the way `document.json` gives it, so that an agent can look at one part
//! of a large document (`$.artboards[*].name`, `$.layers[0].kind.children[?@.kind.type == 'path']`,
//! `$..[?@.name == 'logo']`) without reading the whole model. The answer is bounded: at most
//! `limit` matches (`total` counts them all), each match's value cut at `depth` levels with what
//! lies below summarized, and never more than [`MAX_BYTES`] of JSON.

use serde_json::{Value, json};
use serde_json_path::JsonPath;

/// Most matches listed unless `limit` says otherwise.
pub const DEFAULT_LIMIT: u64 = 100;
/// Most bytes of JSON an answer carries: past it, the query must be narrowed.
pub const MAX_BYTES: usize = 1 << 20;

/// How much of the matches to answer.
#[derive(Clone, Copy, Debug, Default)]
pub struct QueryOpts {
    /// At most this many matches listed (`None`: [`DEFAULT_LIMIT`]); 0 counts only.
    pub limit: Option<u64>,
    /// Matches skipped before the first listed.
    pub offset: u64,
    /// Levels of each match's value shown; containers below are summarized (`None`: all).
    pub depth: Option<u64>,
}

/// The matches of `path` in `value`: `{matches: [{path, pointer, value}], total}`, where `path`
/// is the match's normalized path (`$['layers'][0]`), so a later query names it exactly, and
/// `pointer` the same as a JSON Pointer (`/layers/0`), what `document.patch` takes.
pub fn run(value: &Value, path: &str, opts: QueryOpts) -> Result<Value, String> {
    let path = JsonPath::parse(path).map_err(|e| format!("`path` is not a JSONPath (RFC 9535): {e}"))?;
    let located = path.query_located(value);
    let total = located.len();
    let limit = usize::try_from(opts.limit.unwrap_or(DEFAULT_LIMIT)).unwrap_or(usize::MAX);
    let offset = usize::try_from(opts.offset).unwrap_or(usize::MAX);
    let matches: Vec<Value> = located
        .iter()
        .skip(offset)
        .take(limit)
        .map(|m| json!({"path": m.location().to_string(), "pointer": m.location().to_json_pointer(), "value": clip(m.node(), opts.depth)}))
        .collect();
    let answer = json!({"matches": matches, "total": total});
    let bytes = serde_json::to_string(&answer).map_or(0, |s| s.len());
    if bytes > MAX_BYTES {
        return Err(format!("the matches come to {bytes} bytes, past the {MAX_BYTES} a query answers: narrow `path`, lower `limit` or set `depth`"));
    }
    Ok(answer)
}

/// `v` cut at `depth` levels: a container below that is replaced by a summary of what it holds
/// (`{…12 keys}`, `[…54 items]`), never silently dropped.
pub fn clip(v: &Value, depth: Option<u64>) -> Value {
    match (v, depth) {
        (Value::Object(o), Some(0)) => Value::String(format!("{{…{} keys}}", o.len())),
        (Value::Array(a), Some(0)) => Value::String(format!("[…{} items]", a.len())),
        (Value::Object(o), d) => Value::Object(o.iter().map(|(k, v)| (k.clone(), clip(v, d.map(|d| d - 1)))).collect()),
        (Value::Array(a), d) => Value::Array(a.iter().map(|v| clip(v, d.map(|d| d - 1))).collect()),
        (v, _) => v.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Value {
        json!({
            "artboards": [{"name": "ace", "rect": {"x0": 0.0}}, {"name": "two", "rect": {"x0": 200.0}}],
            "layers": [{"id": 1, "kind": {"type": "layer", "children": [
                {"id": 2, "name": "paper", "kind": {"type": "path"}},
                {"id": 3, "name": "pip", "kind": {"type": "group", "children": [{"id": 4, "kind": {"type": "path"}}]}}
            ]}}]
        })
    }

    #[test]
    fn matches_come_with_their_normalized_paths() {
        let v = run(&doc(), "$.artboards[*].name", QueryOpts::default()).unwrap();
        assert_eq!(v["total"], 2);
        assert_eq!(v["matches"][0], json!({"path": "$['artboards'][0]['name']", "pointer": "/artboards/0/name", "value": "ace"}));
        let v = run(&doc(), "$..[?@.kind.type == 'path'].id", QueryOpts::default()).unwrap();
        assert_eq!(v["matches"].as_array().unwrap().iter().map(|m| m["value"].as_u64().unwrap()).collect::<Vec<_>>(), [2, 4]);
    }

    #[test]
    fn limit_offset_and_depth_bound_the_answer() {
        let v = run(&doc(), "$.layers[0].kind.children[*]", QueryOpts { limit: Some(1), offset: 1, depth: Some(1) }).unwrap();
        assert_eq!(v["total"], 2);
        assert_eq!(v["matches"].as_array().unwrap().len(), 1);
        assert_eq!(v["matches"][0]["value"], json!({"id": 3, "name": "pip", "kind": "{…2 keys}"}));
        let v = run(&doc(), "$.layers[*]", QueryOpts { limit: Some(0), ..QueryOpts::default() }).unwrap();
        assert_eq!((v["total"].as_u64(), v["matches"].as_array().map(Vec::len)), (Some(1), Some(0)));
        assert_eq!(clip(&json!([[1, 2], 3]), Some(1)), json!(["[…2 items]", 3]));
    }

    #[test]
    fn a_bad_path_and_an_oversized_answer_are_errors_that_say_what_to_do() {
        assert!(run(&doc(), "layers[", QueryOpts::default()).unwrap_err().contains("RFC 9535"));
        let big = Value::Array((0..3000).map(|i| json!({"i": i, "text": "x".repeat(400)})).collect());
        let err = run(&big, "$[*]", QueryOpts { limit: Some(3000), ..QueryOpts::default() }).unwrap_err();
        assert!(err.contains("narrow `path`"), "{err}");
    }
}
