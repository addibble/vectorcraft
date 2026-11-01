//! Panels a host page fills (`vectorcraft:panel`, docs/web-embed.md, host-only): the page says
//! what a panel holds in a small vocabulary of items (text, buttons, a row of buttons, a text area,
//! a checkbox, a separator, a list), VectorCraft draws them with its own widgets, and every click
//! and change goes back to the page (`vectorcraft:panelEvent`). Nothing here runs a command.
//!
//! A panel is shown one of two ways:
//! - `taskBar`: a popover under the Contextual Task Bar, opened and closed by the page's task bar
//!   action that names it ([`crate::state::HostTaskBarAction::panel`]), as Shape Properties opens.
//! - `dock`: a panel of the docking system, `host:<id>`, which floats, tabs, docks and closes as any
//!   other; `window.panel {panel: "host:<id>"}` shows it.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use egui::Ui;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets;

/// What every docked host panel's id starts with.
pub const PREFIX: &str = "host:";

/// The most host panels a session names: their docking ids are interned ([`intern`]).
const MAX_INTERNED: usize = 64;

/// A panel the host page filled.
#[derive(Clone, Debug, PartialEq)]
pub struct HostPanel {
    /// The page's name for it, unique among its panels.
    pub id: String,
    /// Its tab's title, already in the user's language.
    pub title: String,
    pub anchor: HostPanelAnchor,
    pub content: Vec<HostItem>,
}

/// Where a host panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostPanelAnchor {
    /// A popover under the Contextual Task Bar, from the page's action naming it.
    TaskBar,
    /// A panel of the docking system, `host:<id>`.
    Dock,
}

/// How a text reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Plain,
    Hint,
    Error,
}

/// A button of a host panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostButton {
    pub id: String,
    pub label: String,
    pub primary: bool,
    pub enabled: bool,
    /// One of the app's icons, drawn in place of the label (which becomes its tooltip and its
    /// name to a screen reader); none, or one the app doesn't have, and the label is shown.
    pub icon: Option<String>,
}

/// A row of a host panel's list, with its buttons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostListItem {
    pub id: String,
    pub text: String,
    pub tone: Tone,
    pub buttons: Vec<HostButton>,
}

/// One item of a host panel, top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub enum HostItem {
    Text { text: String, tone: Tone },
    Button(HostButton),
    Row(Vec<HostButton>),
    TextArea { id: String, label: String, value: String, placeholder: String, max_chars: usize },
    Checkbox { id: String, label: String, checked: bool, enabled: bool },
    Separator,
    List(Vec<HostListItem>),
}

/// What the page hears from its panels: a click (with the objects selected then) or a change.
#[derive(Clone, Debug, PartialEq)]
pub enum HostPanelEvent {
    Click { panel: String, id: String, selection: Vec<vectorcraft_doc::NodeId> },
    Change { panel: String, id: String, value: Value },
}

/// The host panels' state that lives between frames: the text each text area holds while the user
/// edits it (by `<panel>/<field>`), the field that has the keyboard, and the events not yet heard.
#[derive(Clone, Debug, Default)]
pub struct HostPanels {
    pub panels: Vec<HostPanel>,
    pub texts: HashMap<String, String>,
    pub focused: Option<String>,
    pub events: Vec<HostPanelEvent>,
    /// The task bar popover open, by its panel's id. Kept here, not by the frame it was last drawn
    /// in, so a step that selects something else for a moment (the page's own, run on the
    /// selection) doesn't close it: only its button, Escape or a click outside it does.
    pub open: Option<String>,
}

fn key(panel: &str, field: &str) -> String {
    format!("{panel}/{field}")
}

/// A panel the page sent, added or put in place of the one of its id. A text area's value the page
/// sent again is taken unless the user is editing that field, so the page can send the panel again
/// without undoing what is being typed. A docked panel keeps its place.
pub fn show_panel(app: &mut VectorcraftApp, panel: HostPanel) {
    let state = &mut app.ui.host_panels;
    for item in &panel.content {
        if let HostItem::TextArea { id, value, .. } = item {
            let k = key(&panel.id, id);
            if state.focused.as_deref() != Some(k.as_str()) {
                state.texts.insert(k, value.clone());
            }
        }
    }
    match state.panels.iter_mut().find(|p| p.id == panel.id) {
        Some(p) => *p = panel,
        None => state.panels.push(panel),
    }
}

