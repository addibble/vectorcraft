//! A host page's region on an image (`vectorcraft:region`, host-only): drawn through the image's
//! placement, so it follows the image and the view; its box dragged and resized with the Selection
//! tool while the image is selected, the page told once where it was let go, and nothing in the
//! document changed.

use egui::{Event, PointerButton, Pos2, pos2};
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use crate::canvas::Xf;
use crate::control::{ControlRequest, Outcome};
use crate::host_region::{box_on_screen, clear_region, dragged, set_region};
use crate::{Services, VectorcraftApp};

fn request(app: &mut VectorcraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    let Outcome::Done(r) = crate::control::handle(app, ctx, &req) else { panic!("not done") };
    r
}

/// One frame of the app in a 1600 × 900 window, with `events` as its input.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) {
    let mut raw =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))), events, ..Default::default() };
    app.raw_input_hook(ctx, &mut raw);
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
}

/// The canvas's view as the last frame drew it (`ui.inspect`), as the canvas maps the document.
fn view(app: &mut VectorcraftApp, ctx: &egui::Context) -> Xf {
    let v = request(app, ctx, "ui.inspect", json!({}));
    let r = &v["result"];
    let c: Vec<f64> = r["canvasRect"].as_array().unwrap().iter().map(|n| n.as_f64().unwrap()).collect();
    let center = &r["view"]["center"];
    let (cx, cy) = match center.as_array() {
        Some(a) => (a[0].as_f64().unwrap(), a[1].as_f64().unwrap()),
        None => (center["x"].as_f64().unwrap(), center["y"].as_f64().unwrap()),
    };
    Xf {
        rect: egui::Rect::from_min_size(pos2(c[0] as f32, c[1] as f32), egui::vec2(c[2] as f32, c[3] as f32)),
        zoom: r["view"]["zoom"].as_f64().unwrap(),
        center: Point::new(cx, cy),
        rot: r["view"]["rotation"].as_f64().unwrap_or(0.0).to_radians(),
    }
}

/// A document with one 200 × 100 picture at (100, 100) selected with the Selection tool, its
/// region marked editable on its middle half, and the window drawn.
fn picture_with_region(ctx: &egui::Context) -> (VectorcraftApp, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let img = image::RgbaImage::from_pixel(200, 100, image::Rgba([90, 120, 200, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    app.run("file.place", json!({"name": "p.png", "dataBase64": vectorcraft_format::base64_encode(&png), "link": false, "at": [200, 150]})).unwrap();
    let image = app.session.active().unwrap().selection.objects[0].0;
    app.select_tool("selection");
    let overlay = {
        let map = image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 128]));
        let mut b = vec![];
        map.write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        b
    };
    set_region(&mut app, image, [0.25, 0.25, 0.75, 0.75], Some(&overlay), true).unwrap();
    for _ in 0..3 {
        frame(&mut app, ctx, vec![]);
    }
    (app, image)
}

fn press(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// A drag from `from` by `by`, a frame per step, with `before_release` sent just before letting go.
fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, from: Pos2, by: egui::Vec2, before_release: Vec<Event>) {
    frame(app, ctx, vec![Event::PointerMoved(from)]);
    frame(app, ctx, vec![press(from, true)]);
    for i in 1..=6 {
        frame(app, ctx, vec![Event::PointerMoved(from + by * (i as f32 / 6.0))]);
    }
    if !before_release.is_empty() {
        frame(app, ctx, before_release);
    }
    frame(app, ctx, vec![press(from + by, false)]);
    frame(app, ctx, vec![]);
}

fn middle(q: [Pos2; 4]) -> Pos2 {
    pos2((q[0].x + q[2].x) / 2.0, (q[0].y + q[2].y) / 2.0)
}

/// The box follows the image: moved, turned, and the view zoomed, with nothing asked of the page.
#[test]
fn the_region_follows_the_image_and_the_view() {
    let ctx = egui::Context::default();
    let (mut app, image) = picture_with_region(&ctx);
    let xf = Xf { rect: egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0)), zoom: 1.0, center: Point::new(200.0, 150.0), rot: 0.0 };
    let before = box_on_screen(&app, &xf).unwrap();
    // The image is 200 × 100 at (100, 100): its middle half is (150, 125) to (250, 175).
    assert!((before[0].x - (400.0 - 50.0)).abs() < 0.5 && (before[0].y - (300.0 - 25.0)).abs() < 0.5, "{before:?}");
    app.run("select.set", json!({"ids": [image]})).unwrap();
    app.run("object.move", json!({"ids": [image], "dx": 30, "dy": -10})).unwrap();
    let moved = box_on_screen(&app, &xf).unwrap();
    assert!((moved[0].x - before[0].x - 30.0).abs() < 0.5 && (moved[0].y - before[0].y + 10.0).abs() < 0.5, "{moved:?}");
    app.run("object.rotate", json!({"ids": [image], "angle": 90})).unwrap();
    let turned = box_on_screen(&app, &xf).unwrap();
    let (w, h) = ((turned[1].x - turned[0].x).abs(), (turned[1].y - turned[0].y).abs());
    assert!(w < 0.5 && (h - 100.0).abs() < 0.5, "the box's top edge turned upright: {turned:?}");
    let zoomed = box_on_screen(&app, &Xf { zoom: 2.0, ..xf }).unwrap();
    assert!((zoomed[0].distance(zoomed[2]) - 2.0 * turned[0].distance(turned[2])).abs() < 0.5);
}

