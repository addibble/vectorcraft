use std::cell::Cell;

thread_local! {
    static SAVING: Cell<bool> = const { Cell::new(false) };
}

/// Whether the model is being serialized as a saved file holds it, on this thread ([`crate::as_saved`]).
pub fn saving() -> bool {
    SAVING.with(Cell::get)
}

/// Restores the previous mode when dropped, so a panic inside [`crate::as_saved`] cannot
/// leave it on.
pub(crate) struct Restore(pub(crate) bool);
impl Drop for Restore {
    fn drop(&mut self) {
        SAVING.with(|c| c.set(self.0));
    }
}
pub(crate) fn enter() -> Restore {
    Restore(SAVING.with(|c| c.replace(true)))
}

pub fn left_out<T: Default + PartialEq>(v: &T) -> bool {
    saving() && is_default(v)
}
pub fn true_left_out(v: &bool) -> bool {
    saving() && *v
}
pub fn one_left_out(v: &f32) -> bool {
    saving() && is_one(v)
}
/// The `skip_serializing_if` predicates of a struct read with `#[serde(default)]`, one per field
/// and named after it: true, while the model is serialized as saved ([`crate::as_saved`]), when the field holds
/// the value the struct's `Default` gives it, which is what a reader fills in when the field is
/// missing.
macro_rules! container_defaults {
    ($ty:ty { $($field:ident: $fty:ty),* $(,)? }) => {
        $(
            pub(crate) fn $field(v: &$fty) -> bool {
                crate::skip::saving() && *v == <$ty as Default>::default().$field
            }
        )*
    };
}
pub(crate) use container_defaults;
pub fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}
pub fn is_true(v: &bool) -> bool {
    *v
}
pub fn is_one(v: &f32) -> bool {
    *v == 1.0
}
pub fn is_empty_vec<T>(v: &[T]) -> bool {
    v.is_empty()
}
pub fn is_empty_map(v: &serde_json::Map<String, serde_json::Value>) -> bool {
    v.is_empty()
}
