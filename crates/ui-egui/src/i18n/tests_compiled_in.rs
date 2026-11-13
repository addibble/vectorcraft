//! The languages the interface offers, whichever language features (`lang-<code>`, all of them with
//! `translations`) the build turns on: a left-out language through a registry of the tests' own,
//! so the default test run covers it, and each catalog's feature in the manifest.

use super::*;

/// A registry with Japanese compiled in and Korean left out (its catalog empty, as its feature off
/// leaves it).
static JA_NOT_KO: [LangInfo; 3] = [
    LangInfo { code: "en", name: "English", source: "", plural: plural_one_other, complete_menus: false, catalog: OnceLock::new() },
    LangInfo { code: "ja", name: "日本語", source: "# catalog", plural: plural_one_other, complete_menus: true, catalog: OnceLock::new() },
    LangInfo { code: "ko", name: "한국어", source: "", plural: plural_one_other, complete_menus: true, catalog: OnceLock::new() },
];

/// A language left out of the build is not offered, not found by its code or a locale, and
/// claims no complete menus; a system language list skips it for the next one compiled in, and
/// with none, English. English and a compiled-in language are offered.
#[test]
fn a_left_out_language_is_neither_offered_nor_found() {
    let ja = Lang(&JA_NOT_KO[1]);
    let ko = Lang(&JA_NOT_KO[2]);
    assert_eq!(all_in(&JA_NOT_KO).collect::<Vec<_>>(), [Lang::EN, ja]);
    assert_eq!(from_code_in(&JA_NOT_KO, "JA"), Some(ja));
    assert_eq!(from_code_in(&JA_NOT_KO, "ko"), None);
    assert_eq!(lang_from_tag_in(&JA_NOT_KO, "ja_JP.UTF-8"), Some(ja));
    assert_eq!(lang_from_tag_in(&JA_NOT_KO, "ko_KR.UTF-8"), None);
    assert_eq!(preferred_in(&JA_NOT_KO, &["ko-KR", "ja-JP", "en-US"]), ja);
    assert_eq!(preferred_in(&JA_NOT_KO, &["ko-KR"]), Lang::EN);
    assert!(ja.complete_menus() && !ko.complete_menus());
}

/// English is always offered, and with `translations` (the apps' default) every registered
/// language is.
#[test]
fn english_always_and_with_translations_every_language_is_offered() {
    assert_eq!(Lang::from_code("en"), Some(Lang::EN));
    if cfg!(feature = "translations") {
        assert_eq!(Lang::all().count(), LANGUAGES.len());
        assert!(Lang::from_code("ja").is_some_and(Lang::complete_menus));
    }
}

/// A catalog whose feature is off is empty, and one whose feature is on is its file: `lang-left-out`
/// is a feature no build has, and `translations` turns on `lang-el`.
#[test]
#[allow(unexpected_cfgs)]
fn a_catalog_is_its_file_only_with_its_feature() {
    assert_eq!(catalog_tsv!("el.tsv", "lang-left-out"), "");
    if cfg!(feature = "lang-el") {
        assert_eq!(catalog_tsv!("el.tsv", "lang-el"), include_str!("el.tsv"));
    }
}

