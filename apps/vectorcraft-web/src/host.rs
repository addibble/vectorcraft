//! The `postMessage` bridge to a same-origin parent page (`?host=parent`), for pages that embed
//! VectorCraft in an `<iframe>` (such as a Nextcloud app). The protocol is described for host
//! authors in `docs/web-embed.md`; its rules are in `protocol` (tested natively). Messages are plain
//! objects with a `type`:
//!
//! - `vectorcraft:ready` (to the parent): `{ version }`, the app is listening; send a file now.
//! - `vectorcraft:open` (from the parent): `{ name, bytes, title? }`, bytes an `ArrayBuffer` or
//!   `Uint8Array`. It opens like a file picked with File › Open; its tab shows `title`, when given,
//!   in place of `name`. Without a `name` or `bytes`, or with a `title` that isn't a short string,
//!   it is refused with `vectorcraft:failed`.
//! - `vectorcraft:save` (to the parent): `{ id, name, bytes }` (a transferred `ArrayBuffer`) for
//!   every file VectorCraft would otherwise download (`Services::download`): Save, Save As, Save a
//!   Copy and exports. `id` numbers the saves of the page's life. The parent stores it and reports
//!   failures itself.
//! - `vectorcraft:saved` (from the parent): `{ id, name, ok, error? }`, the parent stored save
//!   `id` of `name` (`ok: true`) or couldn't. With `saveOnClose`, closing a modified document saves
//!   it and closes it only on this answer (`vectorcraft_ui_egui::unsaved::host_saved`); any other
//!   answer changes nothing. A malformed one is refused with `vectorcraft:failed`.
//! - `vectorcraft:dirty` (to the parent): `{ dirty }` whenever unsaved changes appear or go away.
//! - `vectorcraft:configure` (from the parent): `{ hide, hideMenus, hideMark, emptyMessage,
//!   emptyLink, saveOnClose, saveRecoveredInPlace }`. `hide` lists command ids (or prefixes ending in `*`) the user isn't offered: they
//!   leave the menus, the command palette and their buttons, and their shortcuts do nothing; the
//!   parent's own control requests still run them. `hideMenus` lists menu bar menus (English
//!   titles) to leave out, their commands still running; `hideMark` hides the brand mark.
//!   `emptyMessage` is what the window says while no document is open, in place of the Home
//!   screen, and `emptyLink` a link after it. `saveOnClose` saves a modified document on closing
//!   instead of asking Save Changes, closing it once the parent answers `vectorcraft:saved`. Each
//!   replaces the last whole; a malformed one is refused with `vectorcraft:failed`.
//! - `vectorcraft:emptyLink` (to the parent): `{}`, the user clicked the empty window's link.
//! - With `&recovery=parent`, Data Recovery keeps its copies with the parent (`parent_store`):
//!   `vectorcraft:recoveryWrite` (to the parent) `{ name, bytes }` (a transferred `ArrayBuffer`)
//!   creates or replaces an entry and `vectorcraft:recoveryRemove` `{ name }` deletes one;
//!   `vectorcraft:recoveryEntries` (from the parent) `{ entries: [{ name, bytes }] }` gives the
//!   entries it keeps (those of earlier pages), which join the ones held, and the copies a crash
//!   left behind among them are offered (Recover Documents). A malformed one is refused with
//!   `vectorcraft:failed`, and none of it is taken.
//! - `vectorcraft:failed` (to the parent): `{ error }`. Before `ready`, the app couldn't start;
//!   after it, the last message from the parent was refused (the app keeps running).
//!
//! Only the parent window on the page's own origin is heard or answered, and a page with an opaque
//! origin (`null`) has no bridge. The parent guards leaving the page with unsaved changes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use vectorcraft_ui_egui::{Services, VectorcraftApp};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use crate::parent_store::{Change, ParentStore};
use crate::protocol;

use vectorcraft_ui_egui::{Inbox, OpenArrival};

#[derive(Clone)]
pub struct Bridge {
    window: web_sys::Window,
    parent: web_sys::Window,
    origin: String,
    /// The unsaved-changes state last reported to the parent.
    reported: Rc<Cell<Option<bool>>>,
    /// The last `vectorcraft:configure` heard, until the next frame applies it ([`Bridge::configure`]).
    configured: Rc<RefCell<Option<protocol::Configure>>>,
    /// The last save's `id` (the first is 1).
    last_save: Rc<Cell<u64>>,
    /// The saves posted during this frame, by id and name ([`Bridge::follow_saves`]).
    posted: Rc<RefCell<Vec<(u64, String)>>>,
    /// The `vectorcraft:saved` answers heard, until the next frame applies them.
    answers: Rc<RefCell<Vec<protocol::Saved>>>,
    /// Data Recovery's store when the parent keeps the copies (`&recovery=parent`).
    recovery: Option<Arc<ParentStore>>,
    /// The parent gave recovery entries the app hasn't offered yet.
    offer: Rc<Cell<bool>>,
}

