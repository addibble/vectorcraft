//! The control channel's `ui.widgets`: every widget the last presented pass drew that screen
//! readers can name (menu titles and items, buttons, tools, fields), with its role and where it is
//! in window points. A script finds a menu or a button by its name and clicks it with `ui.click`,
//! as a person would, instead of running the command behind it. The names come from egui's
//! screen-reader (AccessKit) tree, which the first request turns on.

use serde::Serialize;

/// One widget of a pass: its screen-reader role and name, and its rectangle in window points.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Widget {
    pub role: String,
    pub label: String,
    /// Left, top, width and height.
    pub rect: [f32; 4],
}

/// The named widgets of the pass under way, read at its end (after every bar, panel, menu and
/// dialog has drawn), in no particular order. Empty while the screen-reader tree is off.
pub fn of_pass(ctx: &egui::Context) -> Vec<Widget> {
    let found: Vec<(egui::LayerId, egui::Rect, String, String)> = ctx.viewport(|viewport| {
        let pass = &viewport.this_pass;
        let Some(state) = pass.accesskit_state.as_ref() else { return vec![] };
        state
            .nodes
            .iter()
            .filter_map(|(id, node)| {
                let label = node.label().filter(|l| !l.trim().is_empty())?;
                let widget = pass.widgets.get(*id)?;
                Some((widget.layer_id, widget.interact_rect, format!("{:?}", node.role()), label.to_string()))
            })
            .collect()
    });
    // Outside `viewport`, which holds the context's lock: a layer's transform reads it too.
    found
        .into_iter()
        .filter(|(_, rect, ..)| rect.is_finite() && rect.is_positive())
        .map(|(layer, rect, role, label)| {
            let rect = ctx.layer_transform_to_global(layer).map_or(rect, |t| t * rect);
            Widget { role, label, rect: [rect.left(), rect.top(), rect.width(), rect.height()] }
        })
        .collect()
}

/// `ui.widgets`: the last presented pass's widgets, those whose name contains `label` when given.
/// The first request turns the screen-reader tree on and answers with none (`pending`): ask again
/// after the next frame.
pub fn request(app: &mut crate::VectorcraftApp, ctx: &egui::Context, label: Option<&str>) -> serde_json::Value {
    let Some(widgets) = app.widget_list.as_ref() else {
        ctx.enable_accesskit();
        app.widget_list = Some(vec![]);
        ctx.request_repaint();
        return serde_json::json!({"widgets": [], "pending": true});
    };
    let chosen: Vec<&Widget> = widgets.iter().filter(|w| label.is_none_or(|l| w.label.contains(l))).collect();
    // A frame drawn after the request carries the new tree.
    ctx.request_repaint();
    serde_json::json!({"widgets": chosen, "pending": false})
}
