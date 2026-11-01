//! The rules of the embedding protocol (`host`, `docs/web-embed.md`), kept free of browser types so
//! they are tested natively: which messages the bridge reads, and what a `vectorcraft:open`, a
//! `vectorcraft:configure`, a `vectorcraft:saved`, a `vectorcraft:recoveryEntries` and a
//! `vectorcraft:taskBar` must carry.

use vectorcraft_ui_egui::host_panels::{HostButton, HostItem, HostListItem, HostPanel, HostPanelAnchor, Tone};
use vectorcraft_ui_egui::{HostTaskBarAction, HostTaskBarWhen};

/// What a `vectorcraft:open` delivers: a file to open like one picked with File › Open.
#[derive(Debug, PartialEq, Eq)]
pub struct Open {
    /// The file name: the document's name, and the name it is saved under.
    pub name: String,
    pub bytes: Vec<u8>,
    /// What the document's tab shows in place of the name.
    pub title: Option<String>,
}

/// The messages the parent may send.
pub const FROM_PARENT: [&str; 8] = [
    "vectorcraft:open",
    "vectorcraft:configure",
    "vectorcraft:saved",
    "vectorcraft:recoveryEntries",
    "vectorcraft:taskBar",
    "vectorcraft:panel",
    "vectorcraft:region",
    "vectorcraft:masks",
];

/// Whether the bridge reads a message at all: one of [`FROM_PARENT`] from the parent window, on
/// the page's own origin. Anything else is ignored without a reply, so another page learns nothing.
///
/// `own_origin` is the page's origin and `origin` the event's; `from_parent` says whether the
/// event's source is the parent window; `kind` is the message's `type`, when it is a string. An
/// opaque origin (`"null"`: a sandboxed frame, a `file:` page) can't be told apart from another
/// page's, so the bridge hears nothing there.
pub fn accepts(own_origin: &str, origin: &str, from_parent: bool, kind: Option<&str>) -> bool {
    from_parent && !is_opaque(own_origin) && origin == own_origin && kind.is_some_and(|k| FROM_PARENT.contains(&k))
}

/// An origin that names no site (`"null"`).
pub fn is_opaque(origin: &str) -> bool {
    origin.is_empty() || origin == "null"
}

/// Check a `vectorcraft:open` the bridge accepted: it needs a file name and the file's bytes (an
/// empty file is passed on: opening it reports the problem like any unreadable file), and may give
/// the tab a `title` ([`title`]). The error is the text of the `vectorcraft:failed` reply.
pub fn open(name: Option<String>, bytes: Option<Vec<u8>>, title: Field<String>) -> Result<Open, String> {
    let Some(name) = name.filter(|n| !n.trim().is_empty()) else {
        return Err("vectorcraft:open needs the file's name (`name`, a non-empty string)".to_string());
    };
    let Some(bytes) = bytes else {
        return Err(format!("vectorcraft:open of {name} needs the file's contents (`bytes`, an ArrayBuffer or Uint8Array)"));
    };
    let title = self::title(title).map_err(|e| format!("vectorcraft:open of {name}: {e}"))?;
    Ok(Open { name, bytes, title })
}

/// The longest title a document's tab is given.
pub const MAX_TITLE_CHARS: usize = 200;

/// A title for a document's tab, shown in place of its file name: left out (or blank) for none,
/// else a string of at most [`MAX_TITLE_CHARS`]. Also `window.vectorcraft.openFile`'s `title`.
pub fn title(title: Field<String>) -> Result<Option<String>, String> {
    match title {
        Field::Absent => Ok(None),
        Field::Given(t) if t.chars().count() <= MAX_TITLE_CHARS => Ok(Some(t.trim().to_string()).filter(|t| !t.is_empty())),
        _ => Err(format!("`title` is a string of at most {MAX_TITLE_CHARS} characters")),
    }
}

/// The most commands (or menus) a page may hide, the longest id or title, and the longest message
/// or link: a page's message is untrusted input.
pub const MAX_HIDDEN: usize = 1_000;
pub const MAX_ID_CHARS: usize = 200;
pub const MAX_MESSAGE_CHARS: usize = 1_000;

/// What a `vectorcraft:configure` sets: the commands, menus and brand mark hidden from the user, and
/// the empty window's message and link. Each message replaces the last whole, so a field left out
/// is cleared.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Configure {
    /// Command ids, or id prefixes ending in `*` (`file.openRecent*`).
    pub hide: Vec<String>,
    /// The menu bar's menus, by their English titles (`File`, `Help`).
    pub hide_menus: Vec<String>,
    /// The application bar's brand mark is hidden.
    pub hide_mark: bool,
    /// Shown in the window while no document is open, in place of the Home screen.
    pub empty_message: Option<String>,
    /// A link after the message; a click on it is posted to the parent as `vectorcraft:emptyLink`.
    pub empty_link: Option<String>,
    /// Closing a modified document saves it and closes it once the parent answers
    /// `vectorcraft:saved`, in place of asking Save Changes.
    pub save_on_close: bool,
    /// Save writes a copy Data Recovery restored under its own name, as any document, rather than
    /// asking where: the host merges each save with what changed since.
    pub save_recovered_in_place: bool,
}

/// A field of a message: left out (or `null`), of the type it should be, or of another.
#[derive(Debug, Default)]
pub enum Field<T> {
    #[default]
    Absent,
    Given(T),
    Wrong,
}

/// A `vectorcraft:configure`'s fields as the page sent them (`hide`, `hideMenus`, `hideMark`,
/// `emptyMessage`, `emptyLink`, `saveOnClose`, `saveRecoveredInPlace`). Fields it doesn't name are
/// never read.
#[derive(Debug, Default)]
pub struct ConfigureFields {
    pub hide: Field<Vec<String>>,
    pub hide_menus: Field<Vec<String>>,
    pub hide_mark: Field<bool>,
    pub empty_message: Field<String>,
    pub empty_link: Field<String>,
    pub save_on_close: Field<bool>,
    pub save_recovered_in_place: Field<bool>,
}

/// Check a `vectorcraft:configure` the bridge accepted. `hide` is an array of strings, each a
/// command id or a prefix ending in `*`; `hideMenus` an array of menu titles; `hideMark` and
/// `saveOnClose` booleans; `emptyMessage` and `emptyLink` strings. The error is the text of the `vectorcraft:failed`
/// reply, and nothing changes.
pub fn configure(f: ConfigureFields) -> Result<Configure, String> {
    let names = |field: Field<Vec<String>>, key: &str, what: &str| match field {
        Field::Absent => Ok(Vec::new()),
        Field::Given(ids) if ids.len() <= MAX_HIDDEN && ids.iter().all(|id| !id.trim().is_empty() && id.chars().count() <= MAX_ID_CHARS) => Ok(ids),
        _ => Err(format!("vectorcraft:configure's `{key}` is an array of at most {MAX_HIDDEN} {what} (strings of 1 to {MAX_ID_CHARS} characters)")),
    };
    let text = |field: Field<String>, key: &str| match field {
        Field::Absent => Ok(None),
        Field::Given(m) if m.chars().count() <= MAX_MESSAGE_CHARS => Ok(Some(m).filter(|m| !m.trim().is_empty())),
        _ => Err(format!("vectorcraft:configure's `{key}` is a string of at most {MAX_MESSAGE_CHARS} characters")),
    };
    let flag = |field: Field<bool>, key: &str| match field {
        Field::Absent => Ok(false),
        Field::Given(b) => Ok(b),
        Field::Wrong => Err(format!("vectorcraft:configure's `{key}` is a boolean")),
    };
    Ok(Configure {
        hide: names(f.hide, "hide", "command ids (a prefix ends in `*`)")?,
        hide_menus: names(f.hide_menus, "hideMenus", "menu titles")?,
        hide_mark: flag(f.hide_mark, "hideMark")?,
        empty_message: text(f.empty_message, "emptyMessage")?,
        empty_link: text(f.empty_link, "emptyLink")?,
        save_on_close: flag(f.save_on_close, "saveOnClose")?,
        save_recovered_in_place: flag(f.save_recovered_in_place, "saveRecoveredInPlace")?,
    })
}

