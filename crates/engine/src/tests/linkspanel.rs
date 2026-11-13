//! The Links panel's commands: listing images with their status (`links.list`), Go To, Embed and
//! Unembed, Link Info and the Placement Options a relinked file follows.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeKind, PlacementOptions};

use crate::tests_links::{BLUE, Folder, RED, bounds, centre_colour, image, near, open, place, png, save, session, write};
use crate::*;

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
    png_header_as(w, h, [1, 0, 0, 0, 0])
}

/// A PNG that says it is `w` × `h` pixels of the bit depth, color type, compression, filter and
/// interlace method `kind` gives (its header and no rows).
fn png_header_as(w: u32, h: u32, kind: [u8; 5]) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let ihdr: Vec<u8> = [&b"IHDR"[..], &w.to_be_bytes(), &h.to_be_bytes(), &kind].concat();
    for chunk in [&ihdr[..], b"IDAT\x78\x9c", b"IEND"] {
        out.extend((chunk.len() as u32 - 4).to_be_bytes());
        out.extend(chunk);
        out.extend(crc32fast::hash(chunk).to_be_bytes());
    }
    out
}

/// An image past what a decode may take is drawn at reduced size: Place and Open say so and the
/// Links panel's row carries the note. One the renderer can't read at reduced size (an interlaced
/// RGBA PNG past the allowance as decoded, or an image of more pixels than the allowance has
/// bytes) isn't drawn, and the note says it is too large to draw (#1479).
#[test]
fn an_image_too_large_to_draw_is_reported() {
    const NOTE: &str = "an image of 40000 × 40000 pixels is shown at reduced size";
    const UNDRAWN: &str = "an image of 30000 × 30000 pixels is too large to draw";
    let dir = Folder::new("huge");
    let (big, small, laced) = (dir.file("wall.png"), dir.file("small.png"), dir.file("laced.png"));
    write(&big, &png_header(40_000, 40_000));
    write(&small, &png(40, 20, BLUE));
    write(&laced, &png_header_as(30_000, 30_000, [8, 6, 0, 0, 1]));
    let mut s = session();
    assert_eq!(run(&mut s, "file.place", json!({"path": big, "at": [200, 150], "link": false}))["warnings"], json!([NOTE]));
    assert_eq!(run(&mut s, "file.place", json!({"path": small, "at": [20, 20], "link": false}))["warnings"], json!([]));
    assert_eq!(run(&mut s, "file.place", json!({"path": laced, "at": [20, 20], "link": false}))["warnings"], json!([UNDRAWN]));
    let l = run(&mut s, "links.list", json!({}));
    let notes: Vec<&Value> = l["links"].as_array().unwrap().iter().map(|r| &r["note"]).collect();
    assert_eq!(notes, [&json!(UNDRAWN), &Value::Null, &json!(NOTE)], "top first: the small image has nothing to say");
    // Opened as a document: said too. An image that decodes (16,000 px square) has no note.
    assert_eq!(crate::cmd::fileio::load("wall.png", &png_header(40_000, 40_000)).unwrap().warnings, [NOTE]);
    let more = crate::cmd::fileio::load("more.png", &png_header(200_000, 200_000)).unwrap().warnings;
    assert_eq!(more, ["an image of 200000 × 200000 pixels is too large to draw"], "more pixels than the allowance has bytes");
    assert!(crate::cmd::fileio::load("fine.png", &png_header(16_000, 16_000)).unwrap().warnings.is_empty());
    // 20,000 px square at 16 bits a sample is 3.2 GB decoded and 1.6 GB as RGBA: read a row at a
    // time at its own size, so nothing is said of it.
    assert!(crate::cmd::fileio::load("deep.png", &png_header_as(20_000, 20_000, [16, 6, 0, 0, 0])).unwrap().warnings.is_empty());
}

/// A gray image is counted as RGBA, 4 bytes a pixel: 23,171 × 23,170 1-bit pixels decode to 537 MB
/// of gray and are 2,147,488,280 bytes as RGBA, past the 2 GiB allowance (shown at reduced size);
/// 23,170 × 23,170 are 2,147,395,600 bytes, within it. The file within the allowance holds no
/// pixels, so Place decodes it and refuses it without the note.
#[test]
fn a_gray_image_too_large_as_rgba_is_reported_on_place() {
    const NOTE: &str = "an image of 23171 × 23170 pixels is shown at reduced size";
    let dir = Folder::new("huge-gray-place");
    let (over, within) = (dir.file("over.png"), dir.file("within.png"));
    write(&over, &png_header(23_171, 23_170));
    write(&within, &png_header(23_170, 23_170));
    let mut s = session();
    assert_eq!(run(&mut s, "file.place", json!({"path": over, "at": [200, 150], "link": false}))["warnings"], json!([NOTE]));
    assert_eq!(run(&mut s, "file.place.info", json!({"path": within}))["warnings"], json!([]), "the image within the allowance has nothing to say");
    let e = s.execute("file.place", &json!({"path": within, "at": [20, 20], "link": false})).unwrap_err();
    assert_eq!(e.to_string(), "the image's pixels can't be read");
    let l = run(&mut s, "links.list", json!({}));
    let notes: Vec<&Value> = l["links"].as_array().unwrap().iter().map(|r| &r["note"]).collect();
    assert_eq!(notes, [&json!(NOTE)]);
}

