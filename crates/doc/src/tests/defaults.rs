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
    // A paragraph always says its alignment: builds from before Auto alignment require it. Korean
    // Line Break Anywhere, off in a new paragraph, is written: a paragraph saved without it reads
    // with it on.
    assert_eq!(written(&ParaStyle::default()), json!({"justify": "Left", "korean_line_break_anywhere": false}));
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

/// Document Setup's ruler mode is left out only of a saved file: a command's answer and the
/// model's schema keep it, with its default.
#[test]
fn the_ruler_mode_is_left_out_of_a_saved_file_only() {
    let mut d = crate::Document::new(100.0, 100.0);
    d.rulers = crate::RulerMode::legacy();
    assert_eq!(serde_json::to_value(&d).unwrap()["rulers"], json!("global"));
    assert!(crate::as_saved(|| serde_json::to_value(&d)).unwrap().get("rulers").is_none());
}

/// -0.0 is not 0.0 to a saved file: it is written, and reads back with its sign.
#[test]
fn negative_zero_is_written() {
    let area = AreaOptions { inset: -0.0, ..AreaOptions::default() };
    let json = written(&area);
    let inset = json.get("inset").and_then(Value::as_f64).unwrap_or_else(|| panic!("{json}"));
    assert!(inset.is_sign_negative());
}

/// A saved file leaves out what equals a type's `Default`, so a reader fills in that `Default`:
/// changing one would change what every file written before reads as. Each one a file depends on
/// is pinned here as it is written in full.
#[test]
fn the_defaults_a_saved_file_leaves_out_are_pinned() {
    fn pinned<T: Default + Serialize>(name: &str, json: &str) {
        let now = serde_json::to_value(T::default()).unwrap();
        let pinned: Value = serde_json::from_str(json).unwrap();
        assert_eq!(now, pinned, "{name}::default() changed: files saved before read differently now");
    }
    pinned::<crate::setup::DocSetup>(
        "DocSetup",
        r#"{"background":"transparent","bleed":[0.0,0.0,0.0,0.0],"discardWhiteOverprint":true,"exportText":"editable","flattenerPreset":null,"gridColors":[{"b":1.0,"g":1.0,"model":"rgb","r":1.0},{"b":0.800000011920929,"g":0.800000011920929,"model":"rgb","r":0.800000011920929}],"gridSize":"medium","highlightSubstitutedFonts":true,"highlightSubstitutedGlyphs":false,"language":"English: USA","outlineImages":false,"quotes":{"double":["“","”"],"single":["‘","’"]},"simulatePaper":false,"smallCapsSize":70.0,"subscript":{"position":33.3,"size":58.3},"superscript":{"position":33.3,"size":58.3},"typographersQuotes":true}"#,
    );
    pinned::<crate::dimension::DimensionOptions>(
        "DimensionOptions",
        r#"{"arrowScale":100.0,"arrowStyle":"filled","extensionColor":[0,0,0],"extensionLength":6.0,"extensionOffset":4.0,"extensionType":"solid","extensionWeight":0.5,"fontFamily":"Source Sans 3","fontSize":12.0,"fontStyle":"Regular","hideExtensions":false,"hideUnits":false,"lineColor":[0,0,0],"lineType":"solid","lineWeight":0.5,"precision":2,"scale":"1:1","textAlignment":"center","textColor":[0,0,0],"textOffset":4.0,"textPosition":"away","units":"pt"}"#,
    );
    pinned::<CharStyle>(
        "CharStyle",
        r#"{"all_caps":false,"baseline_shift":0.0,"fill":{"color":{"b":0.0,"g":0.0,"model":"rgb","r":0.0},"type":"solid"},"font_family":"Source Sans 3","font_style":"Regular","h_scale":100.0,"rotation":0.0,"size":12.0,"strikethrough":false,"stroke":{"type":"none"},"stroke_width":0.0,"tracking":0.0,"underline":false,"v_scale":100.0}"#,
    );
    pinned::<ParaStyle>(
        "ParaStyle",
        r#"{"first_line_indent":0.0,"hyphenate":false,"justify":"Left","korean_line_break_anywhere":false,"left_indent":0.0,"right_indent":0.0,"space_after":0.0,"space_before":0.0}"#,
    );
    pinned::<AreaOptions>(
        "AreaOptions",
        r#"{"columns":1,"firstBaseline":"ascent","firstBaselineMin":0.0,"gutter":18.0,"inset":0.0,"rows":1,"verticalAlign":"top"}"#,
    );
    pinned::<HyphenationOptions>(
        "HyphenationOptions",
        r#"{"after_first":3,"before_last":3,"capitalized":true,"hyphen_limit":0,"words_longer_than":5,"zone":0.0}"#,
    );
    pinned::<JustificationOptions>(
        "JustificationOptions",
        r#"{"auto_leading":120.0,"glyph_scaling":[100.0,100.0,100.0],"letter_spacing":[0.0,0.0,0.0],"single_word":"fullJustify","word_spacing":[80.0,100.0,133.0]}"#,
    );
    pinned::<TextWrap>("TextWrap", r#"{"invert":false,"offset":6.0}"#);
    pinned::<crate::graph::ColumnDesign>("ColumnDesign", r#"{"design":"","each":10.0}"#);
    pinned::<crate::graph::GraphSpec>(
        "GraphSpec",
        r#"{"axisMax":null,"axisMin":null,"categories":[],"clusterWidth":80.0,"columnWidth":90.0,"connectPoints":true,"edgeToEdge":false,"kind":"column","legend":true,"markPoints":true,"rect":{"x0":0.0,"x1":200.0,"y0":0.0,"y1":150.0},"rows":[[1.0]],"series":[],"seriesPaints":[],"ticks":0}"#,
    );
    pinned::<crate::live::EnvelopeOptions>(
        "EnvelopeOptions",
        r#"{"antiAlias":true,"distortAppearance":false,"distortLinearGradients":false,"distortPatternFills":false,"preserveShape":"clippingMask"}"#,
    );
    pinned::<crate::live_paint::GapOptions>("GapOptions", r#"{"color":[255,79,79],"detect":false,"size":"small"}"#);
    pinned::<crate::metadata::DocMetadata>(
        "DocMetadata",
        r#"{"author":"","authorTitle":"","copyrightNotice":"","copyrightStatus":"unknown","copyrightUrl":"","created":null,"description":"","keywords":[],"modified":null,"rating":0}"#,
    );
    pinned::<crate::rastersettings::RasterEffectsSettings>(
        "RasterEffectsSettings",
        r#"{"addAround":0.0,"antiAlias":true,"background":"transparent","clippingMask":false,"colorModel":"document","preserveSpotColors":true}"#,
    );
}
