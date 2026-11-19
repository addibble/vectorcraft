//! The font menus list the installed fonts without asking for them (#36).

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_text::{FontDb, system_font_dirs};

use crate::{VectorcraftApp, menus};

#[test]
fn type_font_menu_lists_every_available_family() {
    // The font list is read twice: no test adds to it meanwhile.
    let _fonts = crate::adding_fonts();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    // The app starts the installed fonts' scan, and until it ends Type › Font lists only the
    // loaded fonts (#5483). The scan changes the generation a moment before it counts as done, so
    // a read in between sees the old list under the new generation: wait for it to end first.
    FontDb::global().family_list();
    app.run("file.new", json!({})).unwrap();
    let fonts = |app: &VectorcraftApp| -> Vec<String> {
        menus::menu_entries(app).into_iter().filter(|e| e.path == ["Type", "Font"]).map(|e| e.label).collect()
    };
    let installed = FontDb::with_font_dirs(system_font_dirs());
    installed.load_system_fonts();
    // A test running alongside can add a font (and the installed fonts' scan can end), and the menu
    // is then built again: judge a run in which the fonts held still.
    for _ in 0..20 {
        let generation = FontDb::global().generation();
        let labels = fonts(&app);
        let listed = FontDb::global().menu_family_list();
        // The menu is built every frame, from a list built once.
        let again = fonts(&app);
        if FontDb::global().generation() != generation {
            continue;
        }
        assert_eq!(labels, *listed);
        let missing: Vec<String> = installed.menu_family_list().iter().filter(|f| !labels.contains(f)).cloned().collect();
        assert!(missing.is_empty(), "installed but not in Type › Font: {missing:?}");
        assert_eq!(again, labels);
        return;
    }
    panic!("the fonts kept changing");
}

#[test]
fn the_character_panel_menu_refreshes_the_font_list() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    // The installed fonts are always listed: the menu offers to look for new ones instead.
    let text = crate::tests_labels::painted_text(&mut app, crate::panels::character::menu);
    assert!(text.contains("Refresh Font List") && !text.contains("System Fonts"), "{text}");
}

/// RFC-0034 acceptance test 7: only the host gives font indexes. No command adds one, so neither
/// the control channel, MCP, a script nor a plug-in can; the host's own call is taken even during
/// a frame of synthetic input (the help guides drive the pointer while the page gives its fonts),
/// when the set has already left the shell's queue (review round 1, item 1).
#[test]
fn only_the_host_s_call_gives_font_indexes() {
    use vectorcraft_text::test_fonts::{HostIndexRemoved, IndexFile, host_index};
    let _fonts = crate::adding_fonts();
    let ids = vectorcraft_engine::cmd::command_specs().iter().map(|c| c.id).chain(crate::menus::UI_COMMANDS.iter().map(|c| c.0));
    for id in ids {
        let lower = id.to_ascii_lowercase();
        assert!(!lower.contains("fontindex") && !lower.contains("fontsource") && lower != "text.addfont", "{id} would give fonts");
    }
    const FAMILY: &str = "Auto Taken 01";
    let index = host_index(&[(FAMILY, &[IndexFile { path: "f/Regular.ttf", bytes: b"a face", italic: false, weight: 400 }])]);
    let db = vectorcraft_text::FontDb::global();
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.synthetic_frame = true;
    let _index = HostIndexRemoved("page".into());
    assert_eq!(app.set_font_indexes(vec![("page".into(), index)]).unwrap()["families"], 1);
    assert!(db.is_source(FAMILY), "taken during a synthetic frame");
}
