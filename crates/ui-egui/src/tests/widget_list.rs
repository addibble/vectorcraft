//! The control channel's `ui.widgets` names what the window shows and where, so a script can open
//! a menu and choose an item with `ui.click` as a person does; `ui.set` `cursorScale` sizes the
//! tool cursors the canvas paints.

use std::sync::mpsc::{Receiver, Sender};

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::control::{ControlRequest, Outcome, widget_list};
use crate::{Services, VectorcraftApp};

/// `method` with `params` handled as the channel handles it; `ui.widgets` answered by the frames
/// after it, two at most.
fn request(app: &mut VectorcraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    match crate::control::handle(app, ctx, &req) {
        Outcome::Done(r) => r,
        Outcome::Widgets(asked) => {
            let (tx, rx) = std::sync::mpsc::channel();
            widget_list::queue(app, ctx, asked, tx);
            for _ in 0..2 {
                frame(app, ctx);
                if let Ok(r) = rx.try_recv() {
                    return r;
                }
            }
            panic!("no reply within two frames");
        }
        _ => panic!("not done"),
    }
}

/// One whole frame of the app in a 1600 × 900 window, its synthetic input taken as a host takes
/// it, and what it put out.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context) -> egui::FullOutput {
    let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0))), ..Default::default() };
    app.raw_input_hook(ctx, &mut raw);
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    out
}

fn frames(app: &mut VectorcraftApp, ctx: &egui::Context, count: usize) {
    for _ in 0..count {
        frame(app, ctx);
    }
}

/// An app with a control channel and a document, drawn twice.
fn app_with_channel() -> (VectorcraftApp, Sender<ControlRequest>, egui::Context) {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Services::default()).with_control(rx);
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    frames(&mut app, &ctx, 2);
    (app, tx, ctx)
}

