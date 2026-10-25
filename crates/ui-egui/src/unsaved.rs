//! Closing documents with unsaved changes. File → Close / Close All, a tab's ×, Quit and the
//! window's close button first ask Save / Don't Save / Cancel for each modified document (one
//! dialog per document); Cancel stops the whole operation and a cancelled Save As keeps the
//! document open.
//!
//! The question is an ordinary dialog (`UiState::dialog`, kind [`KIND`]), so agents answer it
//! like any other: `ui.dialog.confirm` saves, `ui.dialog.set {field: "discard", value: true}` then
//! confirm discards, `ui.dialog.cancel` cancels. `file.close {discard: true}` closes a document
//! without asking or saving ([`discard`]): no menu item or shortcut passes it, only a caller of the
//! command (an agent, or a page embedding the web build).
//!
//! A page that embeds the web build and stores its saves itself can ask for no question
//! (`UiState::host_save_on_close`, `docs/web-embed.md`): closing a modified document then saves it
//! as Save would and keeps it open until the page answers that it stored that save
//! ([`host_saved`]); only then does it close and the Close All or Quit go on. A save the page
//! couldn't store leaves the document open and modified, and so does no answer.

use serde_json::{Value, json};

use crate::state::Dialog;
use crate::{VectorcraftApp, io};

/// Dialog kind. Fields: `index` (document), `uid` (the same document, which `index` may no longer
/// point at once a file dialog answers), `name` (its title), `then` (`close`, `closeAll` or `quit`:
/// what continues after this document) and `discard` (Don't Save).
pub const KIND: &str = "saveChanges";

/// Nothing closes while a file dialog's sheet is on the window (macOS): what asked for it runs
/// again when it answers. (Elsewhere the dialog is a window of its own, and closing goes on.)
fn refuse_while_picking(app: &VectorcraftApp) -> Result<(), String> {
    if app.file_sheet_open() { Err(crate::picks::busy().into()) } else { Ok(()) }
}

/// What `close` and friends answer while a document's save waits for the embedding page.
pub const HOST_SAVE: &str = "hostSave";

/// A close waiting for the embedding page to store the save it made (`saveOnClose`): the document
/// `uid`, saved as `name`, closes once the page answers that it stored save `save`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostClose {
    /// The document ([`vectorcraft_engine::DocState::uid`]).
    pub uid: u64,
    /// The file name the save handed to the page.
    pub name: String,
    /// What goes on once it closes: `close`, `closeAll` or `quit`.
    pub then: String,
    /// The page's id of the save it waits for, set by the host once it has posted it; a later save
    /// of the same file takes its place, since the page's answer to that one is what counts.
    pub save: Option<u64>,
}

/// While a document's save waits for the embedding page, nothing else that could close it does.
fn refuse_while_waiting(app: &VectorcraftApp) -> Result<(), String> {
    match &app.ui.host_closing {
        Some(c) => Err(format!("{} closes once the page has stored it", c.name)),
        None => Ok(()),
    }
}

/// File → Close (or a tab's ×) for document `i`.
pub fn close(app: &mut VectorcraftApp, i: usize) -> Result<Value, String> {
    refuse_while_picking(app)?;
    // A save still running decides whether there is anything left to save.
    crate::background::wait_all(app);
    let doc = app.session.documents().get(i).ok_or("no such document")?;
    let (dirty, uid) = (doc.is_dirty(), doc.uid);
    // Another document, saved and stored, may close while one waits; this one may not.
    if dirty || app.ui.host_closing.as_ref().is_some_and(|c| c.uid == uid) {
        refuse_while_waiting(app)?;
    }
    if !dirty {
        close_now(app, i)
    } else if app.ui.host_save_on_close {
        save_for_host(app, i, "close")
    } else {
        ask(app, i, "close")
    }
}

/// `file.close {discard: true}`: close document `i` unsaved, without asking, though it has unsaved
/// changes. A question already asked about it, or a close waiting for the page to store it, goes
/// with it.
pub fn discard(app: &mut VectorcraftApp, i: usize) -> Result<Value, String> {
    refuse_while_picking(app)?;
    crate::background::wait_all(app);
    let uid = app.session.documents().get(i).ok_or("no such document")?.uid;
    if app.ui.host_closing.as_ref().is_some_and(|c| c.uid == uid) {
        app.ui.host_closing = None;
    }
    if app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND && d.fields.get("uid").and_then(Value::as_u64) == Some(uid)) {
        app.ui.dialog = None;
    }
    close_now(app, i)?;
    Ok(Value::Null)
}