/// The largest save id: every integer up to it is exact in a JavaScript number.
pub const MAX_SAVE_ID: f64 = 9_007_199_254_740_991.0;

/// What a `vectorcraft:saved` says: whether the parent stored the `vectorcraft:save` it answers.
#[derive(Debug, PartialEq, Eq)]
pub struct Saved {
    /// The answered save's `id`.
    pub id: u64,
    /// The answered save's `name`.
    pub name: String,
    /// Stored (`true`), or not.
    pub ok: bool,
    /// Why not, in the parent's words, for the status line.
    pub error: Option<String>,
}

/// Check a `vectorcraft:saved` the bridge accepted: `id` is the answered save's (a whole number),
/// `name` its file name, `ok` a boolean and `error`, if given, a string of at most
/// [`MAX_MESSAGE_CHARS`]. The error is the text of the `vectorcraft:failed` reply.
pub fn saved(id: Field<f64>, name: Field<String>, ok: Field<bool>, error: Field<String>) -> Result<Saved, String> {
    let id = match id {
        Field::Given(n) if n.is_finite() && n.fract() == 0.0 && (0.0..=MAX_SAVE_ID).contains(&n) => n as u64,
        _ => return Err("vectorcraft:saved needs the answered save's `id` (a whole number)".to_string()),
    };
    let Field::Given(name) = name.into_non_empty() else {
        return Err("vectorcraft:saved needs the answered save's `name` (a non-empty string)".to_string());
    };
    let Field::Given(ok) = ok else {
        return Err(format!("vectorcraft:saved of {name} needs `ok`, a boolean"));
    };
    let error = match error {
        Field::Absent => None,
        Field::Given(e) if e.chars().count() <= MAX_MESSAGE_CHARS => Some(e).filter(|e| !e.trim().is_empty()),
        _ => return Err(format!("vectorcraft:saved's `error` is a string of at most {MAX_MESSAGE_CHARS} characters")),
    };
    Ok(Saved { id, name, ok, error })
}

/// The most entries one `vectorcraft:recoveryEntries` gives, and the longest entry name.
pub const MAX_RECOVERY_ENTRIES: usize = 100_000;
pub const MAX_ENTRY_NAME_CHARS: usize = 300;

/// A `vectorcraft:recoveryEntries`' entry as the page sent it: its `name` and `bytes`, each if it
/// was of the right type.
pub type EntryFields = (Option<String>, Option<Vec<u8>>);

/// Check a `vectorcraft:recoveryEntries` the bridge accepted: `entries` is an array (`None`: not
/// one, or longer than [`MAX_RECOVERY_ENTRIES`]) of `{ name, bytes }`, each name a Data Recovery
/// entry's (`<area>/<entry>`, no other `/`, at most [`MAX_ENTRY_NAME_CHARS`]) and its bytes an
/// `ArrayBuffer` or `Uint8Array`. The error is the text of the `vectorcraft:failed` reply, and
/// none of them is taken.
pub fn recovery_entries(entries: Option<Vec<EntryFields>>) -> Result<Vec<(String, Vec<u8>)>, String> {
    let Some(entries) = entries.filter(|e| e.len() <= MAX_RECOVERY_ENTRIES) else {
        return Err(format!("vectorcraft:recoveryEntries needs `entries`, an array of at most {MAX_RECOVERY_ENTRIES} {{ name, bytes }}"));
    };
    let named = |n: &str| {
        n.chars().count() <= MAX_ENTRY_NAME_CHARS
            && n.split_once('/').is_some_and(|(area, entry)| !area.trim().is_empty() && !entry.trim().is_empty() && !entry.contains('/'))
    };
    entries
        .into_iter()
        .enumerate()
        .map(|(i, (name, bytes))| match (name.filter(|n| named(n)), bytes) {
            (Some(name), Some(bytes)) => Ok((name, bytes)),
            (None, _) => Err(format!(
                "vectorcraft:recoveryEntries' entry {i} needs `name`, a recovery entry's (`<area>/<entry>`, at most {MAX_ENTRY_NAME_CHARS} characters)"
            )),
            (Some(name), None) => Err(format!("vectorcraft:recoveryEntries' entry {name} needs `bytes`, an ArrayBuffer or Uint8Array")),
        })
        .collect()
}

/// The most actions a page puts on the Contextual Task Bar, and the longest id, label and icon name.
pub const MAX_TASK_BAR_ACTIONS: usize = 16;
pub const MAX_ACTION_ID_CHARS: usize = 100;
pub const MAX_ACTION_LABEL_CHARS: usize = 60;
pub const MAX_ACTION_ICON_CHARS: usize = 64;

/// A `vectorcraft:taskBar` action's fields as the page sent them. Fields it doesn't name are never
/// read.
#[derive(Debug, Default)]
pub struct TaskBarActionFields {
    pub id: Field<String>,
    pub label: Field<String>,
    pub icon: Field<String>,
    pub when: Field<String>,
    pub linked: Field<bool>,
    pub enabled: Field<bool>,
    pub panel: Field<String>,
}

