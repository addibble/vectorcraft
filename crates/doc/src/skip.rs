use std::any::Any;
use std::cell::Cell;

/// Whether the model is being serialized as a saved file holds it, on this thread ([`crate::as_saved`]).
pub fn saving() -> bool {
    vectorcraft_geom::saved::saving()
}

thread_local! {
    static DESCRIBING: Cell<bool> = const { Cell::new(false) };
}

/// `f` run while the model's JSON Schema is derived (schemars): the predicates here then leave
/// nothing out, so the schema keeps each field's default, which schemars drops when a field's
/// `skip_serializing_if` would leave the default out.
pub fn describing<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            DESCRIBING.with(|c| c.set(self.0));
        }
    }
    let _restore = Restore(DESCRIBING.with(|c| c.replace(true)));
    f()
}

fn writing() -> bool {
    !DESCRIBING.with(Cell::get)
}

/// `v` is `default`, bit for bit for a number: -0.0 is not left out as 0.0, so it reads back as
/// it was written.
pub fn same<T: PartialEq + 'static>(v: &T, default: &T) -> bool {
    let (a, b): (&dyn Any, &dyn Any) = (v, default);
    match (a.downcast_ref::<f64>(), b.downcast_ref::<f64>(), a.downcast_ref::<f32>(), b.downcast_ref::<f32>()) {
        (Some(x), Some(y), ..) => x.to_bits() == y.to_bits(),
        (.., Some(x), Some(y)) => x.to_bits() == y.to_bits(),
        _ => v == default,
    }
}

/// While saving ([`saving`]), whether `v` is `default`, the value a reader fills in when the
/// field is missing: a `skip_serializing_if` predicate's body.
pub fn saved_as<T: PartialEq + 'static>(v: &T, default: &T) -> bool {
    saving() && same(v, default)
}

pub fn left_out<T: Default + PartialEq + 'static>(v: &T) -> bool {
    saved_as(v, &T::default())
}
pub fn true_left_out(v: &bool) -> bool {
    saving() && *v
}
pub fn one_left_out(v: &f32) -> bool {
    saved_as(v, &1.0)
}
/// The `skip_serializing_if` predicates of a struct read with `#[serde(default)]`, one per field
/// and named after it: true, while the model is serialized as saved ([`crate::as_saved`]), when the field holds
/// the value the struct's `Default` gives it, which is what a reader fills in when the field is
/// missing.
macro_rules! container_defaults {
    ($ty:ty { $($field:ident: $fty:ty),* $(,)? }) => {
        /// The struct's `Default`, made once.
        fn defaults() -> &'static $ty {
            static DEFAULTS: std::sync::OnceLock<$ty> = std::sync::OnceLock::new();
            DEFAULTS.get_or_init(Default::default)
        }
        $(
            pub(crate) fn $field(v: &$fty) -> bool {
                crate::skip::saving() && crate::skip::same(v, &defaults().$field)
            }
        )*
    };
}
pub(crate) use container_defaults;
// The predicates below leave a field out of every serialization, as they did before saved files
// left out defaults; while the schema is derived ([`describing`]) they leave out nothing.
pub fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    writing() && *v == T::default()
}
pub fn is_true(v: &bool) -> bool {
    writing() && *v
}
pub fn is_false(v: &bool) -> bool {
    writing() && !*v
}
pub fn is_one(v: &f32) -> bool {
    writing() && *v == 1.0
}
pub fn is_empty_vec<T>(v: &[T]) -> bool {
    writing() && v.is_empty()
}
pub fn is_empty_map(v: &serde_json::Map<String, serde_json::Value>) -> bool {
    writing() && v.is_empty()
}
/// Read a field's value and drop it, giving `None`: for state that is never taken from a file.
pub fn read_as_none<'de, D: serde::Deserializer<'de>, T>(d: D) -> Result<Option<T>, D::Error> {
    <serde::de::IgnoredAny as serde::Deserialize>::deserialize(d)?;
    Ok(None)
}