/// File → Close All (`then` = `closeAll`) or Quit (`quit`): ask about the next modified document,
/// or finish once none is left.
pub fn close_all(app: &mut VectorcraftApp, then: &str) -> Result<Value, String> {
    refuse_while_picking(app)?;
    refuse_while_waiting(app)?;
    crate::background::wait_all(app);
    if let Some(i) = app.session.documents().iter().position(|d| d.is_dirty()) {
        return if app.ui.host_save_on_close { save_for_host(app, i, then) } else { ask(app, i, then) };
    }
    if then == "quit" {
        // Nothing unsaved is left: no recovery copies either.
        vectorcraft_engine::cmd::recovery::forget_all(&mut app.session);
        // The host closes the window.
        app.ui.status = "quit".into();
        return Ok(Value::Null);
    }
    if app.session.documents().is_empty() {
        return Ok(json!({ "closed": 0 }));
    }
    let r = app.session.execute("file.closeAll", &json!({})).map_err(|e| e.to_string());
    app.sync_views();
    r
}

/// Is any open document modified, or a save waiting for the embedding page (the window's close
/// button must ask first)?
pub fn any_dirty(app: &VectorcraftApp) -> bool {
    app.ui.host_closing.is_some() || app.session.documents().iter().any(|d| d.is_dirty())
}

/// Save document `i` for the embedding page as Save does, and keep it open until the page says it
/// stored the save ([`host_saved`]); `then` is what goes on after it closes.
fn save_for_host(app: &mut VectorcraftApp, i: usize, then: &str) -> Result<Value, String> {
    app.session.set_active(i);
    let r = io::save(app, vectorcraft_engine::cmd::fileio::SaveMode::Save, &json!({}), false)?;
    // A save that asks first keeps the document open; it closes when closed again.
    if r.get("pending").is_some() {
        return Ok(r);
    }
    if r["background"] == true {
        crate::background::wait_all(app);
    }
    let doc = app.session.documents().get(i).ok_or("no such document")?;
    if doc.is_dirty() {
        return Err(format!("{}: not saved, so not closed", doc.title()));
    }
    let name = r["path"].as_str().map(vectorcraft_engine::cmd::fileio::file_name).unwrap_or_else(|| doc.title());
    app.ui.host_closing = Some(HostClose { uid: doc.uid, name: name.clone(), then: then.into(), save: None });
    app.status(format!("Saving {name}: it closes once the page has stored it"));
    Ok(json!({ "pending": HOST_SAVE, "name": name }))
}

/// The host handed the page save `id` of the file `name` (the web build's `vectorcraft:save`): a
/// close waiting for that file waits for this save's answer, the newest of it, from now on.
pub fn host_posted(app: &mut VectorcraftApp, id: u64, name: &str) {
    if let Some(c) = app.ui.host_closing.as_mut().filter(|c| c.name == name) {
        c.save = Some(id);
    }
}

/// The embedding page's answer to its save `id` of the file `name`: stored (`ok`), or not, with
/// the page's `error`. The answer to the save a close waits for closes the document and goes on
/// with Close All or Quit; if it wasn't stored, the document stays open, counts as modified again
/// and the status line says why. A document edited after the save stays open too. Any other answer
/// changes nothing.
pub fn host_saved(app: &mut VectorcraftApp, id: u64, name: &str, ok: bool, error: Option<&str>) {
    if app.ui.host_closing.as_ref().is_none_or(|c| c.save != Some(id) || c.name != name) {
        return;
    }
    let Some(c) = app.ui.host_closing.take() else { return };
    // Discarded or closed meanwhile: nothing left to close.
    let Some(i) = app.session.documents().iter().position(|d| d.uid == c.uid) else { return };
    if !ok {
        if let Some(d) = app.session.document_mut(c.uid).filter(|d| !d.is_dirty()) {
            d.mark_unsaved();
        }
        app.status(match error.map(str::trim).filter(|e| !e.is_empty()) {
            Some(e) => format!("The page couldn't store {name}, so it stays open: {e}"),
            None => format!("The page couldn't store {name}, so it stays open"),
        });
        return;
    }
    if app.session.documents().get(i).is_some_and(|d| d.is_dirty()) {
        app.status(format!("{name} changed after it was saved, so it stays open"));
        return;
    }
    let r = close_now(app, i).and_then(|_| if c.then == "close" { Ok(Value::Null) } else { close_all(app, &c.then) });
    if let Err(e) = r {
        app.status(e);
    }
}