/// Each catalog is compiled in by the feature named after its file (`catalog_tsv!` reads the file
/// exactly when that feature is on: the test above), the row that reads it has the file's code and
/// every catalog file has text, so each row's catalog is its own file or, its feature off, empty
/// (with `translations`, its file); and the manifest declares exactly one `lang-<code>` feature per language, every one of them listed in
/// `translations`. So no build can offer a language under another's feature, or leave one out of
/// `translations`. (A feature the manifest lacks is also an `unexpected_cfgs` warning, which clippy
/// refuses.)
#[test]
fn each_catalog_has_its_own_feature_and_translations_lists_every_one() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/i18n");
    let module = std::fs::read_to_string(format!("{dir}.rs")).unwrap();
    let mut files = Vec::new();
    // The registry's rows only (`source: catalog_tsv!(…)`), not the module's documentation or the
    // macro's own definition.
    for call in module.split("source: catalog_tsv!(").skip(1) {
        let args: Vec<&str> = call.split(')').next().unwrap().split(',').map(|a| a.trim().trim_matches('"')).collect();
        let [file, feature] = args[..] else { panic!("catalog_tsv!({call:.40}…) takes a file and a feature") };
        let code = file.strip_prefix("i18n/").and_then(|f| f.strip_suffix(".tsv")).unwrap();
        assert_eq!(feature, format!("lang-{code}"), "{file}'s feature");
        files.push(code);
    }
    let registered: Vec<&str> = LANGUAGES.iter().map(|l| l.code).filter(|c| *c != "en").collect();
    assert_eq!(files, registered, "one catalog_tsv! per language, in the table's order");
    for code in &registered {
        let text = std::fs::read_to_string(format!("{dir}/{code}.tsv")).unwrap();
        assert!(!text.is_empty(), "{code}.tsv is empty");
        let row = LANGUAGES.iter().find(|l| l.code == *code).unwrap();
        let own = if cfg!(feature = "translations") { [text.as_str(), text.as_str()] } else { [text.as_str(), ""] };
        assert!(own.contains(&row.source), "{code}: its own catalog with lang-{code}, else none");
    }

    let manifest: toml::Table = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap().parse().unwrap();
    let features = manifest["features"].as_table().unwrap();
    let declared: std::collections::BTreeSet<&str> = features.keys().filter_map(|k| k.strip_prefix("lang-")).collect();
    let wanted: std::collections::BTreeSet<&str> = registered.iter().copied().collect();
    assert_eq!(declared, wanted, "one lang-<code> feature per language, and none without one");
    for code in &registered {
        assert_eq!(features[&format!("lang-{code}")].as_array().map(Vec::len), Some(0), "lang-{code} turns on nothing else");
    }
    let translations: std::collections::BTreeSet<&str> =
        features["translations"].as_array().unwrap().iter().filter_map(|f| f.as_str()?.strip_prefix("lang-")).collect();
    assert_eq!(translations, wanted, "translations turns on every language's feature");
}

/// A build that leaves a language out (with no language feature, English only, as `cargo xtask
/// ci`'s `no-bundled-fonts` step builds the panels) draws its window in English, and reads a saved
/// preference for that language as Automatic everywhere: the Language menu ticks Automatic,
/// Preferences shows Auto, and `app.language` takes the code, as the preference keeps it for a
/// build that has the language. With every language compiled in there is none to leave out.
#[test]
fn a_saved_language_this_build_leaves_out_reads_as_automatic() {
    use crate::menus::{checked, run_ui_command};
    use serde_json::json;
    let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let shown = crate::tests_labels::painted_text(&mut app, |a, ui| {
        a.logic(ui.ctx());
        a.ui(ui);
    });
    assert!(shown.lines().any(|l| l == "File"), "the window draws its menus in English: {shown}");
    let Some(left_out) = LANGUAGES.iter().find(|l| !l.compiled_in()) else { return };
    let code = left_out.code;
    run_ui_command(&mut app, "app.language", &json!({"lang": code})).unwrap().unwrap_or_else(|e| panic!("app.language {code}: {e}"));
    assert_eq!(app.session.prefs.interface_language, code, "the preference keeps {code}");
    assert_eq!(app.ui_language(), Lang::EN, "{code} follows the system, English in tests");
    assert_eq!(checked(&app, "app.language", &json!({"lang": "auto"})), Some(true), "Automatic is ticked");
    assert_eq!(checked(&app, "app.language", &json!({"lang": code})), Some(false));
    crate::prefs_dialog::open(&mut app, Some("User Interface"));
    let prefs = crate::tests_labels::painted_text(&mut app, |a, ui| crate::prefs_dialog::show(a, ui.ctx()));
    assert!(prefs.lines().any(|l| l == "Auto"), "Preferences shows Auto for {code}: {prefs}");
    assert!(!prefs.lines().any(|l| l == code), "not the code itself: {prefs}");
    crate::prefs_dialog::confirm(&mut app).unwrap();
    assert_eq!(app.session.prefs.interface_language, code, "OK keeps {code}");
}
