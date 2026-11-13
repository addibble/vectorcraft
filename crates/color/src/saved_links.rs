//! A linked colour written by its swatch's name: while a document is written as a saved file holds
//! it, it lends its paints the colours of its global swatches ([`with`]), and a solid paint linked
//! to one, whose colour is exactly that swatch's at the paint's tint, is written as the swatch's
//! name (`"fill": "Accent"`) or, at another tint, as `{"type": "solid", "swatch", "tint"}` without
//! its colour. Read back, such a paint holds [`Color::UNRESOLVED`] until the document fills in its
//! swatch's colour, before anything else sees it.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Color, GradientPaint, Paint, full_tint, is_full_tint, is_identity};

// [`Paint`] as a saved file writes it as an object, and the schema of those objects. serde's
// remote derive takes apart and builds a `Paint` field by field, so this mirror can't drift from
// it: a field or variant added to one and not the other fails to compile. Its doc comments are
// the schema's descriptions, which agents read.
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
#[serde(remote = "Paint", rename = "Paint", tag = "type", rename_all = "lowercase")]
enum PaintDef {
    /// No paint.
    None,
    /// A solid colour, linked to a global swatch or not.
    Solid {
        /// The colour shown: with a link, the swatch's colour at `tint`. Left out in a saved file
        /// when it is exactly that (the swatch gives it); required without `swatch`.
        #[serde(default = "Color::unresolved")]
        color: Color,
        /// Name of the global swatch this colour is linked to (edits to the swatch update it).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        swatch: Option<String>,
        /// Tint of the linked swatch (0..1; 1 without a link): a spot tint prints this much of its
        /// plate, and swatch edits keep it.
        #[serde(default = "full_tint", skip_serializing_if = "is_full_tint")]
        tint: f32,
    },
    Gradient(Box<GradientPaint>),
    /// A pattern swatch, by name.
    Pattern {
        pattern: String,
        /// Pattern space → document placement (Transform Patterns edits it; identity = tiles
        /// anchored at the document origin).
        #[serde(default, skip_serializing_if = "is_identity")]
        xf: kurbo::Affine,
    },
}

/// A paint is one of the paint objects or, for a solid colour linked to a global swatch at 100%,
/// that swatch's name alone (`"fill": "Accent"`), as the reader takes it: the objects' own choice
/// with the name as one more branch, so a schema walker picks the branch a value is in one step.
impl schemars::JsonSchema for Paint {
    fn schema_name() -> String {
        "Paint".into()
    }

    fn json_schema(generator: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        use schemars::schema::{InstanceType, ObjectValidation, Schema, SchemaObject};
        let tagged = |schema: &SchemaObject, tag: &str| {
            schema
                .object
                .as_ref()
                .and_then(|o| o.properties.get("type"))
                .is_some_and(|t| matches!(t, Schema::Object(t) if t.enum_values.as_deref() == Some(&[serde_json::Value::from(tag)])))
        };
        let mut paint = <PaintDef as schemars::JsonSchema>::json_schema(generator).into_object();
        if let Some(branches) = paint.subschemas.as_mut().and_then(|s| s.one_of.as_mut()) {
            for branch in branches.iter_mut() {
                let Schema::Object(solid) = branch else { continue };
                if !tagged(solid, "solid") {
                    continue;
                }
                // The colour's default is the unresolved colour the swatch fills in, which no
                // writer may give.
                if let Some(Schema::Object(color)) = solid.object().properties.get_mut("color") {
                    color.metadata().default = None;
                }
                // A solid colour needs its colour unless it names a swatch, as the reader does.
                let names_a_swatch = SchemaObject {
                    object: Some(Box::new(ObjectValidation {
                        properties: [(
                            "swatch".to_string(),
                            SchemaObject { instance_type: Some(InstanceType::String.into()), ..SchemaObject::default() }.into(),
                        )]
                        .into(),
                        required: ["swatch".to_string()].into(),
                        ..ObjectValidation::default()
                    })),
                    ..SchemaObject::default()
                };
                let has_a_colour = SchemaObject {
                    object: Some(Box::new(ObjectValidation { required: ["color".to_string()].into(), ..ObjectValidation::default() })),
                    ..SchemaObject::default()
                };
                let subschemas = solid.subschemas();
                subschemas.if_schema = Some(Box::new(names_a_swatch.into()));
                subschemas.else_schema = Some(Box::new(has_a_colour.into()));
            }
            let mut name = SchemaObject { instance_type: Some(InstanceType::String.into()), ..SchemaObject::default() };
            name.metadata().description = Some(
                "The name of a global swatch: a solid colour linked to it at 100%, which takes its colour from the swatch. Every string is read as a swatch's name; for no paint, write {\"type\": \"none\"}.".into(),
            );
            branches.insert(0, name.into());
        }
        debug_assert!(
            paint.subschemas.as_ref().and_then(|s| s.one_of.as_ref()).is_some_and(|b| b.len() == 5),
            "PaintDef's schema is no longer a choice of the four tagged objects; the name branch and the solid's rule weren't added"
        );
        paint.metadata().description =
            Some("What fills or strokes an object: a paint object, or a global swatch's name for a solid colour linked to it at 100%.".into());
        paint.into()
    }
}

