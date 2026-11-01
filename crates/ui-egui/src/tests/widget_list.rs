//! The control channel's `ui.widgets` names what the window shows and where, so a script can open
//! a menu and choose an item with `ui.click` as a person does; `ui.set` `cursorScale` sizes the
//! tool cursors the canvas paints.

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::control::{ControlRequest, Outcome};
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

/// The widgets of a `ui.widgets` answer named `label`: exactly, or a menu item's name before its
/// shortcut (`Fit All in Window ⌥⌘0`).
fn named(answer: &Value, label: &str) -> Vec<Value> {
    let item = format!("{label} ");
    answer["result"]["widgets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["label"].as_str().is_some_and(|l| l == label || l.trim_start().starts_with(&item)))
        .cloned()
        .collect()
}

#[test]
fn ui_widgets_names_the_menus_and_a_click_on_one_opens_it() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    frames(&mut app, &ctx, 2);
    let first = request(&mut app, &ctx, "ui.widgets", json!({}));
    assert_eq!(first["result"]["pending"], true, "the first request turns the tree on: {first}");
    frames(&mut app, &ctx, 2);
    let all = request(&mut app, &ctx, "ui.widgets", json!({}));
    assert_eq!(all["result"]["pending"], false);
    let view = named(&all, "View");
    assert_eq!(view.len(), 1, "one View menu: {view:?}");
    let rect: Vec<f64> = view[0]["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    assert!(rect[2] > 0.0 && rect[3] > 0.0 && rect[0] >= 0.0 && rect[1] >= 0.0 && rect[1] < 100.0, "in the bar at the top: {rect:?}");
    assert!(named(&all, "Fit All in Window").is_empty(), "the View menu is closed");

    let only = request(&mut app, &ctx, "ui.widgets", json!({"label": "View"}));
    assert!(only["result"]["widgets"].as_array().unwrap().iter().all(|w| w["label"].as_str().unwrap().contains("View")), "{only}");

    let (x, y) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    assert_eq!(request(&mut app, &ctx, "ui.click", json!({"x": x, "y": y}))["ok"], true);
    frames(&mut app, &ctx, 6);
    let open = request(&mut app, &ctx, "ui.widgets", json!({"label": "Fit All in Window"}));
    assert_eq!(named(&open, "Fit All in Window").len(), 1, "the click opened the View menu: {open}");
}

#[test]
fn ui_set_sizes_the_painted_tool_cursors_within_bounds() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let ctx = egui::Context::default();
    assert_eq!(app.cursor_scale, 1.0);
    assert_eq!(request(&mut app, &ctx, "ui.set", json!({"cursorScale": 2.5}))["ok"], true);
    assert_eq!(app.cursor_scale, 2.5);
    for bad in [json!(0.1), json!(5), json!(-1)] {
        assert_eq!(request(&mut app, &ctx, "ui.set", json!({"cursorScale": bad}))["ok"], false, "{bad}");
        assert_eq!(app.cursor_scale, 2.5, "a refused size leaves it");
    }
}

/// A host page's task bar action is a button screen readers (and so `ui.widgets`) name, so a script
/// clicks it by its label as a person does.
#[test]
fn ui_widgets_names_the_host_pages_task_bar_actions() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let img = image::RgbaImage::from_pixel(20, 20, image::Rgba([90, 120, 200, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    app.run("file.place", json!({"name": "p.png", "dataBase64": vectorcraft_format::base64_encode(&png), "link": false, "at": [200, 150]})).unwrap();
    app.select_tool("selection");
    app.ui.host_task_bar = vec![crate::state::HostTaskBarAction {
        id: "crop".into(),
        label: "Crop to Subject".into(),
        icon: None,
        when: crate::state::HostTaskBarWhen::Image,
        linked: false,
        enabled: true,
        panel: None,
    }];
    let ctx = egui::Context::default();
    frames(&mut app, &ctx, 2);
    request(&mut app, &ctx, "ui.widgets", json!({}));
    frames(&mut app, &ctx, 2);
    let answer = request(&mut app, &ctx, "ui.widgets", json!({"label": "Crop to Subject"}));
    let found = named(&answer, "Crop to Subject");
    assert_eq!(found.len(), 1, "{answer}");
    assert_eq!(found[0]["role"], "Button");
}
