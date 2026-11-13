//! The control channel's `ui.widgets`: every widget a pass drew that screen readers can name (menu
//! titles and items, buttons, tools, fields), with its role, name, value and where it is in window
//! points. A script finds a menu or a button by its name and clicks it with `ui.click`, as a person
//! would, instead of running the command behind it. The names come from egui's screen-reader
//! (AccessKit) tree: a request is answered at the end of the first pass after it that builds the
//! tree, which the request turns on for that pass when no screen reader has it on, and off again
//! once it is answered. A request no pass answers within [`TIMEOUT_MS`] is refused. Under a read
//! root the reply hides what every reply hides ([`super::hide_in_reply`]: libraries and recovery
//! folders outside it), and the recent files outside it.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::{Value, json};

/// The most `ui.widgets` requests waiting for a pass at once; more are refused.
const MAX_WAITING: usize = 16;

/// How long a request waits for a pass that builds the screen-reader tree, in milliseconds, as a
/// screenshot waits for its frame; then it is refused ([`expire`]).
pub(crate) const TIMEOUT_MS: f64 = crate::state::SCREENSHOT_TIMEOUT_MS;

/// One widget of a pass.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Widget {
    /// Its screen-reader role: the name of AccessKit's `Role` (`Button`, `MenuItem`, `TextInput`…).
    pub role: String,
    /// Its name as a screen reader reads it: its own label, else the labels of the widgets that
    /// label it, else the text of a label.
    pub label: String,
    /// Its value (a field's text, a slider's number), when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Left, top, width and height.
    pub rect: [f32; 4],
    /// Does it take clicks (not disabled)?
    pub enabled: bool,
    /// Is another layer (a menu, a dialog) over its middle, so a click there would reach that
    /// layer instead?
    pub covered: bool,
}

/// A `ui.widgets` request waiting for a pass with the screen-reader tree.
pub(crate) struct Waiting {
    asked: Asked,
    /// When it is refused ([`crate::clock::now_ms`]).
    deadline_ms: f64,
    reply: std::sync::mpsc::Sender<Value>,
}

/// What a request asked: its label filter, and what it may not read, worked out under its read
/// root ([`crate::file_access::confine`]) when it is handled.
pub struct Asked {
    label: Option<String>,
    /// Names the request may not read (recent files outside its read root), left out.
    hidden: Hidden,
    /// The libraries hidden when it was handled, hidden in the reply too ([`super::handle`]).
    pub(super) before: super::Hiding,
}

impl Asked {
    /// Does the reply list `widget`: does it name no hidden recent file? A full path hides any
    /// widget; a bare file name only a button or a menu item, which is how the window lists recent
    /// files (the Home screen's rows, File › Open Recent Files), so a layer of that name stays.
    pub(crate) fn shows(&self, widget: &Widget) -> bool {
        let row = matches!(widget.role.as_str(), "Button" | "MenuItem" | "MenuItemCheckBox" | "MenuItemRadio" | "Link");
        let names = |text: &str| self.hidden.holds_a_path(text) || (row && self.hidden.is_a_name(text));
        !names(&widget.label) && !widget.value.as_deref().is_some_and(names)
    }
}

/// The recent files a request under a read root may not read, as the window names them: their
/// full path (a Recent Files row's tooltip) and their file name (the row, and File › Open Recent
/// Files' item), as `file.recentFiles` leaves them out.
#[derive(Default)]
struct Hidden {
    paths: Vec<String>,
    names: Vec<String>,
}

impl Hidden {
    fn of(app: &crate::VectorcraftApp) -> Self {
        let mut hidden = Hidden::default();
        for path in crate::io::recent_files(app).iter().filter(|p| vectorcraft_engine::file_access::check_read(p).is_err()) {
            let name = std::path::Path::new(path).file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
            hidden.paths.push(path.clone());
            hidden.names.push(name);
        }
        hidden
    }

    /// Does `text` hold one of the files' paths?
    fn holds_a_path(&self, text: &str) -> bool {
        self.paths.iter().any(|p| text.contains(p.as_str()))
    }

