//! Chords the browser would take from the app. The View menu's zoom chords (`Cmd+=`, `Cmd+-`,
//! `Cmd+0`, `Cmd+Alt+0`) are also the browser's page zoom, and Edit's `Cmd+Z` and `Cmd+Shift+Z`
//! its own Undo and Redo (in Safari, Undo Close Tab when the page has nothing to undo). eframe
//! lets them through, so Safari zoomed the whole page instead of the canvas and undid in the
//! browser as well as in the document. The page's default action is cancelled for them; the key
//! still reaches egui, which runs the command (and a text field's own undo).

/// Whether a key press with these modifiers is a chord the browser would also act on: page zoom,
/// undo or redo. `key` is the event's `key` (the character typed, which the browser matches) and
/// `code` its `code` (the physical key, for layouts where the character differs).
pub(crate) fn kept_from_browser(key: &str, code: &str, ctrl: bool, meta: bool) -> bool {
    (ctrl || meta)
        && (matches!(key, "=" | "+" | "-" | "_" | "0" | "z" | "Z")
            || matches!(code, "Equal" | "Minus" | "Digit0" | "NumpadAdd" | "NumpadSubtract" | "Numpad0" | "KeyZ"))
}

/// Cancel the page's default action for [`kept_from_browser`] chords, before eframe's listener
/// sees the event (it doesn't stop it).
#[cfg(target_arch = "wasm32")]
pub(crate) fn keep_app_keys() {
    use wasm_bindgen::JsCast as _;
    let Some(window) = web_sys::window() else { return };
    let on_key = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(|e: web_sys::KeyboardEvent| {
        if kept_from_browser(&e.key(), &e.code(), e.ctrl_key(), e.meta_key()) {
            e.prevent_default();
        }
    });
    if let Err(e) = window.add_event_listener_with_callback_and_bool("keydown", on_key.as_ref().unchecked_ref(), true) {
        log::error!("couldn't follow keydown events: {e:?}");
    }
    // The listener lives as long as the page.
    on_key.forget();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_chords_are_kept_from_the_browser() {
        for (key, code) in [
            ("=", "Equal"),
            ("+", "Equal"),
            ("-", "Minus"),
            ("_", "Minus"),
            ("0", "Digit0"),
            ("º", "Digit0"),
            ("+", "NumpadAdd"),
            ("-", "NumpadSubtract"),
        ] {
            assert!(kept_from_browser(key, code, false, true), "Cmd with {key} ({code})");
            assert!(kept_from_browser(key, code, true, false), "Ctrl with {key} ({code})");
        }
        // German's `+` key, where US has `]`: the character decides.
        assert!(kept_from_browser("+", "BracketRight", false, true));
    }

    #[test]
    fn undo_and_redo_are_kept_from_the_browser() {
        // Cmd+Z, and Cmd+Shift+Z, whose key is the capital.
        for key in ["z", "Z"] {
            assert!(kept_from_browser(key, "KeyZ", false, true), "Cmd with {key}");
            assert!(kept_from_browser(key, "KeyZ", true, false), "Ctrl with {key}");
        }
        // German's Z is where US has Y: the character decides.
        assert!(kept_from_browser("z", "KeyY", false, true));
    }

    #[test]
    fn other_keys_and_plain_presses_stay_with_the_browser() {
        for (key, code, ctrl, meta) in [
            ("=", "Equal", false, false),
            ("0", "Digit0", false, false),
            ("z", "KeyZ", false, false),
            ("r", "KeyR", false, true),
            ("c", "KeyC", true, false),
            ("v", "KeyV", false, true),
            ("1", "Digit1", false, true),
        ] {
            assert!(!kept_from_browser(key, code, ctrl, meta), "{key} ({code}) ctrl {ctrl} meta {meta}");
        }
    }
}
