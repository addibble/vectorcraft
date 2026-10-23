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

use crate::{Color, Paint};

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
        Paint::serialize(self, s)
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
                let paint = Paint::deserialize(de::value::MapAccessDeserializer::new(map))?;
                // Only a linked colour may leave out its value: its swatch gives it.
                if let Paint::Solid { color, swatch: None, .. } = &paint
                    && color.is_unresolved()
                {
                    return Err(de::Error::custom("a solid paint needs its color, or the swatch it links to"));
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
        assert!(err.contains("needs its color"), "{err}");
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
}
