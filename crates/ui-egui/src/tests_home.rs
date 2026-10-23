//! The Home icon shows the Home screen over open documents; Cmd+N opens the New Document dialog.

use serde_json::json;

use crate::{VectorcraftApp, canvas};

fn app_with_doc() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app
}

/// Draw one canvas frame; whether it drew the document (else the Home screen).
fn shows_document(app: &mut VectorcraftApp) -> bool {
    app.canvas_rect = None;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
    app.canvas_rect.is_some()
}

#[test]
fn home_shows_over_open_documents_until_a_document_is_chosen() {
    let mut app = app_with_doc();
    assert!(shows_document(&mut app));
    app.run("app.home", json!({})).unwrap();
    assert!(!shows_document(&mut app), "Home replaces the canvas");
    assert_eq!(app.session.documents().len(), 1, "the document stays open");
    assert!(app.ui.dialog.is_none(), "Home is not the New Document dialog");
    // Choosing the document's tab returns to it.
    app.ui.home = None;
    assert!(shows_document(&mut app));
    // A new document (from Home's presets, New… or Open) replaces Home too.
    app.run("app.home", json!({})).unwrap();
    app.run("file.new", json!({})).unwrap();
    assert!(shows_document(&mut app));
    assert!(app.ui.home.is_none());
}

/// Draw one canvas frame; whether it drew any text (the Home screen's labels; an empty window
/// draws none).
fn draws_text(app: &mut VectorcraftApp) -> bool {
    fn has_text(s: &egui::Shape) -> bool {
        match s {
            egui::Shape::Text(_) => true,
            egui::Shape::Vec(v) => v.iter().any(has_text),
            _ => false,
        }
    }
    app.canvas_rect = None;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
    out.shapes.iter().any(|c| has_text(&c.shape))
}

/// General › Show The Home Screen When No Documents Are Open (#394): off, an app with no document
/// shows an empty window instead of the Home screen, and the Home button still opens it.
#[test]
fn home_screen_without_documents_follows_its_preference() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    assert!(app.session.active().is_none());
    assert!(draws_text(&mut app), "Home with no document open (the default)");
    assert!(crate::menus::home_showing(&app));
    app.session.execute("prefs.set", &json!({"key": "showHomeScreen", "value": false})).unwrap();
    assert!(!draws_text(&mut app), "off: an empty window");
    assert!(!shows_document(&mut app));
    assert!(!crate::menus::home_showing(&app), "the Home button isn't lit");
    app.run("app.home", json!({})).unwrap();
    assert!(draws_text(&mut app), "the Home button still shows the Home screen");
    app.run("file.new", json!({})).unwrap();
    assert!(shows_document(&mut app));
    assert!(app.ui.home.is_none());
    app.run("file.close", json!({})).unwrap();
    assert!(app.session.active().is_none());
    assert!(!draws_text(&mut app), "closing the last document leaves the window empty");
    app.session.execute("prefs.set", &json!({"key": "showHomeScreen", "value": true})).unwrap();
    assert!(draws_text(&mut app), "on again: Home");
}

#[test]
fn cmd_n_opens_the_new_document_dialog() {
    assert_eq!(crate::shortcut_editor::command_for_key("Cmd+N"), Some("file.newDialog"));
    let mut app = app_with_doc();
    app.run("file.newDialog", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("newDocument"));
    assert_eq!(app.session.documents().len(), 1, "no document is made until the dialog's OK");
}

