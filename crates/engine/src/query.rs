//! `document.query`: JSONPath ([RFC 9535](https://www.rfc-editor.org/rfc/rfc9535)) over the
//! document model's JSON, the way `document.json` gives it, by `craft_jsonquery::query`: an agent
//! looks at one part of a large document (`$.artboards[*].name`,
//! `$.layers[0].kind.children[?@.kind.type == 'path']`, `$..[?@.name == 'logo']`) without reading
//! the whole model. The answer is bounded by `limit`, `offset`, `depth` and [`MAX_BYTES`], counted
//! as the caller writes the answer (`layout`): the MCP server writes it pretty-printed and says so
//! for every query it runs (`vectorcraft_mcp`), so an agent never reads more than the limit.

use craft_jsonquery::{Layout, QueryOpts};
use serde_json::Value;

/// Most bytes a query's answer comes to, written as its caller writes it.
pub const MAX_BYTES: usize = 1 << 20;

/// The options a query runs with: [`MAX_BYTES`], and the layout `p` names (`layout: compact`,
/// the default, or `pretty`). `limit`, `offset` and `depth` are read with the path
/// (`craft_jsonquery::query::from_params`).
pub fn opts(p: &Value) -> Result<QueryOpts, String> {
    let layout = match p.get("layout") {
        None | Some(Value::Null) => Layout::Compact,
        Some(Value::String(l)) if l == "compact" => Layout::Compact,
        Some(Value::String(l)) if l == "pretty" => Layout::Pretty,
        Some(_) => return Err("`layout` must be compact or pretty".into()),
    };
    Ok(QueryOpts::new(MAX_BYTES, layout))
}
