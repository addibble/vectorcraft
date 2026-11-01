//! A host page's panels (`vectorcraft:panel`, host-only): drawn with VectorCraft's own widgets, as a
//! popover from the page's task bar action or a docked panel, and every click and change handed to
//! the page with the panel, the item and (for a click) what was selected.

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::control::{ControlRequest, Outcome};
use crate::host_panels::{HostButton, HostItem, HostPanel, HostPanelAnchor, HostPanelEvent, Tone, close_panel, show_panel};
use crate::state::{HostTaskBarAction, HostTaskBarWhen};
use crate::{Services, VectorcraftApp};

fn request(app: &mut VectorcraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    let Outcome::Done(r) = crate::control::handle(app, ctx, &req) else { panic!("not done") };
    r
}

/// Whole frames of the app in a 1600 × 900 window, its synthetic input taken as a host takes it.
fn frames(app: &mut VectorcraftApp, ctx: &egui::Context, count: usize) {
    for _ in 0..count {
        let mut raw =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0))), ..Default::default() };
        app.raw_input_hook(ctx, &mut raw);
        let mut out = ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
    }
}

/// The middle of the widget named `label` the last frames drew, if any (the first request turns the
/// screen-reader tree on).
fn widget(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) -> Option<(f64, f64)> {
    request(app, ctx, "ui.widgets", json!({}));
    frames(app, ctx, 2);
    let answer = request(app, ctx, "ui.widgets", json!({"label": label}));
    let found = answer["result"]["widgets"].as_array()?.iter().find(|w| w["label"] == label)?.clone();
    let r: Vec<f64> = found["rect"].as_array()?.iter().filter_map(Value::as_f64).collect();
    Some((r[0] + r[2] / 2.0, r[1] + r[3] / 2.0))
}

fn click(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    let (x, y) = widget(app, ctx, label).unwrap_or_else(|| panic!("no {label} shows"));
    assert_eq!(request(app, ctx, "ui.click", json!({"x": x, "y": y}))["ok"], true);
    frames(app, ctx, 3);
}

fn button(id: &str, label: &str, enabled: bool) -> HostButton {
    HostButton { id: id.into(), label: label.into(), primary: false, enabled }
}