thread_local! {
    static LINKS: RefCell<Option<BTreeMap<String, Color>>> = const { RefCell::new(None) };
    static READ: Cell<u64> = const { Cell::new(0) };
    static WHOLE: Cell<bool> = const { Cell::new(false) };
}

/// `f` run with every colour written whole, whatever a document lends: for files readers that do
/// not know names open.
pub fn written_whole<R>(f: impl FnOnce() -> R) -> R {
    struct Back(bool);
    impl Drop for Back {
        fn drop(&mut self) {
            WHOLE.with(|w| w.set(self.0));
        }
    }
    let _back = Back(WHOLE.with(|w| w.replace(true)));
    f()
}

/// How many linked colours have been read without their value on this thread, ever: a reader
/// compares it before and after reading to learn whether it has any to fill in.
pub fn unresolved_read() -> u64 {
    READ.with(Cell::get)
}

pub(crate) fn note_unresolved() -> Color {
    READ.with(|r| r.set(r.get().wrapping_add(1)));
    Color::UNRESOLVED
}

/// Restores the previous table when dropped, so a panic inside [`with`] cannot leave one lent.
struct Restore(Option<BTreeMap<String, Color>>);
impl Drop for Restore {
    fn drop(&mut self) {
        let prev = self.0.take();
        LINKS.with(|l| *l.borrow_mut() = prev);
    }
}

/// `f` run with `links` (each global swatch's name and the colour a paint linked to it at 100%
/// holds) lent to the paints it serializes, so a linked paint whose colour follows from its link
/// is written by name.
pub fn with<R>(links: BTreeMap<String, Color>, f: impl FnOnce() -> R) -> R {
    let _restore = Restore(LINKS.with(|l| l.borrow_mut().replace(links)));
    f()
}

/// Whether a paint linked to `name` at `tint` holding `color` is written by name: its colour is
/// exactly what the link gives, so a reader gets it back from the swatch.
fn follows_from_link(name: &str, color: Color, tint: f32) -> bool {
    !WHOLE.with(Cell::get) && LINKS.with(|l| l.borrow().as_ref().and_then(|t| t.get(name)).is_some_and(|base| base.tinted(tint) == color))
}

impl Serialize for Paint {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if let Paint::Solid { color, swatch: Some(name), tint } = self
            && follows_from_link(name, *color, *tint)
        {
            if *tint == 1.0 {
                return s.serialize_str(name);
            }
            let mut m = s.serialize_struct("Paint", 3)?;
            m.serialize_field("type", "solid")?;
            m.serialize_field("swatch", name)?;
            m.serialize_field("tint", tint)?;
            return m.end();
        }
        PaintDef::serialize(self, s)
    }
}

