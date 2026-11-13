//! Link sources (`links.addSources`): a document opened from bytes, whose linked file can't be
//! read, shows the preview until the image is drawn larger than it, then the file is asked for at
//! its placement's size, drawn once supplied, exported, and never saved or undone; the Links
//! panel shows a sourced link as found, and missing again once its source fails.

use serde_json::json;

use super::links::{Folder, centre_colour, image, near, place, png, session, write};
use crate::*;

/// Colours and sizes no other test links, so the shared pixels are this test's alone.
const FILE: [u8; 3] = [10, 200, 90];
const SUPPLIED: [u8; 3] = [200, 120, 10];

/// Tests that keep supplied pixels or set the process's linked budget hold this, so a small budget
/// one sets drops none of another's.
static BUDGET: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn hold() -> std::sync::MutexGuard<'static, ()> {
    let held = BUDGET.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // As the web shell: what the engine asks for is fetched.
    link_sources::set_fetcher(true);
    held
}

#[test]
fn a_sourced_link_is_asked_for_when_drawn_large_supplied_drawn_and_exported() {
    let _held = hold();
    let dir = Folder::new("sources");
    let pic = dir.file("sourced.png");
    write(&pic, &png(640, 320, FILE));
    let mut s = session();
    let id = place(&mut s, &pic);
    let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    std::fs::remove_file(&pic).unwrap();
    s.execute("document.open", &json!({"name": "sourced.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(&saved)})).unwrap();
    let im = image(&s, id);
    let key = im.key.clone();
    let path = im.link.as_ref().unwrap().path.clone();
    assert!(s.doc().unwrap().doc.images[&key].is_proxy(), "the web shows the preview");

    let status = |s: &mut Session| s.execute("links.list", &json!({})).unwrap()["links"][0]["status"].clone();
    assert_eq!(status(&mut s), "missing", "no file and no source");
    let revision = s.doc().unwrap().revision;
    let added = s.execute("links.addSources", &json!({"sources": [{"path": path, "url": "pictures/sourced.png?side={side}"}]})).unwrap();
    assert_eq!(s.doc().unwrap().revision, revision, "sources are no edit");
    assert_eq!(status(&mut s), "ok", "the host supplies it: found, as the Links panel shows");
    assert_eq!(s.execute("links.check", &json!({})).unwrap()["missing"], 0);
    assert_eq!((added["added"].clone(), added["resolution"].clone()), (json!(1), json!(300.0)), "{added}");
    assert!(link_sources::requests(&s).iter().all(|r| r.path != path), "nothing drawn yet: nothing asked for");

    // Drawn at 640 device pixels, past the 256 px preview: asked for at its placement's size (1 pt
    // a pixel at 300 ppi needs more than the file has, so the file whole).
    assert!(near(centre_colour(&s.doc().unwrap().doc), FILE));
    let asked: Vec<_> = link_sources::requests(&s).into_iter().filter(|r| r.path == path).collect();
    assert_eq!(asked, [link_sources::SourceRequest { path: path.clone(), url: "pictures/sourced.png?side=640".into(), width: 640, height: 320 }]);
    centre_colour(&s.doc().unwrap().doc);
    assert!(link_sources::requests(&s).iter().all(|r| r.path != path), "asked for once while it is on its way");

    let revision = s.doc().unwrap().revision;
    let generation = vectorcraft_render::link_pixels::generation();
    let data = vectorcraft_format::base64_encode(&png(640, 320, SUPPLIED));
    let r = s.execute("links.supplySource", &json!({"path": path, "dataBase64": data})).unwrap();
    assert_eq!(r, json!({"images": 1, "pixelWidth": 640, "pixelHeight": 320}));
    // Not an edit, so no revision: a screenshot or a drag under way goes on. The canvas draws
    // again by the pixels' generation.
    assert_eq!(s.doc().unwrap().revision, revision, "no revision for what no document holds");
    assert!(vectorcraft_render::link_pixels::generation() > generation, "the canvas draws again");
    assert!(near(centre_colour(&s.doc().unwrap().doc), SUPPLIED), "drawn over the preview");
    assert!(link_sources::requests(&s).iter().all(|r| r.path != path), "supplied: not asked for again");
    assert_eq!(s.execute("links.loadSources", &json!({})).unwrap(), json!({"loaded": 1, "loading": 0, "failed": 0, "overBudget": 0}));

    // Not an edit: the document still holds the preview, nothing to undo, and a save writes it.
    let st = s.doc().unwrap();
    assert!(st.doc.images[&key].is_proxy() && !st.is_dirty(), "the document is unchanged");
    // Output uses the supplied pixels, the preview kept as the preview.
    let out = link_sources::for_output(&st.doc);
    assert_eq!(out.images[&key].bytes.as_ref(), &png(640, 320, SUPPLIED));
    assert!(!out.images[&key].is_proxy() && out.images[&key].proxy.is_some());
    let pdf = s.execute("document.export", &json!({"format": "pdf"})).unwrap();
    assert!(pdf["dataBase64"].as_str().is_some_and(|d| !d.is_empty()));

    // Dropped (past its budget), it is drawn from the preview. Drawn, it is not asked for again
    // (each fetch could drop another); `links.loadSources` asks for it.
    vectorcraft_render::link_pixels::forget(&key);
    assert!(near(centre_colour(&s.doc().unwrap().doc), FILE));
    let again = [self::asked(&s, &path).len(), load(&mut s)["loading"].as_u64().unwrap_or(0) as usize, self::asked(&s, &path).len()];
    assert_eq!(again, [0, 1, 1], "drawn, dropped pixels are not asked for again; loadSources asks for them");
    let failed = s.execute("links.supplySource", &json!({"path": path, "dataBase64": vectorcraft_format::base64_encode(b"no picture")}));
    assert!(failed.is_err(), "bytes that are no picture are refused");
    centre_colour(&s.doc().unwrap().doc);
    assert!(link_sources::requests(&s).iter().all(|r| r.path != path), "a failed source is not asked for again by itself");
    assert_eq!(s.execute("links.loadSources", &json!({})).unwrap(), json!({"loaded": 0, "loading": 0, "failed": 1, "overBudget": 0}));
    assert_eq!(status(&mut s), "missing", "a source whose file failed is missing again");

    for bad in
        [json!({}), json!({"sources": []}), json!({"sources": [{"path": "p"}]}), json!({"sources": [{"path": "p", "url": "u"}], "resolution": 0})]
    {
        assert!(s.execute("links.addSources", &bad).is_err(), "{bad}");
    }
}

#[test]
fn the_image_caches_budgets_are_told_and_set() {
    let _held = hold();
    let before = s_cache(&json!({}));
    let linked = before["linked"]["budget"].as_u64().unwrap();
    let set = s_cache(&json!({"linked": linked + (1 << 20)}));
    assert_eq!(set["linked"]["budget"], json!(linked + (1 << 20)));
    s_cache(&json!({"linked": linked}));
    assert!(Session::new().execute("view.imageCache", &json!({"decoded": 1024})).is_err(), "at least 16 MB");
}

fn s_cache(p: &serde_json::Value) -> serde_json::Value {
    Session::new().execute("view.imageCache", p).unwrap()
}

/// A source may carry the file's XMP sidecar: its main subject (an IPTC image region) is the
/// link's, before the one kept when the file was read, for Link Info and Placement Options ›
/// Preserve: Subject, in the web build where no `<name>.xmp` beside the file can be read. A new
/// sidecar alone changes no pixels.
#[test]
fn a_source_s_sidecar_names_the_link_s_subject() {
    let dir = Folder::new("sources-sidecar");
    let pic = dir.file("subject-sourced.png");
    write(&pic, &png(600, 300, FILE));
    let mut s = session();
    let id = place(&mut s, &pic);
    let path = image(&s, id).link.unwrap().path;
    assert_eq!(s.execute("links.info", &json!({"id": id.0})).unwrap().get("subject"), None);
    let xmp = |x: f64| {
        format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:Iptc4xmpExt="http://iptc.org/std/Iptc4xmpExt/2008-02-29/"><Iptc4xmpExt:ImageRegion><rdf:Bag><rdf:li rdf:parseType="Resource"><Iptc4xmpExt:RegionBoundary rdf:parseType="Resource"><Iptc4xmpExt:rbShape>rectangle</Iptc4xmpExt:rbShape><Iptc4xmpExt:rbUnit>relative</Iptc4xmpExt:rbUnit><Iptc4xmpExt:rbX>{x}</Iptc4xmpExt:rbX><Iptc4xmpExt:rbY>0.2</Iptc4xmpExt:rbY><Iptc4xmpExt:rbW>0.25</Iptc4xmpExt:rbW><Iptc4xmpExt:rbH>0.4</Iptc4xmpExt:rbH></Iptc4xmpExt:RegionBoundary><Iptc4xmpExt:rRole><rdf:Bag><rdf:li rdf:parseType="Resource"><Iptc4xmpExt:Identifier><rdf:Bag><rdf:li>http://cv.iptc.org/newscodes/imageregionrole/mainSubjectArea</rdf:li></rdf:Bag></Iptc4xmpExt:Identifier></rdf:li></rdf:Bag></Iptc4xmpExt:rRole></rdf:li></rdf:Bag></Iptc4xmpExt:ImageRegion></rdf:Description></rdf:RDF></x:xmpmeta>"#
        )
    };
    let add = |s: &mut Session, x: f64| {
        s.execute("links.addSources", &json!({"sources": [{"path": path, "url": "subject-sourced.png?side={side}", "sidecar": xmp(x)}]})).unwrap()
    };
    assert_eq!(add(&mut s, 0.25)["added"], 1);
    let subject = s.execute("links.info", &json!({"id": id.0})).unwrap()["subject"].clone();
    let sides = ["left", "top", "right", "bottom"].map(|k| (subject[k].as_f64().unwrap() * 1e9).round() / 1e9);
    assert_eq!(sides, [0.25, 0.2, 0.5, 0.6], "{subject}");
    // Its subject (50, 60, 200, 180 on the artboard) fits a target: 75, 0, 225, 120 at scale 1.
    s.execute("links.placementOptions", &json!({"ids": [id.0], "preserve": "subject", "target": {"x": 0, "y": 0, "width": 300, "height": 120}}))
        .unwrap();
    let b = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    assert_eq!([b.x0, b.y0, b.x1, b.y1].map(f64::round), [-75.0, -60.0, 525.0, 240.0]);
    // Moved by the host: the new sidecar's subject, the same source.
    assert_eq!(add(&mut s, 0.5)["added"], 0);
    assert_eq!(s.execute("links.info", &json!({"id": id.0})).unwrap()["subject"]["left"], json!(0.5));
    let too_big = "x".repeat(300 << 10);
    assert!(s.execute("links.addSources", &json!({"sources": [{"path": path, "url": "u", "sidecar": too_big}]})).is_err());
}

/// Colour adjustment effects recolour a linked image live: its preview, the pixels a source
/// supplied over it, and the supplied pixels for output; Edit Colors, an edit, leaves its file be.
#[test]
fn a_colour_adjustment_recolours_a_sourced_link_live_and_for_output() {
    let _held = hold();
    // Colours no other test links, so the shared pixels are this test's alone.
    const FILE: [u8; 3] = [30, 60, 220];
    const SUPPLIED: [u8; 3] = [220, 40, 40];
    let dir = Folder::new("sources-adjust");
    let pic = dir.file("adjusted.png");
    write(&pic, &png(600, 300, FILE));
    let mut s = session();
    let id = place(&mut s, &pic);
    let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    std::fs::remove_file(&pic).unwrap();
    s.execute("document.open", &json!({"name": "adjusted.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(&saved)})).unwrap();
    let im = image(&s, id);
    let (key, path) = (im.key.clone(), im.link.as_ref().unwrap().path.clone());
    s.execute("links.addSources", &json!({"sources": [{"path": path, "url": "pictures/adjusted.png?side={side}"}]})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();

    // Edit Colors would write the file's pixels into the document: a linked image is left alone.
    assert_eq!(s.execute("edit.colors.invert", &json!({})).unwrap()["changed"], json!(0));
    assert_eq!(image(&s, id).key, key);

    let effect = vectorcraft_doc::Effect { id: "adjust.hueSaturation".into(), params: json!({"hue": 120}), visible: true };
    let map = vectorcraft_render::effects::color_map(std::slice::from_ref(&effect)).unwrap();
    s.execute("effect.apply", &json!({"effect": effect.id, "params": effect.params})).unwrap();
    assert!(near(centre_colour(&s.doc().unwrap().doc), map.apply_rgb8(FILE)), "the preview, recoloured");
    let r = s.execute("links.supplySource", &json!({"path": path, "dataBase64": vectorcraft_format::base64_encode(&png(600, 300, SUPPLIED))}));
    assert_eq!(r.unwrap()["images"], json!(1));
    assert!(near(centre_colour(&s.doc().unwrap().doc), map.apply_rgb8(SUPPLIED)), "the supplied pixels, recoloured");

    // Output recolours the supplied pixels at their size, into a new embedded image: it no longer
    // shows its file, so an export writes its pixels and Update Links leaves it be.
    let st = s.doc().unwrap();
    let node = st.doc.node(id).unwrap().clone();
    let mut out = link_sources::for_output(&st.doc).into_owned();
    let NodeKind::Image(baked) = vectorcraft_render::effects::adjust_in_document(&mut out, &node, None).unwrap().kind else { panic!("an image") };
    assert!(baked.key != key && baked.link.is_none());
    let blob = &out.images[&baked.key];
    assert!(!blob.is_proxy());
    let rgba = image::load_from_memory(&blob.bytes).unwrap().to_rgba8();
    assert_eq!(rgba.dimensions(), (600, 300));
    let [r, g, b, _] = rgba.get_pixel(300, 150).0;
    assert!(near([r, g, b], map.apply_rgb8(SUPPLIED)));
    // Without supplied pixels the preview is never baked in its file's place.
    let mut doc = (*st.doc).clone();
    let kept = vectorcraft_render::effects::adjust_in_document(&mut doc, &node, None);
    assert!(kept.is_none_or(|n| matches!(&n.kind, NodeKind::Image(im) if im.key == key)));
}

/// A `w`×`h` PNG of noise from `seed`: its bytes barely compress, so its size is known roughly.
fn noise_png(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    let img = image::RgbImage::from_fn(w, h, |_, _| {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let [r, g, b, _] = x.to_le_bytes();
        image::Rgb([r, g, b])
    });
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// Supplied pixels past the linked budget: a file alone larger than it is refused (never kept
/// and dropped at once, which asked for it again forever), and files that together pass it leave
/// `links.loadSources` reporting them over budget rather than loading them forever.
#[test]
fn supplied_pixels_past_the_linked_budget_end_loading() {
    let _held = hold();
    let dir = Folder::new("sources-budget");
    let files: Vec<(String, Vec<u8>)> = (0..3).map(|i| (dir.file(&format!("budget-{i}.png")), noise_png(320, 320, 7700 + i))).collect();
    let mut s = session();
    for (pic, bytes) in &files {
        write(pic, bytes);
        place(&mut s, pic);
    }
    let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    for (pic, _) in &files {
        std::fs::remove_file(pic).unwrap();
    }
    s.execute("document.open", &json!({"name": "budget.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(&saved)})).unwrap();
    let mut paths = vec![];
    s.doc().unwrap().doc.visit_images(|_, im| paths.push(im.link.as_ref().unwrap().path.clone()));
    let sources: Vec<_> = paths.iter().map(|p| json!({"path": p, "url": "budget?side={side}"})).collect();
    s.execute("links.addSources", &json!({"sources": sources})).unwrap();
    let size = files.iter().map(|(_, b)| b.len()).max().unwrap();
    let old = vectorcraft_render::link_pixels::budget();
    let supply = |s: &mut Session, path: &str| {
        let name = path.rsplit(['/', '\\']).next().unwrap();
        let bytes = &files.iter().find(|(p, _)| p.ends_with(name)).unwrap().1;
        s.execute("links.supplySource", &json!({"path": path, "dataBase64": vectorcraft_format::base64_encode(bytes)}))
    };
    let load = |s: &mut Session| s.execute("links.loadSources", &json!({})).unwrap();

    // Two of the three fit: the third supplied drops the first.
    vectorcraft_render::link_pixels::set_budget(size * 5 / 2);
    assert_eq!(load(&mut s)["loading"], 3);
    for p in &paths {
        supply(&mut s, p).unwrap();
    }
    let mut asks = 0;
    loop {
        let l = load(&mut s);
        if l["loading"] == 0 {
            assert_eq!((l["loaded"].clone(), l["failed"].clone(), l["overBudget"].clone()), (json!(2), json!(1), json!(1)), "{l}");
            break;
        }
        asks += 1;
        assert!(asks < 4, "loadSources asks for what the budget drops again and again: {l}");
        for r in link_sources::requests(&s) {
            supply(&mut s, &r.path).unwrap();
        }
    }

    // A budget smaller than one file: it is refused, kept for no image, and not asked for again.
    vectorcraft_render::link_pixels::set_budget(size / 2);
    let refused = supply(&mut s, &paths[0]);
    assert!(refused.is_err(), "larger than the budget: {refused:?}");
    let l = load(&mut s);
    assert_eq!((l["loading"].clone(), l["failed"].clone(), l["overBudget"].clone()), (json!(0), json!(3), json!(3)), "{l}");
    // Drawn magnified past their previews.
    vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, vectorcraft_geom::Rect::new(0.0, 0.0, 400.0, 300.0), 4.0, true);
    let again: Vec<_> = link_sources::requests(&s).into_iter().filter(|r| paths.contains(&r.path)).collect();
    assert!(again.is_empty(), "drawn, a file past the budget is not asked for again: {again:?}");
    vectorcraft_render::link_pixels::set_budget(old);
}

/// A file of `size` pixels in `rgb`, placed linked at `scale` (%), the document saved and opened
/// again from its bytes with the file gone (as the web opens one) → the session, the image, its key
/// and the path its link names.
fn opened_without_its_file(name: &str, size: (u32, u32), rgb: [u8; 3], scale: f64) -> (Session, NodeId, String, String) {
    let dir = Folder::new(name);
    let pic = dir.file(&format!("{name}.png"));
    write(&pic, &png(size.0, size.1, rgb));
    let mut s = session();
    let id = place(&mut s, &pic);
    if scale != 100.0 {
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("object.scale", &json!({"sx": scale, "sy": scale})).unwrap();
    }
    let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    std::fs::remove_file(&pic).unwrap();
    s.execute("document.open", &json!({"name": format!("{name}.vectorcraft"), "dataBase64": vectorcraft_format::base64_encode(&saved)})).unwrap();
    let im = image(&s, id);
    (s, id, im.key.clone(), im.link.unwrap().path)
}

fn add_source(s: &mut Session, path: &str, url: &str) {
    s.execute("links.addSources", &json!({"sources": [{"path": path, "url": url}]})).unwrap();
}

fn supply(s: &mut Session, path: &str, bytes: &[u8]) -> Result<serde_json::Value> {
    s.execute("links.supplySource", &json!({"path": path, "dataBase64": vectorcraft_format::base64_encode(bytes)}))
}

fn load(s: &mut Session) -> serde_json::Value {
    s.execute("links.loadSources", &json!({})).unwrap()
}

/// The requests for `path` now.
fn asked(s: &Session, path: &str) -> Vec<link_sources::SourceRequest> {
    link_sources::requests(s).into_iter().filter(|r| r.path == path).collect()
}

/// A source given a new URL while its file is on its way is asked for again from the new one; what
/// the old one brings then is not credited to it.
#[test]
fn a_new_url_asks_again_and_drops_what_the_old_one_brings() {
    let _held = hold();
    let (mut s, _, key, path) = opened_without_its_file("source-moved", (640, 320), [11, 22, 33], 100.0);
    add_source(&mut s, &path, "old/{side}");
    centre_colour(&s.doc().unwrap().doc);
    let old = asked(&s, &path);
    assert_eq!(old.iter().map(|r| r.url.as_str()).collect::<Vec<_>>(), ["old/640"]);
    add_source(&mut s, &path, "new/{side}");
    centre_colour(&s.doc().unwrap().doc);
    let new = asked(&s, &path);
    assert_eq!(new.iter().map(|r| r.url.as_str()).collect::<Vec<_>>(), ["new/640"], "asked again from the new URL");
    let stale = link_sources::supply_fetched(&mut s, &old[0], png(640, 320, [1, 2, 3]));
    assert!(stale.is_err() && vectorcraft_render::link_pixels::peek(&key).is_none(), "the old URL's file is dropped: {stale:?}");
    assert!(!link_sources::fetch_failed(&old[0], "gone"), "nor does its failure count");
    assert_eq!(load(&mut s)["failed"], 0);
    link_sources::supply_fetched(&mut s, &new[0], png(640, 320, [1, 2, 3])).unwrap();
    assert!(vectorcraft_render::link_pixels::peek(&key).is_some());
    vectorcraft_render::link_pixels::forget(&key);
}

/// A file supplied smaller than a later request asks for answers for its own width: the larger
/// one is still to come.
#[test]
fn a_smaller_file_answers_for_its_own_width() {
    let _held = hold();
    let (mut s, id, key, path) = opened_without_its_file("source-grown", (1280, 640), [12, 23, 34], 12.5);
    add_source(&mut s, &path, "grown/{side}");
    load(&mut s);
    let small = asked(&s, &path);
    assert!(small.len() == 1 && small[0].width < 1280, "{small:?}");
    // Grown back to 100% while that is on its way: the whole file is asked for.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.scale", &json!({"sx": 800, "sy": 800})).unwrap();
    load(&mut s);
    assert_eq!(asked(&s, &path).iter().map(|r| r.width).collect::<Vec<_>>(), [1280]);
    let (w, h) = (small[0].width, small[0].height);
    supply(&mut s, &path, &png(w, h, [1, 2, 3])).unwrap();
    assert_eq!(load(&mut s)["loading"], 1, "{w} px is not the 1280 asked for since");
    vectorcraft_render::link_pixels::forget(&key);
}

/// A placement smaller than the preview asks for at least the preview's pixels, so what arrives
/// is never blurrier than what it replaces.
#[test]
fn a_small_placement_asks_for_no_fewer_pixels_than_the_preview() {
    let _held = hold();
    let (mut s, _, _, path) = opened_without_its_file("source-tiny", (1280, 640), [13, 24, 35], 2.0);
    add_source(&mut s, &path, "tiny/{side}");
    load(&mut s);
    let r = asked(&s, &path);
    assert!(r.len() == 1 && r[0].width >= 256, "the preview is 256 px across: {r:?}");
}

/// A supplied file of another shape than the link's is refused: it would be stretched on screen
/// and in output.
#[test]
fn a_supplied_file_of_another_shape_is_refused() {
    let _held = hold();
    let (mut s, _, key, path) = opened_without_its_file("source-shape", (640, 320), [14, 25, 36], 100.0);
    add_source(&mut s, &path, "shape/{side}");
    let e = supply(&mut s, &path, &png(320, 320, [1, 2, 3]));
    assert!(e.is_err(), "{e:?}");
    assert!(vectorcraft_render::link_pixels::peek(&key).is_none());
    // Rounded to the pixel, a smaller file of the same shape is fine.
    supply(&mut s, &path, &png(321, 160, [1, 2, 3])).unwrap();
    vectorcraft_render::link_pixels::forget(&key);
}

/// A source whose fetch failed and then is supplied after all is found again.
#[test]
fn a_source_supplied_after_failing_is_found_again() {
    let _held = hold();
    let (mut s, _, key, path) = opened_without_its_file("source-again", (640, 320), [15, 26, 37], 100.0);
    add_source(&mut s, &path, "again/{side}");
    assert!(supply(&mut s, &path, b"no picture").is_err());
    assert_eq!(load(&mut s)["failed"], 1);
    supply(&mut s, &path, &png(640, 320, [1, 2, 3])).unwrap();
    assert_eq!(load(&mut s), json!({"loaded": 1, "loading": 0, "failed": 0, "overBudget": 0}));
    assert_eq!(s.execute("links.list", &json!({})).unwrap()["links"][0]["status"], "ok");
    vectorcraft_render::link_pixels::forget(&key);
}

/// A truncated file whose header reads is refused: it would draw, and print, half a picture.
#[test]
fn a_truncated_file_is_refused() {
    let _held = hold();
    let (mut s, _, key, path) = opened_without_its_file("source-cut", (640, 320), [16, 27, 38], 100.0);
    add_source(&mut s, &path, "cut/{side}");
    let whole = noise_png(640, 320, 4242);
    let e = supply(&mut s, &path, &whole[..whole.len() * 3 / 5]);
    assert!(e.is_err(), "{e:?}");
    assert!(vectorcraft_render::link_pixels::peek(&key).is_none());
}

/// A linked image used only in a pattern's art is asked for by `links.loadSources` too, at its
/// placement in the pattern.
#[test]
fn a_linked_image_in_a_pattern_is_asked_for() {
    let _held = hold();
    let dir = Folder::new("source-pattern");
    let pic = dir.file("in-pattern.png");
    write(&pic, &png(640, 320, [17, 28, 39]));
    let mut s = session();
    let id = place(&mut s, &pic);
    s.execute("object.pattern.make", &json!({"ids": [id.0], "edit": false})).unwrap();
    if s.doc().unwrap().doc.node(id).is_some() {
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("edit.clear", &json!({})).unwrap();
    }
    let mut path = None;
    s.doc().unwrap().doc.visit_images(|_, im| path = im.link.as_ref().map(|l| l.path.clone()));
    let path = path.expect("the pattern's art links the file");
    let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    std::fs::remove_file(&pic).unwrap();
    s.execute("document.open", &json!({"name": "pattern.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(&saved)})).unwrap();
    add_source(&mut s, &path, "pattern/{side}");
    assert_eq!(load(&mut s)["loading"], 1);
    assert_eq!(asked(&s, &path).len(), 1);
}

/// What `links.loadSources` asked for is handed out once, whether its document is still open or
/// not: no repaint looks for it again.
#[test]
fn what_load_sources_asks_for_is_handed_out_once() {
    let _held = hold();
    let (mut s, _, _, path) = opened_without_its_file("source-closed", (640, 320), [18, 29, 40], 100.0);
    add_source(&mut s, &path, "closed/{side}");
    assert_eq!(load(&mut s)["loading"], 1);
    // Both tabs: the one it was placed in links the file too.
    while s.doc().is_ok() {
        s.execute("file.close", &json!({"discard": true})).unwrap();
    }
    assert_eq!(asked(&s, &path).len(), 1);
    assert!(asked(&s, &path).is_empty(), "asked for once");
}

/// A source's sidecar in pixel units names the subject in the file's pixels.
#[test]
fn a_source_s_sidecar_in_pixels_names_the_subject() {
    let dir = Folder::new("sources-sidecar-px");
    let pic = dir.file("px-subject.png");
    write(&pic, &png(600, 300, FILE));
    let mut s = session();
    let id = place(&mut s, &pic);
    let path = image(&s, id).link.unwrap().path;
    let xmp = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:Iptc4xmpExt="http://iptc.org/std/Iptc4xmpExt/2008-02-29/"><Iptc4xmpExt:ImageRegion><rdf:Bag><rdf:li rdf:parseType="Resource"><Iptc4xmpExt:RegionBoundary rdf:parseType="Resource"><Iptc4xmpExt:rbShape>rectangle</Iptc4xmpExt:rbShape><Iptc4xmpExt:rbUnit>pixel</Iptc4xmpExt:rbUnit><Iptc4xmpExt:rbX>150</Iptc4xmpExt:rbX><Iptc4xmpExt:rbY>60</Iptc4xmpExt:rbY><Iptc4xmpExt:rbW>150</Iptc4xmpExt:rbW><Iptc4xmpExt:rbH>120</Iptc4xmpExt:rbH></Iptc4xmpExt:RegionBoundary><Iptc4xmpExt:rRole><rdf:Bag><rdf:li rdf:parseType="Resource"><Iptc4xmpExt:Identifier><rdf:Bag><rdf:li>http://cv.iptc.org/newscodes/imageregionrole/mainSubjectArea</rdf:li></rdf:Bag></Iptc4xmpExt:Identifier></rdf:li></rdf:Bag></Iptc4xmpExt:rRole></rdf:li></rdf:Bag></Iptc4xmpExt:ImageRegion></rdf:Description></rdf:RDF></x:xmpmeta>"#;
    s.execute("links.addSources", &json!({"sources": [{"path": path, "url": "px-subject.png?side={side}", "sidecar": xmp}]})).unwrap();
    let subject = s.execute("links.info", &json!({"id": id.0})).unwrap()["subject"].clone();
    assert_eq!(subject, json!({"left": 0.25, "top": 0.2, "right": 0.5, "bottom": 0.6}));
}

/// Supplied pixels take the preview's place, not paint over it: where they are clear, nothing of
/// the preview shows through.
#[test]
fn supplied_pixels_take_the_preview_s_place() {
    let _held = hold();
    let (mut s, _, key, path) = opened_without_its_file("source-clear", (640, 320), [19, 30, 41], 100.0);
    add_source(&mut s, &path, "clear/{side}");
    let mut clear = vec![];
    image::RgbaImage::from_pixel(640, 320, image::Rgba([0, 0, 0, 0]))
        .write_to(&mut std::io::Cursor::new(&mut clear), image::ImageFormat::Png)
        .unwrap();
    supply(&mut s, &path, &clear).unwrap();
    assert!(!near(centre_colour(&s.doc().unwrap().doc), [19, 30, 41]), "the preview does not show through");
    vectorcraft_render::link_pixels::forget(&key);
}

/// A quick fill (`kept_images_only`) decodes nothing: supplied pixels not yet decoded draw as the
/// preview the store holds.
#[test]
fn a_quick_fill_decodes_no_supplied_pixels() {
    let _held = hold();
    let (mut s, _, key, path) = opened_without_its_file("source-quick", (640, 320), [20, 31, 42], 100.0);
    add_source(&mut s, &path, "quick/{side}");
    // The preview decoded.
    centre_colour(&s.doc().unwrap().doc);
    supply(&mut s, &path, &noise_png(640, 320, 4242)).unwrap();
    let opts = vectorcraft_render::RenderOptions { kept_images_only: true, ..Default::default() };
    vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 400, 300, vectorcraft_geom::Affine::IDENTITY, &opts);
    let supplied = vectorcraft_render::link_pixels::peek(&key).unwrap();
    assert!(!vectorcraft_render::image_cached(&supplied.blob), "decoded by a quick fill");
    vectorcraft_render::link_pixels::forget(&key);
}

/// The image caches' budgets have a ceiling: a wrong number can't let them hold all of memory.
#[test]
fn the_image_caches_budgets_have_a_ceiling() {
    let _held = hold();
    let huge = json!(1u64 << 50);
    assert!(Session::new().execute("view.imageCache", &json!({"linked": huge})).is_err());
    assert!(Session::new().execute("view.imageCache", &json!({"decoded": huge})).is_err());
}

/// The straight RGBA `d` draws at document point `at`, on nothing.
fn drawn_at(d: &vectorcraft_doc::Document, at: (f64, f64)) -> [u8; 4] {
    let r =
        vectorcraft_render::Renderer::new().render_region(d, vectorcraft_geom::Rect::new(at.0 - 0.5, at.1 - 0.5, at.0 + 0.5, at.1 + 0.5), 1.0, false);
    let px = r.to_straight();
    [px[0], px[1], px[2], px[3]]
}

/// A colour adjustment on a linked image with a color mask keeps what the mask hides hidden, on
/// the canvas and in output, where the masked pixels it shows link to no file.
#[test]
fn a_colour_adjustment_keeps_a_linked_images_color_mask() {
    let dir = Folder::new("adjust-masked");
    let pic = dir.file("masked.png");
    // 12 × 10: white, a red block in the middle.
    let mut bytes = vec![];
    image::RgbaImage::from_fn(12, 10, |x, y| {
        image::Rgba(if (3..9).contains(&x) && (3..7).contains(&y) { [220, 30, 30, 255] } else { [255, 255, 255, 255] })
    })
    .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
    .unwrap();
    write(&pic, &bytes);
    let mut s = session();
    let id = place(&mut s, &pic);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("addon.image.colorMask", &json!({"background": "colors", "colors": "#ffffff", "contiguous": false, "tolerance": 0, "softness": 0}))
        .unwrap();
    let corner = image(&s, id).xf * vectorcraft_geom::Point::new(0.5, 0.5);
    assert_eq!(drawn_at(&s.doc().unwrap().doc, (corner.x, corner.y))[3], 0, "white is hidden");
    s.execute("effect.apply", &json!({"effect": "adjust.hueSaturation", "params": {"hue": 120}})).unwrap();
    assert_eq!(drawn_at(&s.doc().unwrap().doc, (corner.x, corner.y))[3], 0, "still hidden once recoloured");
    let st = s.doc().unwrap();
    let node = st.doc.node(id).unwrap().clone();
    let mut out = (*st.doc).clone();
    let NodeKind::Image(baked) = vectorcraft_render::effects::adjust_in_document(&mut out, &node, None).unwrap().kind else { panic!("an image") };
    assert!(baked.link.is_none(), "the masked pixels link to no file");
}

/// Expand Appearance of a colour adjustment on a linked image a host supplied: the recoloured
/// image is made from the supplied pixels, not the preview, and is embedded.
#[test]
fn expanding_an_adjusted_sourced_link_recolours_the_supplied_pixels() {
    let _held = hold();
    let (mut s, id, key, path) = opened_without_its_file("adjust-expand", (640, 320), [21, 32, 43], 100.0);
    add_source(&mut s, &path, "expand/{side}");
    supply(&mut s, &path, &png(640, 320, [210, 40, 40])).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("effect.apply", &json!({"effect": "adjust.hueSaturation", "params": {"hue": 120}})).unwrap();
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let mut found = None;
    doc.visit_images(|_, im| found = Some(im.clone()));
    let im = found.unwrap();
    assert!(im.key != key && im.link.is_none(), "an embedded, recoloured image: {:?}", im.link);
    let rgba = image::load_from_memory(&doc.images[&im.key].bytes).unwrap().to_rgba8();
    assert_eq!(rgba.dimensions(), (640, 320), "made from the supplied pixels");
    vectorcraft_render::link_pixels::forget(&key);
}

/// A linked image showing the missing-file placeholder keeps it unrecoloured: a broken link looks
/// broken under a colour adjustment too.
#[test]
fn a_missing_files_placeholder_is_not_recoloured() {
    let mut s = session();
    let png = vectorcraft_doc::links::missing_placeholder().unwrap();
    let id = s
        .edit("t", |d, _| {
            let bytes = std::sync::Arc::new(png.clone());
            d.images.insert("missing-art".into(), vectorcraft_doc::ImageBlob { mime: "image/png".into(), bytes: bytes.clone(), proxy: Some(bytes) });
            let im = vectorcraft_doc::ImageObject {
                key: "missing-art".into(),
                width: 64,
                height: 64,
                xf: vectorcraft_geom::Affine::translate((100.0, 100.0)),
                link: Some(vectorcraft_doc::LinkInfo::new("/nowhere/art.png")),
                placement: Default::default(),
                pixels: None,
            };
            let n = vectorcraft_doc::Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
            let id = n.id;
            let layer = d.layers[0].id;
            d.insert(Some(layer), 0, n)?;
            Ok(id)
        })
        .unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("effect.apply", &json!({"effect": "adjust.hueSaturation", "params": {"hue": 120}})).unwrap();
    let [r, g, b, _] = drawn_at(&s.doc().unwrap().doc, (100.5, 100.5));
    assert!(near([r, g, b], [200, 60, 60]), "the placeholder's red edge: {:?}", [r, g, b]);
}

/// A `w`×`h` JPEG, its left half red and the rest white, whose EXIF orientation 6 says it shows
/// turned a quarter clockwise (`h`×`w`, the red half on top), as a camera held upright stores it.
fn turned_jpeg(w: u32, h: u32) -> Vec<u8> {
    let rgb = image::RgbImage::from_fn(w, h, |x, _| image::Rgb(if x < w / 2 { [220, 0, 0] } else { [255, 255, 255] }));
    let mut jpg = vec![];
    image::DynamicImage::ImageRgb8(rgb).write_to(&mut std::io::Cursor::new(&mut jpg), image::ImageFormat::Jpeg).unwrap();
    let exif: Vec<u8> =
        [b"Exif\0\0".as_slice(), b"II", &[0x2a, 0, 8, 0, 0, 0], &[1, 0], &[0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0], &[0, 0, 0, 0]].concat();
    let mut turned = jpg[..2].to_vec();
    turned.extend([0xff, 0xe1]);
    turned.extend(u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
    turned.extend(&exif);
    turned.extend(&jpg[2..]);
    turned
}

/// A file whose EXIF orientation turns or flips it is refused: a supplied file is drawn and
/// printed as its pixels are stored, so the host supplies it upright.
#[test]
fn a_supplied_photo_turned_by_its_exif_orientation_is_refused() {
    let _held = hold();
    let (mut s, _, key, path) = opened_without_its_file("source-turned", (320, 640), [22, 33, 44], 100.0);
    add_source(&mut s, &path, "turned/{side}");
    let e = supply(&mut s, &path, &turned_jpeg(640, 320)).unwrap_err().to_string();
    assert!(e.contains("supply the picture upright (its EXIF orientation is 6)"), "{e}");
    assert!(vectorcraft_render::link_pixels::peek(&key).is_none());
    assert_eq!(load(&mut s)["failed"], 1, "the source failed");
}

/// A fetch that comes after a larger one for the same file, asked for before it, is dropped: the
/// larger pixels stay. Its failure (a timeout) then says nothing of the source either.
#[test]
fn a_late_smaller_fetch_leaves_the_larger_pixels() {
    let _held = hold();
    let (mut s, id, key, path) = opened_without_its_file("source-late", (1280, 640), [23, 34, 45], 12.5);
    add_source(&mut s, &path, "late/{side}");
    load(&mut s);
    let small = asked(&s, &path);
    assert!(small.len() == 1 && small[0].width < 1280, "{small:?}");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.scale", &json!({"sx": 800, "sy": 800})).unwrap();
    load(&mut s);
    let large = asked(&s, &path);
    assert_eq!(large.iter().map(|r| r.width).collect::<Vec<_>>(), [1280]);
    link_sources::supply_fetched(&mut s, &large[0], png(1280, 640, [1, 2, 3])).unwrap();
    let late = link_sources::supply_fetched(&mut s, &small[0], png(small[0].width, small[0].height, [4, 5, 6]));
    assert!(late.is_err(), "{late:?}");
    assert_eq!(vectorcraft_render::link_pixels::peek(&key).map(|p| p.size), Some((1280, 640)), "the larger pixels stay");
    assert!(!link_sources::fetch_failed(&small[0], "timed out"), "a smaller request's failure says nothing now");
    assert_eq!(load(&mut s), json!({"loaded": 1, "loading": 0, "failed": 0, "overBudget": 0}));
    vectorcraft_render::link_pixels::forget(&key);
}

/// A file refused for the linked budget while smaller pixels of it are kept: `links.loadSources`
/// says it is over budget, not loaded (the smaller pixels would print), and it is not asked for
/// again.
#[test]
fn a_file_refused_for_the_budget_is_not_loaded_by_older_pixels() {
    let _held = hold();
    let (mut s, _, key, path) = opened_without_its_file("source-refused", (1280, 640), [24, 35, 46], 100.0);
    add_source(&mut s, &path, "refused/{side}");
    supply(&mut s, &path, &png(640, 320, [1, 2, 3])).unwrap();
    assert_eq!(load(&mut s)["loading"], 1, "640 px is not the 1280 its placement needs");
    let big = noise_png(1280, 640, 9100);
    let old = vectorcraft_render::link_pixels::budget();
    vectorcraft_render::link_pixels::set_budget(big.len() / 2);
    let refused = supply(&mut s, &path, &big);
    let l = load(&mut s);
    let again = asked(&s, &path);
    vectorcraft_render::link_pixels::set_budget(old);
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(l, json!({"loaded": 0, "loading": 0, "failed": 1, "overBudget": 1}));
    assert!(again.is_empty(), "{again:?}");
    vectorcraft_render::link_pixels::forget(&key);
}

/// A linked image drawn larger than its preview before the host gave its source is asked for
/// once the source comes, without being drawn again.
#[test]
fn an_image_drawn_before_its_source_is_asked_for_once_it_comes() {
    let _held = hold();
    let (mut s, _, _, path) = opened_without_its_file("source-later", (640, 320), [25, 36, 47], 100.0);
    centre_colour(&s.doc().unwrap().doc);
    assert!(asked(&s, &path).is_empty(), "no source yet");
    add_source(&mut s, &path, "later/{side}");
    assert_eq!(asked(&s, &path).iter().map(|r| r.width).collect::<Vec<_>>(), [640]);
}

/// Linked images drawn together whose files don't all fit the linked budget: the one dropped is
/// not asked for again and again while they all show (each fetch would drop another).
#[test]
fn images_drawn_together_past_the_budget_are_not_fetched_forever() {
    let _held = hold();
    let dir = Folder::new("sources-shown");
    let files: Vec<(String, Vec<u8>)> = (0..3).map(|i| (dir.file(&format!("shown-{i}.png")), noise_png(320, 320, 8800 + i))).collect();
    let mut s = session();
    for (pic, bytes) in &files {
        write(pic, bytes);
        place(&mut s, pic);
    }
    let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    for (pic, _) in &files {
        std::fs::remove_file(pic).unwrap();
    }
    s.execute("document.open", &json!({"name": "shown.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(&saved)})).unwrap();
    let mut paths = vec![];
    s.doc().unwrap().doc.visit_images(|_, im| paths.push(im.link.as_ref().unwrap().path.clone()));
    let sources: Vec<_> = paths.iter().map(|p| json!({"path": p, "url": "shown?side={side}"})).collect();
    s.execute("links.addSources", &json!({"sources": sources})).unwrap();
    let size = files.iter().map(|(_, b)| b.len()).max().unwrap();
    let old = vectorcraft_render::link_pixels::budget();
    vectorcraft_render::link_pixels::set_budget(size * 5 / 2);
    let mut rounds = 0;
    let mut fetched = 0;
    while rounds < 8 {
        vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, vectorcraft_geom::Rect::new(0.0, 0.0, 400.0, 300.0), 4.0, true);
        let asked: Vec<_> = link_sources::requests(&s).into_iter().filter(|r| paths.contains(&r.path)).collect();
        if asked.is_empty() {
            break;
        }
        for r in asked {
            let name = r.path.rsplit(['/', '\\']).next().unwrap().to_string();
            let bytes = files.iter().find(|(p, _)| p.ends_with(&name)).unwrap().1.clone();
            fetched += 1;
            link_sources::supply_fetched(&mut s, &r, bytes).unwrap();
        }
        rounds += 1;
    }
    vectorcraft_render::link_pixels::set_budget(old);
    assert!(fetched <= 3, "fetched {fetched} times in {rounds} rounds");
}

/// Pixels a source supplied go when the source is given another URL (it may serve other pixels):
/// the preview shows, and prints, until the new one comes.
#[test]
fn pixels_of_an_old_source_go_with_it() {
    let _held = hold();
    let (mut s, _, key, path) = opened_without_its_file("source-old-pixels", (640, 320), [26, 37, 48], 100.0);
    add_source(&mut s, &path, "first/{side}");
    supply(&mut s, &path, &png(640, 320, [1, 2, 3])).unwrap();
    assert!(vectorcraft_render::link_pixels::peek(&key).is_some());
    add_source(&mut s, &path, "second/{side}");
    assert!(vectorcraft_render::link_pixels::peek(&key).is_none(), "the old URL's pixels go");
    assert!(link_sources::for_output(&s.doc().unwrap().doc).images[&key].is_proxy(), "the preview prints");
    vectorcraft_render::link_pixels::forget(&key);
}

/// A fetch for more pixels that fails after fewer were kept leaves those drawn, and printed, but
/// counts the image failed: they are fewer than it needs.
#[test]
fn a_failed_larger_fetch_keeps_the_smaller_pixels() {
    let _held = hold();
    let (mut s, id, key, path) = opened_without_its_file("source-timeout", (1280, 640), [29, 40, 51], 12.5);
    add_source(&mut s, &path, "timeout/{side}");
    load(&mut s);
    let small = asked(&s, &path);
    link_sources::supply_fetched(&mut s, &small[0], png(small[0].width, small[0].height, [1, 2, 3])).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.scale", &json!({"sx": 800, "sy": 800})).unwrap();
    load(&mut s);
    let large = asked(&s, &path);
    assert!(link_sources::fetch_failed(&large[0], "timed out"));
    assert_eq!(vectorcraft_render::link_pixels::peek(&key).map(|p| p.size.0), Some(small[0].width), "the smaller pixels stay");
    assert_eq!(load(&mut s), json!({"loaded": 0, "loading": 0, "failed": 1, "overBudget": 0}));
    vectorcraft_render::link_pixels::forget(&key);
}

/// Expand Appearance of a colour adjustment on a linked image whose file's pixels aren't loaded
/// (only its preview shows) is refused: expanded, the adjustment would be dropped.
#[test]
fn expanding_an_adjusted_unloaded_link_is_refused() {
    let _held = hold();
    let (mut s, id, key, _) = opened_without_its_file("adjust-unloaded", (640, 320), [28, 39, 50], 100.0);
    vectorcraft_render::link_pixels::forget(&key);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("effect.apply", &json!({"effect": "adjust.hueSaturation", "params": {"hue": 120}})).unwrap();
    let revision = s.doc().unwrap().revision;
    let e = s.execute("effect.expandAppearance", &json!({}));
    assert!(e.as_ref().is_err_and(|e| e.to_string().contains("isn't loaded")), "{e:?}");
    assert_eq!(s.doc().unwrap().revision, revision, "nothing expanded");
}

/// A linked image whose preview is the missing-file placeholder, drawn from pixels a host supplied
/// for its file, is recoloured by a colour adjustment: the placeholder no longer shows.
#[test]
fn supplied_pixels_over_a_placeholder_are_recoloured() {
    let _held = hold();
    let mut s = session();
    let placeholder = vectorcraft_doc::links::missing_placeholder().unwrap();
    let key = "missing-art-supplied";
    let id = s
        .edit("t", |d, _| {
            let bytes = std::sync::Arc::new(placeholder.clone());
            d.images.insert(key.into(), vectorcraft_doc::ImageBlob { mime: "image/png".into(), bytes: bytes.clone(), proxy: Some(bytes) });
            let im = vectorcraft_doc::ImageObject {
                key: key.into(),
                width: 640,
                height: 640,
                xf: vectorcraft_geom::Affine::translate((100.0, 100.0)) * vectorcraft_geom::Affine::scale(0.1),
                link: Some(vectorcraft_doc::LinkInfo::new("/nowhere/supplied-art.png")),
                placement: Default::default(),
                pixels: None,
            };
            let n = vectorcraft_doc::Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
            let id = n.id;
            let layer = d.layers[0].id;
            d.insert(Some(layer), 0, n)?;
            Ok(id)
        })
        .unwrap();
    let blob = vectorcraft_doc::ImageBlob::new("image/png", png(640, 640, [210, 40, 40]));
    assert!(vectorcraft_render::link_pixels::insert(key, vectorcraft_render::link_pixels::Supplied { size: (640, 640), blob }));
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("effect.apply", &json!({"effect": "adjust.hueSaturation", "params": {"hue": 120}})).unwrap();
    let [r, g, b, _] = drawn_at(&s.doc().unwrap().doc, (132.0, 132.0));
    vectorcraft_render::link_pixels::forget(key);
    assert!(g > r && g > b, "recoloured from red: {:?}", [r, g, b]);
}

/// A `w`×`h` noise PNG for path `path`'s source.
fn noisy(w: u32, h: u32, seed: u32) -> Vec<u8> {
    noise_png(w, h, seed)
}

/// `links.loadSources` charges the room pixels asked for again need against the document's own:
/// pixels another document keeps are dropped to make it, so a print never stops at them.
#[test]
fn load_sources_room_counts_this_documents_pixels_alone() {
    let _held = hold();
    let (mut s, _, key, path) = opened_without_its_file("source-room", (640, 320), [31, 42, 53], 100.0);
    add_source(&mut s, &path, "room/{side}");
    let bytes = noisy(640, 320, 5150);
    let old = vectorcraft_render::link_pixels::budget();
    vectorcraft_render::link_pixels::set_budget(bytes.len() * 2);
    supply(&mut s, &path, &bytes).unwrap();
    // Another deck's pixels, drawn since: this one's go.
    let other = vectorcraft_doc::ImageBlob::new("image/png", vec![0; bytes.len() * 3 / 2]);
    vectorcraft_render::link_pixels::insert("source-room-other", vectorcraft_render::link_pixels::Supplied { size: (9, 9), blob: other });
    let l = load(&mut s);
    asked(&s, &path);
    vectorcraft_render::link_pixels::set_budget(old);
    vectorcraft_render::link_pixels::forget("source-room-other");
    vectorcraft_render::link_pixels::forget(&key);
    assert_eq!(l, json!({"loaded": 0, "loading": 1, "failed": 0, "overBudget": 0}));
}

/// A file a pixel narrower than asked (a host rounding the other side) and refused for the budget
/// is not asked for again at the width it nearly answers: it is over budget.
#[test]
fn a_refused_file_a_pixel_short_is_not_asked_for_again() {
    let _held = hold();
    let (mut s, _, key, path) = opened_without_its_file("source-short-refused", (1280, 640), [32, 43, 54], 100.0);
    add_source(&mut s, &path, "short-refused/{width}x{height}");
    load(&mut s);
    let r = asked(&s, &path);
    assert_eq!(r.iter().map(|r| r.width).collect::<Vec<_>>(), [1280]);
    let big = noisy(1279, 639, 6160);
    let old = vectorcraft_render::link_pixels::budget();
    vectorcraft_render::link_pixels::set_budget(big.len() / 2);
    assert!(link_sources::supply_fetched(&mut s, &r[0], big).is_err());
    let l = load(&mut s);
    let again = asked(&s, &path);
    vectorcraft_render::link_pixels::set_budget(old);
    vectorcraft_render::link_pixels::forget(&key);
    assert_eq!(l, json!({"loaded": 0, "loading": 0, "failed": 1, "overBudget": 1}));
    assert!(again.is_empty(), "{again:?}");
}

/// A source that failed, or whose larger file the budget refused, while pixels that still cover
/// what its placement needs are kept: those print, so the image is loaded.
#[test]
fn kept_pixels_that_cover_the_need_are_loaded_after_a_failure() {
    let _held = hold();
    let (mut s, id, key, path) = opened_without_its_file("source-covered", (1280, 640), [33, 44, 55], 12.5);
    add_source(&mut s, &path, "covered/{side}");
    load(&mut s);
    let small = asked(&s, &path);
    link_sources::supply_fetched(&mut s, &small[0], png(small[0].width, small[0].height, [1, 2, 3])).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.scale", &json!({"sx": 800, "sy": 800})).unwrap();
    load(&mut s);
    let large = asked(&s, &path);
    assert!(link_sources::fetch_failed(&large[0], "timed out"));
    s.execute("object.scale", &json!({"sx": 12.5, "sy": 12.5})).unwrap();
    assert_eq!(load(&mut s), json!({"loaded": 1, "loading": 0, "failed": 0, "overBudget": 0}), "failed, but covered");
    // Refused for the budget the same way.
    add_source(&mut s, &path, "covered-again/{side}");
    supply(&mut s, &path, &png(small[0].width, small[0].height, [1, 2, 3])).unwrap();
    let big = noisy(1280, 640, 7170);
    let old = vectorcraft_render::link_pixels::budget();
    vectorcraft_render::link_pixels::set_budget(big.len() / 2);
    assert!(supply(&mut s, &path, &big).is_err());
    let l = load(&mut s);
    vectorcraft_render::link_pixels::set_budget(old);
    vectorcraft_render::link_pixels::forget(&key);
    assert_eq!(l, json!({"loaded": 1, "loading": 0, "failed": 0, "overBudget": 0}), "refused, but covered");
}

/// Pixels the budget dropped are asked for again at the width kept, and a reply narrower than
/// that is not kept in their place: the source fails, the wider pixels' record stays.
#[test]
fn a_reply_narrower_than_the_pixels_kept_does_not_replace_them() {
    let _held = hold();
    let (mut s, id, key, path) = opened_without_its_file("source-narrower", (1280, 640), [34, 45, 56], 100.0);
    add_source(&mut s, &path, "narrower/{side}");
    supply(&mut s, &path, &png(1280, 640, [1, 2, 3])).unwrap();
    vectorcraft_render::link_pixels::forget(&key);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.scale", &json!({"sx": 12.5, "sy": 12.5})).unwrap();
    assert_eq!(load(&mut s)["loading"], 1);
    let r = asked(&s, &path);
    assert_eq!(r.iter().map(|r| r.width).collect::<Vec<_>>(), [1280], "asked again at the width kept");
    assert!(link_sources::supply_fetched(&mut s, &r[0], png(640, 320, [4, 5, 6])).is_err());
    assert!(vectorcraft_render::link_pixels::peek(&key).is_none(), "the narrower pixels are not kept");
    assert_eq!(link_sources::kept_width(&path), Some(1280));
}

/// A fetch the source no longer waits for changes nothing, unless nothing is kept or asked for
/// (it is taken), or it brings more pixels than are kept: a failed source stays failed, an idle one
/// is not failed by it.
#[test]
fn a_fetch_not_waited_for_changes_nothing() {
    let _held = hold();
    let (mut s, _, key, path) = opened_without_its_file("source-unawaited", (640, 320), [35, 46, 57], 100.0);
    add_source(&mut s, &path, "unawaited/{side}");
    load(&mut s);
    let r = asked(&s, &path);
    assert!(supply(&mut s, &path, b"no picture").is_err());
    assert!(link_sources::supply_fetched(&mut s, &r[0], png(640, 320, [1, 2, 3])).is_err(), "a failed source is not waiting");
    assert_eq!(load(&mut s)["failed"], 1, "still failed");
    // Moved away and back: the source is idle, and the old request's URL is its URL again.
    add_source(&mut s, &path, "elsewhere/{side}");
    add_source(&mut s, &path, "unawaited/{side}");
    let _ = link_sources::supply_fetched(&mut s, &r[0], b"no picture".to_vec());
    assert_eq!(load(&mut s)["failed"], 0, "bad bytes it did not wait for fail nothing");
    vectorcraft_render::link_pixels::forget(&key);
}

/// Without a host that fetches (the desktop, a headless host), `links.loadSources` asks for
/// nothing: an image without its pixels is failed, not loading forever, until they are supplied by
/// hand.
#[test]
fn without_a_fetcher_load_sources_ends() {
    let _held = hold();
    let (mut s, _, key, path) = opened_without_its_file("source-desktop", (640, 320), [36, 47, 58], 100.0);
    add_source(&mut s, &path, "desktop/{side}");
    link_sources::set_fetcher(false);
    let first = load(&mut s);
    let asked_then = asked(&s, &path);
    supply(&mut s, &path, &png(640, 320, [1, 2, 3])).unwrap();
    let then = load(&mut s);
    link_sources::set_fetcher(true);
    vectorcraft_render::link_pixels::forget(&key);
    assert_eq!(first, json!({"loaded": 0, "loading": 0, "failed": 1, "overBudget": 0}));
    assert!(asked_then.is_empty());
    assert_eq!(then["loaded"], 1);
}

/// What the model drives the fetch machine with: the host's calls, the canvas drawing, the web
/// shell's fetches answering in any order, the linked budget, documents closing and opening.
#[derive(Clone, Debug)]
enum Event {
    /// `links.addSources` for path `0`; with `1`, a new URL (a re-point).
    Add(usize, bool),
    /// Image `0` placed at the `1`th of [`MODEL_SCALES`].
    Scale(usize, usize),
    /// Both images drawn magnified past their previews.
    Draw,
    /// The `0`th fetch under way (modulo how many there are) answers, as [`Reply`] `1`.
    Reply(usize, usize),
    /// The linked budget set to the `0`th of the model's budgets.
    Budget(usize),
    /// `links.loadSources`.
    Load,
    /// Every document closed.
    Close,
    /// The document opened again in two tabs, when none is: the first's placements at the
    /// smallest scale, the second's (active) at the model's.
    Open,
    /// `links.supplySource` for path `0`, at the width of the `1`th of [`MODEL_SCALES`].
    Hand(usize, usize),
}

/// The placements' scales: the file's pixels a placement needs at 300 ppi.
const MODEL_SCALES: [f64; 4] = [0.25, 0.5, std::f64::consts::FRAC_1_SQRT_2, 1.0];
/// The model's files' pixel size.
const MODEL_FILE: (u32, u32) = (1024, 512);

fn event() -> impl proptest::strategy::Strategy<Value = Event> {
    use proptest::prelude::*;
    prop_oneof![
        1 => (0usize..2, any::<bool>()).prop_map(|(p, r)| Event::Add(p, r)),
        1 => (0usize..2, 0usize..MODEL_SCALES.len()).prop_map(|(i, k)| Event::Scale(i, k)),
        3 => Just(Event::Draw),
        4 => (any::<usize>(), 0usize..6).prop_map(|(i, r)| Event::Reply(i, r)),
        1 => (0usize..2, 0usize..MODEL_SCALES.len()).prop_map(|(i, k)| Event::Hand(i, k)),
        1 => (0usize..3).prop_map(Event::Budget),
        2 => Just(Event::Load),
        1 => Just(Event::Close),
        1 => Just(Event::Open),
    ]
}

/// One run of the model: the paths, their URLs' versions and epochs (each `links.addSources`
/// starts one), the fetches under way, and what each supplied file's bytes came from.
struct Model {
    s: Session,
    saved: Vec<u8>,
    ids: [NodeId; 2],
    keys: [String; 2],
    paths: [String; 2],
    scales: [usize; 2],
    added: [bool; 2],
    version: [u32; 2],
    epoch: [u32; 2],
    /// Fetches under way, with the epoch of their path they were asked in.
    flying: Vec<(link_sources::SourceRequest, u32)>,
    /// Supplied bytes → (path, version).
    made: std::collections::HashMap<Vec<u8>, (usize, u32)>,
    /// The widest pixels supplied for each path's URL now.
    widest: [u32; 2],
    budgets: [usize; 3],
}

impl Model {
    fn new(run: u32) -> Self {
        let dir = Folder::new(&format!("model-{run}"));
        let pics = [0, 1].map(|i| dir.file(&format!("model-{run}-{i}.png")));
        let mut s = session();
        let mut ids = [NodeId(0); 2];
        for (i, pic) in pics.iter().enumerate() {
            write(pic, &png(MODEL_FILE.0, MODEL_FILE.1, [61, 62 + i as u8, 63]));
            ids[i] = place(&mut s, pic);
        }
        let keys = ids.map(|id| image(&s, id).key);
        let paths = ids.map(|id| image(&s, id).link.unwrap().path);
        let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
        for pic in &pics {
            std::fs::remove_file(pic).unwrap();
        }
        while s.doc().is_ok() {
            s.execute("file.close", &json!({"discard": true})).unwrap();
        }
        for key in &keys {
            vectorcraft_render::link_pixels::forget(key);
        }
        let one = png(MODEL_FILE.0, MODEL_FILE.1, [1, 2, 3]).len();
        let mut m = Model {
            s,
            saved,
            ids,
            keys,
            paths,
            scales: [3, 3],
            added: [false; 2],
            version: [0; 2],
            epoch: [0; 2],
            flying: vec![],
            made: Default::default(),
            widest: [0; 2],
            budgets: [64 << 20, one * 3 / 2, 0],
        };
        m.open();
        m
    }

    fn open(&mut self) {
        if self.s.doc().is_ok() {
            return;
        }
        let data = vectorcraft_format::base64_encode(&self.saved);
        let scales = self.scales;
        for tab in 0..2 {
            self.s.execute("document.open", &json!({"name": format!("model-{tab}.vectorcraft"), "dataBase64": data})).unwrap();
            self.scales = if tab == 0 { [0, 0] } else { scales };
            for i in 0..2 {
                self.place(i);
            }
        }
    }

    /// Image `i` placed side by side with the other, at its scale.
    fn place(&mut self, i: usize) {
        if self.s.doc().is_err() {
            return;
        }
        // Points per file pixel for the scale asked at 300 ppi.
        let k = MODEL_SCALES[self.scales[i]] * 72.0 / 300.0;
        let (id, x) = (self.ids[i], 20.0 + 260.0 * i as f64);
        self.s
            .edit("t", |d, _| {
                if let Some(NodeKind::Image(im)) = d.node_mut(id).map(|n| &mut n.kind) {
                    im.xf = vectorcraft_geom::Affine::translate((x, 20.0)) * vectorcraft_geom::Affine::scale(k);
                }
                Ok(())
            })
            .unwrap();
    }

    fn url(&self, i: usize) -> String {
        format!("model/{i}/v{}/{{width}}x{{height}}", self.version[i])
    }

    fn colour(i: usize, version: u32) -> [u8; 3] {
        [10 + i as u8 * 100, (version % 250) as u8, 77]
    }

    /// Bytes from path `i`'s URL of version `version`, `w` × `h`.
    fn bytes(&mut self, i: usize, version: u32, w: u32, h: u32) -> Vec<u8> {
        let b = png(w, h, Self::colour(i, version));
        self.made.insert(b.clone(), (i, version));
        b
    }

    /// The web shell's frame: what the engine asks for now is fetched.
    fn frame(&mut self) {
        for r in link_sources::requests(&self.s) {
            let Some(i) = self.paths.iter().position(|p| *p == r.path) else { continue };
            self.flying.push((r, self.epoch[i]));
        }
    }

    fn step(&mut self, e: &Event) {
        match *e {
            Event::Add(i, repoint) => {
                if repoint {
                    self.version[i] += 1;
                    self.widest[i] = 0;
                }
                let source = link_sources::LinkSource { url: self.url(i), sidecar: None };
                link_sources::add(vec![(self.paths[i].clone(), source)], None).unwrap();
                self.added[i] = true;
                self.epoch[i] += 1;
            }
            Event::Scale(i, k) => {
                self.scales[i] = k;
                self.place(i);
            }
            Event::Draw => {
                if let Ok(st) = self.s.doc() {
                    for i in 0..2 {
                        let x = 20.0 + 260.0 * i as f64 + 10.0;
                        let r = vectorcraft_geom::Rect::new(x, 25.0, x + 1.0, 26.0);
                        vectorcraft_render::Renderer::new().render_region(&st.doc, r, 8.0, true);
                    }
                }
            }
            Event::Reply(at, how) => {
                if self.flying.is_empty() {
                    return;
                }
                let (r, _) = self.flying.remove(at % self.flying.len());
                let i = self.paths.iter().position(|p| *p == r.path).unwrap();
                // The version of the URL it was asked from.
                let version = (0..=self.version[i]).rev().find(|v| r.url.starts_with(&format!("model/{i}/v{v}/"))).unwrap();
                let (w, h) = match how {
                    0 => (r.width, r.height),
                    1 => ((r.width / 2).max(1), (r.height / 2).max(1)),
                    2 => MODEL_FILE,
                    // A pixel short, as a host rounding the other side.
                    5 => (r.width.saturating_sub(1).max(1), (r.width.saturating_sub(1).max(1) * MODEL_FILE.1).div_ceil(MODEL_FILE.0)),
                    _ => (0, 0),
                };
                match how {
                    3 => {
                        let _ = link_sources::supply_fetched(&mut self.s, &r, b"no picture".to_vec());
                    }
                    4 => {
                        link_sources::fetch_failed(&r, "timed out");
                    }
                    _ => {
                        let bytes = self.bytes(i, version, w, h);
                        let _ = link_sources::supply_fetched(&mut self.s, &r, bytes.clone());
                        self.note_kept(i, &bytes, w);
                    }
                }
            }
            Event::Budget(b) => vectorcraft_render::link_pixels::set_budget(self.budgets[b]),
            Event::Load => {
                self.load();
            }
            Event::Close => {
                while self.s.doc().is_ok() {
                    self.s.execute("file.close", &json!({"discard": true})).unwrap();
                }
            }
            Event::Open => self.open(),
            Event::Hand(i, k) => {
                let w = (f64::from(MODEL_FILE.0) * MODEL_SCALES[k]).ceil() as u32;
                let h = (w * MODEL_FILE.1).div_ceil(MODEL_FILE.0);
                let bytes = self.bytes(i, self.version[i], w, h);
                let _ = link_sources::supply(&mut self.s, &self.paths[i].clone(), bytes.clone());
                self.note_kept(i, &bytes, w);
            }
        }
        self.frame();
    }

    /// Note `bytes`, `w` pixels wide, as supplied for path `i` when they were kept.
    fn note_kept(&mut self, i: usize, bytes: &[u8], w: u32) {
        if vectorcraft_render::link_pixels::peek(&self.keys[i]).is_some_and(|p| p.blob.bytes.as_slice() == bytes) {
            self.widest[i] = self.widest[i].max(w);
        }
    }

    /// `links.loadSources`, its counts held to (d): each image with a source counted once.
    fn load(&mut self) -> Option<serde_json::Value> {
        let l = self.s.execute("links.loadSources", &json!({})).ok()?;
        let n = |k: &str| l[k].as_u64().unwrap();
        let sourced = self.added.iter().filter(|a| **a).count() as u64;
        assert_eq!(n("loaded") + n("loading") + n("failed"), sourced, "(d) each image counted once: {l}");
        assert!(n("overBudget") <= n("failed"), "{l}");
        Some(l)
    }

    /// Each path's failed or not, and the width its kept pixels answer for, with its URL's version.
    fn standing(&self) -> [(bool, Option<u32>, u32); 2] {
        [0, 1].map(|i| (link_sources::is_failed(&self.paths[i]), link_sources::kept_width(&self.paths[i]), self.version[i]))
    }

    /// The invariants across step `e`, from `before`: a failed path stays failed until its source is
    /// given again or a file supplied by hand; kept pixels never give way to fewer (but by a pixel)
    /// while the URL stays.
    fn check_after(&self, e: &Event, before: [(bool, Option<u32>, u32); 2]) {
        let now = self.standing();
        for i in 0..2 {
            let given = matches!(*e, Event::Add(p, _) | Event::Hand(p, _) if p == i);
            assert!(!before[i].0 || now[i].0 || given, "a failed path {i} stopped failing on {e:?}");
            if let (Some(w), true) = (before[i].1, before[i].2 == now[i].2) {
                assert!(now[i].1.is_some_and(|n| n.saturating_add(1) >= w), "path {i}'s kept pixels shrank from {w} to {:?} on {e:?}", now[i].1);
            }
        }
    }

    /// The invariants after every step.
    fn check(&self) {
        for i in 0..2 {
            // (a) The fetches under way for a path since its source was last given are few: one a
            // size it is asked for at, and one asked for again after the budget dropped it.
            let now = self.flying.iter().filter(|(r, e)| r.path == self.paths[i] && *e == self.epoch[i]).count();
            assert!(now <= MODEL_SCALES.len() + 1, "(a) {now} fetches under way for one path");
            let kept = vectorcraft_render::link_pixels::peek(&self.keys[i]);
            // (c) No pixels of an old URL are drawn or output.
            if let Some(p) = &kept {
                let from = self.made.get(p.blob.bytes.as_slice()).copied();
                assert_eq!(from, Some((i, self.version[i])), "(c) pixels kept for path {i} come from its URL now");
            }
            if let Ok(st) = self.s.doc() {
                let out = link_sources::for_output(&st.doc);
                let blob = &out.images[&self.keys[i]];
                if !blob.is_proxy() {
                    assert_eq!(self.made.get(blob.bytes.as_slice()).copied(), Some((i, self.version[i])), "(c) output");
                }
            }
            // (e) What the kept pixels answer for is no more than was supplied, and is them.
            if let Some(w) = link_sources::kept_width(&self.paths[i]) {
                assert!(w <= self.widest[i], "(e) kept pixels answer for {w} of the {} supplied", self.widest[i]);
                if let Some(p) = &kept {
                    assert_eq!(p.size.0, w, "(e) the kept pixels are what the state says");
                }
            }
        }
    }

    /// (b) With nothing drawn, `links.loadSources` called a frame at a time, and every fetch
    /// answered, `loading` reaches 0 within a few rounds.
    fn settle(&mut self) {
        self.open();
        for _ in 0..12 {
            let l = self.load().unwrap();
            self.frame();
            self.check();
            if l["loading"] == 0 {
                return;
            }
            while !self.flying.is_empty() {
                self.step(&Event::Reply(0, 0));
                self.check();
            }
        }
        panic!("(b) loadSources still loading after 12 rounds: {}", self.load().unwrap());
    }

    fn finish(self) {
        for key in &self.keys {
            vectorcraft_render::link_pixels::forget(key);
        }
    }
}

proptest::proptest! {
    /// The fetch machine driven by any sequence of events keeps its invariants: (a) the fetches
    /// under way are bounded; (b) `links.loadSources` stops loading once nothing more is drawn;
    /// (c) no pixels of an old URL are drawn or output after a re-point; (d) an image is never
    /// counted both loaded and failed; (e) what kept pixels answer for never passes what was
    /// supplied. No clock: the order of events is the model's.
    #[test]
    fn the_fetch_machine_keeps_its_invariants(events in proptest::collection::vec(event(), 1..40)) {
        static RUN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let _held = hold();
        let old = vectorcraft_render::link_pixels::budget();
        let mut m = Model::new(RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for e in &events {
                let before = m.standing();
                m.step(e);
                m.check();
                m.check_after(e, before);
            }
            m.settle();
            m
        }));
        vectorcraft_render::link_pixels::set_budget(old);
        match outcome {
            Ok(m) => m.finish(),
            Err(e) => std::panic::resume_unwind(e),
        }
    }
}
