//! The Links panel's commands: listing images with their status (`links.list`), Go To, Embed and
//! Unembed, Link Info and the Placement Options a relinked file follows.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeKind, PlacementOptions};

use super::tests_links::{BLUE, Folder, RED, bounds, centre_colour, image, near, open, place, png, save, session, write};
use super::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Place the file at `path` embedded → its id.
fn place_embedded(s: &mut Session, path: &str) -> NodeId {
    let r = run(s, "file.place", json!({"path": path, "at": [200, 150], "link": false}));
    NodeId(r["ids"][0].as_u64().unwrap())
}

/// A PNG that says it is `w` × `h` 1-bit grey pixels (its header and no rows, which is all that
/// placing and listing read).
pub(crate) fn png_header(w: u32, h: u32) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let ihdr: Vec<u8> = [&b"IHDR"[..], &w.to_be_bytes(), &h.to_be_bytes(), &[1, 0, 0, 0, 0]].concat();
    for chunk in [&ihdr[..], b"IDAT\x78\x9c", b"IEND"] {
        out.extend((chunk.len() as u32 - 4).to_be_bytes());
        out.extend(chunk);
        out.extend(crc32fast::hash(chunk).to_be_bytes());
    }
    out
}

/// An image past what a decode may take isn't drawn: Place and Open say so and the Links panel's
/// row carries the note, where the image used to be blank with no word (#1479).
#[test]
fn an_image_too_large_to_draw_is_reported() {
    const NOTE: &str = "an image of 200000 × 200000 pixels is too large to draw";
    let dir = Folder::new("huge");
    let (big, small) = (dir.file("wall.png"), dir.file("small.png"));
    write(&big, &png_header(200_000, 200_000));
    write(&small, &png(40, 20, BLUE));
    let mut s = session();
    assert_eq!(run(&mut s, "file.place", json!({"path": big, "at": [200, 150], "link": false}))["warnings"], json!([NOTE]));
    assert_eq!(run(&mut s, "file.place", json!({"path": small, "at": [20, 20], "link": false}))["warnings"], json!([]));
    let l = run(&mut s, "links.list", json!({}));
    let notes: Vec<&Value> = l["links"].as_array().unwrap().iter().map(|r| &r["note"]).collect();
    assert_eq!(notes, [&Value::Null, &json!(NOTE)], "top first: the small image has nothing to say");
    // Opened as a document: said too. An image that decodes (16,000 px square) has no note.
    assert_eq!(crate::cmd::fileio::load("wall.png", &png_header(200_000, 200_000)).unwrap().warnings, [NOTE]);
    assert!(crate::cmd::fileio::load("fine.png", &png_header(16_000, 16_000)).unwrap().warnings.is_empty());
}

/// A gray image is counted as RGBA, 4 bytes a pixel: 23,171 × 23,170 1-bit pixels decode to 537 MB
/// of gray and are 2,147,488,280 bytes as RGBA, past the 2 GiB allowance; 23,170 × 23,170 are
/// 2,147,395,600 bytes, within it.
#[test]
fn a_gray_image_too_large_as_rgba_is_reported_on_place() {
    const NOTE: &str = "an image of 23171 × 23170 pixels is too large to draw";
    let dir = Folder::new("huge-gray-place");
    let (over, within) = (dir.file("over.png"), dir.file("within.png"));
    write(&over, &png_header(23_171, 23_170));
    write(&within, &png_header(23_170, 23_170));
    let mut s = session();
    assert_eq!(run(&mut s, "file.place", json!({"path": over, "at": [200, 150], "link": false}))["warnings"], json!([NOTE]));
    assert_eq!(run(&mut s, "file.place", json!({"path": within, "at": [20, 20], "link": false}))["warnings"], json!([]));
    let l = run(&mut s, "links.list", json!({}));
    let notes: Vec<&Value> = l["links"].as_array().unwrap().iter().map(|r| &r["note"]).collect();
    assert_eq!(notes, [&Value::Null, &json!(NOTE)], "top first: the image within the allowance has nothing to say");
}