/// Show document `i` and ask whether to save it.
fn ask(app: &mut VectorcraftApp, i: usize, then: &str) -> Result<Value, String> {
    app.session.set_active(i);
    let doc = &app.session.documents()[i];
    let (name, uid) = (doc.title(), doc.uid);
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "index": i, "uid": uid, "name": name, "then": then })));
    Ok(json!({ "pending": KIND }))
}

/// Close document `i` through the engine command, keeping `views` aligned with the documents.
fn close_now(app: &mut VectorcraftApp, i: usize) -> Result<Value, String> {
    let r = app.session.execute("file.close", &json!({ "index": i })).map_err(|e| e.to_string())?;
    if i < app.views.len() {
        app.views.remove(i);
    }
    app.sync_views();
    if app.session.tool_id() == "artboard"
        && let Some(n) = app.session.active().map(|d| d.doc.artboards.len())
    {
        // Closing recreates the tool; restore the surviving document's displayed board.
        let selected = crate::panels::artboards::selected(app, n);
        crate::panels::artboards::select(app, selected);
    }
    Ok(r)
}

/// Answer the open dialog: save (or, with `discard`, don't), close the document and carry on.
pub fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    let d = app.ui.dialog.take().ok_or("no dialog open")?;
    let docs = app.session.documents();
    // By its uid: documents may have closed while its Save dialog was open.
    let i = match d.fields.get("uid").and_then(Value::as_u64) {
        Some(uid) => docs.iter().position(|doc| doc.uid == uid),
        None => d.fields.get("index").and_then(Value::as_u64).map(|i| i as usize).filter(|i| *i < docs.len()),
    };
    let i = i.ok_or("no such document")?;
    if !d.bool("discard") {
        app.session.set_active(i);
        let r = io::save(app, vectorcraft_engine::cmd::fileio::SaveMode::Save, &json!({}), false)?;
        // A save that asks first (replacing a file that would lose what opening it left out) keeps
        // the document open: it closes once saved.
        if r.get("pending").is_some() {
            return Ok(r);
        }
        // Closing needs the file written: wait for a background save, and stop if it failed.
        if r["background"] == true {
            crate::background::wait_all(app);
            if let Some(d) = app.session.documents().get(i).filter(|d| d.is_dirty()) {
                return Err(format!("{}: not saved, so not closed", d.title()));
            }
        }
    }
    close_now(app, i)?;
    match d.str("then").as_str() {
        "close" => Ok(Value::Null),
        then => close_all(app, then),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use vectorcraft_engine::Session;

    /// An app whose Save As picks `out.vectorcraft` and records written paths.
    fn app() -> (VectorcraftApp, Arc<Mutex<Vec<String>>>) {
        let written = Arc::new(Mutex::new(vec![]));
        let w = written.clone();
        let services = crate::Services {
            pick_save: Some(Box::new(|_: &crate::FilePick| Some("out.vectorcraft".to_string()))),
            write: Some(Box::new(move |p: &str, _: &[u8]| {
                w.lock().unwrap().push(p.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        (VectorcraftApp::new(Session::new(), services), written)
    }

    fn new_doc(app: &mut VectorcraftApp, dirty: bool) {
        app.run("file.new", json!({ "width": 100, "height": 100 })).unwrap();
        if dirty {
            app.run("shape.rectangle", json!({ "x": 0, "y": 0, "width": 10, "height": 10 })).unwrap();
        }
    }

    fn docs(app: &VectorcraftApp) -> usize {
        app.session.documents().len()
    }

    fn pending(app: &VectorcraftApp) -> Option<usize> {
        app.ui.dialog.as_ref().filter(|d| d.kind == KIND).and_then(|d| d.fields["index"].as_u64()).map(|i| i as usize)
    }

    #[test]
    fn clean_documents_close_at_once_keeping_views_aligned() {
        let (mut app, _) = app();
        for _ in 0..3 {
            new_doc(&mut app, false);
        }
        assert!(!any_dirty(&app));
        app.views[0].zoom = 2.0;
        app.views[2].zoom = 3.0;
        close(&mut app, 1).unwrap();
        assert_eq!(docs(&app), 2);
        assert_eq!(app.views.iter().map(|v| v.zoom).collect::<Vec<_>>(), vec![2.0, 3.0]);
        assert_eq!(app.run("file.close", json!({})).unwrap(), Value::Null);
        assert_eq!(docs(&app), 1);
        assert!(app.ui.dialog.is_none());
    }

    #[test]
    fn closing_a_modified_document_asks_and_cancel_keeps_it() {
        let (mut app, written) = app();
        new_doc(&mut app, true);
        new_doc(&mut app, false);
        assert!(any_dirty(&app));
        assert_eq!(app.run("file.close", json!({ "index": 0 })).unwrap(), json!({ "pending": KIND }));
        assert_eq!((docs(&app), pending(&app), app.session.active_index()), (2, Some(0), Some(0)));
        // Cancel (Esc / Cancel button / ui.dialog.cancel) closes nothing.
        app.ui.dialog = None;
        assert_eq!(docs(&app), 2);
        // Save writes the file (Save As for an untitled document), then closes it.
        app.run("file.close", json!({ "index": 0 })).unwrap();
        confirm(&mut app).unwrap();
        assert_eq!(*written.lock().unwrap(), vec!["out.vectorcraft".to_string()]);
        assert_eq!((docs(&app), app.views.len()), (1, 1));
    }

    #[test]
    fn a_cancelled_save_as_keeps_the_document_open() {
        let (mut app, _) = app();
        app.services.pick_save = Some(Box::new(|_: &crate::FilePick| None));
        new_doc(&mut app, true);
        close(&mut app, 0).unwrap();
        assert!(confirm(&mut app).is_err());
        assert_eq!(docs(&app), 1);
        assert!(app.ui.dialog.is_none());
    }

    #[test]
    fn close_all_and_quit_ask_once_per_modified_document() {
        let (mut app, written) = app();
        new_doc(&mut app, false);
        new_doc(&mut app, true);
        new_doc(&mut app, true);
        // Close All: asks about each modified document in turn, then closes the rest.
        app.run("file.closeAll", json!({})).unwrap();
        assert_eq!(pending(&app), Some(1));
        app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
        confirm(&mut app).unwrap();
        assert_eq!((docs(&app), pending(&app)), (2, Some(1)));
        app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
        confirm(&mut app).unwrap();
        assert_eq!((docs(&app), app.views.len(), app.ui.dialog.is_none()), (0, 0, true));
        assert!(written.lock().unwrap().is_empty());
        // Quit: nothing modified quits at once; otherwise only after every answer.
        new_doc(&mut app, true);
        app.run("app.quit", json!({})).unwrap();
        assert_ne!(app.ui.status, "quit");
        assert_eq!(pending(&app), Some(0));
        confirm(&mut app).unwrap();
        assert_eq!(app.ui.status, "quit");
        app.ui.status.clear();
        app.run("app.quit", json!({})).unwrap();
        assert_eq!(app.ui.status, "quit");
    }

    /// An app embedded by a page that stores its saves (`saveOnClose`): every save is handed over
    /// by name, as the web build's downloads are.
    fn hosted() -> (VectorcraftApp, Arc<Mutex<Vec<String>>>) {
        let handed = Arc::new(Mutex::new(vec![]));
        let h = handed.clone();
        let download: crate::DownloadFn = Box::new(move |name: &str, _: &[u8]| {
            h.lock().unwrap().push(name.to_string());
            Ok(())
        });
        let mut app = VectorcraftApp::new(Session::new(), crate::Services { download: Some(download), ..Default::default() });
        app.ui.host_save_on_close = true;
        (app, handed)
    }

    /// A modified document saved before as `name`.
    fn named_doc(app: &mut VectorcraftApp, name: &str) {
        new_doc(app, false);
        app.run("file.save", json!({ "path": format!("/docs/{name}") })).unwrap();
        app.run("shape.rectangle", json!({ "x": 0, "y": 0, "width": 10, "height": 10 })).unwrap();
    }

    /// What the host does once it has posted the save a close made: the save's id, by file name.
    fn posted(app: &mut VectorcraftApp, id: u64) {
        let name = app.ui.host_closing.as_ref().unwrap().name.clone();
        host_posted(app, id, &name);
    }

    #[test]
    fn with_save_on_close_a_modified_document_closes_once_the_page_stored_its_save() {
        let (mut app, handed) = hosted();
        named_doc(&mut app, "Logo.vectorcraft");
        handed.lock().unwrap().clear();
        // No question: the document is saved as Save saves it, and stays open.
        assert_eq!(app.run("file.close", json!({})).unwrap(), json!({ "pending": HOST_SAVE, "name": "Logo.vectorcraft" }));
        assert_eq!(*handed.lock().unwrap(), ["Logo.vectorcraft"]);
        assert!(app.ui.dialog.is_none());
        assert_eq!(docs(&app), 1);
        assert!(app.ui.status.contains("closes once the page has stored it"), "{}", app.ui.status);
        posted(&mut app, 4);
        // Answers to other saves, or of another file, close nothing; nor does a save of another file.
        host_saved(&mut app, 3, "Logo.vectorcraft", true, None);
        host_saved(&mut app, 4, "Other.vectorcraft", true, None);
        host_posted(&mut app, 5, "Other.png");
        assert_eq!(docs(&app), 1);
        // A later Save of the file is the one waited for.
        host_posted(&mut app, 6, "Logo.vectorcraft");
        host_saved(&mut app, 4, "Logo.vectorcraft", true, None);
        assert_eq!(docs(&app), 1);
        host_saved(&mut app, 6, "Logo.vectorcraft", true, None);
        assert_eq!((docs(&app), app.views.len(), app.ui.host_closing.is_none()), (0, 0, true));
    }

    #[test]
    fn with_save_on_close_a_save_the_page_could_not_store_keeps_the_document_open_and_modified() {
        let (mut app, handed) = hosted();
        named_doc(&mut app, "Logo.vectorcraft");
        app.run("file.close", json!({})).unwrap();
        // The save counts as done once handed over, as a download does…
        assert!(!any_dirty_document(&app));
        posted(&mut app, 2);
        host_saved(&mut app, 2, "Logo.vectorcraft", false, Some("the server is offline"));
        // …until the page says it wasn't stored.
        assert_eq!(docs(&app), 1);
        assert!(app.session.documents()[0].is_dirty());
        assert!(app.ui.status.contains("couldn't store Logo.vectorcraft") && app.ui.status.contains("the server is offline"), "{}", app.ui.status);
        // No answer at all leaves it open just the same; closing again saves again.
        handed.lock().unwrap().clear();
        app.run("file.close", json!({})).unwrap();
        assert_eq!((docs(&app), handed.lock().unwrap().len()), (1, 1));
    }

    #[test]
    fn with_save_on_close_a_document_edited_after_its_save_stays_open() {
        let (mut app, _) = hosted();
        named_doc(&mut app, "Logo.vectorcraft");
        app.run("file.close", json!({})).unwrap();
        posted(&mut app, 1);
        app.run("shape.rectangle", json!({ "x": 20, "y": 0, "width": 10, "height": 10 })).unwrap();
        host_saved(&mut app, 1, "Logo.vectorcraft", true, None);
        assert_eq!(docs(&app), 1);
        assert!(app.session.documents()[0].is_dirty());
        assert!(app.ui.status.contains("changed after it was saved"), "{}", app.ui.status);
    }

    #[test]
    fn while_a_save_waits_for_the_page_nothing_else_closes_that_document() {
        let (mut app, handed) = hosted();
        new_doc(&mut app, false);
        named_doc(&mut app, "Logo.vectorcraft");
        app.run("file.close", json!({ "index": 1 })).unwrap();
        handed.lock().unwrap().clear();
        // Saved and handed over, but not stored yet: closing it again, Close All, Quit and the
        // window's close button wait.
        assert!(app.run("file.close", json!({ "index": 1 })).unwrap_err().contains("closes once the page has stored it"));
        assert!(app.run("file.closeAll", json!({})).is_err());
        assert!(app.run("app.quit", json!({})).is_err());
        assert!(any_dirty(&app));
        assert!(handed.lock().unwrap().is_empty());
        // Another document with nothing unsaved closes.
        app.run("file.close", json!({ "index": 0 })).unwrap();
        assert_eq!(docs(&app), 1);
        // Discarding it ends the wait: a late answer finds nothing to close.
        app.run("file.close", json!({ "discard": true })).unwrap();
        assert_eq!((docs(&app), app.ui.host_closing.is_none()), (0, true));
        host_saved(&mut app, 1, "Logo.vectorcraft", true, None);
    }

    #[test]
    fn with_save_on_close_close_all_and_quit_save_each_modified_document_in_turn() {
        let (mut app, handed) = hosted();
        new_doc(&mut app, false);
        named_doc(&mut app, "A.vectorcraft");
        named_doc(&mut app, "B.vectorcraft");
        handed.lock().unwrap().clear();
        app.run("file.closeAll", json!({})).unwrap();
        assert_eq!((handed.lock().unwrap().clone(), docs(&app)), (vec!["A.vectorcraft".to_string()], 3));
        posted(&mut app, 1);
        host_saved(&mut app, 1, "A.vectorcraft", true, None);
        // A closed; then B is saved and waited for.
        assert_eq!((handed.lock().unwrap().len(), docs(&app)), (2, 2));
        posted(&mut app, 2);
        host_saved(&mut app, 2, "B.vectorcraft", true, None);
        // Then the rest, which has nothing unsaved, closes.
        assert_eq!(docs(&app), 0);
        // Quit: a save the page couldn't store stops it.
        named_doc(&mut app, "C.vectorcraft");
        app.run("app.quit", json!({})).unwrap();
        posted(&mut app, 3);
        host_saved(&mut app, 3, "C.vectorcraft", false, None);
        assert_ne!(app.ui.status, "quit");
        assert_eq!(docs(&app), 1);
        app.run("app.quit", json!({})).unwrap();
        posted(&mut app, 4);
        host_saved(&mut app, 4, "C.vectorcraft", true, None);
        assert_eq!((docs(&app), app.ui.status.as_str()), (0, "quit"));
    }

    #[test]
    fn discard_closes_a_modified_document_unsaved_without_asking() {
        let (mut app, written) = app();
        new_doc(&mut app, true);
        new_doc(&mut app, true);
        // Asked about the second, then discarded by the caller: the question goes with it.
        app.run("file.close", json!({ "index": 1 })).unwrap();
        assert_eq!(pending(&app), Some(1));
        assert_eq!(app.run("file.close", json!({ "index": 1, "discard": true })).unwrap(), Value::Null);
        assert_eq!((docs(&app), app.ui.dialog.is_none()), (1, true));
        // From the control channel, as a host or an agent sends it.
        let ctx = egui::Context::default();
        let execute = |app: &mut VectorcraftApp, params: Value| {
            let (req, _rx) = crate::control::ControlRequest::new("engine.execute", json!({ "command": "file.close", "params": params }));
            let crate::control::Outcome::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("not done") };
            r
        };
        assert_eq!(execute(&mut app, json!({ "discard": "yes" }))["ok"], false);
        assert_eq!(execute(&mut app, json!({ "discard": true })), json!({ "ok": true, "result": null }));
        assert_eq!(docs(&app), 0);
        assert!(written.lock().unwrap().is_empty());
        // `discard: false` asks as before.
        new_doc(&mut app, true);
        assert_eq!(app.run("file.close", json!({ "discard": false })).unwrap(), json!({ "pending": KIND }));
    }

    fn any_dirty_document(app: &VectorcraftApp) -> bool {
        app.session.documents().iter().any(|d| d.is_dirty())
    }

    #[test]
    fn the_control_channels_quit_says_when_it_asks_first() {
        // #830: it answered null at once, before the question.
        let (mut app, _) = app();
        let ctx = egui::Context::default();
        let quit = |app: &mut VectorcraftApp| {
            let (req, _rx) = crate::control::ControlRequest::new("app.quit", json!({}));
            let crate::control::Outcome::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("not done") };
            r
        };
        new_doc(&mut app, true);
        assert_eq!(quit(&mut app), json!({"ok": true, "result": {"pending": KIND}}));
        assert_eq!((pending(&app), app.ui.status.as_str()), (Some(0), ""));
        app.ui.dialog = None;
        app.run("file.save", json!({"path": "/docs/saved.vectorcraft"})).unwrap();
        assert_eq!(quit(&mut app), json!({"ok": true, "result": null}));
        assert_eq!(app.ui.status, "quit");
    }
}