/// File › Open says the same of a gray image past the allowance as RGBA, and nothing of one within it.
#[test]
fn a_gray_image_too_large_as_rgba_is_reported_on_open() {
    let dir = Folder::new("huge-gray-open");
    let (over, within) = (dir.file("over.png"), dir.file("within.png"));
    write(&over, &png_header(23_171, 23_170));
    write(&within, &png_header(23_170, 23_170));
    let mut s = session();
    assert_eq!(open(&mut s, &over)["warnings"], json!(["an image of 23171 × 23170 pixels is shown at reduced size"]));
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

/// An embedded image whose pixels can't be read (a document stores its bytes without decoding
/// them) is not unembedded, and no file is written.
#[test]
fn unembed_of_pixels_that_cant_be_read_writes_no_file() {
    let dir = Folder::new("unembed-unread");
    let pic = dir.file("photo.png");
    write(&pic, &png(60, 30, RED));
    let mut s = session();
    let id = place_embedded(&mut s, &pic);
    let key = image(&s, id).key;
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).images.insert(key, vectorcraft_doc::ImageBlob::png(png_header(60, 30)));
    let out = dir.file("out.png");
    let e = s.execute("links.unembed", &json!({"id": id.0, "path": out})).unwrap_err();
    assert!(e.to_string().ends_with("the image's pixels can't be read"), "{e}");
    assert!(!std::path::Path::new(&out).exists(), "no file left behind");
    assert!(image(&s, id).link.is_none());
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
    assert_eq!(r, json!({"id": id.0, "bounds": [-100.0, 0.0, 500.0, 300.0], "onCanvas": true}));
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

/// Link Info reads within the opener's roots for an unconfined caller, and never past a confined
/// caller's own roots.
#[test]
fn link_info_stays_within_the_asking_callers_roots() {
    let dir = Folder::new("info-roots");
    let other = Folder::new("info-other");
    let pic = dir.file("photo.png");
    write(&pic, &png(60, 30, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    let roots = |p: &std::path::Path| crate::file_access::AutomationRoots::new(&[p], &[]).unwrap().unwrap();
    s.active_mut().unwrap().opened_by = Some(roots(&dir.0));
    assert_eq!(run(&mut s, "links.info", json!({"id": id.0}))["status"], "ok");
    let narrow = roots(&other.0);
    let i = crate::file_access::confine(Some(&narrow), || run(&mut s, "links.info", json!({"id": id.0})));
    assert_ne!(i["status"], "ok", "{i}");
}

/// A confined caller's `links.list`, `links.check` and `links.info` replies, and its
/// `document.open` reply, leave out the path and folder of a linked file outside its read roots
/// and keep the row with the file's name. A file found by name inside the roots shows its path.
#[test]
fn link_paths_outside_the_read_roots_are_left_out() {
    let away = Folder::new("lrr-away");
    let home = Folder::new("lrr-home");
    let pic = away.file("photo.png");
    let bytes = png(60, 30, RED);
    write(&pic, &bytes);
    let mut s = session();
    let id = place(&mut s, &pic);
    let roots = crate::file_access::AutomationRoots::new(&[&home.0], &[]).unwrap().unwrap();
    let ask = |s: &mut Session, cmd: &str, p: Value| crate::file_access::confine(Some(&roots), || run(s, cmd, p));
    let hidden = |r: &Value| assert!(!r.to_string().contains("lrr-away"), "{r}");
    let list = ask(&mut s, "links.list", json!({}));
    hidden(&list);
    let row = &list["links"][0];
    assert_eq!((row["id"].as_u64(), row["name"].as_str(), row["status"].as_str()), (Some(id.0), Some("photo.png"), Some("missing")), "{list}");
    let check = ask(&mut s, "links.check", json!({}));
    hidden(&check);
    assert_eq!((check["links"][0]["name"].as_str(), check["missing"].as_u64()), (Some("photo.png"), Some(1)), "{check}");
    let info = ask(&mut s, "links.info", json!({"id": id.0}));
    hidden(&info);
    assert_eq!(
        (info["fileName"].as_str(), info["status"].as_str(), info["linked"].as_bool()),
        (Some("photo.png"), Some("missing"), Some(true)),
        "{info}"
    );
    for k in ["path", "link", "location", "fileSize", "modified", "created"] {
        assert!(info.get(k).is_none(), "{k}: {info}");
    }
    // Unconfined, the same replies carry the paths.
    assert_eq!(run(&mut s, "links.list", json!({}))["links"][0]["path"], json!(pic));
    let i = run(&mut s, "links.info", json!({"id": id.0}));
    assert_eq!((&i["path"], &i["link"], i["fileSize"].as_u64()), (&json!(pic), &json!(pic), Some(bytes.len() as u64)), "{i}");
    // Saved inside the roots next to a copy of the file: the copy is found by name and shown.
    save(&mut s, &home.file("poster.vectorcraft"));
    let copy = home.file("photo.png");
    write(&copy, &bytes);
    let check = ask(&mut s, "links.check", json!({}));
    hidden(&check);
    assert_eq!((check["links"][0]["status"].as_str(), &check["links"][0]["found"]), (Some("ok"), &json!(copy)), "{check}");
    let list = ask(&mut s, "links.list", json!({}));
    hidden(&list);
    assert_eq!(list["links"][0]["found"], json!(copy), "{list}");
    // Opened again without the copy: the missing link's row has no path.
    std::fs::remove_file(&copy).unwrap();
    let opened = ask(&mut Session::new(), "document.open", json!({"path": home.file("poster.vectorcraft")}));
    hidden(&opened);
    assert_eq!(opened["missingLinks"][0]["name"], "photo.png", "{opened}");
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

/// A preview doesn't keep its file's resolution, so with the file missing File Dimensions leaves
/// the image in place (answered as `previewing`) rather than size it at 72 ppi; Fit and Fill, which
/// need only its proportions, still apply. With the file read, File Dimensions honors its 300 ppi.
#[test]
fn file_dimensions_waits_for_a_missing_file() {
    // 600 × 300 px at 300 ppi: 144 × 72 pt at 100%, placed centred on (200, 150) and squeezed
    // to a 72 pt square.
    let squeezed_300 = |name: &str| {
        let dir = Folder::new(name);
        let pic = dir.file("photo.png");
        write(&pic, &crate::cmd::fileio::ppi::with_png_resolution(&png(600, 300, RED), (300.0, 300.0)));
        let mut s = session();
        let id = place(&mut s, &pic);
        run(&mut s, "select.set", json!({"ids": [id.0]}));
        run(&mut s, "object.scale", json!({"sx": 50, "sy": 100}));
        assert_eq!(corners(bounds(&s, id)), [164.0, 114.0, 236.0, 186.0]);
        (s, id, dir)
    };
    let (mut s, id, _d) = squeezed_300("apply-file-300");
    let (r, b) = apply(&mut s, id, json!({"preserve": "fileDimensions"}));
    assert_eq!((b, r["applied"].clone(), r.get("previewing")), ([128.0, 114.0, 272.0, 186.0], json!([id.0]), None), "{r}");

    let (mut s, id, d) = squeezed_300("apply-file-300-missing");
    let doc_path = d.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    std::fs::remove_file(d.file("photo.png")).unwrap();
    let opened = open(&mut s, &doc_path);
    assert_eq!(opened["missingLinks"].as_array().map(Vec::len), Some(1), "{opened}");
    let xf = image(&s, id).xf;
    let (r, b) = apply(&mut s, id, json!({"preserve": "fileDimensions"}));
    assert_eq!((b, r["applied"].clone(), r["previewing"].clone()), ([164.0, 114.0, 236.0, 186.0], json!([]), json!([id.0])), "{r}");
    assert_eq!(image(&s, id).xf, xf, "exactly where it was");
    assert_eq!(image(&s, id).placement.preserve.id(), "fileDimensions", "but the option is set");
    let (r, b) = apply(&mut s, id, json!({"preserve": "fit"}));
    assert_eq!((b, r["applied"].clone(), r.get("previewing")), ([164.0, 132.0, 236.0, 168.0], json!([id.0]), None), "{r}");
    run(&mut s, "edit.undo", json!({}));
    let (r, b) = apply(&mut s, id, json!({"preserve": "fill"}));
    assert_eq!((b, r["applied"].clone()), ([128.0, 114.0, 272.0, 186.0], json!([id.0])), "{r}");
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

/// The rectangle of the clip group holding just image `id` of `s`, as [`corners`].
fn own_clip(s: &Session, id: NodeId) -> [f64; 4] {
    let doc = &s.doc().unwrap().doc;
    let g = doc.node(doc.parent_of(id).unwrap()).unwrap();
    let NodeKind::Group { children, clip: true } = &g.kind else { panic!("not clipped: {g:?}") };
    assert_eq!(children.len(), 2);
    corners(children[0].geometric_bounds().unwrap())
}

/// Once Clip to Bounding Box clips a filled image, its clip's rectangle is the bounds the options
/// place it in: a new alignment moves the image within it, and Fit fits it inside.
#[test]
fn a_clipped_image_is_placed_in_its_clip() {
    let (mut s, id, _d) = squeezed("apply-clip-align");
    apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    let (r, b) = apply(&mut s, id, json!({"align": "left"}));
    assert_eq!((b, r["applied"].clone(), own_clip(&s, id)), ([50.0, 0.0, 650.0, 300.0], json!([id.0]), [50.0, 0.0, 350.0, 300.0]));
    let (_, b) = apply(&mut s, id, json!({"align": "right"}));
    assert_eq!((b, own_clip(&s, id)), ([-250.0, 0.0, 350.0, 300.0], [50.0, 0.0, 350.0, 300.0]));
    let (_, b) = apply(&mut s, id, json!({"preserve": "fit", "align": "center"}));
    assert_eq!((b, own_clip(&s, id)), ([50.0, 75.0, 350.0, 225.0], [50.0, 0.0, 350.0, 300.0]), "fitted inside the clip");
}

/// A clipped, filled image whose file changes fills its clip again: the clip keeps its rectangle.
#[test]
fn a_clipped_image_updates_in_its_clip() {
    let (mut s, id, d) = squeezed("apply-clip-update");
    apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    write(&d.file("photo.png"), &png(300, 600, RED));
    assert_eq!(run(&mut s, "links.update", json!({})), json!({"updated": [id.0], "missing": []}));
    assert_eq!((corners(bounds(&s, id)), own_clip(&s, id)), ([50.0, -150.0, 350.0, 450.0], [50.0, 0.0, 350.0, 300.0]));
}

/// Transforms keeps a clipped image where it is when its file changes, and its clip as it was.
#[test]
fn a_clipped_image_keeps_its_place_and_clip_with_transforms() {
    let (mut s, id, d) = squeezed("apply-clip-transforms");
    apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    let (r, b) = apply(&mut s, id, json!({"align": "left"}));
    assert_eq!((b, r["applied"].clone()), ([50.0, 0.0, 650.0, 300.0], json!([id.0])));
    let (r, b) = apply(&mut s, id, json!({"preserve": "transforms", "align": "center"}));
    assert_eq!((b, r["applied"].clone()), ([50.0, 0.0, 650.0, 300.0], json!([])));
    write(&d.file("photo.png"), &png(600, 300, BLUE));
    run(&mut s, "links.update", json!({}));
    assert_eq!((corners(bounds(&s, id)), own_clip(&s, id)), ([50.0, 0.0, 650.0, 300.0], [50.0, 0.0, 350.0, 300.0]), "the same size: unmoved");
    // A bigger file at the same scale, centred on the art's own bounds: the clip isn't widened.
    write(&d.file("photo.png"), &png(1200, 600, RED));
    run(&mut s, "links.update", json!({}));
    assert_eq!((corners(bounds(&s, id)), own_clip(&s, id)), ([-250.0, -150.0, 950.0, 450.0], [50.0, 0.0, 350.0, 300.0]));
}

/// The clipping path of the clip group holding image `id` of `s` (with whether Clip to Bounding
/// Box made it) and the clip groups between it and its layer.
fn clips_around(s: &Session, id: NodeId) -> (vectorcraft_geom::PathData, bool, usize) {
    let doc = &s.doc().unwrap().doc;
    let g = doc.node(doc.parent_of(id).unwrap()).unwrap();
    let NodeKind::Group { children, clip: true } = &g.kind else { panic!("not clipped: {g:?}") };
    let NodeKind::Path { path, .. } = &children[0].kind else { panic!("no clipping path") };
    let mut depth = 0;
    let mut at = id;
    while let Some(p) = doc.parent_of(at) {
        if matches!(doc.node(p).unwrap().kind, NodeKind::Group { clip: true, .. }) {
            depth += 1;
        }
        at = p;
    }
    (path.clone(), g.extras().placement_clip, depth)
}

/// A rectangle the user clips a linked image with is the user's crop, not Clip to Bounding Box's
/// clip, even with Clip on: Placement Options and an update place the image in its own bounds and
/// never reshape the crop.
#[test]
fn a_rectangular_crop_the_user_made_is_left_alone() {
    let (mut s, id, d) = squeezed("apply-user-crop");
    apply(&mut s, id, json!({"preserve": "fit", "clip": true}));
    assert_eq!(corners(bounds(&s, id)), [50.0, 75.0, 350.0, 225.0], "in proportion, nothing to clip");
    let crop = run(&mut s, "shape.rectangle", json!({"x": 50, "y": 75, "width": 100, "height": 100}));
    run(&mut s, "object.clippingMask.make", json!({"ids": [crop["id"], id.0]}));
    let (path, ours, _) = clips_around(&s, id);
    assert!(!ours);
    let (r, b) = apply(&mut s, id, json!({"align": "topLeft"}));
    assert_eq!((b, r["applied"].clone()), ([50.0, 75.0, 350.0, 225.0], json!([])), "placed in its own bounds, not the crop");
    write(&d.file("photo.png"), &png(600, 600, RED));
    run(&mut s, "links.update", json!({}));
    assert_eq!(corners(bounds(&s, id)), [50.0, 75.0, 200.0, 225.0], "fitted to its own bounds");
    assert_eq!(clips_around(&s, id), (path, false, 1), "the crop as drawn");
}

/// An update that clips an image inside a mask the user drew gives it a clip group of its own, as
/// OK does; the mask keeps its shape, and the next update reshapes only that clip group.
#[test]
fn an_update_never_reshapes_a_mask_the_user_drew() {
    let (mut s, id, d) = squeezed("update-mask");
    let mask = NodeId(run(&mut s, "shape.ellipse", json!({"x": 100, "y": 50, "width": 200, "height": 200}))["id"].as_u64().unwrap());
    run(&mut s, "object.clippingMask.make", json!({"ids": [mask.0, id.0]}));
    let ellipse = |s: &Session| match &s.doc().unwrap().doc.node(mask).unwrap().kind {
        NodeKind::Path { path, .. } => path.clone(),
        k => panic!("not a path: {k:?}"),
    };
    let drawn = ellipse(&s);
    // Fill and Clip set without applying them, then a wider file comes in.
    let o = PlacementOptions { preserve: vectorcraft_doc::links::Preserve::Fill, clip: true, ..Default::default() };
    s.edit("t", |d, _| {
        if let NodeKind::Image(im) = &mut d.node_mut(id).unwrap().kind {
            im.placement = o;
        }
        Ok(())
    })
    .unwrap();
    write(&d.file("photo.png"), &png(900, 300, RED));
    run(&mut s, "links.update", json!({}));
    let (clip, ours, depth) = clips_around(&s, id);
    assert_eq!((corners(clip.bounds().unwrap()), ours, depth), ([50.0, 0.0, 350.0, 300.0], true, 2), "a clip group of its own inside the mask");
    assert_eq!(ellipse(&s), drawn, "the mask keeps its shape");
    write(&d.file("photo.png"), &png(1200, 300, BLUE));
    run(&mut s, "links.update", json!({}));
    let (clip, ours, depth) = clips_around(&s, id);
    assert_eq!((corners(clip.bounds().unwrap()), ours, depth), ([50.0, 0.0, 350.0, 300.0], true, 2), "no third clip");
    assert_eq!(ellipse(&s), drawn);
}

/// The clip groups around image `inner` in symbol `name`'s art, outermost first: each one's
/// clipping path and whether it is Clip to Bounding Box's own.
fn symbol_clips(s: &Session, name: &str, inner: NodeId) -> Vec<(vectorcraft_geom::PathData, bool)> {
    fn walk(n: &vectorcraft_doc::Node, inner: NodeId, out: &mut Vec<(vectorcraft_geom::PathData, bool)>) -> bool {
        if n.id == inner {
            return true;
        }
        let Some(children) = n.children() else { return false };
        for c in children {
            let at = out.len();
            if let (NodeKind::Group { clip: true, children: kids }, Some(first)) = (&n.kind, children.first())
                && let NodeKind::Path { path, clipping: true, .. } = &first.kind
                && !kids.is_empty()
            {
                out.push((path.clone(), n.extras().placement_clip));
            }
            if walk(c, inner, out) {
                return true;
            }
            out.truncate(at);
        }
        false
    }
    let doc = &s.doc().unwrap().doc;
    let sym = doc.symbols.iter().find(|y| y.name == name).unwrap();
    let mut out = vec![];
    assert!(walk(&sym.art, inner, &mut out), "the image is in the symbol");
    out
}

/// Outside the layers too (a symbol's art), an update that clips an image inside a mask the user
/// drew gives it a clip group of its own, marked as Clip to Bounding Box's, and never reshapes the
/// mask; the next update reshapes only that clip group.
#[test]
fn an_update_in_a_symbol_never_reshapes_a_mask_the_user_drew() {
    let (mut s, id, d) = squeezed("update-mask-symbol");
    let mask = run(&mut s, "shape.ellipse", json!({"x": 100, "y": 50, "width": 200, "height": 200}))["id"].clone();
    run(&mut s, "object.clippingMask.make", json!({"ids": [mask, id.0]}));
    // An ellipse: its anchors have handles, a rectangle's none.
    let curved = |p: &vectorcraft_geom::PathData| p.anchors().any(|(_, _, a)| a.h_in != a.p);
    let o = PlacementOptions { preserve: vectorcraft_doc::links::Preserve::Fill, clip: true, ..Default::default() };
    s.edit("t", |d, _| {
        if let NodeKind::Image(im) = &mut d.node_mut(id).unwrap().kind {
            im.placement = o;
        }
        Ok(())
    })
    .unwrap();
    let group = s.doc().unwrap().doc.parent_of(id).unwrap();
    run(&mut s, "symbol.new", json!({"name": "Masked", "ids": [group.0]}));
    let mut inner = None;
    s.doc().unwrap().doc.symbols.iter().find(|y| y.name == "Masked").unwrap().art.walk(&mut |n: &vectorcraft_doc::Node| {
        if matches!(n.kind, NodeKind::Image(_)) {
            inner = Some(n.id);
        }
    });
    let inner = inner.expect("the image in the symbol");
    assert_eq!(symbol_clips(&s, "Masked", inner).len(), 1, "the mask alone");
    write(&d.file("photo.png"), &png(900, 300, RED));
    run(&mut s, "links.update", json!({}));
    let clips = symbol_clips(&s, "Masked", inner);
    assert_eq!(clips.len(), 2, "a clip group of its own inside the mask");
    assert!(!clips[0].1 && clips[1].1, "the mask is the user's, the inner group Clip to Bounding Box's: {clips:?}");
    assert!(curved(&clips[0].0) && !curved(&clips[1].0), "the mask keeps its shape: {clips:?}");
    write(&d.file("photo.png"), &png(1200, 300, BLUE));
    run(&mut s, "links.update", json!({}));
    let again = symbol_clips(&s, "Masked", inner);
    assert_eq!(again.len(), 2, "no third clip");
    assert!(curved(&again[0].0) && again[1].1, "the mask keeps its shape: {again:?}");
}

/// Turning Clip off lets the art out of the clip group Clip to Bounding Box made, in one undo step.
#[test]
fn clip_off_lets_the_art_go() {
    let (mut s, id, _d) = squeezed("apply-clip-off");
    apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    let (r, b) = apply(&mut s, id, json!({"clip": false}));
    assert_eq!((b, r["applied"].clone()), ([-100.0, 0.0, 500.0, 300.0], json!([id.0])));
    let doc = &s.doc().unwrap().doc;
    assert!(matches!(doc.node(doc.parent_of(id).unwrap()).unwrap().kind, NodeKind::Layer { .. }), "unclipped");
    run(&mut s, "edit.undo", json!({}));
    assert!(clips_around(&s, id).1, "clipped again");
}

/// OK with the options as they were and nothing to move records no undo step.
#[test]
fn placement_options_that_change_nothing_leave_no_undo_step() {
    let (mut s, id, _d) = squeezed("apply-no-op");
    apply(&mut s, id, json!({"preserve": "fit"}));
    let steps = s.doc().unwrap().history.undo.len();
    let (r, _) = apply(&mut s, id, json!({"preserve": "fit", "align": "center", "clip": false}));
    assert_eq!(r["applied"], json!([]));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps, "nothing changed");
    apply(&mut s, id, json!({"align": "top"}));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
}

/// File Dimensions on a rotated image Clip to Bounding Box has clipped: the art comes back at 100%,
/// unrotated, and stays in the same clip group, its rectangle turned with the old bounds.
#[test]
fn file_dimensions_keeps_a_rotated_clip() {
    let (mut s, id, _d) = squeezed("apply-rotated-clip");
    run(&mut s, "object.rotate", json!({"angle": 30}));
    apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    let (clip, ours, depth) = clips_around(&s, id);
    assert!(ours && depth == 1);
    let (r, _) = apply(&mut s, id, json!({"preserve": "fileDimensions"}));
    assert_eq!(r["applied"], json!([id.0]));
    let i = run(&mut s, "links.info", json!({"id": id.0}));
    assert!(close(i["rotation"].as_f64().unwrap(), 0.0), "{i}");
    assert_eq!(scale_of(&mut s, id), [100.0, 100.0]);
    assert_eq!(clips_around(&s, id), (clip, true, 1), "the same clip, not a second");
}

/// Fit takes a shear off as well as a stretch: the image comes back square-cornered, in
/// proportion.
#[test]
fn fit_takes_a_shear_off() {
    let (mut s, id, _d) = squeezed("apply-shear");
    let xf = image(&s, id).xf * vectorcraft_geom::Affine::new([1.0, 0.0, 0.5, 1.0, 0.0, 0.0]);
    s.edit("t", |d, _| {
        if let NodeKind::Image(im) = &mut d.node_mut(id).unwrap().kind {
            im.xf = xf;
        }
        Ok(())
    })
    .unwrap();
    let (r, _) = apply(&mut s, id, json!({"preserve": "fit"}));
    assert_eq!(r["applied"], json!([id.0]));
    let [a, b, c, d, ..] = image(&s, id).xf.as_coeffs();
    assert!(close(a * c + b * d, 0.0), "square corners: {:?}", image(&s, id).xf);
    assert!(close(a.hypot(b), c.hypot(d)), "in proportion: {:?}", image(&s, id).xf);
}

/// Placement Options read for images whose options differ name the differing ones as `mixed`;
/// ids are checked.
#[test]
fn placement_options_read_mixed_and_check_ids() {
    let (mut s, a, d) = squeezed("apply-mixed");
    let b = place(&mut s, &d.file("photo.png"));
    run(&mut s, "links.placementOptions", json!({"ids": [b.0], "preserve": "fit", "clip": true}));
    let r = run(&mut s, "links.placementOptions", json!({"ids": [a.0, b.0]}));
    assert_eq!(r, json!({"placement": {"preserve": "bounds", "align": "center", "clip": false}, "mixed": ["preserve", "clip"]}));
    assert_eq!(run(&mut s, "links.placementOptions", json!({"ids": [a.0, a.0]})).get("mixed"), None);
    assert!(s.execute("links.placementOptions", &json!({"ids": [a.0, "x"], "preserve": "fit"})).is_err());
    assert!(s.execute("links.placementOptions", &json!({"ids": "12"})).is_err());
}

/// The clip group holding image `id` of `s`.
fn clip_group_of(s: &Session, id: NodeId) -> NodeId {
    s.doc().unwrap().doc.parent_of(id).unwrap()
}

/// Clip off keeps what the clip group carries besides the clip: one given an opacity stays, a
/// plain group without its clipping path.
#[test]
fn clip_off_keeps_the_groups_opacity() {
    let (mut s, id, _d) = squeezed("apply-clip-off-opacity");
    apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    let g = clip_group_of(&s, id);
    s.edit("t", |d, _| {
        d.node_mut(g).unwrap().opacity = 0.5;
        Ok(())
    })
    .unwrap();
    let (r, _) = apply(&mut s, id, json!({"clip": false}));
    assert_eq!(r["applied"], json!([id.0]));
    let doc = &s.doc().unwrap().doc;
    let group = doc.node(g).unwrap();
    assert!(matches!(&group.kind, NodeKind::Group { children, clip: false } if children.len() == 1), "{group:?}");
    assert_eq!((group.opacity, group.extras().placement_clip), (0.5, false));
}

/// A clip group Clip to Bounding Box made is the user's once released: made a clipping mask again
/// around a shape the user drew, Clip off and an update leave it.
#[test]
fn a_released_placement_clip_is_the_users() {
    let (mut s, id, d) = squeezed("apply-released-clip");
    apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    let g = clip_group_of(&s, id);
    run(&mut s, "select.set", json!({"ids": [g.0]}));
    run(&mut s, "object.clippingMask.release", json!({}));
    assert!(!s.doc().unwrap().doc.node(g).unwrap().extras().placement_clip);
    // Clipping again, with the user's ellipse for its path (as the Layers panel's clip toggle does).
    let ellipse = vectorcraft_geom::shapes::ellipse(vectorcraft_geom::Rect::new(100.0, 50.0, 300.0, 250.0));
    s.edit("t", |doc, _| {
        let c = doc.node(g).unwrap().children().unwrap()[0].id;
        if let NodeKind::Path { path, clipping, .. } = &mut doc.node_mut(c).unwrap().kind {
            (*path, *clipping) = (ellipse.clone(), true);
        }
        doc.node_mut(g).unwrap().set_clips(true);
        Ok(())
    })
    .unwrap();
    apply(&mut s, id, json!({"clip": false}));
    assert_eq!(clip_group_of(&s, id), g, "the user's mask stays");
    write(&d.file("photo.png"), &png(900, 300, RED));
    run(&mut s, "links.update", json!({}));
    let doc = &s.doc().unwrap().doc;
    let NodeKind::Path { path, .. } = &doc.node(g).unwrap().children().unwrap()[0].kind else { panic!() };
    assert_eq!(*path, ellipse, "never reshaped");
}

/// After File Dimensions has unrotated a clipped image, its clip, still turned with the old bounds,
/// is still its clip: a new alignment moves the art in it, and a bigger file is clipped by it.
#[test]
fn a_rotated_clip_stays_the_images_after_file_dimensions() {
    let (mut s, id, d) = squeezed("apply-rotated-clip-later");
    run(&mut s, "object.rotate", json!({"angle": 45}));
    apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    apply(&mut s, id, json!({"preserve": "fileDimensions"}));
    let (clip, _, _) = clips_around(&s, id);
    let box_ = clip.bounds().unwrap();
    let (r, b) = apply(&mut s, id, json!({"align": "topLeft"}));
    assert_eq!(r["applied"], json!([id.0]));
    assert_eq!([b[0], b[1]], corners(box_)[..2], "at the clip's bounding box's top left");
    write(&d.file("photo.png"), &png(1200, 600, RED));
    run(&mut s, "links.update", json!({}));
    assert_eq!(clips_around(&s, id), (clip, true, 1), "the rotated clip, not the art's own rectangle");
}

/// File Dimensions centres a sheared image in the bounding box of the art as it was placed.
#[test]
fn file_dimensions_centres_a_sheared_image_in_its_bounding_box() {
    let (mut s, id, _d) = squeezed("apply-shear-file");
    let before = bounds(&s, id);
    let xf = image(&s, id).xf * vectorcraft_geom::Affine::new([1.0, 0.0, 0.5, 1.0, 0.0, 0.0]);
    s.edit("t", |d, _| {
        if let NodeKind::Image(im) = &mut d.node_mut(id).unwrap().kind {
            im.xf = xf;
        }
        Ok(())
    })
    .unwrap();
    let sheared = bounds(&s, id);
    assert!(sheared.width() > before.width());
    let (_, b) = apply(&mut s, id, json!({"preserve": "fileDimensions"}));
    let mid = |r: [f64; 4]| [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0];
    assert_eq!(mid(b), mid(corners(sheared)));
}

/// Clip off on the clip group isolation mode is in ends isolation mode.
#[test]
fn clip_off_ends_isolating_its_group() {
    let (mut s, id, _d) = squeezed("apply-clip-off-isolated");
    apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    let g = clip_group_of(&s, id);
    run(&mut s, "select.set", json!({"ids": [g.0]}));
    run(&mut s, "object.isolate", json!({}));
    assert_eq!(s.doc().unwrap().isolation, Some(g));
    apply(&mut s, id, json!({"clip": false}));
    assert_eq!(s.doc().unwrap().isolation, None);
}

/// Far from the origin a clip a little wider than the bounds is still not the bounds' rectangle,
/// and a thin image's overflow across its short side is still an overflow.
#[test]
fn clip_tolerances_follow_the_art_not_its_place() {
    let (mut s, id, _d) = squeezed("apply-far");
    run(&mut s, "object.move", json!({"dx": 1_000_000, "dy": 0}));
    let r = run(&mut s, "shape.rectangle", json!({"x": 1_000_049.5, "y": 0, "width": 301, "height": 300}));
    run(&mut s, "object.clippingMask.make", json!({"ids": [r["id"], id.0]}));
    apply(&mut s, id, json!({"preserve": "fill", "clip": true}));
    let (clip, ours, depth) = clips_around(&s, id);
    assert_eq!((corners(clip.bounds().unwrap()), ours, depth), ([1_000_050.0, 0.0, 1_000_350.0, 300.0], true, 2), "a clip of its own inside");
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

/// An XMP sidecar whose main subject (an IPTC image region, relative units) is `[left, top, right, bottom]`.
fn subject_xmp(r: [f64; 4]) -> String {
    format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:Iptc4xmpExt="http://iptc.org/std/Iptc4xmpExt/2008-02-29/"><Iptc4xmpExt:ImageRegion><rdf:Bag><rdf:li rdf:parseType="Resource"><Iptc4xmpExt:RegionBoundary rdf:parseType="Resource"><Iptc4xmpExt:rbShape>rectangle</Iptc4xmpExt:rbShape><Iptc4xmpExt:rbUnit>relative</Iptc4xmpExt:rbUnit><Iptc4xmpExt:rbX>{}</Iptc4xmpExt:rbX><Iptc4xmpExt:rbY>{}</Iptc4xmpExt:rbY><Iptc4xmpExt:rbW>{}</Iptc4xmpExt:rbW><Iptc4xmpExt:rbH>{}</Iptc4xmpExt:rbH></Iptc4xmpExt:RegionBoundary><Iptc4xmpExt:rRole><rdf:Bag><rdf:li rdf:parseType="Resource"><Iptc4xmpExt:Identifier><rdf:Bag><rdf:li>http://cv.iptc.org/newscodes/imageregionrole/mainSubjectArea</rdf:li></rdf:Bag></Iptc4xmpExt:Identifier></rdf:li></rdf:Bag></Iptc4xmpExt:rRole></rdf:li></rdf:Bag></Iptc4xmpExt:ImageRegion></rdf:Description></rdf:RDF></x:xmpmeta>"#,
        r[0],
        r[1],
        r[2] - r[0],
        r[3] - r[1]
    )
}

/// A 600×300 px file whose sidecar names its subject `subject`, placed linked at the centre of the
/// artboard (-100, 0, 500, 300) → session, id, its folder.
fn with_subject(name: &str, subject: [f64; 4]) -> (Session, NodeId, Folder) {
    let dir = Folder::new(name);
    write(&dir.file("photo.png"), &png(600, 300, RED));
    write(&dir.file("photo.xmp"), subject_xmp(subject).as_bytes());
    let mut s = session();
    let id = place(&mut s, &dir.file("photo.png"));
    (s, id, dir)
}

/// A linked file's main subject is read from its XMP sidecar (`<name>.xmp`, as tools that won't
/// rewrite a file keep its metadata; IPTC Image Region, role mainSubjectArea) and kept with the
/// link: Link Info shows it, and a document saved and opened again keeps it.
#[test]
fn link_info_shows_the_subject_its_sidecar_names() {
    let (mut s, id, dir) = with_subject("subject-info", [0.25, 0.2, 0.5, 0.6]);
    let i = run(&mut s, "links.info", json!({"id": id.0}));
    assert_eq!(i["subject"], json!({"left": 0.25, "top": 0.2, "right": 0.5, "bottom": 0.6}), "{i}");
    let path = dir.file("doc.vectorcraft");
    save(&mut s, &path);
    // The file gone: the link keeps the subject it read.
    std::fs::rename(dir.file("photo.png"), dir.file("away.png")).unwrap();
    let mut t = session();
    open(&mut t, &path);
    assert_eq!(run(&mut t, "links.info", json!({"id": id.0}))["subject"]["right"], json!(0.5), "kept with the link");
    // Found again with its sidecar gone: read again, it names none.
    std::fs::rename(dir.file("away.png"), dir.file("photo.png")).unwrap();
    std::fs::remove_file(dir.file("photo.xmp")).unwrap();
    write(&dir.file("photo.png"), &png(600, 300, BLUE));
    run(&mut t, "links.update", json!({}));
    assert_eq!(run(&mut t, "links.info", json!({"id": id.0})).get("subject"), None, "read again");
    // A file with no sidecar and no XMP of its own has none.
    let plain = dir.file("plain.png");
    write(&plain, &png(10, 10, BLUE));
    let other = place(&mut t, &plain);
    assert_eq!(run(&mut t, "links.info", json!({"id": other.0})).get("subject"), None);
}

/// Preserve: Subject on a relink: the new file is scaled uniformly so its subject fits inside the
/// old file's subject, at the alignment point.
#[test]
fn a_relinked_file_puts_its_subject_where_the_old_one_was() {
    let (mut s, id, dir) = with_subject("subject-relink", [0.25, 0.2, 0.5, 0.6]);
    // The old subject: 50, 60, 200, 180 (150 × 120 pt). The new file's: its left half, 150 × 100 px.
    write(&dir.file("blue.png"), &png(300, 100, BLUE));
    write(&dir.file("blue.xmp"), subject_xmp([0.0, 0.0, 0.5, 1.0]).as_bytes());
    let r = run(&mut s, "links.placementOptions", json!({"ids": [id.0], "preserve": "subject"}));
    assert_eq!(
        (corners(bounds(&s, id)), r["applied"].clone()),
        ([-100.0, 0.0, 500.0, 300.0], json!([])),
        "on OK with nothing clipping it, its subject stays"
    );
    run(&mut s, "links.placementOptions", json!({"ids": [id.0], "preserve": "subject", "target": {"x": 50, "y": 60, "width": 150, "height": 120}}));
    assert_eq!(corners(bounds(&s, id)), [-100.0, 0.0, 500.0, 300.0], "its own subject's place: nothing moves");
    run(&mut s, "links.relink", json!({"ids": [id.0], "path": dir.file("blue.png")}));
    assert_eq!(corners(bounds(&s, id)), [50.0, 70.0, 350.0, 170.0], "the new subject (150 × 100) inside the old one's place, centred");
    let i = run(&mut s, "links.info", json!({"id": id.0}));
    assert_eq!((i["subject"]["right"].clone(), i["placement"]["preserve"].clone()), (json!(0.5), json!("subject")));
}

/// Placement Options › Subject with a target: each subject fits inside it, at the alignment point,
/// as Content-Aware Fit fits an image to its frame; target and key go with subject only.
#[test]
fn subject_placement_fits_the_subject_to_a_target() {
    let (mut s, id, _d) = with_subject("subject-target", [0.25, 0.2, 0.5, 0.6]);
    let (r, b) = apply(&mut s, id, json!({"preserve": "subject", "target": {"x": 0, "y": 0, "width": 300, "height": 120}}));
    // The subject (150 × 120) at scale 1, centred across the target: 75, 0, 225, 120.
    assert_eq!((b, r["applied"].clone()), ([-75.0, -60.0, 525.0, 240.0], json!([id.0])));
    let (_, b) = apply(&mut s, id, json!({"preserve": "subject", "align": "left", "target": {"x": 0, "y": 0, "width": 600, "height": 60}}));
    assert_eq!(b, [-75.0, -30.0, 225.0, 120.0], "half size, at the target's left");
    for (p, why) in [
        (json!({"preserve": "fit", "target": {"x": 0, "y": 0, "width": 1, "height": 1}}), "with preserve: subject"),
        (json!({"preserve": "subject", "target": {"x": 0, "y": 0, "width": 0, "height": 1}}), "more than 0"),
        (json!({"preserve": "subject", "target": {"x": 0}}), "finite numbers"),
        (json!({"preserve": "subject", "key": 999_999}), "an object with bounds"),
    ] {
        let mut p = p;
        p["ids"] = json!([id.0]);
        let e = s.execute("links.placementOptions", &p).unwrap_err().to_string();
        assert!(e.contains(why), "{p}: {e}");
    }
}

/// Like this one: with a key object, each image's subject takes the key's subject's place
/// relative to the frame clipping it, at the same relative size; the key itself stays. The key
/// is the selection's key object (as the Align panel takes it) when none is named.
#[test]
fn subject_placement_places_each_subject_like_the_key_s() {
    let (mut s, a, dir) = with_subject("subject-key", [0.25, 0.2, 0.5, 0.6]);
    write(&dir.file("other.png"), &png(600, 300, BLUE));
    write(&dir.file("other.xmp"), subject_xmp([0.5, 0.5, 1.0, 1.0]).as_bytes());
    let b = place(&mut s, &dir.file("other.png"));
    assert_eq!(corners(bounds(&s, b)), [-100.0, 0.0, 500.0, 300.0]);
    let r = run(&mut s, "links.placementOptions", json!({"ids": [a.0, b.0], "preserve": "subject", "key": a.0}));
    assert_eq!(r["applied"], json!([b.0]), "{r}");
    // B's subject (300 × 150 px) into A's subject's place (50, 60, 200, 180) at half size, centred.
    assert_eq!(corners(bounds(&s, b)), [-100.0, 7.5, 200.0, 157.5]);
    assert_eq!(corners(bounds(&s, a)), [-100.0, 0.0, 500.0, 300.0]);
    // The selection's key object stands in for key.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    s.doc_mut().unwrap().selection.key = Some(a);
    run(&mut s, "links.placementOptions", json!({"preserve": "subject"}));
    assert_eq!(corners(bounds(&s, b)), [-100.0, 7.5, 200.0, 157.5]);
}

/// The angle (degrees) image `id`'s across axis is turned by.
fn turned(s: &Session, id: NodeId) -> f64 {
    let NodeKind::Image(im) = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!("not an image") };
    let [a, b, ..] = im.xf.as_coeffs();
    b.atan2(a).to_degrees()
}

/// OK with Subject in a frame the user turned keeps the art turned with the frame: the subject is
/// fitted in the frame's own axes.
#[test]
fn subject_placement_in_a_turned_frame_keeps_the_turn() {
    let (mut s, id, _d) = with_subject("subject-turned", [0.25, 0.2, 0.5, 0.6]);
    let frame = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 50, "width": 400, "height": 200}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [frame, id.0]}));
    run(&mut s, "object.rotate", json!({"angle": 30}));
    let before = turned(&s, id);
    assert!((before.abs() - 30.0).abs() < 1e-6, "{before}");
    run(&mut s, "object.clippingMask.make", json!({"ids": [frame, id.0]}));
    run(&mut s, "links.placementOptions", json!({"ids": [id.0], "preserve": "subject"}));
    assert!((turned(&s, id) - before).abs() < 1e-6, "turned with its frame: {} → {}", before, turned(&s, id));
}