    /// Does `text` read as one of the files' names (a menu item's name before its shortcut too)?
    fn is_a_name(&self, text: &str) -> bool {
        let t = text.trim();
        self.names.iter().any(|n| t == n || t.strip_prefix(n.as_str()).is_some_and(|rest| rest.starts_with(char::is_whitespace)))
    }
}

/// `ui.widgets` with `params`: what it asks, or why it is refused.
pub fn ask(app: &crate::VectorcraftApp, params: &Value) -> Result<Asked, String> {
    let label = match params.get("label") {
        None | Some(Value::Null) => None,
        Some(Value::String(l)) => Some(l.clone()),
        Some(_) => return Err("`label` must be a string".into()),
    };
    Ok(Asked { label, hidden: Hidden::of(app), before: super::Hiding::default() })
}

/// Keep `asked` until a pass with the screen-reader tree answers it on `reply`
/// ([`end_of_pass`]); refused while [`MAX_WAITING`] requests wait.
pub(crate) fn queue(app: &mut crate::VectorcraftApp, ctx: &egui::Context, asked: Asked, reply: std::sync::mpsc::Sender<Value>) {
    if app.widget_requests.len() >= MAX_WAITING {
        let _ = reply.send(json!({"ok": false, "error": format!("already {MAX_WAITING} ui.widgets requests are waiting for a frame")}));
        return;
    }
    app.widget_requests.push(Waiting { asked, deadline_ms: crate::clock::now_ms() + TIMEOUT_MS, reply });
    ctx.request_repaint();
}

/// Refuses the requests whose time is up at `now` ([`crate::clock::now_ms`]): a minimized window
/// may draw no pass. Called on every tick, drawn or not; while requests wait it asks for another
/// tick by the first deadline. The tree goes off once none waits, as [`end_of_pass`] leaves it.
pub(crate) fn expire(app: &mut crate::VectorcraftApp, ctx: &egui::Context, now: f64) {
    if app.widget_requests.is_empty() {
        return;
    }
    let (late, waiting): (Vec<Waiting>, Vec<Waiting>) = std::mem::take(&mut app.widget_requests).into_iter().partition(|w| now >= w.deadline_ms);
    app.widget_requests = waiting;
    for w in late {
        let _ = w.reply.send(json!({"ok": false, "error": "no frame was drawn (window minimized or hidden) to list the widgets of"}));
    }
    match app.widget_requests.iter().map(|w| w.deadline_ms).reduce(f64::min) {
        Some(first) => ctx.request_repaint_after(std::time::Duration::from_secs_f64(((first - now) / 1000.0).clamp(0.0, TIMEOUT_MS / 1000.0))),
        None => {
            if std::mem::take(&mut app.widget_tree_ours) {
                ctx.disable_accesskit();
            }
        }
    }
}

/// At the end of a pass, after every bar, panel, menu and dialog drew: answer the waiting
/// requests with its widgets when it built the screen-reader tree, else turn the tree on for the
/// next pass (again, when a screen reader turned it off meanwhile). The tree goes off once they
/// are answered, unless something else (a screen reader) had it on, or a screen reader acted on
/// it meanwhile (an AccessKit action in a pass's input): egui says nothing else of one that comes
/// while a request has the tree on, so one that starts and does nothing in those passes still loses
/// it.
pub(crate) fn end_of_pass(app: &mut crate::VectorcraftApp, ctx: &egui::Context) {
    if app.widget_tree_ours && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::AccessKitActionRequest(_)))) {
        app.widget_tree_ours = false;
    }
    if app.widget_requests.is_empty() {
        return;
    }
    // A pass egui throws away is not what the window shows; it asks for another.
    if ctx.will_discard() {
        return;
    }
    let Some(widgets) = of_pass(ctx) else {
        ctx.enable_accesskit();
        app.widget_tree_ours = true;
        ctx.request_repaint();
        return;
    };
    let roots = app.automation_roots.clone();
    for w in std::mem::take(&mut app.widget_requests) {
        let Waiting { asked, reply, .. } = w;
        let chosen: Vec<&Widget> = widgets
            .iter()
            .filter(|x| asked.shows(x))
            .filter(|x| asked.label.as_deref().is_none_or(|l| x.label.contains(l) || x.value.as_deref().is_some_and(|v| v.contains(l))))
            .collect();
        let mut answer = json!({"ok": true, "result": {"widgets": chosen}});
        // Under the request's read root, as a screenshot's reply is hidden when it is sent.
        vectorcraft_engine::file_access::confine(roots.as_ref(), || super::hide_in_deferred_reply_with(app, &mut answer, &json!({}), asked.before));
        let _ = reply.send(answer);
    }
    if std::mem::take(&mut app.widget_tree_ours) {
        ctx.disable_accesskit();
    }
}