/// Cmd+Alt+N is New Without Dialog by default: Ctrl+Alt+N on Windows and Linux, Option+Command+N
/// on macOS. No other command, tool or panel has that key by default.
#[test]
fn cmd_alt_n_is_new_without_dialog() {
    assert_eq!(crate::shortcut_editor::default_command_shortcut("file.newWithoutDialog"), Some("Cmd+Alt+N"));
    assert_eq!(crate::shortcut_editor::command_for_key("Cmd+Alt+N"), Some("file.newWithoutDialog"));
    let chord = crate::shortcuts::parse("Cmd+Alt+N").expect("parses");
    assert_eq!((chord.modifiers, chord.logical_key), (egui::Modifiers::COMMAND | egui::Modifiers::ALT, egui::Key::N));
    let shown = crate::menus::pretty_shortcut("Cmd+Alt+N");
    assert_eq!(shown, if cfg!(target_os = "macos") { "⌥⌘N" } else { "Ctrl+Alt+N" });
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let listed = app.run("shortcuts.list", json!({"query": "file.newWithoutDialog"})).unwrap();
    let rows: Vec<_> = listed.as_array().expect("rows").iter().map(|r| (r["label"].clone(), r["group"].clone(), r["default"].clone())).collect();
    assert_eq!(rows, [(json!("New Without Dialog"), json!("File"), json!("Cmd+Alt+N"))], "Keyboard Shortcuts lists it under File");
    assert_eq!(app.run("shortcuts.conflicts", json!({})).unwrap(), json!([]), "no other default has the key");
}

/// Cmd+Alt+N, pressed through the shortcut handler, makes a document with the settings New
/// Document opens on (the Print category's Letter while Recent is empty, then the first Recent
/// entry), named as the dialog names it, and does not open the dialog.
#[test]
fn cmd_alt_n_makes_a_document_without_the_new_document_dialog() {
    use vectorcraft_doc::ColorMode;
    fn press(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        let modifiers = egui::Modifiers::COMMAND | egui::Modifiers::ALT;
        let ev = egui::Event::Key { key: egui::Key::N, physical_key: None, pressed: true, repeat: false, modifiers };
        let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
        out.textures_delta.clear();
    }
    let made = |app: &VectorcraftApp| {
        let d = &app.session.active().expect("a document").doc;
        let r = d.artboards[0].rect;
        (d.title.clone(), r.width(), r.height(), d.color_mode, d.raster_effects_ppi)
    };
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    assert!(app.session.active().is_none());
    let name = app.session.peek_untitled();
    press(&mut app);
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.ui.dialog.is_none(), "the New Document dialog does not open");
    assert_eq!(made(&app), (name, 612.0, 792.0, ColorMode::Cmyk, 300.0), "Letter from the Print category, as New Document opens on");
    // Then the first Recent entry, the 400 × 300 pt document's settings.
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let name = app.session.peek_untitled();
    press(&mut app);
    assert_eq!(app.session.documents().len(), 3);
    assert!(app.ui.dialog.is_none());
    assert_eq!(made(&app), (name, 400.0, 300.0, ColorMode::Rgb, 72.0));
    // Run as a command while a dialog is open, it leaves that dialog open.
    app.run("file.newDialog", json!({})).unwrap();
    app.run("file.newWithoutDialog", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), 4);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("newDocument"));
}

/// One Home screen frame (no document open) with `events`: the texts drawn and where.
fn home_texts(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
    fn walk(s: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let raw =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 900.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
    let mut texts = vec![];
    out.shapes.iter().for_each(|c| walk(&c.shape, &mut texts));
    texts
}

/// The Home screen lists the first recent files (#663), each by name and folder; a click opens it,
/// and one that can't be opened says so. With none, there is no Recent Files section.
#[test]
fn home_lists_recent_files_and_a_click_opens_one() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let has = |texts: &[(String, egui::Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
    assert!(!has(&home_texts(&mut app, &ctx, vec![]), "Recent Files"), "none: no section");
    let dir = std::env::temp_dir().join("vectorcraft-home-recent");
    let missing = dir.join("Gone.svg").to_string_lossy().into_owned();
    app.ui.recent_files = (0..9).map(|i| dir.join(format!("Drawing {i}.svg")).to_string_lossy().into_owned()).collect();
    app.ui.recent_files.insert(0, missing);
    let texts = home_texts(&mut app, &ctx, vec![]);
    assert!(has(&texts, "Recent Files") && has(&texts, "Gone.svg") && has(&texts, "Drawing 4.svg"), "{texts:?}");
    assert!(!has(&texts, "Drawing 5.svg"), "the first six only");
    assert!(has(&texts, &dir.to_string_lossy()), "with its folder");
    let at = texts.iter().find(|(t, _)| t == "Gone.svg").map(|(_, r)| r.center()).unwrap();
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    home_texts(&mut app, &ctx, vec![egui::Event::PointerMoved(at), press(true)]);
    home_texts(&mut app, &ctx, vec![press(false)]);
    assert!(app.session.documents().is_empty());
    assert!(!app.ui.status.is_empty(), "a file that can't be opened says so");
}

/// One Home screen frame with `events`: the filled rectangles drawn in `fill`, and the cursor.
fn home_fills(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, fill: egui::Color32) -> (Vec<egui::Rect>, egui::CursorIcon) {
    fn walk(s: &egui::Shape, fill: egui::Color32, out: &mut Vec<egui::Rect>) {
        match s {
            egui::Shape::Rect(r) if r.fill == fill => out.push(r.rect),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, fill, out)),
            _ => {}
        }
    }
    let raw =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 900.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
    let mut rects = vec![];
    out.shapes.iter().for_each(|c| walk(&c.shape, fill, &mut rects));
    (rects, out.platform_output.cursor_icon)
}