/// Subject with a key object or a target keeps a turned image turned: like the key's, its subject
/// is placed in the image's own turned frame; into a target, a quarter-turned image stays
/// quarter-turned (the target's sides nearest its own, as a clip group's rectangle is taken).
#[test]
fn subject_placement_by_a_key_or_a_target_keeps_the_turn() {
    let (mut s, a, dir) = with_subject("subject-key-turned", [0.25, 0.2, 0.5, 0.6]);
    write(&dir.file("other.png"), &png(600, 300, BLUE));
    write(&dir.file("other.xmp"), subject_xmp([0.5, 0.5, 1.0, 1.0]).as_bytes());
    let b = place(&mut s, &dir.file("other.png"));
    run(&mut s, "select.set", json!({"ids": [b.0]}));
    run(&mut s, "object.rotate", json!({"angle": 30}));
    let before = turned(&s, b);
    run(&mut s, "links.placementOptions", json!({"ids": [a.0, b.0], "preserve": "subject", "key": a.0}));
    assert!((turned(&s, b) - before).abs() < 1e-6, "like the key's, turned: {before} → {}", turned(&s, b));
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    run(&mut s, "object.rotate", json!({"angle": 90}));
    let quarter = turned(&s, a);
    run(&mut s, "links.placementOptions", json!({"ids": [a.0], "preserve": "subject", "target": {"x": 0, "y": 0, "width": 300, "height": 150}}));
    assert!((turned(&s, a) - quarter).abs() < 1e-6, "into the target, quarter-turned: {quarter} → {}", turned(&s, a));
}

