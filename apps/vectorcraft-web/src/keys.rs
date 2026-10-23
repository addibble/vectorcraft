//! Chords the browser would take from the app: the View menu's zoom chords (`Cmd+=`, `Cmd+-`,
//! `Cmd+0`, `Cmd+Alt+0`) are also the browser's page zoom, and eframe lets them through, so Safari
//! zoomed the whole page instead of the canvas. The page's default action is cancelled for them;
//! the key still reaches egui, which runs the command.

/// Whether a key press with these modifiers is a browser page-zoom chord: `key` is the event's
/// `key` (the character typed, which the browser matches) and `code` its `code` (the physical key,
/// for layouts where the character differs).
pub(crate) fn browser_zoom(key: &str, code: &str, ctrl: bool, meta: bool) -> bool {
    (ctrl || meta)
        && (matches!(key, "=" | "+" | "-" | "_" | "0") || matches!(code, "Equal" | "Minus" | "Digit0" | "NumpadAdd" | "NumpadSubtract" | "Numpad0"))
}

/// Cancel the page's default action for [`browser_zoom`] chords, before eframe's listener sees
/// the event (it doesn't stop it).
#[cfg(target_arch = "wasm32")]
pub(crate) fn keep_zoom_keys() {
    use wasm_bindgen::JsCast as _;
    let Some(window) = web_sys::window() else { return };
    let on_key = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(|e: web_sys::KeyboardEvent| {
        if browser_zoom(&e.key(), &e.code(), e.ctrl_key(), e.meta_key()) {
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
            assert!(browser_zoom(key, code, false, true), "Cmd with {key} ({code})");
            assert!(browser_zoom(key, code, true, false), "Ctrl with {key} ({code})");
        }
        // German's `+` key, where US has `]`: the character decides.
        assert!(browser_zoom("+", "BracketRight", false, true));
    }

    #[test]
    fn other_keys_and_plain_presses_stay_with_the_browser() {
        for (key, code, ctrl, meta) in [
            ("=", "Equal", false, false),
            ("0", "Digit0", false, false),
            ("r", "KeyR", false, true),
            ("c", "KeyC", true, false),
            ("1", "Digit1", false, true),
        ] {
            assert!(!browser_zoom(key, code, ctrl, meta), "{key} ({code}) ctrl {ctrl} meta {meta}");
        }
    }
}