/// `ui.widgets` with `params` sent through the channel, and the reply, which must come within two
/// frames: the frame that takes the request turns the screen-reader tree on, the next names the
/// widgets.
fn widgets(app: &mut VectorcraftApp, tx: &Sender<ControlRequest>, ctx: &egui::Context, params: Value) -> Value {
    let (req, rx): (ControlRequest, Receiver<Value>) = ControlRequest::new("ui.widgets", params);
    tx.send(req).unwrap();
    for _ in 0..2 {
        frame(app, ctx);
        if let Ok(r) = rx.try_recv() {
            return r;
        }
    }
    panic!("no reply within two frames");
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
    let (mut app, tx, ctx) = app_with_channel();
    let all = widgets(&mut app, &tx, &ctx, json!({}));
    assert_eq!(all["ok"], true, "{all}");
    let view = named(&all, "View");
    assert_eq!(view.len(), 1, "one View menu: {view:?}");
    assert_eq!(view[0]["enabled"], true);
    assert_eq!(view[0]["covered"], false);
    let rect: Vec<f64> = view[0]["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    assert!(rect[2] > 0.0 && rect[3] > 0.0 && rect[0] >= 0.0 && rect[1] >= 0.0 && rect[1] < 100.0, "in the bar at the top: {rect:?}");
    assert!(named(&all, "Fit All in Window").is_empty(), "the View menu is closed");

    let only = widgets(&mut app, &tx, &ctx, json!({"label": "View"}));
    let only = only["result"]["widgets"].as_array().unwrap();
    assert!(!only.is_empty() && only.iter().all(|w| w["label"].as_str().unwrap().contains("View")), "{only:?}");

    let (x, y) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    assert_eq!(request(&mut app, &ctx, "ui.click", json!({"x": x, "y": y}))["ok"], true);
    frames(&mut app, &ctx, 6);
    let open = widgets(&mut app, &tx, &ctx, json!({"label": "Fit All in Window"}));
    assert_eq!(named(&open, "Fit All in Window").len(), 1, "the click opened the View menu: {open}");
}

/// Every request is answered with the widgets of a frame drawn after it, however often it is
/// asked, and the screen-reader tree goes off once it is answered: no frame after builds one.
#[test]
fn every_request_is_answered_and_the_tree_goes_off_after() {
    let (mut app, tx, ctx) = app_with_channel();
    for _ in 0..3 {
        let r = widgets(&mut app, &tx, &ctx, json!({}));
        assert_eq!(named(&r, "View").len(), 1, "{r}");
        frames(&mut app, &ctx, 1);
        assert!(frame(&mut app, &ctx).platform_output.accesskit_update.is_none(), "the tree is off between requests");
    }
}

/// A screen reader that turns the tree on keeps it after a request; one that turns it off while
/// requests come doesn't stop them being answered.
#[test]
fn a_screen_readers_tree_is_left_as_it_is() {
    let (mut app, tx, ctx) = app_with_channel();
    ctx.enable_accesskit();
    frames(&mut app, &ctx, 1);
    let r = widgets(&mut app, &tx, &ctx, json!({}));
    assert_eq!(named(&r, "View").len(), 1, "{r}");
    assert!(frame(&mut app, &ctx).platform_output.accesskit_update.is_some(), "the screen reader's tree stays on");
    ctx.disable_accesskit();
    frames(&mut app, &ctx, 1);
    let r = widgets(&mut app, &tx, &ctx, json!({}));
    assert_eq!(named(&r, "View").len(), 1, "after the screen reader left: {r}");
}

/// A request under a read root is not told the names of the recent files outside it, which the
/// Home screen lists, as `file.recentFiles` leaves them out.
#[test]
fn recent_files_outside_the_read_root_are_not_named() {
    let base = std::env::temp_dir().join(format!("vc-ui-widgets-roots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["in", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&base.join("in")], &[]).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Services::default()).with_control(rx).with_automation_roots(roots);
    let path = |d: &str, f: &str| base.join(d).join(f).to_string_lossy().into_owned();
    app.ui.recent_files = vec![path("in", "inside-file.svg"), path("outside", "outside-file.svg")];
    let ctx = egui::Context::default();
    frames(&mut app, &ctx, 2);
    let r = widgets(&mut app, &tx, &ctx, json!({}));
    let text = r.to_string();
    assert!(text.contains("inside-file.svg"), "the Home screen's row inside the root is named: {r}");
    assert!(!text.contains("outside-file.svg"), "the row outside it is not: {r}");
    let _ = std::fs::remove_dir_all(&base);
}

/// A field labelled by a label next to it is named by that label, as a screen reader names it, and
/// shows its text as its value; a disabled button is flagged, and so is one under another layer.
#[test]
fn widgets_are_named_as_a_screen_reader_names_them() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut found = None;
    for _ in 0..3 {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            let label = ui.label("Width");
            let mut text = String::from("12 pt");
            ui.text_edit_singleline(&mut text).labelled_by(label.id);
            ui.add_enabled(false, egui::Button::new("Off"));
            let under = ui.button("Under");
            egui::Area::new(egui::Id::unique("over")).fixed_pos(under.rect.left_top()).show(ui.ctx(), |ui| {
                let _ = ui.button("Over the button below");
            });
            found = widget_list::of_pass(ui.ctx());
        });
        out.textures_delta.clear();
    }
    let found = found.expect("the pass built the tree");
    let one = |label: &str| found.iter().find(|w| w.label == label).unwrap_or_else(|| panic!("no {label}: {found:?}")).clone();
    let field = found.iter().find(|w| w.role == "TextInput").unwrap_or_else(|| panic!("no field: {found:?}"));
    assert_eq!((field.label.as_str(), field.value.as_deref()), ("Width", Some("12 pt")), "named by its label: {found:?}");
    assert!(!one("Off").enabled);
    assert!(one("Under").covered, "{found:?}");
    assert!(!one("Over the button below").covered);
}

#[test]
fn ui_widgets_refuses_a_label_that_is_not_text() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let ctx = egui::Context::default();
    let r = request(&mut app, &ctx, "ui.widgets", json!({"label": 5}));
    assert_eq!(r["ok"], false, "{r}");
}