impl Bridge {
    /// `None` when the page isn't framed (there is no parent to talk to).
    pub fn new() -> Option<Self> {
        let window = web_sys::window()?;
        let parent = window.parent().ok().flatten()?;
        if js_sys::Object::is(&parent, &window) {
            return None;
        }
        let origin = window.location().origin().ok().filter(|o| !protocol::is_opaque(o))?;
        Some(Self {
            window,
            parent,
            origin,
            reported: Rc::default(),
            configured: Rc::default(),
            last_save: Rc::default(),
            posted: Rc::default(),
            answers: Rc::default(),
            recovery: None,
            offer: Rc::default(),
        })
    }

    /// Data Recovery keeps its copies with the parent in `store`, which takes the entries it gives.
    pub fn keep_recovery(&mut self, store: Arc<ParentStore>) {
        self.recovery = Some(store);
    }

    /// Send what VectorCraft would download to the parent instead (each time the services are
    /// made, also after a graphics restart).
    pub fn connect(&self, services: &mut Services) {
        let bridge = self.clone();
        // A download now answers whether it happened; a failed hand-over is reported like one.
        services.download = Some(Box::new(move |name: &str, bytes: &[u8]| {
            bridge.post_save(name, bytes).map_err(|e| format!("couldn't hand {name} to the page: {e}"))
        }));
    }

    pub fn post_ready(&self) {
        self.send(&[("type", "vectorcraft:ready".into()), ("version", env!("CARGO_PKG_VERSION").into())]);
    }

    pub fn post_failed(&self, error: &str) {
        self.send(&[("type", "vectorcraft:failed".into()), ("error", error.into())]);
    }

    /// Before each frame: what the parent last configured, if it said anything since, and its
    /// answers to saves.
    pub fn configure(&self, app: &mut VectorcraftApp) {
        if let Some(c) = self.configured.borrow_mut().take() {
            app.ui.host_hidden = c.hide;
            app.ui.host_hidden_menus = c.hide_menus;
            app.ui.host_hide_mark = c.hide_mark;
            app.ui.host_empty_message = c.empty_message;
            app.ui.host_empty_link = c.empty_link;
            app.ui.host_save_on_close = c.save_on_close;
            app.session.save_recovered_in_place = c.save_recovered_in_place;
        }
        let answers = std::mem::take(&mut *self.answers.borrow_mut());
        for a in answers {
            vectorcraft_ui_egui::unsaved::host_saved(app, a.id, &a.name, a.ok, a.error.as_deref());
        }
        // The copies a crash left behind among the entries given, once no other dialog is up.
        if self.offer.get() && app.ui.dialog.is_none() {
            self.offer.set(false);
            vectorcraft_ui_egui::recovery::offer(app);
        }
    }

    /// After each frame: the saves posted in it, so a close waiting for the parent to store a
    /// document waits for the last save of its file (the one the close made, or a later Save).
    pub fn follow_saves(&self, app: &mut VectorcraftApp) {
        let posted = std::mem::take(&mut *self.posted.borrow_mut());
        for (id, name) in posted {
            vectorcraft_ui_egui::unsaved::host_posted(app, id, &name);
        }
    }

    /// After each frame: tell the parent the user clicked the empty window's link.
    pub fn report_link(&self, app: &mut VectorcraftApp) {
        if std::mem::take(&mut app.ui.host_empty_link_clicked) {
            self.send(&[("type", "vectorcraft:emptyLink".into())]);
        }
    }

    /// After each frame: tell the parent when unsaved changes appear or go away.
    pub fn report_dirty(&self, app: &VectorcraftApp) {
        let dirty = app.session.documents().iter().any(|d| d.is_dirty());
        if self.reported.get() != Some(dirty) && self.send(&[("type", "vectorcraft:dirty".into()), ("dirty", dirty.into())]) {
            self.reported.set(Some(dirty));
        }
    }

