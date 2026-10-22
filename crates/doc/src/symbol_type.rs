//! Symbol Options › Symbol Type: Dynamic or Static
//! ([helpx Symbols](https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/create-and-place-symbols.html)).
//!
//! An instance of a dynamic symbol can show one or more objects of the symbol's art with a fill,
//! stroke, opacity or blend mode of its own and stay linked to the symbol: redefining the symbol
//! changes every instance, and each instance keeps its own overrides
//! ([`crate::SymbolOverride`]). An instance of a static symbol always shows the art as
//! the symbol holds it. Illustrator's default for a new symbol is Dynamic, and so is this one:
//! stored under [`SYMBOL_TYPE`] keyed by symbol name, with the default (Dynamic) omitted from the
//! map, so a document saved before the choice existed opens with dynamic symbols (they draw as
//! static ones until an instance is given an override).

use serde_json::Value;

/// Document unknown key: `{ symbolName: "static" }` (dynamic symbols are not listed).
pub const SYMBOL_TYPE: &str = "symbolType";

/// Symbol Options › Symbol Type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SymbolType {
    /// Default: instances may override the appearance of objects in the art.
    #[default]
    Dynamic,
    /// Instances show the art as the symbol holds it.
    Static,
}

impl SymbolType {
    pub const IDS: [&'static str; 2] = ["dynamic", "static"];

    pub fn id(self) -> &'static str {
        match self {
            Self::Dynamic => "dynamic",
            Self::Static => "static",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        match s {
            "dynamic" => Some(Self::Dynamic),
            "static" => Some(Self::Static),
            _ => None,
        }
    }

    /// True when this is the document default (nothing stored for the symbol).
    pub fn is_default(self) -> bool {
        matches!(self, Self::Dynamic)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        v.as_str().and_then(Self::from_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_dynamic_is_the_default() {
        assert_eq!(SymbolType::default(), SymbolType::Dynamic);
        for id in SymbolType::IDS {
            assert_eq!(SymbolType::from_id(id).map(SymbolType::id), Some(id));
        }
        assert_eq!(SymbolType::from_value(&serde_json::json!("static")), Some(SymbolType::Static));
        assert_eq!(SymbolType::from_value(&serde_json::json!(1)), None);
        assert!(SymbolType::Dynamic.is_default() && !SymbolType::Static.is_default());
    }
}