/// Check a `vectorcraft:taskBar` the bridge accepted: `actions` is an array (`None`: not one, or
/// longer than [`MAX_TASK_BAR_ACTIONS`]) of `{ id, label, icon?, when, linked?, enabled? }`: `id`
/// 1 to [`MAX_ACTION_ID_CHARS`] characters and unique, `label` 1 to [`MAX_ACTION_LABEL_CHARS`],
/// `icon` at most [`MAX_ACTION_ICON_CHARS`], `when` `image` or `clipGroup`, `linked` (false) and
/// `enabled` (true) booleans. The list replaces the page's last whole; an empty one clears it. The
/// error is the text of the `vectorcraft:failed` reply, and nothing changes.
pub fn task_bar(actions: Option<Vec<TaskBarActionFields>>) -> Result<Vec<HostTaskBarAction>, String> {
    let Some(actions) = actions.filter(|a| a.len() <= MAX_TASK_BAR_ACTIONS) else {
        return Err(format!(
            "vectorcraft:taskBar needs `actions`, an array of at most {MAX_TASK_BAR_ACTIONS} {{ id, label, icon?, when, linked?, enabled? }}"
        ));
    };
    let mut out: Vec<HostTaskBarAction> = Vec::with_capacity(actions.len());
    for (i, f) in actions.into_iter().enumerate() {
        let text = |field: Field<String>, key: &str, max: usize| match field {
            Field::Given(v) if !v.trim().is_empty() && v.chars().count() <= max => Ok(v),
            _ => Err(format!("vectorcraft:taskBar's action {i} needs `{key}`, a string of 1 to {max} characters")),
        };
        let flag = |field: Field<bool>, key: &str, default: bool| match field {
            Field::Absent => Ok(default),
            Field::Given(b) => Ok(b),
            Field::Wrong => Err(format!("vectorcraft:taskBar's action {i}: `{key}` is a boolean")),
        };
        let id = text(f.id, "id", MAX_ACTION_ID_CHARS)?;
        if out.iter().any(|a| a.id == id) {
            return Err(format!("vectorcraft:taskBar's actions each need their own `id`: {id} is there twice"));
        }
        let icon = match f.icon {
            Field::Absent => None,
            Field::Given(v) if v.chars().count() <= MAX_ACTION_ICON_CHARS => Some(v).filter(|v| !v.trim().is_empty()),
            _ => return Err(format!("vectorcraft:taskBar's action {id}: `icon` is a string of at most {MAX_ACTION_ICON_CHARS} characters")),
        };
        let when = match f.when {
            Field::Given(w) if w == "image" => HostTaskBarWhen::Image,
            Field::Given(w) if w == "clipGroup" => HostTaskBarWhen::ClipGroup,
            _ => return Err(format!("vectorcraft:taskBar's action {id} needs `when`, `image` or `clipGroup`")),
        };
        let label = text(f.label, "label", MAX_ACTION_LABEL_CHARS)?;
        let linked = flag(f.linked, "linked", false)?;
        let enabled = flag(f.enabled, "enabled", true)?;
        let panel = match f.panel {
            Field::Absent => None,
            Field::Given(v) if !v.trim().is_empty() && v.chars().count() <= MAX_ACTION_ID_CHARS => Some(v),
            _ => return Err(format!("vectorcraft:taskBar's action {id}: `panel` is a host panel's id, 1 to {MAX_ACTION_ID_CHARS} characters")),
        };
        out.push(HostTaskBarAction { id, label, icon, when, linked, enabled, panel });
    }
    Ok(out)
}

/// The most panels a page fills, items in one, rows in its lists, and the longest id, label or title,
/// text, and text area limit.
pub const MAX_PANELS: usize = 8;
pub const MAX_PANEL_ITEMS: usize = 64;
pub const MAX_LIST_ITEMS: usize = 200;
pub const MAX_PANEL_ID_CHARS: usize = 100;
pub const MAX_PANEL_LABEL_CHARS: usize = 100;
pub const MAX_PANEL_TEXT_CHARS: usize = 2_000;
pub const MAX_TEXT_AREA_CHARS: usize = 4_000;

/// A `vectorcraft:panel`: a panel to show (in place of the one of its id), or the id of one to close.
#[derive(Debug, PartialEq)]
pub enum PanelMessage {
    Show(HostPanel),
    Close(String),
}

type Json = serde_json::Value;

/// A string field of a panel's JSON: `needed` ones non-blank, every one at most `max` characters.
fn panel_text(v: &Json, key: &str, max: usize, needed: bool) -> Result<String, String> {
    match v.get(key) {
        None | Some(Json::Null) if !needed => Ok(String::new()),
        Some(Json::String(s)) if s.chars().count() <= max && (!needed || !s.trim().is_empty()) => Ok(s.clone()),
        _ => Err(format!("vectorcraft:panel: `{key}` is a string of {} to {max} characters", u8::from(needed))),
    }
}

/// A boolean field of a panel's JSON, `default` when left out.
fn panel_flag(v: &Json, key: &str, default: bool) -> Result<bool, String> {
    match v.get(key) {
        None | Some(Json::Null) => Ok(default),
        Some(Json::Bool(b)) => Ok(*b),
        _ => Err(format!("vectorcraft:panel: `{key}` is a boolean")),
    }
}

fn panel_tone(v: &Json) -> Result<Tone, String> {
    match v.get("tone") {
        None | Some(Json::Null) => Ok(Tone::Plain),
        Some(t) if t.as_str() == Some("hint") => Ok(Tone::Hint),
        Some(t) if t.as_str() == Some("error") => Ok(Tone::Error),
        _ => Err("vectorcraft:panel: `tone` is `hint` or `error`".to_string()),
    }
}

fn panel_button(b: &Json) -> Result<HostButton, String> {
    if b.get("kind").and_then(Json::as_str).is_some_and(|k| k != "button") {
        return Err("vectorcraft:panel: a row or a list holds buttons only".to_string());
    }
    Ok(HostButton {
        id: panel_text(b, "id", MAX_PANEL_ID_CHARS, true)?,
        label: panel_text(b, "label", MAX_PANEL_LABEL_CHARS, true)?,
        primary: panel_flag(b, "primary", false)?,
        enabled: panel_flag(b, "enabled", true)?,
        icon: match b.get("icon") {
            None | Some(Json::Null) => None,
            Some(Json::String(s)) if !s.is_empty() && s.chars().count() <= MAX_ACTION_ICON_CHARS => Some(s.clone()),
            _ => return Err(format!("vectorcraft:panel: a button's `icon` is a string of at most {MAX_ACTION_ICON_CHARS} characters")),
        },
    })
}

fn panel_buttons(v: &Json, key: &str) -> Result<Vec<HostButton>, String> {
    match v.get(key) {
        None | Some(Json::Null) => Ok(vec![]),
        Some(Json::Array(a)) if a.len() <= MAX_PANEL_ITEMS => a.iter().map(panel_button).collect(),
        _ => Err(format!("vectorcraft:panel: `{key}` is an array of at most {MAX_PANEL_ITEMS} buttons")),
    }
}

/// `id`, once in `ids` (the panel's, or a list row's buttons').
fn unique(id: String, ids: &mut Vec<String>) -> Result<String, String> {
    if ids.contains(&id) {
        return Err(format!("vectorcraft:panel: each item needs its own `id`: {id} is there twice"));
    }
    ids.push(id.clone());
    Ok(id)
}

