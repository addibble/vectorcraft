//! Whether the model is being serialized as a saved file holds it, on this thread.
//!
//! The native format's writer serializes a document inside [`as_saved`] (re-exported as
//! `vectorcraft_doc::as_saved`), and so does anything that shows a document as its file holds it.
//! Types then write their saved form: fields at the value a reader fills in are left out, and
//! geometry is written in SVG's notation ([`crate::notation`]). Outside it every type writes its
//! complete structured form, which command answers, the clipboard and the journal use. Readers
//! accept both forms whatever the mode.

use std::cell::Cell;

thread_local! {
    static SAVING: Cell<bool> = const { Cell::new(false) };
    static STRUCTURED: Cell<bool> = const { Cell::new(false) };
}

/// Whether the model is being serialized as a saved file holds it, on this thread ([`as_saved`]).
pub fn saving() -> bool {
    SAVING.with(Cell::get)
}

/// Whether geometry is written in SVG's notation ([`crate::notation`]): while saving, unless
/// inside [`structured_geometry`].
pub fn svg_geometry() -> bool {
    saving() && !STRUCTURED.with(Cell::get)
}

/// Restores a flag's previous value when dropped, so a panic inside [`as_saved`] or
/// [`structured_geometry`] cannot leave it set.
struct Restore(&'static std::thread::LocalKey<Cell<bool>>, bool);

impl Drop for Restore {
    fn drop(&mut self) {
        self.0.with(|c| c.set(self.1));
    }
}

/// `f` run with the model serializing as a saved file holds it ([`saving`] is true inside).
pub fn as_saved<R>(f: impl FnOnce() -> R) -> R {
    let _restore = Restore(&SAVING, SAVING.with(|c| c.replace(true)));
    f()
}

/// `f` run with geometry written in its structured form even while saving: for files of a
/// version whose readers don't know SVG's notation.
pub fn structured_geometry<R>(f: impl FnOnce() -> R) -> R {
    let _restore = Restore(&STRUCTURED, STRUCTURED.with(|c| c.replace(true)));
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_is_on_inside_and_restored_after() {
        assert!(!saving());
        assert!(as_saved(|| saving() && as_saved(saving)));
        assert!(!saving());
        let caught = std::panic::catch_unwind(|| as_saved(|| panic!("inside")));
        assert!(caught.is_err());
        assert!(!saving());
    }

    #[test]
    fn structured_geometry_turns_only_the_notation_off() {
        assert!(!svg_geometry());
        as_saved(|| {
            assert!(svg_geometry());
            structured_geometry(|| assert!(saving() && !svg_geometry()));
            assert!(svg_geometry());
        });
    }
}