/// The page's panel `id` taken away (nothing when there is none); a docked one leaves the dock.
pub fn close_panel(app: &mut VectorcraftApp, id: &str) {
    let state = &mut app.ui.host_panels;
    let was = state.panels.iter().position(|p| p.id == id).map(|i| state.panels.remove(i));
    state.texts.retain(|k, _| !k.starts_with(&format!("{id}/")));
    if was.is_some_and(|p| p.anchor == HostPanelAnchor::Dock) {
        // Out of the dock as the user closes it; one never shown has no place to leave, so the
        // command's error says nothing to anyone.
        let _ = app.run("window.panel.close", json!({ "panel": format!("{PREFIX}{id}") }));
    }
}

/// The page's panel `id` of a docked panel's id (`host:<id>`), when it has one.
pub fn docked(app: &VectorcraftApp, panel: &str) -> Option<HostPanel> {
    let id = panel.strip_prefix(PREFIX)?;
    app.ui.host_panels.panels.iter().find(|p| p.id == id && p.anchor == HostPanelAnchor::Dock).cloned()
}

/// Whether `panel` is a host panel's docking id (`host:<id>`).
pub fn is_host(panel: &str) -> bool {
    panel.strip_prefix(PREFIX).is_some_and(|id| !id.is_empty() && id.chars().count() <= 100)
}

/// Host panel `panel` as the `&'static str` the docking code keeps panel ids as
/// ([`crate::menus::normalize_panel`]); `None` when it isn't a host panel's id, or past
/// [`MAX_INTERNED`] of them in a session.
pub(crate) fn intern(panel: &str) -> Option<&'static str> {
    if !is_host(panel) {
        return None;
    }
    static IDS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut ids = IDS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(id) = ids.iter().find(|id| **id == panel) {
        return Some(id);
    }
    if ids.len() >= MAX_INTERNED {
        return None;
    }
    let id: &'static str = Box::leak(panel.into());
    ids.push(id);
    Some(id)
}

/// A docked host panel's body, as a panel of the docking system draws it.
pub fn show_docked(app: &mut VectorcraftApp, ui: &mut Ui, panel: &str) {
    match docked(app, panel) {
        Some(p) => show(app, ui, &p),
        None => {
            widgets::dim_label(ui, tl!("Nothing here."));
        }
    }
}

/// Panel `p`'s items, drawn with VectorCraft's own widgets; what the user does is kept for the page.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui, p: &HostPanel) {
    let selection = app.session.active().map(|st| st.selection.objects.clone()).unwrap_or_default();
    let state = &mut app.ui.host_panels;
    let click = |id: &str, events: &mut Vec<HostPanelEvent>| {
        events.push(HostPanelEvent::Click { panel: p.id.clone(), id: id.to_string(), selection: selection.clone() });
    };
    ui.set_min_width(240.0);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        for item in &p.content {
            match item {
                HostItem::Text { text, tone } => text_item(ui, text, *tone),
                HostItem::Button(b) => {
                    if button(ui, b) {
                        click(&b.id, &mut state.events);
                    }
                }
                HostItem::Row(buttons) => {
                    ui.horizontal_wrapped(|ui| {
                        for b in buttons {
                            if button(ui, b) {
                                click(&b.id, &mut state.events);
                            }
                        }
                    });
                }
                HostItem::TextArea { id, label, placeholder, max_chars, .. } => {
                    text_area(ui, state, &p.id, id, label, placeholder, *max_chars);
                }
                HostItem::Checkbox { id, label, checked, enabled } => {
                    if widgets::check(ui, label, *checked, *enabled) {
                        state.events.push(HostPanelEvent::Change { panel: p.id.clone(), id: id.clone(), value: Value::Bool(!checked) });
                    }
                }
                HostItem::Separator => widgets::divider(ui),
                HostItem::List(items) => {
                    for it in items {
                        ui.horizontal_wrapped(|ui| {
                            text_item(ui, &it.text, it.tone);
                            for b in &it.buttons {
                                if button(ui, b) {
                                    click(&format!("{}/{}", it.id, b.id), &mut state.events);
                                }
                            }
                        });
                    }
                }
            }
        }
    });
}