/// A request under a read root is told no path outside it that the window shows: a library loaded
/// from there (by its file name, as `ui.inspect` names it) and the recovery folder a failure
/// message names, as every other reply hides them.
#[test]
fn a_library_path_and_the_recovery_folder_outside_the_read_root_are_hidden() {
    let base = std::env::temp_dir().join(format!("vc-ui-widgets-hidden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["in", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&base.join("in")], &[]).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Services::default()).with_control(rx).with_automation_roots(roots);
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let library = base.join("outside").join("swatches.ase").to_string_lossy().into_owned();
    let recovery = base.join("outside").join("Recovery").to_string_lossy().into_owned();
    app.ui.color_guide_limit = format!("loaded/{library}");
    app.recovery.named = vec![recovery.clone()];
    let ctx = egui::Context::default();
    for status in [format!("Loaded {library}"), format!("Couldn't save a recovery copy in {recovery}")] {
        app.ui.status = status.clone();
        frames(&mut app, &ctx, 2);
        let r = widgets(&mut app, &tx, &ctx, json!({}));
        let text = r.to_string();
        let shown = status.split(' ').next().unwrap();
        assert!(text.contains(shown), "the status bar is listed: {r}");
        assert!(!text.contains(&library) && !text.contains(&recovery), "a path outside the read root is named: {r}");
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// A request no pass answers (the window minimized, its frames stopped) is refused once its time
/// is up, as a screenshot is, and the screen-reader tree it turned on goes off.
#[test]
fn a_request_no_frame_answers_is_refused_when_its_time_is_up() {
    let (mut app, _tx, ctx) = app_with_channel();
    let (tx, rx) = std::sync::mpsc::channel();
    let asked = widget_list::ask(&app, &json!({})).unwrap();
    widget_list::queue(&mut app, &ctx, asked, tx);
    // The first pass turns the tree on; no other pass comes.
    let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0))), ..Default::default() };
    app.raw_input_hook(&ctx, &mut raw);
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    assert!(rx.try_recv().is_err(), "answered without a pass that built the tree");
    widget_list::expire(&mut app, &ctx, crate::clock::now_ms());
    assert!(rx.try_recv().is_err(), "refused before its time is up");
    widget_list::expire(&mut app, &ctx, crate::clock::now_ms() + widget_list::TIMEOUT_MS + 1.0);
    let r = rx.try_recv().expect("refused once its time is up");
    assert_eq!(r["ok"], false, "{r}");
    assert!(app.widget_requests.is_empty());
    assert!(frame(&mut app, &ctx).platform_output.accesskit_update.is_none(), "the tree went off");
}

/// A screen reader that starts using the tree while a request has it on keeps it after the
/// request is answered.
#[test]
fn a_screen_reader_that_comes_while_a_request_waits_keeps_the_tree() {
    let (mut app, tx, ctx) = app_with_channel();
    let (req, rx): (ControlRequest, Receiver<Value>) = ControlRequest::new("ui.widgets", json!({}));
    tx.send(req).unwrap();
    frame(&mut app, &ctx);
    // A screen reader acts on the tree the request turned on.
    let action = egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::Focus,
        target_tree: egui::accesskit::TreeId::ROOT,
        target_node: egui::Id::unique("anything").accesskit_id(),
        data: None,
    };
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0))),
        events: vec![egui::Event::AccessKitActionRequest(action)],
        ..Default::default()
    };
    app.raw_input_hook(&ctx, &mut raw);
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    assert_eq!(rx.try_recv().expect("answered")["ok"], true);
    frames(&mut app, &ctx, 1);
    assert!(frame(&mut app, &ctx).platform_output.accesskit_update.is_some(), "the screen reader's tree went off");
}