    fn post_save(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let id = self.last_save.get().saturating_add(1);
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        // A JavaScript number holds the id exactly: ids never come near 2^53.
        let message = message(&[
            ("type", "vectorcraft:save".into()),
            ("id", JsValue::from_f64(id as f64)),
            ("name", name.into()),
            ("bytes", buffer.clone().into()),
        ])
        .ok_or_else(|| "couldn't build the message".to_string())?;
        self.parent
            .post_message_with_transfer(&message, &self.origin, &js_sys::Array::of1(&buffer))
            .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
        self.last_save.set(id);
        self.posted.borrow_mut().push((id, name.to_string()));
        Ok(())
    }

    /// Hear files from the parent: they go to the open inbox, which outlives graphics restarts.
    /// `wake` asks the current runner for a frame. A `vectorcraft:open`, `vectorcraft:configure` or
    /// `vectorcraft:saved` that can't be read is answered with `vectorcraft:failed`; every other
    /// message is ignored (`protocol::accepts`).
    pub fn listen(&self, inbox: Inbox, wake: impl Fn() + 'static) {
        let bridge = self.clone();
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let from_parent = event.source().is_some_and(|s| js_sys::Object::is(&s, &bridge.parent));
            let data = event.data();
            if !protocol::accepts(&bridge.origin, &event.origin(), from_parent, string(&data, "type").as_deref()) {
                return;
            }
            if string(&data, "type").as_deref() == Some("vectorcraft:configure") {
                let fields = protocol::ConfigureFields {
                    hide: strings(&data, "hide"),
                    hide_menus: strings(&data, "hideMenus"),
                    hide_mark: boolean(&data, "hideMark"),
                    empty_message: field(&data, "emptyMessage"),
                    empty_link: field(&data, "emptyLink"),
                    save_on_close: boolean(&data, "saveOnClose"),
                    save_recovered_in_place: boolean(&data, "saveRecoveredInPlace"),
                };
                match protocol::configure(fields) {
                    Ok(configure) => {
                        *bridge.configured.borrow_mut() = Some(configure);
                        wake();
                    }
                    Err(error) => {
                        log::warn!("refused the page's configuration: {error}");
                        bridge.post_failed(&error);
                    }
                }
                return;
            }
            if string(&data, "type").as_deref() == Some("vectorcraft:recoveryEntries") {
                let Some(store) = &bridge.recovery else {
                    bridge.post_failed("vectorcraft:recoveryEntries needs Data Recovery kept by the page (`&recovery=parent`)");
                    return;
                };
                match protocol::recovery_entries(entries(&data)) {
                    Ok(entries) => {
                        store.give(entries);
                        bridge.offer.set(true);
                        wake();
                    }
                    Err(error) => {
                        log::warn!("refused the page's recovery entries: {error}");
                        bridge.post_failed(&error);
                    }
                }
                return;
            }
            if string(&data, "type").as_deref() == Some("vectorcraft:saved") {
                match protocol::saved(number(&data, "id"), field(&data, "name"), boolean(&data, "ok"), field(&data, "error")) {
                    Ok(saved) => {
                        bridge.answers.borrow_mut().push(saved);
                        wake();
                    }
                    Err(error) => {
                        log::warn!("refused the page's answer to a save: {error}");
                        bridge.post_failed(&error);
                    }
                }
                return;
            }
            match protocol::open(string(&data, "name"), bytes(&data), field(&data, "title")) {
                Ok(open) => {
                    let arrival = OpenArrival::hosted(open.name, open.bytes, open.title);
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(arrival);
                    wake();
                }
                Err(error) => {
                    log::warn!("refused a file from the page: {error}");
                    bridge.post_failed(&error);
                }
            }
        });
        if self.window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
            // The listener lives as long as the page.
            on_message.forget();
        }
    }

    /// Post a message to the parent; false if it couldn't be sent.
    fn send(&self, fields: &[(&str, JsValue)]) -> bool {
        message(fields).is_some_and(|m| self.parent.post_message(&m, &self.origin).is_ok())
    }
}

fn message(fields: &[(&str, JsValue)]) -> Option<js_sys::Object> {
    let m = js_sys::Object::new();
    for (key, value) in fields {
        js_sys::Reflect::set(&m, &JsValue::from_str(key), value).ok()?;
    }
    Some(m)
}

fn string(data: &JsValue, key: &str) -> Option<String> {
    js_sys::Reflect::get(data, &JsValue::from_str(key)).ok()?.as_string()
}