/// OK with Subject and nothing clipping the art: its subject stays where it is, so OK again moves
/// nothing and makes no undo step.
#[test]
fn subject_placement_with_nothing_clipping_moves_nothing() {
    let (mut s, id, _d) = with_subject("subject-unclipped", [0.25, 0.2, 0.5, 0.6]);
    for _ in 0..2 {
        run(&mut s, "links.placementOptions", json!({"ids": [id.0], "preserve": "subject"}));
        assert_eq!(corners(bounds(&s, id)), [-100.0, 0.0, 500.0, 300.0]);
    }
}

/// Subject with a target and Clip to Bounding Box: the art is clipped to the target, the frame
/// its subject is fitted to, not to its old bounds it has left.
#[test]
fn subject_placement_with_a_target_clips_to_the_target() {
    let (mut s, id, _d) = with_subject("subject-target-clip", [0.25, 0.2, 0.5, 0.6]);
    let (r, _) = apply(&mut s, id, json!({"preserve": "subject", "clip": true, "target": {"x": 700, "y": 400, "width": 150, "height": 120}}));
    assert_eq!(r["applied"], json!([id.0]));
    let (path, ours, depth) = clips_around(&s, id);
    assert_eq!((corners(path.bounds().unwrap()), ours, depth), ([700.0, 400.0, 850.0, 520.0], true, 1));
}