/// A document with one picture selected with the Selection tool, and the page's task bar action
/// for it naming panel `panel`.
fn picture_with_action(panel: Option<&str>) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let img = image::RgbaImage::from_pixel(20, 20, image::Rgba([90, 120, 200, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    app.run("file.place", json!({"name": "p.png", "dataBase64": vectorcraft_format::base64_encode(&png), "link": false, "at": [200, 150]})).unwrap();
    app.select_tool("selection");
    app.ui.host_task_bar = vec![HostTaskBarAction {
        id: "fill".into(),
        label: "Generative Fill".into(),
        icon: None,
        when: HostTaskBarWhen::Image,
        linked: false,
        enabled: true,
        panel: panel.map(String::from),
    }];
    app
}

/// A task bar action naming a panel opens it as a popover under the bar, and closes it again,
/// without telling the page the action was clicked; a click in the popover is the page's, with what
/// was selected; a disabled button there tells it nothing.
#[test]
fn a_task_bar_action_opens_its_panel_and_the_page_hears_its_buttons() {
    let mut app = picture_with_action(Some("fill"));
    show_panel(
        &mut app,
        HostPanel {
            id: "fill".into(),
            title: "Generative Fill".into(),
            anchor: HostPanelAnchor::TaskBar,
            content: vec![
                HostItem::Text { text: "Paint black over what to replace.".into(), tone: Tone::Hint },
                HostItem::Row(vec![button("setup", "Set Up", true), button("remove", "Remove Setup", false)]),
            ],
        },
    );
    let ctx = egui::Context::default();
    frames(&mut app, &ctx, 2);
    assert!(widget(&mut app, &ctx, "Set Up").is_none(), "closed until the action is clicked");
    click(&mut app, &ctx, "Generative Fill");
    assert!(app.ui.host_task_bar_clicked.is_empty(), "the action opened its panel, the page heard no click");
    assert!(widget(&mut app, &ctx, "Set Up").is_some(), "the panel is open");

    click(&mut app, &ctx, "Remove Setup");
    assert!(app.ui.host_panels.events.is_empty(), "a disabled button tells the page nothing");
    click(&mut app, &ctx, "Set Up");
    let selection = app.session.active().unwrap().selection.objects.clone();
    assert_eq!(app.ui.host_panels.events, vec![HostPanelEvent::Click { panel: "fill".into(), id: "setup".into(), selection }]);

    click(&mut app, &ctx, "Generative Fill");
    assert!(widget(&mut app, &ctx, "Set Up").is_none(), "the action closes it again");
}

/// A docked panel opens as any panel does (`window.panel`), keeps its place when the page sends it
/// again, tells the page of a checkbox's change, and leaves the dock when the page closes it.
#[test]
fn a_docked_panel_opens_keeps_its_place_and_closes() {
    let mut app = picture_with_action(None);
    let list = |checked: bool| HostPanel {
        id: "short".into(),
        title: "Left Short".into(),
        anchor: HostPanelAnchor::Dock,
        content: vec![HostItem::Checkbox { id: "king".into(), label: "King of hearts".into(), checked, enabled: true }],
    };
    show_panel(&mut app, list(true));
    let ctx = egui::Context::default();
    frames(&mut app, &ctx, 2);
    app.run("window.panel", json!({"panel": "host:short"})).unwrap();
    frames(&mut app, &ctx, 3);
    let at = widget(&mut app, &ctx, "King of hearts").expect("the panel shows");
    show_panel(&mut app, list(true));
    frames(&mut app, &ctx, 2);
    assert_eq!(widget(&mut app, &ctx, "King of hearts"), Some(at), "sent again, it stays where it was");

    click(&mut app, &ctx, "King of hearts");
    assert_eq!(app.ui.host_panels.events, vec![HostPanelEvent::Change { panel: "short".into(), id: "king".into(), value: json!(false) }]);

    close_panel(&mut app, "short");
    frames(&mut app, &ctx, 2);
    assert!(app.ui.host_panels.panels.is_empty());
    assert!(widget(&mut app, &ctx, "King of hearts").is_none(), "closed, it leaves the dock");
}

/// A text area keeps what the user types: the page hears each change, the limit holds, and the page
/// sending the panel again doesn't undo the typing while the field has the keyboard.
#[test]
fn a_text_area_keeps_what_is_typed() {
    let mut app = picture_with_action(None);
    let panel = |value: &str| HostPanel {
        id: "ask".into(),
        title: "Ask".into(),
        anchor: HostPanelAnchor::Dock,
        content: vec![HostItem::TextArea {
            id: "instructions".into(),
            label: "Instructions".into(),
            value: value.into(),
            placeholder: String::new(),
            max_chars: 5,
        }],
    };
    show_panel(&mut app, panel(""));
    let ctx = egui::Context::default();
    app.run("window.panel", json!({"panel": "host:ask"})).unwrap();
    frames(&mut app, &ctx, 3);
    click(&mut app, &ctx, "Instructions");
    assert_eq!(app.ui.host_panels.focused.as_deref(), Some("ask/instructions"), "the field has the keyboard");
    let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0))), ..Default::default() };
    raw.events.push(egui::Event::Text("Keep the sky".into()));
    app.raw_input_hook(&ctx, &mut raw);
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    assert_eq!(app.ui.host_panels.texts.get("ask/instructions").map(String::as_str), Some("Keep "), "at most 5 characters");
    assert_eq!(
        app.ui.host_panels.events.last(),
        Some(&HostPanelEvent::Change { panel: "ask".into(), id: "instructions".into(), value: json!("Keep ") })
    );

    show_panel(&mut app, panel("from the page"));
    assert_eq!(app.ui.host_panels.texts.get("ask/instructions").map(String::as_str), Some("Keep "), "the field being edited keeps its text");
    app.ui.host_panels.focused = None;
    show_panel(&mut app, panel("from the page"));
    assert_eq!(app.ui.host_panels.texts.get("ask/instructions").map(String::as_str), Some("from the page"));
}

/// A task bar popover stays open while a step selects something else for a moment (and its button
/// isn't drawn), and comes back with the picture's button; Escape closes it.
#[test]
fn a_task_bar_popover_outlasts_a_moments_other_selection() {
    let mut app = picture_with_action(Some("fill"));
    let image = app.session.active().unwrap().selection.objects[0].0;
    show_panel(
        &mut app,
        HostPanel {
            id: "fill".into(),
            title: "Generative Fill".into(),
            anchor: HostPanelAnchor::TaskBar,
            content: vec![HostItem::Button(button("setup", "Set Up", true))],
        },
    );
    let ctx = egui::Context::default();
    frames(&mut app, &ctx, 2);
    click(&mut app, &ctx, "Generative Fill");
    assert!(widget(&mut app, &ctx, "Set Up").is_some());
    let r = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    app.run("select.set", json!({"ids": [r["id"]]})).unwrap();
    frames(&mut app, &ctx, 3);
    app.run("select.set", json!({"ids": [image]})).unwrap();
    frames(&mut app, &ctx, 3);
    assert!(widget(&mut app, &ctx, "Set Up").is_some(), "open again with the picture's button");
    let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0))), ..Default::default() };
    raw.events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    app.raw_input_hook(&ctx, &mut raw);
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    frames(&mut app, &ctx, 2);
    assert!(widget(&mut app, &ctx, "Set Up").is_none(), "Escape closes it");
}