/// One item of a panel's `content`.
fn panel_item(item: &Json, ids: &mut Vec<String>) -> Result<HostItem, String> {
    Ok(match item.get("kind").and_then(Json::as_str) {
        Some("text") => HostItem::Text { text: panel_text(item, "text", MAX_PANEL_TEXT_CHARS, true)?, tone: panel_tone(item)? },
        Some("button") => {
            let b = panel_button(item)?;
            unique(b.id.clone(), ids)?;
            HostItem::Button(b)
        }
        Some("row") => {
            let row = panel_buttons(item, "items")?;
            for b in &row {
                unique(b.id.clone(), ids)?;
            }
            HostItem::Row(row)
        }
        Some("textArea") => {
            let max_chars = match item.get("maxChars") {
                None | Some(Json::Null) => 0,
                Some(n) => match n.as_u64().filter(|n| (1..=MAX_TEXT_AREA_CHARS as u64).contains(n)) {
                    Some(n) => n as usize,
                    None => return Err(format!("vectorcraft:panel: `maxChars` is a whole number from 1 to {MAX_TEXT_AREA_CHARS}")),
                },
            };
            HostItem::TextArea {
                id: unique(panel_text(item, "id", MAX_PANEL_ID_CHARS, true)?, ids)?,
                label: panel_text(item, "label", MAX_PANEL_LABEL_CHARS, true)?,
                value: panel_text(item, "value", MAX_TEXT_AREA_CHARS, false)?,
                placeholder: panel_text(item, "placeholder", MAX_PANEL_LABEL_CHARS, false)?,
                max_chars,
            }
        }
        Some("checkbox") => HostItem::Checkbox {
            id: unique(panel_text(item, "id", MAX_PANEL_ID_CHARS, true)?, ids)?,
            label: panel_text(item, "label", MAX_PANEL_LABEL_CHARS, true)?,
            checked: item.get("checked").and_then(Json::as_bool).ok_or("vectorcraft:panel: a checkbox needs `checked`, a boolean")?,
            enabled: panel_flag(item, "enabled", true)?,
        },
        Some("separator") => HostItem::Separator,
        Some("list") => {
            let rows = item
                .get("items")
                .and_then(Json::as_array)
                .filter(|a| a.len() <= MAX_LIST_ITEMS)
                .ok_or_else(|| format!("vectorcraft:panel: a list's `items` is an array of at most {MAX_LIST_ITEMS} rows"))?;
            let mut list = Vec::with_capacity(rows.len());
            for row in rows {
                let id = unique(panel_text(row, "id", MAX_PANEL_ID_CHARS, true)?, ids)?;
                let buttons = panel_buttons(row, "buttons")?;
                let mut own = vec![];
                for b in &buttons {
                    unique(b.id.clone(), &mut own)?;
                }
                list.push(HostListItem { id, text: panel_text(row, "text", MAX_PANEL_TEXT_CHARS, true)?, tone: panel_tone(row)?, buttons });
            }
            HostItem::List(list)
        }
        _ => return Err("vectorcraft:panel: an item's `kind` is text, button, row, textArea, checkbox, separator or list".to_string()),
    })
}

/// Check a `vectorcraft:panel` the bridge accepted, as JSON: `{ id, title, anchor: "taskBar" |
/// "dock", content: [...] }`, or `{ id, close: true }`. The items are `text {text, tone?}`, `button
/// {id, label, primary?, enabled?}`, `row {items: [buttons]}`, `textArea {id, label, value,
/// placeholder?, maxChars?}`, `checkbox {id, label, checked, enabled?}`, `separator` and `list
/// {items: [{id, text, tone?, buttons?}]}`; ids unique in the panel (a list's buttons in their
/// row). Fields it doesn't name are ignored. The error is the text of the `vectorcraft:failed`
/// reply, and nothing changes.
pub fn panel(v: &Json) -> Result<PanelMessage, String> {
    let id = panel_text(v, "id", MAX_PANEL_ID_CHARS, true)?;
    if panel_flag(v, "close", false)? {
        return Ok(PanelMessage::Close(id));
    }
    let title = panel_text(v, "title", MAX_PANEL_LABEL_CHARS, true)?;
    let anchor = match v.get("anchor").and_then(Json::as_str) {
        Some("taskBar") => HostPanelAnchor::TaskBar,
        Some("dock") => HostPanelAnchor::Dock,
        _ => return Err("vectorcraft:panel: `anchor` is `taskBar` or `dock`".to_string()),
    };
    let items = v
        .get("content")
        .and_then(Json::as_array)
        .filter(|a| a.len() <= MAX_PANEL_ITEMS)
        .ok_or_else(|| format!("vectorcraft:panel: `content` is an array of at most {MAX_PANEL_ITEMS} items"))?;
    let mut ids = vec![];
    let content = items.iter().map(|item| panel_item(item, &mut ids)).collect::<Result<Vec<_>, _>>()?;
    Ok(PanelMessage::Show(HostPanel { id, title, anchor, content }))
}

/// Whether a page with `count` panels may show one more: a panel it already has is always replaced.
pub fn room_for_panel(known: bool, count: usize) -> Result<(), String> {
    if known || count < MAX_PANELS { Ok(()) } else { Err(format!("vectorcraft:panel: at most {MAX_PANELS} panels; close one first")) }
}

/// A `vectorcraft:region`: a box (and optionally an overlay PNG) on an image, or none.
#[derive(Debug, PartialEq)]
pub enum RegionMessage {
    Set { image: u64, rect: [f64; 4], overlay: Option<Vec<u8>>, editable: bool },
    Clear,
}

/// The largest overlay a region carries, in bytes.
pub const MAX_OVERLAY_BYTES: usize = 8 << 20;

/// A `vectorcraft:region`'s fields as the page sent them.
#[derive(Debug, Default)]
pub struct RegionFields {
    pub clear: Field<bool>,
    pub image: Field<f64>,
    /// `rect`'s `left`, `top`, `right` and `bottom`, each if it was a number (`None`: no object).
    pub rect: Option<[Field<f64>; 4]>,
    /// `overlay`'s bytes (`Wrong`: not an ArrayBuffer or Uint8Array).
    pub overlay: Field<Vec<u8>>,
    pub editable: Field<bool>,
}

/// Check a `vectorcraft:region` the bridge accepted: `{ clear: true }`, or `{ image, rect: { left,
/// top, right, bottom }, overlay?, editable? }`: `image` an object's id (a whole number), `rect` in
/// the image's relative units (0 to 1, left before right, top before bottom), `overlay` a PNG of at
/// most [`MAX_OVERLAY_BYTES`], `editable` a boolean (false). Whether `image` is one is the app's to
/// say. The error is the text of the `vectorcraft:failed` reply, and nothing changes.
pub fn region(f: RegionFields) -> Result<RegionMessage, String> {
    match f.clear {
        Field::Given(true) => return Ok(RegionMessage::Clear),
        Field::Wrong => return Err("vectorcraft:region's `clear` is a boolean".to_string()),
        _ => {}
    }
    let image = match f.image {
        Field::Given(n) if n.is_finite() && n.fract() == 0.0 && (0.0..=MAX_SAVE_ID).contains(&n) => n as u64,
        _ => return Err("vectorcraft:region needs `image`, an image's id (a whole number)".to_string()),
    };
    let unit = |v: &Field<f64>| match v {
        Field::Given(n) if n.is_finite() && (0.0..=1.0).contains(n) => Some(*n),
        _ => None,
    };
    let rect =
        f.rect.as_ref().and_then(|r| Some([unit(&r[0])?, unit(&r[1])?, unit(&r[2])?, unit(&r[3])?])).filter(|r| r[0] < r[2] && r[1] < r[3]).ok_or(
            "vectorcraft:region needs `rect`, { left, top, right, bottom } from 0 to 1 of the image, left before right and top before bottom",
        )?;
    let overlay = match f.overlay {
        Field::Absent => None,
        Field::Given(b) if b.len() <= MAX_OVERLAY_BYTES && b.starts_with(b"\x89PNG\r\n\x1a\n") => Some(b),
        _ => return Err(format!("vectorcraft:region's `overlay` is a PNG of at most {} MB, an ArrayBuffer or Uint8Array", MAX_OVERLAY_BYTES >> 20)),
    };
    let editable = match f.editable {
        Field::Absent => false,
        Field::Given(b) => b,
        Field::Wrong => return Err("vectorcraft:region's `editable` is a boolean".to_string()),
    };
    Ok(RegionMessage::Set { image, rect, overlay, editable })
}

/// A `vectorcraft:masks`: one image's opacity mask shown tinted, or none.
#[derive(Debug, PartialEq)]
pub enum MasksMessage {
    Show { image: u64 },
    Clear,
}