/// A Recent Files row is one button: the pointer over its folder lights the whole row (icon, name
/// and folder) with a pointing hand, and a click there opens the file like a click on its name.
#[test]
fn a_recent_file_row_lights_up_and_opens_from_anywhere_on_it() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let dir = std::env::temp_dir().join("vectorcraft-home-recent-row");
    app.ui.recent_files = vec![dir.join("Gone.svg").to_string_lossy().into_owned()];
    let texts = home_texts(&mut app, &ctx, vec![]);
    let rect_of = |s: &str| texts.iter().find(|(t, _)| t == s).map(|(_, r)| *r).unwrap();
    let (name, folder) = (rect_of("Gone.svg"), rect_of(&dir.to_string_lossy()));
    let hover = crate::theme::Tokens::get(&ctx).hover;
    let lit = |rects: &[egui::Rect]| rects.iter().any(|r| r.contains(name.center()) && r.contains(folder.center()));
    let (rects, cursor) = home_fills(&mut app, &ctx, vec![], hover);
    assert!(!lit(&rects) && cursor != egui::CursorIcon::PointingHand, "no highlight without the pointer");
    let at = folder.center();
    home_fills(&mut app, &ctx, vec![egui::Event::PointerMoved(at)], hover);
    let (rects, cursor) = home_fills(&mut app, &ctx, vec![], hover);
    assert!(lit(&rects), "the pointer over the folder lights the whole row");
    assert_eq!(cursor, egui::CursorIcon::PointingHand);
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    home_texts(&mut app, &ctx, vec![press(true)]);
    home_texts(&mut app, &ctx, vec![press(false)]);
    assert!(app.session.documents().is_empty());
    assert!(!app.ui.status.is_empty(), "the click on the folder tried to open the file");
}

/// One Home screen frame in a `w`×`h` window at `time` with `events`: the texts visible (inside
/// their clip rect and the window) and where.
fn visible_home_texts(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    (w, h): (f32, f32),
    time: f64,
    events: Vec<egui::Event>,
) -> Vec<(String, egui::Rect)> {
    fn walk(s: &egui::Shape, clip: egui::Rect, out: &mut Vec<(String, egui::Rect)>) {
        match s {
            egui::Shape::Text(t) if clip.contains_rect(t.visual_bounding_rect()) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            _ => {}
        }
    }
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
    let raw = egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
    let mut texts = vec![];
    out.shapes.iter().for_each(|c| walk(&c.shape, c.clip_rect.intersect(screen), &mut texts));
    texts
}