/// A slider and an unlabelled drag value show their number as their value; a field labelled by a
/// widget that is itself labelled by a label is named by that label, as a screen reader follows
/// the chain, and widgets labelling each other in a loop end the chain.
#[test]
fn numbers_and_chains_of_labels_are_read_as_a_screen_reader_reads_them() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut found = None;
    for _ in 0..3 {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            let mut opacity = 50.0;
            ui.add(egui::Slider::new(&mut opacity, 0.0..=100.0).text("Opacity"));
            let mut n = 7.5;
            ui.add(egui::DragValue::new(&mut n));
            let size = ui.label("Size");
            let middle = ui.add(egui::Button::new("")).labelled_by(size.id);
            let mut text = String::from("12 pt");
            ui.text_edit_singleline(&mut text).labelled_by(middle.id);
            let (a, b) = (egui::Id::unique("loop a"), egui::Id::unique("loop b"));
            let _ = ui.add(egui::Button::new("")).labelled_by(b);
            let _ = ui.push_id("a", |ui| {
                ui.interact(egui::Rect::from_min_size(egui::pos2(300.0, 0.0), egui::vec2(10.0, 10.0)), a, egui::Sense::click()).labelled_by(b)
            });
            let _ = ui.push_id("b", |ui| {
                ui.interact(egui::Rect::from_min_size(egui::pos2(300.0, 20.0), egui::vec2(10.0, 10.0)), b, egui::Sense::click()).labelled_by(a)
            });
            let mut looped = String::from("in a loop");
            ui.text_edit_singleline(&mut looped).labelled_by(a);
            found = widget_list::of_pass(ui.ctx());
        });
        out.textures_delta.clear();
    }
    let found = found.expect("the pass built the tree");
    let slider = found.iter().find(|w| w.role == "Slider").unwrap_or_else(|| panic!("no slider: {found:?}"));
    assert_eq!(slider.label, "Opacity", "{found:?}");
    assert_eq!(slider.value.as_deref(), Some("50"), "{found:?}");
    assert!(found.iter().any(|w| w.value.as_deref() == Some("7.5")), "the drag value is left out: {found:?}");
    let field = found.iter().find(|w| w.value.as_deref() == Some("12 pt")).unwrap_or_else(|| panic!("no field: {found:?}"));
    assert_eq!(field.label, "Size", "{found:?}");
    assert!(found.iter().any(|w| w.value.as_deref() == Some("in a loop")), "{found:?}");
}

/// A widget that shows a hidden recent file's bare name is left out only where the window lists
/// recent files (a button or a menu item); other text of that name (a layer's) stays.
#[test]
fn a_bare_file_name_hides_only_recent_file_rows() {
    let base = std::env::temp_dir().join(format!("vc-ui-widgets-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["in", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&base.join("in")], &[]).unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Services::default()).with_automation_roots(roots.clone());
    app.ui.recent_files = vec![base.join("outside").join("art.svg").to_string_lossy().into_owned()];
    let asked = vectorcraft_engine::file_access::confine(roots.as_ref(), || widget_list::ask(&app, &json!({})).unwrap());
    let widget =
        |role: &str| widget_list::Widget { role: role.into(), label: "art.svg".into(), value: None, rect: [0.0; 4], enabled: true, covered: false };
    assert!(!asked.shows(&widget("Button")), "the Home screen's row");
    assert!(!asked.shows(&widget("MenuItem")), "File › Open Recent Files' item");
    assert!(asked.shows(&widget("Label")), "a layer's name");
    assert!(asked.shows(&widget_list::Widget { role: "TextInput".into(), value: Some("art.svg".into()), ..widget("TextInput") }), "a field's text");
    let _ = std::fs::remove_dir_all(&base);
}

/// `cursorScale` null leaves the size, as the flags' null does.
#[test]
fn ui_set_cursor_scale_null_leaves_it() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let ctx = egui::Context::default();
    assert_eq!(request(&mut app, &ctx, "ui.set", json!({"cursorScale": 2}))["ok"], true);
    let r = request(&mut app, &ctx, "ui.set", json!({"cursorScale": null}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(app.cursor_scale, 2.0);
}

#[test]
fn ui_set_sizes_the_painted_tool_cursors_within_bounds() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let ctx = egui::Context::default();
    assert_eq!(app.cursor_scale, 1.0);
    // A size refused before anything else in the request is set.
    let (rulers, bar) = (app.ui.view.rulers, app.ui.control_bar);
    assert_eq!(request(&mut app, &ctx, "ui.set", json!({"controlBar": !bar, "rulers": !rulers, "cursorScale": 9}))["ok"], false);
    assert_eq!((app.ui.view.rulers, app.ui.control_bar), (rulers, bar), "nothing is set");
    assert_eq!(request(&mut app, &ctx, "ui.set", json!({"cursorScale": 2.5}))["ok"], true);
    assert_eq!(app.cursor_scale, 2.5);
    for bad in [json!(0.1), json!(5), json!(-1), json!("2"), json!(true)] {
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