/// Check a `vectorcraft:masks` the bridge accepted: `{ clear: true }`, or `{ image }`, an object's
/// id (a whole number). Whether `image` is an image is the app's to say. The error is the text of
/// the `vectorcraft:failed` reply, and nothing changes.
pub fn masks(clear: Field<bool>, image: Field<f64>) -> Result<MasksMessage, String> {
    match clear {
        Field::Given(true) => return Ok(MasksMessage::Clear),
        Field::Wrong => return Err("vectorcraft:masks's `clear` is a boolean".to_string()),
        _ => {}
    }
    match image {
        Field::Given(n) if n.is_finite() && n.fract() == 0.0 && (0.0..=MAX_SAVE_ID).contains(&n) => Ok(MasksMessage::Show { image: n as u64 }),
        _ => Err("vectorcraft:masks needs `image`, an image's id (a whole number)".to_string()),
    }
}

/// What the bridge last told the parent, so it tells it again only when it changes (the selection,
/// `vectorcraft:selection`).
#[derive(Debug, Default)]
pub struct Reported<T>(Option<T>);

impl<T: PartialEq> Reported<T> {
    /// `now` when it differs from what was last reported (and is taken as reported), else nothing.
    pub fn changed(&mut self, now: T) -> Option<&T> {
        if self.0.as_ref() == Some(&now) {
            return None;
        }
        self.0 = Some(now);
        self.0.as_ref()
    }
}