/// A subject a document holds out of the art (or flat) is all of it: no placement out of reach.
#[test]
fn a_subject_out_of_the_art_is_all_of_it() {
    let (mut s, id, _d) = with_subject("subject-out", [0.25, 0.2, 0.5, 0.6]);
    s.edit("t", |d, _| {
        if let Some(NodeKind::Image(im)) = d.node_mut(id).map(|n| &mut n.kind)
            && let Some(l) = im.link.as_mut()
        {
            l.subject = Some(vectorcraft_doc::links::ImageRegion { left: -1e308, top: 0.0, right: 1e308, bottom: 1.0 });
        }
        Ok(())
    })
    .unwrap();
    std::fs::remove_file(_d.file("photo.xmp")).unwrap();
    apply(&mut s, id, json!({"preserve": "subject", "target": {"x": 0, "y": 0, "width": 300, "height": 150}}));
    // The whole 600 × 300 image in the target.
    assert_eq!(corners(bounds(&s, id)), [0.0, 0.0, 300.0, 150.0]);
}

/// A linked image inside a symbol takes Preserve: Subject on a relink too: its new subject in its
/// old subject's place, as one in the layers does.
#[test]
fn a_relinked_image_in_a_symbol_puts_its_subject_where_the_old_one_was() {
    let (mut s, id, dir) = with_subject("subject-symbol", [0.25, 0.2, 0.5, 0.6]);
    write(&dir.file("blue.png"), &png(300, 100, BLUE));
    write(&dir.file("blue.xmp"), subject_xmp([0.0, 0.0, 0.5, 1.0]).as_bytes());
    run(&mut s, "links.placementOptions", json!({"ids": [id.0], "preserve": "subject"}));
    run(&mut s, "symbol.new", json!({"name": "Pic", "ids": [id.0]}));
    let nested = |s: &Session| {
        let doc = &s.doc().unwrap().doc;
        let sym = doc.symbols.iter().find(|y| y.name == "Pic").unwrap();
        let mut found = None;
        sym.art.walk(&mut |n: &vectorcraft_doc::Node| {
            if matches!(n.kind, NodeKind::Image(_)) {
                found = Some((n.id, n.geometric_bounds().unwrap()));
            }
        });
        found.unwrap()
    };
    let (inner, b0) = nested(&s);
    run(&mut s, "links.relink", json!({"ids": [inner.0], "path": dir.file("blue.png")}));
    let (_, b) = nested(&s);
    // As in the layers, in the symbol's art: the old subject (its 0.25, 0.2, 0.5, 0.6), the new
    // one (150 × 100 px of 300 × 100) inscribed in it, centred.
    let (w, h) = (b0.width(), b0.height());
    let (sx0, sy0, sw, sh) = (b0.x0 + 0.25 * w, b0.y0 + 0.2 * h, 0.25 * w, 0.4 * h);
    let k = (sw / 150.0).min(sh / 100.0);
    let (x0, y0) = (sx0 + (sw - 150.0 * k) / 2.0, sy0 + (sh - 100.0 * k) / 2.0);
    assert_eq!(corners(b), corners(vectorcraft_geom::Rect::new(x0, y0, x0 + 300.0 * k, y0 + 100.0 * k)), "from {b0:?}");
}