/// A string field: left out (or `null`), a string, or something else.
fn field(data: &JsValue, key: &str) -> protocol::Field<String> {
    match js_sys::Reflect::get(data, &JsValue::from_str(key)) {
        Ok(v) if v.is_undefined() || v.is_null() => protocol::Field::Absent,
        Ok(v) => v.as_string().map_or(protocol::Field::Wrong, protocol::Field::Given),
        Err(_) => protocol::Field::Wrong,
    }
}

/// A number field: left out (or `null`), a number, or something else.
fn number(data: &JsValue, key: &str) -> protocol::Field<f64> {
    match js_sys::Reflect::get(data, &JsValue::from_str(key)) {
        Ok(v) if v.is_undefined() || v.is_null() => protocol::Field::Absent,
        Ok(v) => v.as_f64().map_or(protocol::Field::Wrong, protocol::Field::Given),
        Err(_) => protocol::Field::Wrong,
    }
}

/// A boolean field: left out (or `null`), a boolean, or something else.
fn boolean(data: &JsValue, key: &str) -> protocol::Field<bool> {
    match js_sys::Reflect::get(data, &JsValue::from_str(key)) {
        Ok(v) if v.is_undefined() || v.is_null() => protocol::Field::Absent,
        Ok(v) => v.as_bool().map_or(protocol::Field::Wrong, protocol::Field::Given),
        Err(_) => protocol::Field::Wrong,
    }
}

/// An array-of-strings field: left out (or `null`), such an array (of at most
/// `protocol::MAX_HIDDEN` + 1 read, so a huge one is refused without copying it), or something else.
fn strings(data: &JsValue, key: &str) -> protocol::Field<Vec<String>> {
    let Ok(v) = js_sys::Reflect::get(data, &JsValue::from_str(key)) else { return protocol::Field::Wrong };
    if v.is_undefined() || v.is_null() {
        return protocol::Field::Absent;
    }
    if !js_sys::Array::is_array(&v) {
        return protocol::Field::Wrong;
    }
    let array = js_sys::Array::from(&v);
    let mut out = Vec::new();
    for item in array.iter().take(protocol::MAX_HIDDEN + 1) {
        let Some(s) = item.as_string() else { return protocol::Field::Wrong };
        out.push(s);
    }
    protocol::Field::Given(out)
}

/// Post a change to Data Recovery's entries to the parent (`ParentStore`'s post): from the
/// window itself, since the store holds no browser objects.
pub fn post_recovery(change: Change) -> Result<(), String> {
    let js = |e: JsValue| e.as_string().unwrap_or_else(|| format!("{e:?}"));
    let window = web_sys::window().ok_or("no window")?;
    let parent = window.parent().map_err(js)?.ok_or("no parent page")?;
    let origin = window.location().origin().map_err(js)?;
    match change {
        Change::Write { name, bytes } => {
            let buffer = js_sys::Uint8Array::from(bytes).buffer();
            let m = message(&[("type", "vectorcraft:recoveryWrite".into()), ("name", name.into()), ("bytes", buffer.clone().into())])
                .ok_or("couldn't build the message")?;
            parent.post_message_with_transfer(&m, &origin, &js_sys::Array::of1(&buffer)).map_err(js)
        }
        Change::Remove { name } => {
            let m = message(&[("type", "vectorcraft:recoveryRemove".into()), ("name", name.into())]).ok_or("couldn't build the message")?;
            parent.post_message(&m, &origin).map_err(js)
        }
    }
}

/// A `vectorcraft:recoveryEntries`' `entries`: each `{ name, bytes }` as far as it reads (`None`:
/// not an array, or one too long to read: `protocol::recovery_entries` refuses it).
fn entries(data: &JsValue) -> Option<Vec<protocol::EntryFields>> {
    let v = js_sys::Reflect::get(data, &JsValue::from_str("entries")).ok()?;
    if !js_sys::Array::is_array(&v) {
        return None;
    }
    let array = js_sys::Array::from(&v);
    if array.length() as usize > protocol::MAX_RECOVERY_ENTRIES {
        return None;
    }
    Some(array.iter().map(|e| (string(&e, "name"), bytes(&e))).collect())
}

/// `bytes` as an `ArrayBuffer` or `Uint8Array`.
fn bytes(data: &JsValue) -> Option<Vec<u8>> {
    let bytes = js_sys::Reflect::get(data, &JsValue::from_str("bytes")).ok()?;
    if let Some(array) = bytes.dyn_ref::<js_sys::Uint8Array>() {
        Some(array.to_vec())
    } else if bytes.is_instance_of::<js_sys::ArrayBuffer>() {
        Some(js_sys::Uint8Array::new(&bytes).to_vec())
    } else {
        None
    }
}
