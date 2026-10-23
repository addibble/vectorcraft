//! Fields equal to their default are left out of a saved file ([`crate::as_saved`]):
//! what a reader fills in for a missing field is exactly what the writer leaves out, so a document
//! reads back as it was written. Outside it, every field is written, as commands answer them.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::text::{AreaOptions, CharStyle, HyphenationOptions, JustificationOptions, ParaStyle, TextWrap};

/// `v`'s JSON as saved, and that JSON read back is `v` again.
fn written<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(v: &T) -> Value {
    let json = crate::as_saved(|| serde_json::to_value(v)).unwrap();
    let back: T = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(&back, v, "{json} reads back as written");
    json
}

/// Every struct read with `#[serde(default)]` writes nothing at its default: each field's
/// predicate (`crate::skip::container_defaults`) agrees with the struct's `Default`.
#[test]
fn a_struct_read_with_defaults_writes_nothing_at_its_default() {
    assert_eq!(written(&AreaOptions::default()), json!({}));
    assert_eq!(written(&HyphenationOptions::default()), json!({}));
    assert_eq!(written(&JustificationOptions::default()), json!({}));
    assert_eq!(written(&TextWrap::default()), json!({}));
    assert_eq!(written(&crate::dimension::DimensionOptions::default()), json!({}));
    assert_eq!(written(&crate::graph::ColumnDesign::default()), json!({}));
    // A graph saved before Edge-to-Edge Lines existed reads as edge to edge; a new one is not, so
    // it says so: the one field whose missing value is not its new value.
    assert_eq!(written(&crate::graph::GraphSpec::default()), json!({"edgeToEdge": false}));
    assert_eq!(written(&crate::live::EnvelopeOptions::default()), json!({}));
    assert_eq!(written(&crate::live_paint::GapOptions::default()), json!({}));
    assert_eq!(written(&crate::metadata::DocMetadata::default()), json!({}));
    assert_eq!(written(&crate::rastersettings::RasterEffectsSettings::default()), json!({}));
    assert_eq!(written(&crate::setup::DocSetup::default()), json!({}));
    // A paragraph always says its alignment: builds from before Auto alignment require it.
    assert_eq!(written(&ParaStyle::default()), json!({"justify": "Left"}));
}

/// A character style at its defaults writes only what has none: its family, size and fill.
#[test]
fn a_default_character_style_writes_only_its_family_size_and_fill() {
    let json = written(&CharStyle::default());
    let keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["fill", "font_family", "size"], "{json}");
}

/// A field set away from its default is written, and reads back.
#[test]
fn a_field_off_its_default_is_written() {
    let style = CharStyle { h_scale: 90.0, underline: true, ..CharStyle::default() };
    let json = written(&style);
    assert_eq!((json["h_scale"].as_f64(), json["underline"].as_bool()), (Some(90.0), Some(true)));
    let area = AreaOptions { gutter: 12.0, ..AreaOptions::default() };
    assert_eq!(written(&area), json!({"gutter": 12.0}));
}

/// Outside as_saved nothing changes: a command answering with a style or options gives
/// every field, so its answer can be passed back.
#[test]
fn outside_as_saved_every_field_is_written() {
    let json = serde_json::to_value(CharStyle::default()).unwrap();
    assert_eq!((json["h_scale"].as_f64(), json["underline"].as_bool()), (Some(100.0), Some(false)));
    assert_eq!(serde_json::to_value(AreaOptions::default()).unwrap()["gutter"], json!(18.0));
    // And the mode ends with the call, even when the call panics.
    let _ = std::panic::catch_unwind(|| crate::as_saved(|| panic!("inside")));
    assert!(!crate::skip::saving());
}
