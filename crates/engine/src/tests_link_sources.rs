//! Link sources (`links.addSources`): a document opened from bytes, whose linked file can't be
//! read, shows the preview until the image is drawn larger than it, then the file is asked for at
//! its placement's size, drawn once supplied, exported, and never saved or undone.

use serde_json::json;

use super::tests_links::{Folder, centre_colour, image, near, place, png, session, write};
use super::*;

/// Colours and sizes no other test links, so the shared pixels are this test's alone.
const FILE: [u8; 3] = [10, 200, 90];
const SUPPLIED: [u8; 3] = [200, 120, 10];

#[test]
fn a_sourced_link_is_asked_for_when_drawn_large_supplied_drawn_and_exported() {
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

    let added = s.execute("links.addSources", &json!({"sources": [{"path": path, "url": "pictures/sourced.png?side={side}"}]})).unwrap();
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
    let data = vectorcraft_format::base64_encode(&png(640, 320, SUPPLIED));
    let r = s.execute("links.supplySource", &json!({"path": path, "dataBase64": data})).unwrap();
    assert_eq!(r, json!({"images": 1, "pixelWidth": 640, "pixelHeight": 320}));
    assert!(s.doc().unwrap().revision > revision, "the canvas draws again");
    assert!(near(centre_colour(&s.doc().unwrap().doc), SUPPLIED), "drawn over the preview");
    assert!(link_sources::requests(&s).iter().all(|r| r.path != path), "supplied: not asked for again");
    assert_eq!(s.execute("links.loadSources", &json!({})).unwrap(), json!({"loaded": 1, "loading": 0, "failed": 0}));

    // Not an edit: the document still holds the preview, nothing to undo, and a save writes it.
    let st = s.doc().unwrap();
    assert!(st.doc.images[&key].is_proxy() && !st.is_dirty(), "the document is unchanged");
    // Output uses the supplied pixels, the preview kept as the preview.
    let out = link_sources::for_output(&st.doc);
    assert_eq!(out.images[&key].bytes.as_ref(), &png(640, 320, SUPPLIED));
    assert!(!out.images[&key].is_proxy() && out.images[&key].proxy.is_some());
    let pdf = s.execute("document.export", &json!({"format": "pdf"})).unwrap();
    assert!(pdf["dataBase64"].as_str().is_some_and(|d| !d.is_empty()));

    // Dropped (past its budget), it is drawn from the preview and asked for again.
    vectorcraft_render::link_pixels::forget(&key);
    assert!(near(centre_colour(&s.doc().unwrap().doc), FILE));
    assert_eq!(link_sources::requests(&s).into_iter().filter(|r| r.path == path).count(), 1);
    let failed = s.execute("links.supplySource", &json!({"path": path, "dataBase64": vectorcraft_format::base64_encode(b"no picture")}));
    assert!(failed.is_err(), "bytes that are no picture are refused");
    centre_colour(&s.doc().unwrap().doc);
    assert!(link_sources::requests(&s).iter().all(|r| r.path != path), "a failed source is not asked for again by itself");
    assert_eq!(s.execute("links.loadSources", &json!({})).unwrap(), json!({"loaded": 0, "loading": 0, "failed": 1}));

    for bad in
        [json!({}), json!({"sources": []}), json!({"sources": [{"path": "p"}]}), json!({"sources": [{"path": "p", "url": "u"}], "resolution": 0})]
    {
        assert!(s.execute("links.addSources", &bad).is_err(), "{bad}");
    }
}

#[test]
fn the_image_caches_budgets_are_told_and_set() {
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