/// A single-file DCS file whose composite is drawn through a clipping path, the left half of its
/// 20 × 10 pt page.
fn dcs_file() -> Vec<u8> {
    let plate = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 20 10\n%%EndComments\n20 10 scale\n2 2 8 [2 0 0 -2 0 2] <0000ffff> image\nshowpage\n%%Trailer\n%%EOF\n";
    let header = |at: usize| {
        format!(
            "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 20 10\n%%PlateFile: (Cyan) EPS #{at:010} {:010}\n%%EndComments\ngsave newpath 0 0 moveto 10 0 lineto 10 10 lineto 0 10 lineto closepath clip newpath\n20 10 scale 2 2 8 [2 0 0 -2 0 2] <808080808080808080808080> false 3 colorimage grestore showpage\n%%EOF\n",
            plate.len()
        )
    };
    let at = header(0).len();
    [header(at).into_bytes(), plate.as_bytes().to_vec()].concat()
}

/// The image of placed object `id` (itself, or inside the DCS file's clip group).
fn placed_image(s: &Session, placed: &Value) -> NodeId {
    let n = s.doc().unwrap().doc.node(NodeId(placed.as_u64().unwrap())).unwrap().clone();
    let mut found = None;
    n.walk(&mut |c: &vectorcraft_doc::Node| {
        if found.is_none() && matches!(c.kind, NodeKind::Image(_)) {
            found = Some(c.id);
        }
    });
    found.expect("an image")
}