/// Dragging moves the box, a corner resizes it, kept inside the image and at least 5% of it.
#[test]
fn the_box_keeps_inside_the_image_and_to_its_smallest() {
    let near = |a: [f64; 4], b: [f64; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
    let start = [0.25, 0.25, 0.75, 0.75];
    let from = Point::new(0.5, 0.5);
    assert!(near(dragged(start, from, Point::new(0.6, 0.4), None), [0.35, 0.15, 0.85, 0.65]));
    assert!(near(dragged(start, from, Point::new(2.0, -2.0), None), [0.5, 0.0, 1.0, 0.5]), "kept inside");
    let r = dragged(start, Point::new(0.75, 0.75), Point::new(0.0, 0.0), Some(2));
    assert!(near(r, [0.25, 0.25, 0.3, 0.3]), "never under 5%: {r:?}");
    assert!(near(dragged(start, Point::new(0.25, 0.25), Point::new(-1.0, -1.0), Some(0)), [0.0, 0.0, 0.75, 0.75]));
}

/// A drag of the box with the Selection tool moves it live and tells the page once, on release; the
/// document isn't touched. Escape during a drag puts it back and tells nothing.
#[test]
fn dragging_the_box_tells_the_page_once_and_escape_tells_nothing() {
    let ctx = egui::Context::default();
    let (mut app, image) = picture_with_region(&ctx);
    let revision = app.session.active().unwrap().revision;
    let dirty = app.session.active().unwrap().is_dirty();
    let xf = view(&mut app, &ctx);
    let q = box_on_screen(&app, &xf).unwrap();
    let by = egui::vec2((q[1].x - q[0].x) * 0.2, 0.0);
    drag(&mut app, &ctx, middle(q), by, vec![]);
    let changed = std::mem::take(&mut app.ui.host_region.changed);
    assert_eq!(changed.len(), 1, "told once, on release");
    let (id, r) = changed[0];
    assert_eq!(id.0, image);
    assert!((r[0] - 0.35).abs() < 0.02 && (r[1] - 0.25).abs() < 0.02, "moved a fifth of the box's width to the right: {r:?}");
    let st = app.session.active().unwrap();
    assert_eq!((st.revision, st.is_dirty()), (revision, dirty), "nothing in the document changed");
    assert_eq!(st.selection.objects.len(), 1, "the image stayed selected: the tool had no part in it");

    let q = box_on_screen(&app, &xf).unwrap();
    let was = app.ui.host_region.region.as_ref().unwrap().rect;
    drag(
        &mut app,
        &ctx,
        middle(q),
        egui::vec2(0.0, 20.0),
        vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
    );
    assert!(app.ui.host_region.changed.is_empty(), "Escape tells nothing");
    assert_eq!(app.ui.host_region.region.as_ref().unwrap().rect, was, "and puts the box back");
}

/// With a drawing tool the box takes no click: the tool draws there.
#[test]
fn a_drawing_tool_draws_over_the_box() {
    let ctx = egui::Context::default();
    let (mut app, _) = picture_with_region(&ctx);
    app.select_tool("rectangle");
    frame(&mut app, &ctx, vec![]);
    let xf = view(&mut app, &ctx);
    let q = box_on_screen(&app, &xf).unwrap();
    let revision = app.session.active().unwrap().revision;
    drag(&mut app, &ctx, middle(q), egui::vec2(30.0, 20.0), vec![]);
    assert!(app.ui.host_region.changed.is_empty());
    assert_ne!(app.session.active().unwrap().revision, revision, "the Rectangle tool drew");
}

/// The region isn't drawn once its image is gone, or while another document is active; clearing it
/// takes it away; a mark on something that isn't an image is refused.
#[test]
fn the_region_goes_with_its_image_and_its_document() {
    let ctx = egui::Context::default();
    let (mut app, image) = picture_with_region(&ctx);
    let xf = Xf { rect: egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0)), zoom: 1.0, center: Point::new(200.0, 150.0), rot: 0.0 };
    assert!(box_on_screen(&app, &xf).is_some());
    // Another document, with an image of its own under the same id (ids are each document's).
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let img = image::RgbaImage::from_pixel(200, 100, image::Rgba([1, 2, 3, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    app.run("file.place", json!({"name": "q.png", "dataBase64": vectorcraft_format::base64_encode(&png), "link": false, "at": [200, 150]})).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects[0].0, image, "the same id, another document");
    assert!(box_on_screen(&app, &xf).is_none(), "another document is active");
    let r = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert!(set_region(&mut app, r["id"].as_u64().unwrap(), [0.0, 0.0, 1.0, 1.0], None, false).unwrap_err().contains("not an image"));
    app.run("window.previousDocument", json!({})).ok();
    app.session.set_active(0);
    if app.session.active().is_some_and(|st| st.doc.node(vectorcraft_doc::NodeId(image)).is_some()) {
        assert!(box_on_screen(&app, &xf).is_some(), "its document is active again");
        app.run("edit.clear", json!({"ids": [image]})).unwrap();
        assert!(box_on_screen(&app, &xf).is_none(), "its image is gone");
    }
    assert!(set_region(&mut app, image, [0.0, 0.0, 1.0, 1.0], Some(b"not a png"), false).is_err());
    clear_region(&mut app);
    assert!(app.ui.host_region.region.is_none());
}