fn text_item(ui: &mut Ui, text: &str, tone: Tone) {
    let t = Tokens::get(ui.ctx());
    let color = match tone {
        Tone::Plain => t.text,
        Tone::Hint => t.text_dim,
        Tone::Error => t.error,
    };
    ui.add(egui::Label::new(egui::RichText::new(text).color(color)).wrap());
}

/// A button of a host panel, primary or flat; true when clicked (never while disabled).
fn button(ui: &mut Ui, b: &HostButton) -> bool {
    if let Some(icon) = b.icon.as_deref().filter(|i| !b.primary && crate::icons::exists(i)) {
        return widgets::icon_button_enabled(ui, icon, &b.label, false, b.enabled, 22.0).clicked() && b.enabled;
    }
    if b.primary {
        return widgets::primary_button_enabled(ui, &b.label, b.enabled).clicked();
    }
    ui.add_enabled_ui(b.enabled, |ui| widgets::flat_button_fit(ui, &b.label, 60.0)).inner.clicked() && b.enabled
}

/// A text area holding what the user types (kept between frames by `<panel>/<field>`), at most
/// `max_chars` (0: no limit); each frame its text changed, and once more as it loses the keyboard,
/// the page hears the text.
fn text_area(ui: &mut Ui, state: &mut HostPanels, panel: &str, id: &str, label: &str, placeholder: &str, max_chars: usize) {
    let t = Tokens::get(ui.ctx());
    let k = key(panel, id);
    let mut text = state.texts.get(&k).cloned().unwrap_or_default();
    widgets::field_label(ui, label);
    let mut edit = egui::TextEdit::multiline(&mut text).frame(egui::Frame::NONE).desired_width(f32::INFINITY).desired_rows(3).hint_text(placeholder);
    if max_chars > 0 {
        edit = edit.char_limit(max_chars);
    }
    let r = egui::Frame::NONE
        .fill(t.input)
        .stroke(egui::Stroke::new(1.0, t.input_border))
        .corner_radius(egui::CornerRadius::same(3))
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| ui.add(edit))
        .inner
        .accessible_name(label);
    let changed = r.changed();
    if r.has_focus() {
        state.focused = Some(k.clone());
    } else if state.focused.as_deref() == Some(k.as_str()) {
        state.focused = None;
        state.events.push(HostPanelEvent::Change { panel: panel.to_string(), id: id.to_string(), value: Value::String(text.clone()) });
    }
    if changed {
        state.events.push(HostPanelEvent::Change { panel: panel.to_string(), id: id.to_string(), value: Value::String(text.clone()) });
    }
    state.texts.insert(k, text);
}

/// Panel `panel`'s popover under its task bar button `resp`: opened and closed by the button
/// (`toggle`), closed by Escape or a press outside it (and outside the popups it opened), and
/// otherwise kept open, also through frames the button isn't drawn in.
pub(crate) fn task_bar_popover(app: &mut VectorcraftApp, resp: &egui::Response, toggle: bool, panel: &HostPanel) {
    let ctx = &resp.ctx;
    let is_open = app.ui.host_panels.open.as_deref() == Some(panel.id.as_str());
    let mut open = if toggle {
        !is_open
    } else {
        let click = ctx.input(|i| i.pointer.any_click().then(|| i.pointer.interact_pos()).flatten());
        let elsewhere = click.is_some_and(|p| !resp.rect.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l.order != egui::Order::Foreground));
        is_open && !elsewhere && !ctx.input(|i| i.key_pressed(egui::Key::Escape))
    };
    if open {
        let id = egui::Id::unique("host-panel-popover").with(&panel.id);
        egui::Popup::menu(resp).id(id).open_bool(&mut open).close_behavior(egui::PopupCloseBehavior::IgnoreClicks).show(|ui| show(app, ui, panel));
    }
    let state = &mut app.ui.host_panels;
    if open {
        state.open = Some(panel.id.clone());
    } else if is_open {
        state.open = None;
    }
}

/// A task bar action's popover: panel `id` when the page has one anchored to the task bar.
pub(crate) fn task_bar_panel(app: &VectorcraftApp, id: &str) -> Option<HostPanel> {
    app.ui.host_panels.panels.iter().find(|p| p.id == id && p.anchor == HostPanelAnchor::TaskBar).cloned()
}