/// The named widgets of the pass under way, read at its end, in no particular order; `None` when
/// the pass built no screen-reader tree.
pub(crate) fn of_pass(ctx: &egui::Context) -> Option<Vec<Widget>> {
    type Found = (egui::LayerId, egui::Rect, String, String, Option<String>, bool);
    let found: Option<Vec<Found>> = ctx.viewport(|viewport| {
        let pass = &viewport.this_pass;
        let state = pass.accesskit_state.as_ref()?;
        let by_node: HashMap<egui::accesskit::NodeId, &egui::accesskit::Node> = state.nodes.iter().map(|(id, n)| (id.accesskit_id(), n)).collect();
        Some(
            state
                .nodes
                .iter()
                .filter_map(|(id, node)| {
                    let by: Vec<String> = node
                        .labelled_by()
                        .iter()
                        .filter_map(|l| by_node.get(l).and_then(|n| name_of(n, &by_node, &mut vec![id.accesskit_id(), *l])))
                        .collect();
                    let value = node
                        .value()
                        .filter(|v| !v.trim().is_empty())
                        .map(str::to_string)
                        .or_else(|| node.numeric_value().filter(|v| v.is_finite()).map(|v| v.to_string()));
                    let label = node
                        .label()
                        .filter(|l| !l.trim().is_empty())
                        .map(str::to_string)
                        .or_else(|| (!by.is_empty()).then(|| by.join(" ")))
                        .or_else(|| (node.role() == egui::accesskit::Role::Label).then(|| value.clone()).flatten());
                    if label.is_none() && value.is_none() {
                        return None;
                    }
                    let widget = pass.widgets.get(*id)?;
                    let value = value.filter(|v| label.as_ref() != Some(v));
                    Some((widget.layer_id, widget.interact_rect, format!("{:?}", node.role()), label.unwrap_or_default(), value, !node.is_disabled()))
                })
                .collect(),
        )
    });
    // Outside `viewport`, which holds the context's lock: a layer's transform reads it too.
    Some(
        found?
            .into_iter()
            .filter(|(_, rect, ..)| rect.is_finite() && rect.is_positive())
            .map(|(layer, rect, role, label, value, enabled)| {
                let rect = ctx.layer_transform_to_global(layer).map_or(rect, |t| t * rect);
                let covered = ctx.layer_id_at(rect.center()).is_some_and(|top| top != layer);
                Widget { role, label, value, rect: [rect.left(), rect.top(), rect.width(), rect.height()], enabled, covered }
            })
            .collect(),
    )
}

/// The name a screen reader reads for `node`, a widget that labels another: its own label or
/// value, else the names of the widgets that label it in turn, joined. `seen` holds the widgets
/// on the way, so widgets labelling each other in a loop end it.
fn name_of(
    node: &egui::accesskit::Node,
    by_node: &HashMap<egui::accesskit::NodeId, &egui::accesskit::Node>,
    seen: &mut Vec<egui::accesskit::NodeId>,
) -> Option<String> {
    if let Some(own) = node.label().or_else(|| node.value()).filter(|t| !t.trim().is_empty()) {
        return Some(own.to_string());
    }
    let mut names = vec![];
    for l in node.labelled_by() {
        if seen.contains(l) {
            continue;
        }
        seen.push(*l);
        if let Some(name) = by_node.get(l).and_then(|n| name_of(n, by_node, seen)) {
            names.push(name);
        }
    }
    (!names.is_empty()).then(|| names.join(" "))
}