/// Relinked with Clip to Bounding Box from a DCS file to a file that overflows its bounds: the DCS
/// file's clip group takes Clip to Bounding Box's rectangle (the old clipping path no longer
/// applies), not a second clip group inside it.
#[test]
fn relinked_from_a_dcs_file_with_clip_the_files_clip_group_takes_the_rectangle() {
    let dir = Folder::new("dcs-clip-relink");
    write(&dir.file("pic.eps"), &dcs_file());
    let mut s = session();
    let placed = run(&mut s, "file.place", json!({"path": dir.file("pic.eps"), "at": [200, 150]}))["ids"][0].clone();
    let id = placed_image(&s, &placed);
    let old = bounds(&s, id);
    run(&mut s, "links.placementOptions", json!({"ids": [id.0], "preserve": "fill", "clip": true}));
    write(&dir.file("wide.png"), &png(600, 100, BLUE));
    run(&mut s, "links.relink", json!({"ids": [id.0], "path": dir.file("wide.png")}));
    let (path, ours, depth) = clips_around(&s, id);
    assert_eq!((ours, depth), (true, 1), "one clip group, Clip to Bounding Box's");
    assert_eq!(path.bounds().map(corners), Some(corners(old)), "the image's old bounds");
}

/// A DCS file relinked inside Clip to Bounding Box's clip group gets its own clip group inside
/// it; Clip off still lets go of Clip to Bounding Box's, leaving the file's.
#[test]
fn clip_off_lets_go_of_clip_to_bounding_boxs_group_around_a_dcs_files() {
    let dir = Folder::new("dcs-nest-off");
    write(&dir.file("photo.png"), &png(600, 300, RED));
    write(&dir.file("wide.png"), &png(900, 300, RED));
    write(&dir.file("pic.eps"), &dcs_file());
    let mut s = session();
    let id = place(&mut s, &dir.file("photo.png"));
    run(&mut s, "links.placementOptions", json!({"ids": [id.0], "preserve": "fill", "clip": true}));
    run(&mut s, "links.relink", json!({"ids": [id.0], "path": dir.file("wide.png")}));
    assert_eq!((clips_around(&s, id).1, clips_around(&s, id).2), (true, 1));
    run(&mut s, "links.relink", json!({"ids": [id.0], "path": dir.file("pic.eps")}));
    assert_eq!((clips_around(&s, id).1, clips_around(&s, id).2), (false, 2), "the file's clip group inside Clip to Bounding Box's");
    run(&mut s, "links.placementOptions", json!({"ids": [id.0], "clip": false}));
    assert_eq!((clips_around(&s, id).1, clips_around(&s, id).2), (false, 1), "only the file's is left");
}