/// File › Open says the same of a gray image past the allowance as RGBA, and nothing of one within it.
#[test]
fn a_gray_image_too_large_as_rgba_is_reported_on_open() {
    let dir = Folder::new("huge-gray-open");
    let (over, within) = (dir.file("over.png"), dir.file("within.png"));
    write(&over, &png_header(23_171, 23_170));
    write(&within, &png_header(23_170, 23_170));
    let mut s = session();
    assert_eq!(open(&mut s, &over)["warnings"], json!(["an image of 23171 × 23170 pixels is too large to draw"]));
    assert_eq!(open(&mut s, &within)["warnings"], json!([]));
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn embed_keeps_the_pixels_without_the_file() {
    let dir = Folder::new("embed");
    let pic = dir.file("photo.png");
    let bytes = png(600, 300, RED);
    write(&pic, &bytes);
    let mut s = session();
    let id = place(&mut s, &pic);
    let placed = bounds(&s, id);
    run(&mut s, "select.set", json!({"ids": []}));
    assert!(s.execute("links.embed", &json!({})).is_err(), "nothing selected");
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    assert_eq!(run(&mut s, "links.embed", json!({})), json!({"embedded": [id.0], "missing": []}));
    let im = image(&s, id);
    assert!(im.link.is_none());
    assert_eq!(s.doc().unwrap().doc.images[&im.key].bytes.as_slice(), bytes.as_slice(), "the file's own bytes");
    assert_eq!(bounds(&s, id), placed);
    run(&mut s, "edit.undo", json!({}));
    assert!(image(&s, id).link.is_some(), "one undo step");
    run(&mut s, "edit.redo", json!({}));
    // Saved and reopened without the file: the full pixels, nothing missing.
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    std::fs::remove_file(&pic).unwrap();
    let r = open(&mut s, &doc_path);
    assert_eq!(r["missingLinks"], json!([]), "{r}");
    let im = image(&s, id);
    assert!(!s.doc().unwrap().doc.images[&im.key].is_proxy());
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
}

#[test]
fn a_missing_file_showing_its_preview_stays_linked() {
    let dir = Folder::new("embed-missing");
    let pic = dir.file("photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    std::fs::remove_file(&pic).unwrap();
    open(&mut s, &doc_path);
    assert_eq!(run(&mut s, "links.embed", json!({"ids": [id.0]})), json!({"embedded": [], "missing": [id.0]}));
    assert!(image(&s, id).link.is_some());
    assert!(s.doc().unwrap().history.undo.is_empty(), "nothing changed");
}

#[test]
fn unembed_writes_the_bytes_and_links_to_them() {
    let dir = Folder::new("unembed");
    let pic = dir.file("photo.png");
    let bytes = png(600, 300, RED);
    write(&pic, &bytes);
    let mut s = session();
    let id = place_embedded(&mut s, &pic);
    let placed = bounds(&s, id);
    // No path: the file's bytes come back and nothing changes.
    let r = run(&mut s, "links.unembed", json!({"id": id.0}));
    assert_eq!(r["name"], "photo.png");
    assert_eq!(vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap(), bytes);
    assert!(image(&s, id).link.is_none());
    // To a path: written there, and the image links to it.
    let out = dir.file("out/photo.png");
    std::fs::create_dir_all(dir.file("out")).unwrap();
    let r = run(&mut s, "links.unembed", json!({"id": id.0, "path": out}));
    assert_eq!(r, json!({"id": id.0, "path": out}));
    assert_eq!(std::fs::read(&out).unwrap(), bytes);
    let link = image(&s, id).link.unwrap();
    assert_eq!((link.path.as_str(), link.size), (out.as_str(), Some(bytes.len() as u64)));
    assert_eq!(bounds(&s, id), placed);
    assert_eq!(run(&mut s, "links.check", json!({}))["links"][0]["status"], "ok");
    assert!(s.execute("links.unembed", &json!({"id": id.0, "path": out})).is_err(), "linked already");
    run(&mut s, "edit.undo", json!({}));
    assert!(image(&s, id).link.is_none(), "one undo step");
    // Another extension converts.
    let jpg = dir.file("photo.jpg");
    run(&mut s, "links.unembed", json!({"id": id.0, "path": jpg}));
    assert_eq!(std::fs::read(&jpg).unwrap()[..2], [0xff, 0xd8]);
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.execute("links.unembed", &json!({"id": id.0, "path": dir.file("photo.psd")})).is_err(), "not a format it writes");
}

#[test]
fn go_to_selects_the_image() {
    let dir = Folder::new("goto");
    let pic = dir.file("photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}));
    assert_ne!(s.doc().unwrap().selection.objects, vec![id]);
    let r = run(&mut s, "links.goTo", json!({"id": id.0}));
    assert_eq!(r, json!({"id": id.0, "bounds": [-100.0, 0.0, 500.0, 300.0]}));
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    let rect = s.doc().unwrap().doc.layers[0].children().unwrap()[1].id;
    assert!(s.execute("links.goTo", &json!({"id": rect.0})).is_err(), "not an image");
}

#[test]
fn the_list_shows_each_image_with_its_status() {
    let dir = Folder::new("list");
    let (a, b) = (dir.file("a.png"), dir.file("b.png"));
    write(&a, &png(600, 300, RED));
    write(&b, &png(40, 20, BLUE));
    let mut s = session();
    let linked = place(&mut s, &a);
    let embedded = place_embedded(&mut s, &b);
    let l = run(&mut s, "links.list", json!({}));
    let rows = l["links"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    // Top first.
    assert_eq!((rows[0]["id"].as_u64(), rows[0]["status"].as_str(), rows[0]["name"].as_str()), (Some(embedded.0), Some("embedded"), Some("b.png")));
    assert_eq!(
        (rows[1]["id"].as_u64(), rows[1]["status"].as_str(), rows[1]["path"].as_str(), rows[1]["format"].as_str()),
        (Some(linked.0), Some("ok"), Some(a.as_str()), Some("PNG"))
    );
    assert_eq!((rows[1]["pixelWidth"].as_u64(), rows[1]["pixelHeight"].as_u64()), (Some(600), Some(300)));
    write(&a, &png(300, 300, BLUE));
    let l = run(&mut s, "links.list", json!({"sort": "status"}));
    assert_eq!((l["modified"].as_u64(), l["embedded"].as_u64(), l["missing"].as_u64()), (Some(1), Some(1), Some(0)));
    assert_eq!(l["links"][0]["status"], "modified", "modified before embedded");
    let l = run(&mut s, "links.list", json!({"show": "embedded"}));
    assert_eq!(l["links"].as_array().unwrap().len(), 1);
    std::fs::remove_file(&a).unwrap();
    assert_eq!(run(&mut s, "links.list", json!({"show": "missing"}))["links"][0]["id"], linked.0);
    assert_eq!(run(&mut s, "links.list", json!({"sort": "name"}))["links"][0]["name"], "a.png");
    assert!(s.execute("links.list", &json!({"show": "broken"})).is_err());
    assert!(s.execute("links.list", &json!({"sort": "size"})).is_err());
}

#[test]
fn link_info_has_the_file_and_placement_details() {
    let dir = Folder::new("info");
    let pic = dir.file("photo.png");
    let bytes = png(600, 300, RED);
    write(&pic, &bytes);
    let mut s = session();
    let id = place(&mut s, &pic);
    let i = run(&mut s, "links.info", json!({"id": id.0}));
    assert_eq!((i["status"].as_str(), i["fileName"].as_str(), i["format"].as_str()), (Some("ok"), Some("photo.png"), Some("PNG")));
    assert_eq!(std::path::Path::new(i["location"].as_str().unwrap()), dir.0.as_path());
    assert_eq!((i["fileSize"].as_u64(), i["pixelWidth"].as_u64(), i["colorMode"].as_str()), (Some(bytes.len() as u64), Some(600), Some("RGB")));
    assert_eq!((i["ppi"].clone(), i["effectivePpi"].clone(), i["scale"].clone()), (json!([72.0, 72.0]), json!([72.0, 72.0]), json!([100.0, 100.0])));
    assert!(i["modified"].as_u64().is_some());
    assert_eq!(i["placement"], json!({"preserve": "bounds", "align": "center", "clip": false}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "object.scale", json!({"sx": 50}));
    run(&mut s, "object.rotate", json!({"angle": 30}));
    let i = run(&mut s, "links.info", json!({}));
    let num = |v: &Value, k: usize| v.get(k).and_then(Value::as_f64).unwrap();
    assert!(close(num(&i["scale"], 0), 50.0) && close(num(&i["effectivePpi"], 0), 144.0), "{i}");
    assert!(close(i["rotation"].as_f64().unwrap(), 30.0), "counter-clockwise: {i}");
    let embedded = place_embedded(&mut s, &pic);
    let i = run(&mut s, "links.info", json!({"id": embedded.0}));
    assert_eq!((i["status"].as_str(), i["linked"].as_bool(), i["fileName"].as_str()), (Some("embedded"), Some(false), None));
}

/// A 600×300 pt image at the centre of the artboard, its placement options set to `options`,
/// relinked to a 300×100 px file → the image's id and session.
fn relinked(name: &str, options: Value) -> (Session, NodeId, Folder) {
    let dir = Folder::new(name);
    let (pic, other) = (dir.file("photo.png"), dir.file("blue.png"));
    write(&pic, &png(600, 300, RED));
    write(&other, &png(300, 100, BLUE));
    let mut s = session();
    let id = place(&mut s, &pic);
    let mut p = options;
    p["ids"] = json!([id.0]);
    run(&mut s, "links.placementOptions", p);
    run(&mut s, "links.relink", json!({"ids": [id.0], "path": other}));
    (s, id, dir)
}

#[test]
fn a_relinked_file_follows_the_placement_options() {
    let b = |s: &Session, id| {
        let r = bounds(s, id);
        [r.x0, r.y0, r.x1, r.y1].map(|v| (v * 1e6).round() / 1e6)
    };
    // The old bounds: -100, 0, 500, 300.
    let (s, id, _d) = relinked("pl-bounds", json!({"preserve": "bounds"}));
    assert_eq!(b(&s, id), [-100.0, 0.0, 500.0, 300.0], "stretched into the bounds");
    let (s, id, _d) = relinked("pl-fit", json!({"preserve": "fit"}));
    assert_eq!(b(&s, id), [-100.0, 50.0, 500.0, 250.0], "proportional, centred");
    let (s, id, _d) = relinked("pl-transforms", json!({"preserve": "transforms"}));
    assert_eq!(b(&s, id), [50.0, 100.0, 350.0, 200.0], "at the same 100%");
    let (s, id, _d) = relinked("pl-file", json!({"preserve": "fileDimensions", "align": "topLeft"}));
    assert_eq!(b(&s, id), [-100.0, 0.0, 200.0, 100.0]);
    let (mut s, id, d) = relinked("pl-fill", json!({"preserve": "fill", "align": "left", "clip": true}));
    assert_eq!(b(&s, id), [-100.0, 0.0, 800.0, 300.0], "covers the bounds");
    let clip_group = |s: &Session| {
        let doc = &s.doc().unwrap().doc;
        let g = doc.node(doc.parent_of(id).unwrap()).unwrap();
        let NodeKind::Group { children, clip: true } = &g.kind else { panic!("not clipped: {g:?}") };
        (g.id, children[0].geometric_bounds().unwrap())
    };
    let (group, clip) = clip_group(&s);
    assert_eq!(clip, vectorcraft_geom::Rect::new(-100.0, 0.0, 500.0, 300.0), "clipped to the old bounds");
    // Relinked again: the same clip group, its path following.
    run(&mut s, "links.relink", json!({"ids": [id.0], "path": d.file("photo.png")}));
    assert_eq!(clip_group(&s).0, group);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(
        matches!(s.doc().unwrap().doc.node(s.doc().unwrap().doc.parent_of(id).unwrap()).unwrap().kind, NodeKind::Layer { .. }),
        "one undo step each"
    );
}

#[test]
fn placement_options_read_set_and_round_trip() {
    let dir = Folder::new("placement");
    let pic = dir.file("photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    assert_eq!(run(&mut s, "links.placementOptions", json!({})), json!({"placement": {"preserve": "bounds", "align": "center", "clip": false}}));
    let r = run(&mut s, "links.placementOptions", json!({"preserve": "fill", "align": "topRight", "clip": true}));
    assert_eq!(
        r,
        json!({"ids": [id.0], "placement": {"preserve": "fill", "align": "topRight", "clip": true}, "applied": []}),
        "placed unstretched: nothing to apply"
    );
    assert!(s.execute("links.placementOptions", &json!({"preserve": "stretch"})).is_err());
    assert!(s.execute("links.placementOptions", &json!({"align": "middle"})).is_err());
    // Saved and opened again.
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    let saved: Value = serde_json::from_slice(&std::fs::read(&doc_path).unwrap()).unwrap();
    assert_eq!(
        saved["document"]["layers"][0]["kind"]["children"][0]["kind"]["placement"],
        json!({"preserve": "fill", "align": "topRight", "clip": true})
    );
    open(&mut s, &doc_path);
    let want = PlacementOptions { preserve: vectorcraft_doc::links::Preserve::Fill, align: vectorcraft_doc::links::Align::TopRight, clip: true };
    assert_eq!(image(&s, id).placement, want);
    run(&mut s, "select.all", json!({}));
    run(&mut s, "links.placementOptions", json!({"preserve": "bounds", "align": "center", "clip": false}));
    save(&mut s, &doc_path);
    let saved: Value = serde_json::from_slice(&std::fs::read(&doc_path).unwrap()).unwrap();
    assert_eq!(saved["document"]["layers"][0]["kind"]["children"][0]["kind"]["placement"], Value::Null, "the default isn't written");
}

#[test]
fn the_links_commands_are_reachable_headless() {
    for id in [
        "links.list",
        "links.goTo",
        "links.embed",
        "links.unembed",
        "links.info",
        "links.placementOptions",
        "links.relink",
        "links.update",
        "links.check",
    ] {
        let c = crate::find_command(id).unwrap_or_else(|| panic!("{id} is not a command"));
        assert!(!c.params.is_empty(), "{id} documents its params");
    }
    // Without a document every one says why instead of failing silently.
    let mut s = Session::new();
    assert!(s.execute("links.list", &json!({})).is_err());
}

/// `r` rounded to a millionth, as `[x0, y0, x1, y1]`.
fn corners(r: vectorcraft_geom::Rect) -> [f64; 4] {
    [r.x0, r.y0, r.x1, r.y1].map(|v| (v * 1e6).round() / 1e6 + 0.0)
}

/// A 600×300 px file placed linked at the centre of the artboard (-100, 0, 500, 300), then
/// squeezed to half its width (50, 0, 350, 300: a 300×300 pt square) → session, id, its folder.
fn squeezed(name: &str) -> (Session, NodeId, Folder) {
    let dir = Folder::new(name);
    let pic = dir.file("photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "object.scale", json!({"sx": 50, "sy": 100}));
    assert_eq!(corners(bounds(&s, id)), [50.0, 0.0, 350.0, 300.0]);
    (s, id, dir)
}

/// Placement Options with `options` for image `id` → its answer and the image's bounds.
fn apply(s: &mut Session, id: NodeId, options: Value) -> (Value, [f64; 4]) {
    let mut p = options;
    p["ids"] = json!([id.0]);
    let r = run(s, "links.placementOptions", p);
    (r, corners(bounds(s, id)))
}

/// The scale Link Info gives image `id` (% of its 100% size, across and down).
fn scale_of(s: &mut Session, id: NodeId) -> [f64; 2] {
    let i = run(s, "links.info", json!({"id": id.0}));
    [0, 1].map(|k| (i["scale"][k].as_f64().unwrap() * 1e6).round() / 1e6)
}

/// Links panel › Placement Options applies the options at once: choosing Proportions (Fit or
/// Fill) or File Dimensions resets a distorted placed image's proportions
/// (https://community.adobe.com/questions-652/how-do-i-reset-a-placed-image-to-the-original-image-s-proportions-768645).
#[test]
fn placement_options_put_a_distorted_linked_image_back_in_proportion() {
    let (mut s, id, _d) = squeezed("apply-fit");
    let (r, b) = apply(&mut s, id, json!({"preserve": "fit"}));
    assert_eq!(b, [50.0, 75.0, 350.0, 225.0], "fitted inside the square, centred");
    assert_eq!(r["applied"], json!([id.0]), "{r}");
    assert_eq!(r.get("embedded"), None);
    assert_eq!(scale_of(&mut s, id), [50.0, 50.0], "uniform");

    let (mut s, id, _d) = squeezed("apply-fit-top");
    assert_eq!(apply(&mut s, id, json!({"preserve": "fit", "align": "top"})).1, [50.0, 0.0, 350.0, 150.0]);
    let (mut s, id, _d) = squeezed("apply-fit-bottom-right");
    assert_eq!(apply(&mut s, id, json!({"preserve": "fit", "align": "bottomRight"})).1, [50.0, 150.0, 350.0, 300.0]);

    let (mut s, id, _d) = squeezed("apply-fill");
    let (r, b) = apply(&mut s, id, json!({"preserve": "fill"}));
    assert_eq!(b, [-100.0, 0.0, 500.0, 300.0], "covers the square, centred");
    assert_eq!(scale_of(&mut s, id), [100.0, 100.0]);
    assert_eq!(r["applied"], json!([id.0]));
    let doc = &s.doc().unwrap().doc;
    assert!(matches!(doc.node(doc.parent_of(id).unwrap()).unwrap().kind, NodeKind::Layer { .. }), "not clipped without Clip");
    let (mut s, id, _d) = squeezed("apply-fill-left");
    assert_eq!(apply(&mut s, id, json!({"preserve": "fill", "align": "left"})).1, [50.0, 0.0, 650.0, 300.0]);

    let (mut s, id, _d) = squeezed("apply-file");
    assert_eq!(apply(&mut s, id, json!({"preserve": "fileDimensions"})).1, [-100.0, 0.0, 500.0, 300.0], "the file's 100%, centred");
    assert_eq!(scale_of(&mut s, id), [100.0, 100.0]);
    let (mut s, id, _d) = squeezed("apply-file-top-left");
    assert_eq!(apply(&mut s, id, json!({"preserve": "fileDimensions", "align": "topLeft"})).1, [50.0, 0.0, 650.0, 300.0]);
}

/// File Dimensions also takes the rotation off, at the art's top left; Fit keeps it and scales
/// the image uniformly.
#[test]
fn placement_options_on_a_rotated_image() {
    let (mut s, id, _d) = squeezed("apply-rotated-file");
    run(&mut s, "object.rotate", json!({"angle": 30}));
    let before = bounds(&s, id);
    let (_, b) = apply(&mut s, id, json!({"preserve": "fileDimensions", "align": "topLeft"}));
    let o = |v: f64| (v * 1e6).round() / 1e6 + 0.0;
    assert_eq!(b, [o(before.x0), o(before.y0), o(before.x0 + 600.0), o(before.y0 + 300.0)]);
    let i = run(&mut s, "links.info", json!({"id": id.0}));
    assert!(close(i["rotation"].as_f64().unwrap(), 0.0), "{i}");

    let (mut s, id, _d) = squeezed("apply-rotated-fit");
    run(&mut s, "object.rotate", json!({"angle": 30}));
    apply(&mut s, id, json!({"preserve": "fit"}));
    let i = run(&mut s, "links.info", json!({"id": id.0}));
    assert!(close(i["rotation"].as_f64().unwrap(), 30.0), "{i}");
    let [sx, sy] = scale_of(&mut s, id);
    assert!(close(sx, 50.0) && close(sy, 50.0), "{sx} {sy}");
}

/// Bounds and Transforms move nothing: the file is the one shown, in the same bounds at the same
/// scale. Nor does choosing Fit again once the image is in proportion.
#[test]
fn placement_options_that_keep_the_art_move_nothing() {
    for preserve in ["bounds", "transforms"] {
        let (mut s, id, _d) = squeezed(&format!("apply-{preserve}"));
        let xf = image(&s, id).xf;
        let (r, b) = apply(&mut s, id, json!({"preserve": preserve, "align": "topLeft", "clip": true}));
        assert_eq!((b, r["applied"].clone()), ([50.0, 0.0, 350.0, 300.0], json!([])), "{preserve}");
        assert_eq!(image(&s, id).xf, xf, "{preserve}: exactly where it was");
        assert_eq!(image(&s, id).placement.preserve.id(), preserve, "but the option is set");
    }
    let (mut s, id, _d) = squeezed("apply-twice");
    apply(&mut s, id, json!({"preserve": "fit"}));
    let (r, b) = apply(&mut s, id, json!({"preserve": "fit"}));
    assert_eq!((b, r["applied"].clone()), ([50.0, 75.0, 350.0, 225.0], json!([])));
}

/// Applying Placement Options is one undo step: the image's place and its options come back.
#[test]
fn applying_placement_options_is_one_undo_step() {
    let (mut s, id, _d) = squeezed("apply-undo");
    let before = image(&s, id);
    apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(image(&s, id), before);
    assert_eq!(corners(bounds(&s, id)), [50.0, 0.0, 350.0, 300.0]);
    let doc = &s.doc().unwrap().doc;
    assert!(matches!(doc.node(doc.parent_of(id).unwrap()).unwrap().kind, NodeKind::Layer { .. }), "the clip group goes too");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(corners(bounds(&s, id)), [-100.0, 0.0, 500.0, 300.0]);
}

/// Nothing is read: an image whose file is gone (showing its saved preview) is placed again from
/// what it holds.
#[test]
fn placement_options_apply_with_the_file_missing() {
    let (mut s, id, d) = squeezed("apply-missing");
    let doc_path = d.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    std::fs::remove_file(d.file("photo.png")).unwrap();
    let opened = open(&mut s, &doc_path);
    assert_eq!(opened["missingLinks"].as_array().map(Vec::len), Some(1), "{opened}");
    let (r, b) = apply(&mut s, id, json!({"preserve": "fit"}));
    assert_eq!((b, r["applied"].clone()), ([50.0, 75.0, 350.0, 225.0], json!([id.0])));
    assert_eq!(run(&mut s, "links.info", json!({"id": id.0}))["status"], "missing");
}

/// Placement Options are for linked files: an embedded image keeps its place, and its options are
/// set for when it is relinked.
#[test]
fn an_embedded_image_keeps_its_place() {
    let dir = Folder::new("apply-embedded");
    let pic = dir.file("photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place_embedded(&mut s, &pic);
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "object.scale", json!({"sx": 50, "sy": 100}));
    let before = corners(bounds(&s, id));
    let (r, b) = apply(&mut s, id, json!({"preserve": "fit"}));
    assert_eq!((b, r["applied"].clone(), r["embedded"].clone()), (before, json!([]), json!([id.0])));
    assert_eq!(image(&s, id).placement.preserve, vectorcraft_doc::links::Preserve::Fit);
}

/// Clip to Bounding Box clips art that now overflows to its old bounds, in a clip group of its
/// own; applied again it adds nothing.
#[test]
fn clip_to_bounding_box_clips_the_filled_image() {
    let (mut s, id, _d) = squeezed("apply-clip");
    let (r, b) = apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    assert_eq!((b, r["applied"].clone()), ([-100.0, 0.0, 500.0, 300.0], json!([id.0])));
    let clip_group = |s: &Session| {
        let doc = &s.doc().unwrap().doc;
        let g = doc.node(doc.parent_of(id).unwrap()).unwrap();
        let NodeKind::Group { children, clip: true } = &g.kind else { panic!("not clipped: {g:?}") };
        (g.id, children.len(), corners(children[0].geometric_bounds().unwrap()))
    };
    let (group, n, clip) = clip_group(&s);
    assert_eq!((n, clip), (2, [50.0, 0.0, 350.0, 300.0]), "clipped to the square it was squeezed into");
    let (r, _) = apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    assert_eq!((clip_group(&s).0, r["applied"].clone()), (group, json!([])), "no second clip group");
}

/// An image already inside a clip group isn't reshaped: a mask the user drew stays as drawn and
/// the image gets a clip group of its own inside it; one already clipped to the rectangle is kept;
/// without Clip, the mask is left alone.
#[test]
fn an_existing_clip_group_is_not_reshaped() {
    let (mut s, id, _d) = squeezed("apply-mask");
    let mask = NodeId(run(&mut s, "shape.ellipse", json!({"x": 100, "y": 50, "width": 200, "height": 200}))["id"].as_u64().unwrap());
    let outer = run(&mut s, "object.clippingMask.make", json!({"ids": [mask.0, id.0]}))["id"].as_u64().unwrap();
    let path_of = |s: &Session, id: NodeId| match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Path { path, .. } => path.clone(),
        k => panic!("not a path: {k:?}"),
    };
    let ellipse = path_of(&s, mask);
    // Without Clip: only the image changes.
    apply(&mut s, id, json!({"preserve": "fill"}));
    assert_eq!(path_of(&s, mask), ellipse);
    assert_eq!(s.doc().unwrap().doc.parent_of(id), Some(NodeId(outer)));
    run(&mut s, "edit.undo", json!({}));
    // With Clip: the ellipse stays, the image clipped inside it by a rectangle of its own.
    let (r, _) = apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    assert_eq!(r["applied"], json!([id.0]));
    assert_eq!(path_of(&s, mask), ellipse, "the user's mask isn't reshaped");
    let doc = &s.doc().unwrap().doc;
    let inner = doc.parent_of(id).unwrap();
    assert_ne!(inner, NodeId(outer));
    assert_eq!(doc.parent_of(inner), Some(NodeId(outer)));
    let NodeKind::Group { children, clip: true } = &doc.node(inner).unwrap().kind else { panic!("not a clip group") };
    assert_eq!(corners(children[0].geometric_bounds().unwrap()), [50.0, 0.0, 350.0, 300.0]);
}

/// A placed document takes its place again the same way, from the artboard it shows.
#[test]
fn placement_options_apply_to_a_placed_document() {
    let dir = Folder::new("apply-placed");
    let path = dir.file("card.vectorcraft");
    let mut d = vectorcraft_doc::Document::new(200.0, 100.0);
    let layer = d.layers[0].id;
    let rect = vectorcraft_doc::Node::path(
        d.alloc_id(),
        vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 200.0, 100.0)),
        vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::None, vectorcraft_color::Paint::None, 0.0),
    );
    d.insert(Some(layer), 0, rect).unwrap();
    write(&path, &vectorcraft_format::save_file(&d));
    let mut s = session();
    let id = place(&mut s, &path);
    assert!(matches!(s.doc().unwrap().doc.node(id).unwrap().kind, NodeKind::PlacedDocument(_)));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "object.scale", json!({"sx": 50, "sy": 100}));
    assert_eq!(corners(bounds(&s, id)), [150.0, 100.0, 250.0, 200.0]);
    std::fs::remove_file(&path).unwrap();
    let (r, b) = apply(&mut s, id, json!({"preserve": "fit"}));
    assert_eq!((b, r["applied"].clone()), ([150.0, 125.0, 250.0, 175.0], json!([id.0])));
}

/// A clip group whose clipping path is already the rectangle of the image's bounds clips it as
/// Clip to Bounding Box would: it is kept, with no second clip group inside it.
#[test]
fn a_clip_to_the_images_own_bounds_is_kept() {
    let (mut s, id, _d) = squeezed("apply-own-clip");
    let r = run(&mut s, "shape.rectangle", json!({"x": 50, "y": 0, "width": 300, "height": 300}));
    let outer = NodeId(run(&mut s, "object.clippingMask.make", json!({"ids": [r["id"], id.0]}))["id"].as_u64().unwrap());
    let (r, b) = apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    assert_eq!((b, r["applied"].clone()), ([-100.0, 0.0, 500.0, 300.0], json!([id.0])), "moved");
    assert_eq!(s.doc().unwrap().doc.parent_of(id), Some(outer), "already clipped to its bounds");
}