impl<'de> Deserialize<'de> for Paint {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Paint;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a paint, or the name of the global swatch a solid colour links to")
            }
            fn visit_str<E: de::Error>(self, name: &str) -> Result<Paint, E> {
                Ok(Paint::Solid { color: note_unresolved(), swatch: Some(name.to_string()), tint: 1.0 })
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Paint, A::Error> {
                let paint = PaintDef::deserialize(de::value::MapAccessDeserializer::new(map))?;
                // Only a linked colour may leave out its value: its swatch gives it.
                if let Paint::Solid { color, swatch: None, .. } = &paint
                    && color.is_unresolved()
                {
                    return Err(de::Error::missing_field("color"));
                }
                Ok(paint)
            }
        }
        d.deserialize_any(V)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;

    fn accent() -> Color {
        Color::Rgb { r: 0.6, g: 0.1, b: 0.1 }
    }

    fn linked(tint: f32) -> Paint {
        Paint::Solid { color: accent().tinted(tint), swatch: Some("Accent".into()), tint }
    }

    fn table() -> BTreeMap<String, Color> {
        BTreeMap::from([("Accent".to_string(), accent())])
    }

    #[test]
    fn a_linked_colour_is_written_by_its_swatchs_name_and_read_back_unresolved() {
        let saved = with(table(), || serde_json::to_value(linked(1.0))).unwrap();
        assert_eq!(saved, json!("Accent"));
        let read: Paint = serde_json::from_value(saved).unwrap();
        assert!(matches!(&read, Paint::Solid { color, swatch: Some(s), tint } if color.is_unresolved() && s == "Accent" && *tint == 1.0));
        // At another tint it keeps the tint and leaves out the colour.
        let saved = with(table(), || serde_json::to_value(linked(0.4))).unwrap();
        assert_eq!(saved, json!({"type": "solid", "swatch": "Accent", "tint": 0.4f32}));
        let before = unresolved_read();
        let read: Paint = serde_json::from_value(saved).unwrap();
        assert!(read.color().is_some_and(|c| c.is_unresolved()));
        assert_eq!(unresolved_read(), before + 1, "a reader learns it has a colour to fill in");
    }

    #[test]
    fn a_colour_its_link_does_not_give_keeps_its_value() {
        let off = Paint::Solid { color: Color::Rgb { r: 0.5, g: 0.1, b: 0.1 }, swatch: Some("Accent".into()), tint: 1.0 };
        let saved = with(table(), || serde_json::to_value(&off)).unwrap();
        assert!(saved["color"].is_object(), "{saved}");
        assert_eq!(serde_json::from_value::<Paint>(saved).unwrap(), off);
        // A link to a swatch the table lacks is written whole too.
        let other = Paint::Solid { color: accent(), swatch: Some("Other".into()), tint: 1.0 };
        assert!(with(table(), || serde_json::to_value(&other)).unwrap()["color"].is_object());
    }

    #[test]
    fn a_solid_paint_without_its_colour_or_a_link_is_refused() {
        let err = serde_json::from_value::<Paint>(json!({"type": "solid"})).unwrap_err().to_string();
        assert!(err.contains("missing field `color`"), "{err}");
        assert!(serde_json::from_value::<Paint>(json!({"type": "solid", "swatch": "Accent"})).is_ok());
    }

    #[test]
    fn without_a_table_every_paint_is_written_whole() {
        let saved = serde_json::to_value(linked(1.0)).unwrap();
        assert_eq!(saved["type"], "solid");
        assert!(saved["color"].is_object());
        assert_eq!(serde_json::from_value::<Paint>(saved).unwrap(), linked(1.0));
        // And the table ends with the call, even when it panics.
        let _ = std::panic::catch_unwind(|| with(table(), || panic!("inside")));
        assert!(serde_json::to_value(linked(1.0)).unwrap().is_object());
    }

    /// The schema describes what the reader takes: a swatch's name alone, or a paint object of one
    /// of the four kinds, all branches of one choice; a solid object needs its colour unless it
    /// names a swatch, and no default it advertises is one the reader refuses.
    #[test]
    fn the_schema_takes_a_swatch_name_or_a_paint_object() {
        let schema = serde_json::to_value(schemars::schema_for!(Paint)).unwrap();
        let branches = schema["oneOf"].as_array().expect("a paint is one choice");
        assert_eq!(branches[0]["type"], "string");
        let kinds: Vec<&str> = branches[1..].iter().filter_map(|v| v["properties"]["type"]["enum"][0].as_str()).collect();
        assert_eq!(kinds, ["none", "solid", "gradient", "pattern"]);
        assert!(schema["definitions"].get("PaintObject").is_none() && schema["definitions"].get("PaintDef").is_none());
        let solid = &branches[2];
        assert_eq!(solid["else"], json!({"required": ["color"]}));
        assert_eq!(solid["if"]["required"], json!(["swatch"]));
        assert_eq!(solid["if"]["properties"]["swatch"]["type"], "string", "a null swatch names none: {solid}");
        assert!(solid["properties"]["color"].get("default").is_none(), "no unresolved colour as a default: {solid}");
        // Each form the writer produces is the form its branch describes.
        let links = BTreeMap::from([("Accent".to_string(), accent())]);
        let by_name = with(links.clone(), || serde_json::to_value(linked(1.0)).unwrap());
        assert!(by_name.is_string());
        let tinted = with(links, || serde_json::to_value(linked(0.4)).unwrap());
        assert_eq!(tinted["type"], "solid");
        assert!(tinted.get("color").is_none(), "a linked colour leaves its value to the swatch");
    }
}