/// A DCS file's clipping path taking Clip to Bounding Box's group, or going from it, leaves the
/// group no longer Clip to Bounding Box's.
#[test]
fn a_dcs_files_clip_leaves_no_clip_to_bounding_box_mark() {
    let dir = Folder::new("dcs-unmark");
    write(&dir.file("pic.eps"), &dcs_file());
    let mut s = session();
    let placed = run(&mut s, "file.place", json!({"path": dir.file("pic.eps"), "at": [200, 150]}))["ids"][0].clone();
    let id = placed_image(&s, &placed);
    let group = clip_group_of(&s, id);
    s.edit("t", |d, _| {
        if let Some(g) = d.node_mut(group) {
            g.extras_mut().placement_clip = true;
        }
        Ok(())
    })
    .unwrap();
    write(&dir.file("plain.png"), &png(20, 10, BLUE));
    run(&mut s, "links.relink", json!({"ids": [id.0], "path": dir.file("plain.png")}));
    let doc = &s.doc().unwrap().doc;
    let g = doc.node(doc.parent_of(id).unwrap()).unwrap();
    assert!(matches!(g.kind, NodeKind::Group { clip: false, .. }) && !g.extras().placement_clip, "{g:?}");
}

/// The subject a sidecar names in a photo's stored pixels is turned with the photo when its EXIF
/// orientation turns it upright (IPTC: a region's orientation is not applied).
#[test]
fn a_turned_photos_subject_turns_with_it() {
    let dir = Folder::new("subject-turned-photo");
    // 16 × 8 stored, Orientation 6 (a quarter clockwise): shown 8 × 16.
    let mut jpg = vec![];
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(16, 8, image::Rgb([200, 0, 0])))
        .write_to(&mut std::io::Cursor::new(&mut jpg), image::ImageFormat::Jpeg)
        .unwrap();
    let exif: Vec<u8> =
        [b"Exif\0\0".as_slice(), b"II", &[0x2a, 0, 8, 0, 0, 0], &[1, 0], &[0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0], &[0, 0, 0, 0]].concat();
    let mut turned = jpg[..2].to_vec();
    turned.extend([0xff, 0xe1]);
    turned.extend(u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
    turned.extend(&exif);
    turned.extend(&jpg[2..]);
    write(&dir.file("photo.jpg"), &turned);
    // The left half of the stored pixels: the top half once turned.
    write(&dir.file("photo.xmp"), subject_xmp([0.0, 0.0, 0.5, 1.0]).as_bytes());
    let mut s = session();
    let id = place(&mut s, &dir.file("photo.jpg"));
    let i = run(&mut s, "links.info", json!({"id": id.0}));
    assert_eq!(i["subject"], json!({"left": 0.0, "top": 0.0, "right": 1.0, "bottom": 0.5}), "{i}");
}

/// A changed sidecar changes the link's subject: the link shows as modified until it is updated.
#[test]
fn a_changed_sidecar_modifies_the_link() {
    let (mut s, id, dir) = with_subject("subject-sidecar-changed", [0.25, 0.2, 0.5, 0.6]);
    let status = |s: &mut Session| run(s, "links.list", json!({}))["links"][0]["status"].clone();
    assert_eq!(status(&mut s), "ok");
    let xmp = dir.file("photo.xmp");
    write(&xmp, subject_xmp([0.5, 0.2, 0.75, 0.6]).as_bytes());
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(10);
    std::fs::File::options().write(true).open(&xmp).unwrap().set_modified(later).unwrap();
    assert_eq!(status(&mut s), "modified");
    run(&mut s, "links.update", json!({}));
    assert_eq!(status(&mut s), "ok");
    assert_eq!(run(&mut s, "links.info", json!({"id": id.0}))["subject"]["left"], json!(0.5));
}

/// A sidecar with a new modification time but the same subject (a copy: a clone, a backup
/// restored, a sync) leaves the link as it is: only the subject it names counts.
#[test]
fn a_copied_sidecar_naming_the_same_subject_leaves_the_link_ok() {
    let (mut s, _, dir) = with_subject("subject-sidecar-copied", [0.25, 0.2, 0.5, 0.6]);
    let status = |s: &mut Session| run(s, "links.list", json!({}))["links"][0]["status"].clone();
    assert_eq!(status(&mut s), "ok");
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(10);
    std::fs::File::options().write(true).open(dir.file("photo.xmp")).unwrap().set_modified(later).unwrap();
    assert_eq!(status(&mut s), "ok");
}

/// Relink with Copy copies the file's sidecar beside the copy, so the copy keeps its subject.
#[test]
fn relink_with_copy_copies_the_sidecar() {
    let (mut s, id, dir) = with_subject("subject-copy", [0.25, 0.2, 0.5, 0.6]);
    let docs = dir.0.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    save(&mut s, &docs.join("doc.vectorcraft").to_string_lossy());
    write(&dir.file("other.png"), &png(600, 300, BLUE));
    write(&dir.file("other.xmp"), subject_xmp([0.0, 0.0, 0.5, 1.0]).as_bytes());
    run(&mut s, "links.relink", json!({"ids": [id.0], "path": dir.file("other.png"), "copy": true}));
    assert!(docs.join("other.xmp").exists(), "the sidecar beside the copy");
    assert_eq!(run(&mut s, "links.info", json!({"id": id.0}))["subject"]["right"], json!(0.5));
}