impl Field<String> {
    /// A blank string counts as left out.
    fn into_non_empty(self) -> Self {
        match self {
            Field::Given(s) if s.trim().is_empty() => Field::Absent,
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SITE: &str = "https://cloud.example.com";
    const OPEN: Option<&str> = Some("vectorcraft:open");

    #[test]
    fn accepts_an_open_from_the_parent_on_the_same_origin() {
        assert!(accepts(SITE, SITE, true, OPEN));
    }

    #[test]
    fn ignores_messages_from_another_origin() {
        for origin in [
            "https://evil.example",
            "http://cloud.example.com",
            "https://cloud.example.com:8443",
            "https://cloud.example.com.evil.example",
            "https://sub.cloud.example.com",
            "null",
            "",
        ] {
            assert!(!accepts(SITE, origin, true, OPEN), "{origin}");
        }
    }

    #[test]
    fn ignores_another_window_on_the_same_origin() {
        // Such as another tab of the same site, or a frame the parent page holds.
        assert!(!accepts(SITE, SITE, false, OPEN));
    }

    #[test]
    fn ignores_other_message_types() {
        for kind in [
            None,
            Some(""),
            Some("vectorcraft:save"),
            Some("vectorcraft:ready"),
            Some("VECTORCRAFT:OPEN"),
            Some("photocraft:open"),
            Some("vectorcraft:open "),
        ] {
            assert!(!accepts(SITE, SITE, true, kind), "{kind:?}");
        }
    }

    #[test]
    fn hears_nothing_on_an_opaque_origin() {
        assert!(!accepts("null", "null", true, OPEN));
        assert!(!accepts("", "", true, OPEN));
        assert!(is_opaque("null"));
        assert!(!is_opaque(SITE));
    }

    #[test]
    fn opens_a_named_file() {
        let opened =
            |name: &str, bytes: &[u8], title: Option<&str>| Open { name: name.into(), bytes: bytes.to_vec(), title: title.map(String::from) };
        assert_eq!(open(Some("Logo.svg".into()), Some(b"<svg/>".to_vec()), Field::Absent), Ok(opened("Logo.svg", b"<svg/>", None)));
        // An empty file is the importer's to report.
        assert_eq!(open(Some("Empty.svg".into()), Some(Vec::new()), Field::Absent), Ok(opened("Empty.svg", b"", None)));
    }

    #[test]
    fn opens_a_file_whose_tab_shows_the_title_given() {
        let open = |title| open(Some("Project v5.vectorcraft".into()), Some(b"x".to_vec()), title);
        assert_eq!(open(Field::Given(" Project ".into())).unwrap().title.as_deref(), Some("Project"));
        // Blank is none: the tab shows the name.
        assert_eq!(open(Field::Given("  ".into())).unwrap().title, None);
        for wrong in [Field::Wrong, Field::Given("x".repeat(MAX_TITLE_CHARS + 1))] {
            let error = open(wrong).unwrap_err();
            assert!(error.contains("`title`") && error.contains("Project v5.vectorcraft"), "{error}");
        }
    }

    #[test]
    fn refuses_an_open_without_a_name() {
        for name in [None, Some(String::new()), Some("   ".to_string()), Some("\t\n".to_string())] {
            let error = open(name.clone(), Some(b"<svg/>".to_vec()), Field::Absent).err();
            assert!(error.as_deref().is_some_and(|e| e.contains("`name`")), "{name:?}: {error:?}");
        }
    }

    #[test]
    fn refuses_an_open_without_bytes() {
        let error = open(Some("Logo.svg".into()), None, Field::Absent).err();
        assert!(error.as_deref().is_some_and(|e| e.contains("`bytes`") && e.contains("Logo.svg")), "{error:?}");
    }

    fn action(id: &str, when: &str) -> TaskBarActionFields {
        TaskBarActionFields {
            id: Field::Given(id.into()),
            label: Field::Given(format!("Do {id}")),
            when: Field::Given(when.into()),
            ..Default::default()
        }
    }

    #[test]
    fn accepts_a_task_bar_from_the_parent_on_the_same_origin() {
        assert!(accepts("https://cloud.example", "https://cloud.example", true, Some("vectorcraft:taskBar")));
        assert!(!accepts("https://cloud.example", "https://evil.example", true, Some("vectorcraft:taskBar")));
        assert!(!accepts("https://cloud.example", "https://cloud.example", false, Some("vectorcraft:taskBar")));
    }

    #[test]
    fn reads_the_pages_task_bar_actions() {
        let mut crop = action("crop", "image");
        crop.icon = Field::Given("dc-pf-crop".into());
        crop.linked = Field::Given(true);
        let mut off = action("expand", "clipGroup");
        off.enabled = Field::Given(false);
        let actions = task_bar(Some(vec![crop, off])).unwrap();
        assert_eq!(
            actions,
            vec![
                HostTaskBarAction {
                    id: "crop".into(),
                    label: "Do crop".into(),
                    icon: Some("dc-pf-crop".into()),
                    when: HostTaskBarWhen::Image,
                    linked: true,
                    enabled: true,
                    panel: None
                },
                HostTaskBarAction {
                    id: "expand".into(),
                    label: "Do expand".into(),
                    icon: None,
                    when: HostTaskBarWhen::ClipGroup,
                    linked: false,
                    enabled: false,
                    panel: None
                },
            ]
        );
        // An empty list clears the page's actions; a blank icon is none.
        assert_eq!(task_bar(Some(vec![])).unwrap(), vec![]);
        let mut blank = action("a", "image");
        blank.icon = Field::Given(" ".into());
        assert_eq!(task_bar(Some(vec![blank])).unwrap()[0].icon, None);
    }

    #[test]
    fn refuses_a_task_bar_of_the_wrong_shape() {
        assert!(task_bar(None).unwrap_err().contains("`actions`"));
        assert!(task_bar(Some((0..=MAX_TASK_BAR_ACTIONS).map(|i| action(&i.to_string(), "image")).collect())).is_err());
        assert!(task_bar(Some((0..MAX_TASK_BAR_ACTIONS).map(|i| action(&i.to_string(), "image")).collect())).is_ok());
        let with = |change: &dyn Fn(&mut TaskBarActionFields)| {
            let mut a = action("a", "image");
            change(&mut a);
            task_bar(Some(vec![a]))
        };
        assert!(with(&|a| a.id = Field::Absent).unwrap_err().contains("`id`"));
        assert!(with(&|a| a.id = Field::Given(" ".into())).is_err());
        assert!(with(&|a| a.id = Field::Given("x".repeat(MAX_ACTION_ID_CHARS + 1))).is_err());
        assert!(with(&|a| a.id = Field::Given("x".repeat(MAX_ACTION_ID_CHARS))).is_ok());
        assert!(with(&|a| a.label = Field::Wrong).unwrap_err().contains("`label`"));
        assert!(with(&|a| a.label = Field::Given("x".repeat(MAX_ACTION_LABEL_CHARS + 1))).is_err());
        assert!(with(&|a| a.icon = Field::Wrong).unwrap_err().contains("`icon`"));
        assert!(with(&|a| a.icon = Field::Given("x".repeat(MAX_ACTION_ICON_CHARS + 1))).is_err());
        assert!(with(&|a| a.when = Field::Given("path".into())).unwrap_err().contains("`when`"));
        assert!(with(&|a| a.when = Field::Absent).is_err());
        assert!(with(&|a| a.linked = Field::Wrong).unwrap_err().contains("`linked`"));
        assert!(with(&|a| a.enabled = Field::Wrong).unwrap_err().contains("`enabled`"));
        assert!(task_bar(Some(vec![action("a", "image"), action("a", "clipGroup")])).unwrap_err().contains("twice"));
    }

    #[test]
    fn accepts_a_panel_from_the_parent_on_the_same_origin() {
        assert!(accepts("https://cloud.example", "https://cloud.example", true, Some("vectorcraft:panel")));
        assert!(!accepts("https://cloud.example", "https://evil.example", true, Some("vectorcraft:panel")));
    }

    #[test]
    fn reads_a_panel_of_every_kind_of_item() {
        let v = serde_json::json!({
            "id": "fill", "title": "Generative Fill", "anchor": "taskBar", "unknown": 1,
            "content": [
                {"kind": "text", "text": "Paint black over what to replace.", "tone": "hint"},
                {"kind": "button", "id": "setup", "label": "Set Up", "primary": true},
                {"kind": "row", "items": [{"id": "paint", "label": "Paint Mask"}, {"id": "remove", "label": "Remove Setup", "enabled": false}]},
                {"kind": "textArea", "id": "instructions", "label": "Instructions", "value": "Keep her face", "placeholder": "What to make", "maxChars": 2000},
                {"kind": "checkbox", "id": "match", "label": "Color match", "checked": true},
                {"kind": "separator"},
                {"kind": "list", "items": [{"id": "sgr_1", "text": "Waiting for an Agent", "tone": "error", "buttons": [{"id": "cancel", "label": "Cancel"}]}]}
            ]
        });
        let PanelMessage::Show(p) = panel(&v).unwrap() else { panic!("a panel") };
        assert_eq!((p.id.as_str(), p.title.as_str(), p.anchor), ("fill", "Generative Fill", HostPanelAnchor::TaskBar));
        assert_eq!(p.content.len(), 7);
        assert_eq!(p.content[0], HostItem::Text { text: "Paint black over what to replace.".into(), tone: Tone::Hint });
        assert_eq!(
            p.content[1],
            HostItem::Button(HostButton { id: "setup".into(), label: "Set Up".into(), primary: true, enabled: true, icon: None })
        );
        assert!(matches!(&p.content[2], HostItem::Row(r) if r.len() == 2 && !r[1].enabled));
        assert!(matches!(&p.content[3], HostItem::TextArea { max_chars: 2000, value, .. } if value == "Keep her face"));
        assert!(matches!(&p.content[4], HostItem::Checkbox { checked: true, enabled: true, .. }));
        assert_eq!(p.content[5], HostItem::Separator);
        assert!(matches!(&p.content[6], HostItem::List(l) if l[0].buttons[0].id == "cancel" && l[0].tone == Tone::Error));
        assert_eq!(panel(&serde_json::json!({"id": "fill", "close": true})).unwrap(), PanelMessage::Close("fill".into()));
    }

    #[test]
    fn refuses_a_panel_of_the_wrong_shape() {
        let with = |content: serde_json::Value| panel(&serde_json::json!({"id": "p", "title": "P", "anchor": "dock", "content": content}));
        assert!(with(serde_json::json!([])).is_ok());
        assert!(panel(&serde_json::json!({"title": "P", "anchor": "dock", "content": []})).unwrap_err().contains("`id`"));
        assert!(panel(&serde_json::json!({"id": "p", "anchor": "dock", "content": []})).unwrap_err().contains("`title`"));
        assert!(panel(&serde_json::json!({"id": "p", "title": "P", "anchor": "window", "content": []})).unwrap_err().contains("`anchor`"));
        assert!(panel(&serde_json::json!({"id": "x".repeat(MAX_PANEL_ID_CHARS + 1), "title": "P", "anchor": "dock", "content": []})).is_err());
        assert!(panel(&serde_json::json!({"id": "p", "title": "x".repeat(MAX_PANEL_LABEL_CHARS + 1), "anchor": "dock", "content": []})).is_err());
        assert!(with(serde_json::json!((0..=MAX_PANEL_ITEMS).map(|_| serde_json::json!({"kind": "separator"})).collect::<Vec<_>>())).is_err());
        assert!(with(serde_json::json!((0..MAX_PANEL_ITEMS).map(|_| serde_json::json!({"kind": "separator"})).collect::<Vec<_>>())).is_ok());
        assert!(with(serde_json::json!([{"kind": "slider"}])).unwrap_err().contains("`kind`"));
        assert!(with(serde_json::json!([{"kind": "text", "text": "x".repeat(MAX_PANEL_TEXT_CHARS + 1)}])).is_err());
        assert!(with(serde_json::json!([{"kind": "text", "text": "a", "tone": "loud"}])).unwrap_err().contains("`tone`"));
        assert!(with(serde_json::json!([{"kind": "button", "id": "a", "label": "A", "primary": "yes"}])).unwrap_err().contains("`primary`"));
        assert!(
            with(serde_json::json!([{"kind": "button", "id": "a", "label": "A"}, {"kind": "checkbox", "id": "a", "label": "B", "checked": false}]))
                .unwrap_err()
                .contains("twice")
        );
        assert!(with(serde_json::json!([{"kind": "row", "items": [{"kind": "text", "text": "a"}]}])).unwrap_err().contains("buttons only"));
        assert!(with(serde_json::json!([{"kind": "checkbox", "id": "c", "label": "C"}])).unwrap_err().contains("`checked`"));
        assert!(
            with(serde_json::json!([{"kind": "textArea", "id": "t", "label": "T", "maxChars": MAX_TEXT_AREA_CHARS + 1}]))
                .unwrap_err()
                .contains("`maxChars`")
        );
        assert!(with(serde_json::json!([{"kind": "textArea", "id": "t", "label": "T", "maxChars": 0}])).is_err());
        let rows = |n: usize| serde_json::json!([{"kind": "list", "items": (0..n).map(|i| serde_json::json!({"id": i.to_string(), "text": "r"})).collect::<Vec<_>>()}]);
        assert!(with(rows(MAX_LIST_ITEMS)).is_ok());
        assert!(with(rows(MAX_LIST_ITEMS + 1)).is_err());
        // A list's buttons are named by their row: the same button id in two rows is fine.
        let two = serde_json::json!([{"kind": "list", "items": [
            {"id": "r1", "text": "a", "buttons": [{"id": "cancel", "label": "Cancel"}]},
            {"id": "r2", "text": "b", "buttons": [{"id": "cancel", "label": "Cancel"}]}
        ]}]);
        assert!(with(two).is_ok());
    }

    #[test]
    fn a_page_fills_at_most_eight_panels() {
        assert!(room_for_panel(false, MAX_PANELS - 1).is_ok());
        assert!(room_for_panel(false, MAX_PANELS).unwrap_err().contains("close one first"));
        assert!(room_for_panel(true, MAX_PANELS).is_ok(), "one it has is replaced");
    }

    fn region_fields(rect: [f64; 4]) -> RegionFields {
        RegionFields { image: Field::Given(12.0), rect: Some(rect.map(Field::Given)), ..Default::default() }
    }

    #[test]
    fn reads_a_region_on_an_image() {
        assert!(accepts("https://cloud.example", "https://cloud.example", true, Some("vectorcraft:region")));
        let png = b"\x89PNG\r\n\x1a\nrest".to_vec();
        let mut f = region_fields([0.1, 0.2, 0.5, 0.6]);
        f.overlay = Field::Given(png.clone());
        f.editable = Field::Given(true);
        assert_eq!(region(f).unwrap(), RegionMessage::Set { image: 12, rect: [0.1, 0.2, 0.5, 0.6], overlay: Some(png), editable: true });
        assert_eq!(
            region(region_fields([0.0, 0.0, 1.0, 1.0])).unwrap(),
            RegionMessage::Set { image: 12, rect: [0.0, 0.0, 1.0, 1.0], overlay: None, editable: false }
        );
        assert_eq!(region(RegionFields { clear: Field::Given(true), ..Default::default() }).unwrap(), RegionMessage::Clear);
    }

    #[test]
    fn refuses_a_region_of_the_wrong_shape() {
        assert!(region(RegionFields { rect: Some([0.0, 0.0, 1.0, 1.0].map(Field::Given)), ..Default::default() }).unwrap_err().contains("`image`"));
        assert!(region(RegionFields { image: Field::Given(1.5), ..region_fields([0.0, 0.0, 1.0, 1.0]) }).is_err());
        assert!(region(RegionFields { image: Field::Given(3.0), rect: None, ..Default::default() }).unwrap_err().contains("`rect`"));
        assert!(region(region_fields([0.5, 0.0, 0.5, 1.0])).is_err(), "no width");
        assert!(region(region_fields([0.0, 0.6, 1.0, 0.2])).is_err(), "top below bottom");
        assert!(region(region_fields([-0.1, 0.0, 1.0, 1.0])).is_err());
        assert!(region(region_fields([0.0, 0.0, 1.1, 1.0])).is_err());
        assert!(region(region_fields([0.0, 0.0, f64::NAN, 1.0])).is_err());
        let with_overlay = |o: Field<Vec<u8>>| region(RegionFields { overlay: o, ..region_fields([0.0, 0.0, 1.0, 1.0]) });
        assert!(with_overlay(Field::Given(b"GIF89a".to_vec())).unwrap_err().contains("PNG"));
        assert!(with_overlay(Field::Wrong).is_err());
        let mut big = b"\x89PNG\r\n\x1a\n".to_vec();
        big.resize(MAX_OVERLAY_BYTES + 1, 0);
        assert!(with_overlay(Field::Given(big)).is_err());
        assert!(region(RegionFields { editable: Field::Wrong, ..region_fields([0.0, 0.0, 1.0, 1.0]) }).unwrap_err().contains("`editable`"));
    }

    #[test]
    fn reports_again_only_what_changed() {
        let mut selection = Reported::default();
        assert_eq!(selection.changed(Vec::<u64>::new()), Some(&vec![]), "the first is always told");
        assert_eq!(selection.changed(vec![]), None);
        assert_eq!(selection.changed(vec![7]), Some(&vec![7]));
        assert_eq!(selection.changed(vec![7]), None);
        assert_eq!(selection.changed(vec![]), Some(&vec![]));
    }

    #[test]
    fn accepts_a_configure_from_the_parent_on_the_same_origin() {
        assert!(accepts("https://cloud.example", "https://cloud.example", true, Some("vectorcraft:configure")));
        assert!(!accepts("https://cloud.example", "https://evil.example", true, Some("vectorcraft:configure")));
        assert!(!accepts("https://cloud.example", "https://cloud.example", false, Some("vectorcraft:configure")));
    }

    fn fields(hide: Field<Vec<String>>, empty_message: Field<String>) -> ConfigureFields {
        ConfigureFields { hide, empty_message, ..Default::default() }
    }

    #[test]
    fn configures_hidden_commands_and_the_empty_message() {
        let c = configure(fields(
            Field::Given(vec!["file.new".into(), "file.openRecent*".into()]),
            Field::Given("Open a file from the Files app".into()),
        ))
        .unwrap();
        assert_eq!(c.hide, ["file.new", "file.openRecent*"]);
        assert_eq!(c.empty_message.as_deref(), Some("Open a file from the Files app"));
        // Left out, each is cleared; a blank message is none.
        assert_eq!(configure(fields(Field::Absent, Field::Given("  ".into()))).unwrap(), Configure::default());
    }

    #[test]
    fn configures_hidden_menus_the_mark_and_the_empty_link() {
        let c = configure(ConfigureFields {
            hide_menus: Field::Given(vec!["File".into(), "Help".into()]),
            hide_mark: Field::Given(true),
            empty_message: Field::Given("No file open.".into()),
            empty_link: Field::Given("Open one from Files".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(c.hide_menus, ["File", "Help"]);
        assert!(c.hide_mark);
        assert_eq!(c.empty_link.as_deref(), Some("Open one from Files"));
        // A blank link is none, as a blank message is.
        assert_eq!(configure(ConfigureFields { empty_link: Field::Given(" ".into()), ..Default::default() }).unwrap().empty_link, None);
    }

    #[test]
    fn refuses_a_configure_of_the_wrong_shape() {
        assert!(configure(fields(Field::Wrong, Field::Absent)).unwrap_err().contains("`hide`"));
        assert!(configure(fields(Field::Given(vec![String::new()]), Field::Absent)).is_err());
        assert!(configure(fields(Field::Given(vec!["x".repeat(MAX_ID_CHARS + 1)]), Field::Absent)).is_err());
        assert!(configure(fields(Field::Given(vec!["file.new".into(); MAX_HIDDEN + 1]), Field::Absent)).is_err());
        assert!(configure(fields(Field::Absent, Field::Wrong)).unwrap_err().contains("`emptyMessage`"));
        assert!(configure(fields(Field::Absent, Field::Given("x".repeat(MAX_MESSAGE_CHARS + 1)))).is_err());
        assert!(configure(ConfigureFields { hide_menus: Field::Wrong, ..Default::default() }).unwrap_err().contains("`hideMenus`"));
        assert!(configure(ConfigureFields { hide_menus: Field::Given(vec![" ".into()]), ..Default::default() }).is_err());
        assert!(configure(ConfigureFields { hide_mark: Field::Wrong, ..Default::default() }).unwrap_err().contains("`hideMark`"));
        assert!(configure(ConfigureFields { empty_link: Field::Wrong, ..Default::default() }).unwrap_err().contains("`emptyLink`"));
        assert!(configure(ConfigureFields { empty_link: Field::Given("x".repeat(MAX_MESSAGE_CHARS + 1)), ..Default::default() }).is_err());
        assert!(configure(ConfigureFields { save_on_close: Field::Wrong, ..Default::default() }).unwrap_err().contains("`saveOnClose`"));
    }

    #[test]
    fn configures_saving_a_recovered_copy_in_place() {
        assert!(configure(ConfigureFields { save_recovered_in_place: Field::Given(true), ..Default::default() }).unwrap().save_recovered_in_place);
        // Left out, a recovered copy is saved as, as before.
        assert!(!configure(ConfigureFields::default()).unwrap().save_recovered_in_place);
        let wrong = ConfigureFields { save_recovered_in_place: Field::Wrong, ..Default::default() };
        assert!(configure(wrong).unwrap_err().contains("`saveRecoveredInPlace`"));
    }

    #[test]
    fn configures_saving_on_close() {
        assert!(configure(ConfigureFields { save_on_close: Field::Given(true), ..Default::default() }).unwrap().save_on_close);
        // Left out, it is off: a host that never sets it is asked Save Changes as before.
        assert!(!configure(ConfigureFields::default()).unwrap().save_on_close);
    }

    #[test]
    fn accepts_a_saved_answer_from_the_parent_on_the_same_origin() {
        assert!(accepts(SITE, SITE, true, Some("vectorcraft:saved")));
        assert!(!accepts(SITE, "https://evil.example", true, Some("vectorcraft:saved")));
        assert!(!accepts(SITE, SITE, false, Some("vectorcraft:saved")));
    }

    #[test]
    fn reads_a_saved_answer() {
        let name = || Field::Given("Logo.vectorcraft".to_string());
        assert_eq!(
            saved(Field::Given(7.0), name(), Field::Given(true), Field::Absent),
            Ok(Saved { id: 7, name: "Logo.vectorcraft".into(), ok: true, error: None })
        );
        let failed = saved(Field::Given(8.0), name(), Field::Given(false), Field::Given("The server is offline".into())).unwrap();
        assert_eq!((failed.ok, failed.error.as_deref()), (false, Some("The server is offline")));
        // A blank reason is none.
        assert_eq!(saved(Field::Given(8.0), name(), Field::Given(false), Field::Given(" ".into())).unwrap().error, None);
    }

    #[test]
    fn refuses_a_saved_answer_of_the_wrong_shape() {
        let name = || Field::Given("Logo.vectorcraft".to_string());
        for id in [
            Field::Absent,
            Field::Wrong,
            Field::Given(-1.0),
            Field::Given(1.5),
            Field::Given(f64::NAN),
            Field::Given(f64::INFINITY),
            Field::Given(MAX_SAVE_ID * 2.0),
        ] {
            assert!(saved(id, name(), Field::Given(true), Field::Absent).unwrap_err().contains("`id`"));
        }
        for n in [Field::Absent, Field::Wrong, Field::Given("  ".to_string())] {
            assert!(saved(Field::Given(1.0), n, Field::Given(true), Field::Absent).unwrap_err().contains("`name`"));
        }
        for ok in [Field::Absent, Field::Wrong] {
            assert!(saved(Field::Given(1.0), name(), ok, Field::Absent).unwrap_err().contains("`ok`"));
        }
        assert!(saved(Field::Given(1.0), name(), Field::Given(false), Field::Wrong).unwrap_err().contains("`error`"));
        assert!(saved(Field::Given(1.0), name(), Field::Given(false), Field::Given("x".repeat(MAX_MESSAGE_CHARS + 1))).is_err());
    }

    #[test]
    fn accepts_recovery_entries_from_the_parent_on_the_same_origin() {
        assert!(accepts(SITE, SITE, true, Some("vectorcraft:recoveryEntries")));
        assert!(!accepts(SITE, "https://evil.example", true, Some("vectorcraft:recoveryEntries")));
        assert!(!accepts(SITE, SITE, false, Some("vectorcraft:recoveryEntries")));
    }

    #[test]
    fn reads_recovery_entries() {
        let entry = |n: &str, b: &[u8]| (Some(n.to_string()), Some(b.to_vec()));
        assert_eq!(
            recovery_entries(Some(vec![entry("1-1/Deck-1.json", b"{}"), entry("1-1/heartbeat", b"1")])),
            Ok(vec![("1-1/Deck-1.json".to_string(), b"{}".to_vec()), ("1-1/heartbeat".to_string(), b"1".to_vec())])
        );
        assert_eq!(recovery_entries(Some(vec![])), Ok(vec![]));
    }

    #[test]
    fn refuses_recovery_entries_of_the_wrong_shape() {
        assert!(recovery_entries(None).unwrap_err().contains("`entries`"));
        assert!(recovery_entries(Some(vec![(None, None); MAX_RECOVERY_ENTRIES + 1])).unwrap_err().contains("`entries`"));
        for name in ["", "no-area", "/entry", "area/", "a/b/c", " /x"] {
            let error = recovery_entries(Some(vec![(Some(name.to_string()), Some(vec![]))])).unwrap_err();
            assert!(error.contains("`name`"), "{name}: {error}");
        }
        let long = format!("a/{}", "x".repeat(MAX_ENTRY_NAME_CHARS));
        assert!(recovery_entries(Some(vec![(Some(long), Some(vec![]))])).unwrap_err().contains("`name`"));
        assert!(recovery_entries(Some(vec![(Some("a/b".into()), None)])).unwrap_err().contains("`bytes`"));
    }

    #[test]
    fn reads_a_panel_buttons_icon() {
        let read = |icon: Json| panel_button(&serde_json::json!({"id": "remove", "label": "Remove", "icon": icon}));
        assert_eq!(read(Json::Null).unwrap().icon, None);
        assert_eq!(read(serde_json::json!("trash-2")).unwrap().icon.as_deref(), Some("trash-2"));
        assert!(read(serde_json::json!(3)).unwrap_err().contains("`icon`"));
        assert!(read(serde_json::json!("")).unwrap_err().contains("`icon`"));
    }

    #[test]
    fn reads_masks_and_refuses_the_wrong_shape() {
        assert_eq!(masks(Field::Given(true), Field::Absent), Ok(MasksMessage::Clear));
        assert_eq!(masks(Field::Absent, Field::Given(12.0)), Ok(MasksMessage::Show { image: 12 }));
        assert!(masks(Field::Wrong, Field::Given(12.0)).unwrap_err().contains("`clear`"));
        for image in [Field::Absent, Field::Wrong, Field::Given(1.5), Field::Given(-1.0), Field::Given(f64::NAN)] {
            assert!(masks(Field::Absent, image).unwrap_err().contains("`image`"));
        }
        assert!(accepts("https://a.example", "https://a.example", true, Some("vectorcraft:masks")));
    }
}
