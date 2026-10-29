//! Links panel → Placement Options: how a file read again (Relink, Update Link) takes the place of
//! the selected images: what it preserves, where it aligns in the old bounds, and whether it is
//! clipped to them. OK also applies them at once to the linked images, from the file each already
//! shows: Fit or Fill puts a distorted image back in proportion, File Dimensions at its 100% size.
//!
//! Fields: `ids` (the images; null: the selected ones), `preserve` (transforms|bounds|fileDimensions|fit|fill), `align`
//! (topLeft … bottomRight) and `clip`; `mixed`: those the images don't share, shown as the first
//! image's (`opened`) and left as each image has them unless changed. OK runs
//! `links.placementOptions` with them: one undo step.

use serde_json::{Value, json};

use super::DialogSpec;
use super::form;
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

pub(super) const KIND: &str = "placementOptions";
const CMD: &str = "links.placementOptions";
/// (value, label) of each Preserve choice.
const PRESERVE: [(&str, &str); 5] = [
    ("transforms", "Transforms"),
    ("bounds", "Bounds"),
    ("fileDimensions", "File Dimensions"),
    ("fit", "Bounds (Fit Proportionally)"),
    ("fill", "Bounds (Fill Proportionally)"),
];
/// The alignment points, row by row from the top left (the reference point widget's order).
const ALIGN: [&str; 9] = ["topLeft", "top", "topRight", "left", "center", "right", "bottomLeft", "bottom", "bottomRight"];
const LABEL: f32 = 90.0;

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Placement Options").into(), body, confirm, min_width: 400.0, max_width: Some(420.0), ..DialogSpec::FORM };

/// Open the dialog on the placement options of images `ids` (default: the selected ones).
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let ids = p.get("ids").cloned().unwrap_or(Value::Null);
    let r = app.session.execute(CMD, &json!({ "ids": ids })).map_err(|e| e.to_string())?;
    let mut fields = r["placement"].clone();
    fields["ids"] = ids;
    if let Some(mixed) = r.get("mixed") {
        fields["mixed"] = mixed.clone();
        fields["opened"] = r["placement"].clone();
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = d.fields.clone();
    let (mixed, opened) = (p.remove("mixed"), p.remove("opened"));
    for k in mixed.as_ref().and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        if opened.as_ref().and_then(|o| o.get(k)) == p.get(k) {
            p.remove(k);
        }
    }
    let r = app.run(CMD, Value::Object(p));
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    form::choice(ui, d, "preserve", tl!("Preserve:"), (LABEL, 240.0), &PRESERVE);
    ui.add_space(8.0);
    // Bounds stretches the art into the old bounds: nothing to align or clip.
    let free = d.str("preserve") != "bounds";
    ui.add_enabled_ui(free, |ui| {
        widgets::label_row(ui, tl!("Alignment:"), LABEL, |ui| {
            let current = ALIGN.iter().position(|a| *a == d.str("align")).unwrap_or(4);
            if let Some(i) = widgets::reference_point(ui, current)
                && let Some(a) = ALIGN.get(i)
            {
                d.fields.insert("align".into(), json!(a));
            }
        });
        ui.add_space(8.0);
        widgets::label_row(ui, "", LABEL, |ui| {
            let clip = d.bool("clip");
            if widgets::check(ui, tl!("Clip to Bounding Box"), clip, free) {
                d.fields.insert("clip".into(), json!(!clip));
            }
        });
    });
    ui.add_space(6.0);
    widgets::dim_label(ui, tl!("Applies to linked files now, and when they are relinked or updated."));
    false
}