/// Home scrolls when it is taller than the window: in a short window the Community section and
/// the last preset card are below the fold, the wheel brings them into view, and a card scrolled
/// into view still opens its preset.
#[test]
fn home_scrolls_in_a_short_window() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let dir = std::env::temp_dir().join("vectorcraft-home-scroll");
    app.ui.recent_files = (0..6).map(|i| dir.join(format!("Drawing {i}.svg")).to_string_lossy().into_owned()).collect();
    let size = (800.0, 300.0);
    let has = |texts: &[(String, egui::Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
    let texts = visible_home_texts(&mut app, &ctx, size, 0.0, vec![]);
    assert!(has(&texts, "Welcome to VectorCraft"), "{texts:?}");
    assert!(!has(&texts, "Community"), "below the fold at first");
    let at = egui::pos2(400.0, 150.0);
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -5000.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    };
    visible_home_texts(&mut app, &ctx, size, 0.1, vec![egui::Event::PointerMoved(at), wheel]);
    let mut texts = vec![];
    for i in 2..40 {
        texts = visible_home_texts(&mut app, &ctx, size, f64::from(i) * 0.1, vec![]);
    }
    assert!(has(&texts, "Community"), "the wheel reaches the bottom: {texts:?}");
    assert!(!has(&texts, "Welcome to VectorCraft"), "the top scrolled away");
    // The last preset card, scrolled into view, still takes a click.
    let card = texts.iter().find(|(t, _)| t.starts_with("Social Square Post")).map(|(_, r)| r.center()).unwrap();
    let press = |pressed| egui::Event::PointerButton { pos: card, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    visible_home_texts(&mut app, &ctx, size, 4.0, vec![egui::Event::PointerMoved(card), press(true)]);
    visible_home_texts(&mut app, &ctx, size, 4.1, vec![press(false)]);
    assert_eq!(app.session.documents().len(), 1, "the card opened its preset");
}

/// Home › Recent Files and File › Open Recent focus an open tab instead of opening a second copy
/// (#3455); `document.open` is unchanged.
#[test]
fn home_recent_reopen_focuses_an_already_open_document() {
    let services = crate::Services {
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), services);
    let d = std::env::temp_dir().join(format!("vc-home-reopen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let path = d.join("doc.vectorcraft").to_string_lossy().to_string();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 1, "y": 2, "width": 10, "height": 10})).unwrap();
    app.run("file.saveAs", json!({"path": path})).unwrap();
    app.run("file.new", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.session.active_index(), Some(1));
    let r = crate::io::open_recent_reporting(&mut app, &path).unwrap();
    assert_eq!(r["activated"], true);
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.session.active_index(), Some(0));
    assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(d);
}

/// An embedding page (`vectorcraft:configure`, docs/web-embed.md) hides commands from the user and
/// says what the window shows with no document open: the message stands in for the Home screen,
/// the Home button and the hidden commands' menu items, shortcuts and runs from the UI are gone,
/// and the page's own control requests still run them.
#[test]
fn an_embedding_page_hides_commands_and_says_what_the_empty_window_shows() {
    fn press(app: &mut VectorcraftApp, key: egui::Key, modifiers: egui::Modifiers) {
        let ctx = egui::Context::default();
        let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
        out.textures_delta.clear();
    }
    fn control(app: &mut VectorcraftApp, method: &str, params: serde_json::Value) -> serde_json::Value {
        let (req, _rx) = crate::control::ControlRequest::new(method, params);
        let crate::control::Outcome::Done(r) = crate::control::handle(app, &egui::Context::default(), &req) else { panic!("not done") };
        r
    }
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.ui.host_hidden = vec!["file.newWithoutDialog".into(), "file.newDialog".into(), "file.openRecent*".into(), "app.home".into()];
    app.ui.host_empty_message = Some("No project open. Open one from the Projects tab".into());

    // The message stands in for the Home screen, whatever its preference says.
    assert!(!crate::menus::home_showing(&app));
    assert!(draws_text(&mut app), "the empty window says the page's message");
    assert!(!shows_document(&mut app));

    // Hidden ids, and ids under a hidden prefix, leave the menus; the rest stay.
    assert!(crate::menus::shown_state(&app, "file.newDialog", &json!(null)).is_none());
    assert!(crate::menus::shown_state(&app, "file.openRecent3", &json!(null)).is_none());
    assert!(crate::menus::shown_state(&app, "file.save", &json!(null)).is_some());
    let listed = control(&mut app, "ui.menu.list", json!({})).to_string();
    assert!(!listed.contains("file.newDialog") && listed.contains("file.save"), "the menus the control channel lists follow");

    // Its shortcut and a run from the UI (menus, palette, buttons: `invoke`) do nothing.
    press(&mut app, egui::Key::N, egui::Modifiers::COMMAND | egui::Modifiers::ALT);
    assert!(app.session.active().is_none(), "Cmd+Alt+N made no document");
    crate::menus::invoke(&mut app, "file.newDialog", json!({}));
    assert!(app.ui.dialog.is_none(), "New Document did not open");
    crate::menus::invoke(&mut app, "app.home", json!({}));
    assert!(app.ui.home.is_none());

    // The page's own control requests still run them.
    control(&mut app, "engine.execute", json!({"command": "file.newWithoutDialog"}));
    assert_eq!(app.session.documents().len(), 1);
    assert!(shows_document(&mut app));

    // A page that clears them gives the user everything back.
    app.ui.host_hidden.clear();
    app.ui.host_empty_message = None;
    app.run("file.close", json!({})).unwrap();
    assert!(crate::menus::home_showing(&app), "Home again with no document");
    crate::menus::invoke(&mut app, "file.newDialog", json!({}));
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("newDocument"));
}

/// An embedding page (`vectorcraft:configure`) leaves menus out of the bar while their commands
/// still run, hides the brand mark and, with `help.discord` hidden, the Discord button; the empty
/// window's link only notes a click for the page to hear (`vectorcraft:emptyLink`).
#[test]
fn an_embedding_page_hides_menus_and_the_mark_and_hears_the_empty_windows_link() {
    fn bar(app: &mut VectorcraftApp) -> (Vec<String>, bool) {
        fn walk(s: &egui::Shape, texts: &mut Vec<String>, meshes: &mut Vec<egui::TextureId>) {
            match s {
                egui::Shape::Text(t) => texts.push(t.galley.text().to_string()),
                egui::Shape::Mesh(m) => meshes.push(m.texture_id),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, texts, meshes)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::chrome::app_bar(app, ui));
        out.textures_delta.clear();
        let (mut texts, mut meshes) = (vec![], vec![]);
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut texts, &mut meshes));
        (texts, meshes.contains(&crate::brand::texture_id(&ctx)))
    }
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let (texts, mark) = bar(&mut app);
    assert!(mark && ["File", "Edit", "Help", "Discord"].iter().all(|t| texts.iter().any(|s| s == t)), "{texts:?}");

    app.ui.host_hidden_menus = vec!["File".into(), "Help".into()];
    app.ui.host_hide_mark = true;
    app.ui.host_hidden = vec!["help.discord".into()];
    let (texts, mark) = bar(&mut app);
    assert!(!mark, "no brand mark");
    assert!(!["File", "Help", "Discord"].iter().any(|t| texts.iter().any(|s| s == t)), "{texts:?}");
    assert!(texts.iter().any(|s| s == "Edit"), "the other menus stay: {texts:?}");
    // A hidden menu's commands still run from the UI: File › New's dialog opens.
    crate::menus::invoke(&mut app, "file.newDialog", json!({}));
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("newDocument"));
    app.ui.dialog = None;

    // The empty window's link: drawn after the message, a click only noted for the page.
    app.ui.host_empty_message = Some("No project open.".into());
    app.ui.host_empty_link = Some("Open one from the Projects tab".into());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    fn find(s: &egui::Shape, text: &str) -> Option<egui::Pos2> {
        match s {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.pos + t.galley.size() / 2.0),
            egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, text)),
            _ => None,
        }
    }
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let raw =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| canvas::show(app, ui));
        out.textures_delta.clear();
        let message = out.shapes.iter().find_map(|c| find(&c.shape, "No project open."));
        let link = out.shapes.iter().find_map(|c| find(&c.shape, "Open one from the Projects tab"));
        assert!(message.zip(link).is_some_and(|(m, l)| m.x < l.x), "the link follows the message: {message:?} {link:?}");
        link.unwrap()
    };
    let at = frame(&mut app, vec![]);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, vec![egui::Event::PointerMoved(at)]);
    assert!(!app.ui.host_empty_link_clicked, "hovering is no click");
    frame(&mut app, vec![button(true)]);
    frame(&mut app, vec![button(false)]);
    assert!(app.ui.host_empty_link_clicked, "the click is noted for the page");
    assert!(app.session.active().is_none() && app.ui.dialog.is_none(), "and does nothing else");
}
