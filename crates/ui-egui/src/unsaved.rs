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
/// what continues after this document) and `discard` (Don't Save). With `then` = `close`, the
/// protected field [`FRONT`] holds the uid of the document that was in front before the question.
pub const KIND: &str = "saveChanges";

/// Field of a [`KIND`] dialog asked by File → Close or a tab's ×: the uid of the document in front
/// before the question showed the asked one. Once the asked document closes, that document is in
/// front again while it is still open.
const FRONT: &str = "__front";

/// Nothing closes while a file dialog's sheet is on the window (macOS): what asked for it runs
/// again when it answers. (Elsewhere the dialog is a window of its own, and closing goes on.)
pub(crate) fn refuse_while_picking(app: &VectorcraftApp) -> Result<(), String> {
    if app.file_sheet_open() { Err(crate::picks::busy().into()) } else { Ok(()) }
}

/// Finishes the work that decides whether there is anything left to save: the saves running in
/// the background, and an edit the active tool is computing on another thread (a Shape Builder
/// merge), which is applied.
pub fn finish_pending(app: &mut VectorcraftApp) {
    crate::background::wait_all(app);
    let r = app.session.finish_tool_edit();
    crate::canvas::apply_requests(app, r);
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
    /// of the same document takes its place (and its `name`), since the page's answer to that one
    /// is what counts. A save of another document under the same file name never does.
    pub save: Option<u64>,
    /// For a single close (`then` = `close`) of a document that wasn't in front: the document in
    /// front before, in front again once this one closes, as after the Save Changes question.
    pub front: Option<u64>,
}

/// The most documents whose newest save to the embedding page is remembered ([`host_posted`]).
const MAX_HOST_SAVES: usize = 64;

/// A file the host handed to the embedding page (the web build's `vectorcraft:save`): the page's
/// id for it, its file name, and the document whose own save (Save, Save As) it was; none for a
/// copy or an export, whose answers mark no document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostedSave {
    pub id: u64,
    pub name: String,
    pub doc: Option<u64>,
}

/// The embedding page's answer to save `id` of the file `name` (`vectorcraft:saved`): stored
/// (`ok`), or not, with the page's `error`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostAnswer {
    pub id: u64,
    pub name: String,
    pub ok: bool,
    pub error: Option<String>,
}

thread_local! {
    /// The document whose own save is being written now ([`saving_document`]).
    static SAVING: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// The document ([`vectorcraft_engine::DocState::uid`]) whose own save (Save, Save As) is being
/// written while a host's download runs: a host that hands its files to a page asks, so the page's
/// answer goes to that document and no other of the same file name. None for a copy or an export.
pub fn saving_document() -> Option<u64> {
    SAVING.with(std::cell::Cell::get)
}

/// `f` run as the writing of document `uid`'s own save ([`saving_document`]).
pub(crate) fn writing_document<R>(uid: u64, f: impl FnOnce() -> R) -> R {
    let before = SAVING.with(|s| s.replace(Some(uid)));
    let r = f();
    SAVING.with(|s| s.set(before));
    r
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
    finish_pending(app);
    let doc = app.session.documents().get(i).ok_or("no such document")?;
    let (dirty, uid) = (doc.is_dirty(), doc.uid);
    // Closed again while it waits for the page: saved again, and the close waits for that save
    // (the page may never answer the first).
    if app.ui.host_closing.as_ref().is_some_and(|c| c.uid == uid) {
        return save_for_host(app, i, "close");
    }
    // Another document, saved and stored, may close while one waits; a modified one may not.
    if dirty {
        refuse_while_waiting(app)?;
    }
    if !dirty {
        close_now(app, i, None)
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
    close_now(app, i, None)?;
    Ok(Value::Null)
}

/// File → Close All (`then` = `closeAll`) or Quit (`quit`): ask about the next modified document,
/// or finish once none is left.
pub fn close_all(app: &mut VectorcraftApp, then: &str) -> Result<Value, String> {
    refuse_while_picking(app)?;
    finish_pending(app);
    // A document waiting for the page is saved again first, and the close waits for that save.
    if let Some(uid) = app.ui.host_closing.as_ref().map(|c| c.uid) {
        match app.session.documents().iter().position(|d| d.uid == uid) {
            Some(i) => return save_for_host(app, i, then),
            None => app.ui.host_closing = None,
        }
    }
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
    let uid = app.session.documents().get(i).ok_or("no such document")?.uid;
    // The document in front before: remembered by a close already waiting for this one.
    let front = match app.ui.host_closing.as_ref().filter(|c| c.uid == uid) {
        Some(c) => c.front,
        None => app.session.active().map(|d| d.uid).filter(|f| *f != uid),
    }
    .filter(|_| then == "close");
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
    app.ui.host_closing = Some(HostClose { uid: doc.uid, name: name.clone(), then: then.into(), save: None, front });
    app.status(format!("Saving {name}: it closes once the page has stored it"));
    Ok(json!({ "pending": HOST_SAVE, "name": name }))
}

/// The saves the host handed the page, then the page's answers, in that order: an answer counts
/// only once its save is known, so a host hears the saves it posted before the answers it heard,
/// whichever pass of a frame posted them (a hidden browser tab runs no UI pass).
pub fn host_exchange(app: &mut VectorcraftApp, posted: impl IntoIterator<Item = PostedSave>, answers: impl IntoIterator<Item = HostAnswer>) {
    for save in posted {
        host_posted(app, save);
    }
    for a in answers {
        host_saved(app, a.id, &a.name, a.ok, a.error.as_deref());
    }
}

/// The host handed the page `save`. A document's own save is that document's newest, whose answer
/// counts ([`host_saved`]), and a close waiting for that document waits for it from now on; a copy
/// or an export is not remembered.
pub fn host_posted(app: &mut VectorcraftApp, save: PostedSave) {
    let Some(doc) = save.doc else { return };
    let open: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    let saves = &mut app.ui.host_saves;
    // One per document, and none of a closed one.
    saves.retain(|s| s.doc.is_some_and(|d| d != doc && open.contains(&d)));
    if saves.len() >= MAX_HOST_SAVES {
        saves.remove(0);
    }
    saves.push(save.clone());
    if let Some(c) = app.ui.host_closing.as_mut().filter(|c| c.uid == doc) {
        c.save = Some(save.id);
        c.name = save.name;
    }
}

/// The embedding page's answer to its save `id` of the file `name`: stored (`ok`), or not, with
/// the page's `error`. The answer to the save a close waits for closes the document and goes on
/// with Close All or Quit; if it wasn't stored, the document stays open, counts as modified again
/// and the status line says why. A document edited after the save stays open too. Any other
/// answer that a document's newest save wasn't stored marks that document modified again and says
/// why, as a close's would; the rest change nothing.
pub fn host_saved(app: &mut VectorcraftApp, id: u64, name: &str, ok: bool, error: Option<&str>) {
    let newest = app.ui.host_saves.iter().position(|s| s.id == id && s.name == name);
    let doc = newest.map(|at| app.ui.host_saves.remove(at)).and_then(|s| s.doc);
    if app.ui.host_closing.as_ref().is_none_or(|c| c.save != Some(id) || c.name != name) {
        if let Some(uid) = doc.filter(|_| !ok) {
            if let Some(d) = app.session.document_mut(uid).filter(|d| !d.is_dirty()) {
                d.mark_unsaved();
            }
            app.status(match error.map(str::trim).filter(|e| !e.is_empty()) {
                Some(e) => format!("The page couldn't store {name}: {e}"),
                None => format!("The page couldn't store {name}"),
            });
        }
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
    // The document in front now stays there; one shown only to be saved gives way to the one
    // in front before it.
    let front = app.session.active().map(|d| d.uid).filter(|f| *f != c.uid).or(c.front);
    let r = close_now(app, i, front).and_then(|_| if c.then == "close" { Ok(Value::Null) } else { close_all(app, &c.then) });
    if let Err(e) = r {
        app.status(e);
    }
}

/// Show document `i` and ask whether to save it. For a single close (`then` = `close`), the
/// dialog also remembers the document in front before it ([`FRONT`]).
fn ask(app: &mut VectorcraftApp, i: usize, then: &str) -> Result<Value, String> {
    let front = app.session.active().map(|d| d.uid);
    app.session.set_active(i);
    let doc = &app.session.documents()[i];
    let (name, uid) = (doc.title(), doc.uid);
    let mut fields = json!({ "index": i, "uid": uid, "name": name, "then": then });
    if let Some(front) = front.filter(|f| then == "close" && *f != uid) {
        fields[FRONT] = json!(front);
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(json!({ "pending": KIND }))
}

/// Close document `i` through the engine command; [`VectorcraftApp::sync_views`] drops its view.
/// When `front` names a document that is still open, that document is in front afterwards.
fn close_now(app: &mut VectorcraftApp, i: usize, front: Option<u64>) -> Result<Value, String> {
    let r = app.session.execute("file.close", &json!({ "index": i })).map_err(|e| e.to_string())?;
    if let Some(j) = front.and_then(|uid| app.session.documents().iter().position(|d| d.uid == uid)) {
        app.session.set_active(j);
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
    close_now(app, i, d.fields.get(FRONT).and_then(Value::as_u64))?;
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

    /// Three documents, A clean, B modified and C clean, with A in front: their uids.
    fn front_and_modified_background_tab(app: &mut VectorcraftApp) -> [u64; 3] {
        new_doc(app, false);
        new_doc(app, true);
        new_doc(app, false);
        app.session.set_active(0);
        let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
        [uids[0], uids[1], uids[2]]
    }

    fn front_uid(app: &VectorcraftApp) -> Option<u64> {
        app.session.active().map(|d| d.uid)
    }

    #[test]
    fn closing_a_modified_background_tab_keeps_the_front_document_after_dont_save() {
        let (mut app, written) = app();
        let [a, _, c] = front_and_modified_background_tab(&mut app);
        assert_eq!(app.run("file.close", json!({ "index": 1 })).unwrap(), json!({ "pending": KIND }));
        // The question shows the document it asks about.
        assert_eq!((pending(&app), app.session.active_index()), (Some(1), Some(1)));
        app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
        confirm(&mut app).unwrap();
        let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
        assert_eq!((uids, front_uid(&app), app.views.len()), (vec![a, c], Some(a), 2));
        assert!(written.lock().unwrap().is_empty());
    }

    #[test]
    fn closing_a_modified_background_tab_keeps_the_front_document_after_save() {
        let (mut app, written) = app();
        let [a, _, c] = front_and_modified_background_tab(&mut app);
        close(&mut app, 1).unwrap();
        confirm(&mut app).unwrap();
        assert_eq!(*written.lock().unwrap(), vec!["out.vectorcraft".to_string()]);
        let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
        assert_eq!((uids, front_uid(&app)), (vec![a, c], Some(a)));
    }

    #[test]
    fn cancel_keeps_the_asked_background_tab_in_front() {
        let (mut app, _) = app();
        let [_, b, _] = front_and_modified_background_tab(&mut app);
        close(&mut app, 1).unwrap();
        app.ui.dialog = None;
        assert_eq!((docs(&app), front_uid(&app)), (3, Some(b)));
        // A later close of the tab in front brings its neighbor to the front.
        close(&mut app, 1).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
        confirm(&mut app).unwrap();
        assert_eq!((docs(&app), app.session.active_index()), (2, Some(1)));
    }

    #[test]
    fn closing_the_modified_front_document_brings_its_neighbor_to_the_front() {
        let (mut app, _) = app();
        new_doc(&mut app, false);
        new_doc(&mut app, true);
        new_doc(&mut app, false);
        let c = app.session.documents()[2].uid;
        app.session.set_active(1);
        close(&mut app, 1).unwrap();
        assert!(app.ui.dialog.as_ref().is_some_and(|d| !d.fields.contains_key(FRONT)));
        app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
        confirm(&mut app).unwrap();
        assert_eq!((docs(&app), front_uid(&app)), (2, Some(c)));
    }

    /// The document in front before the question is remembered by its uid: when it closed while
    /// the question was open, the closed document's neighbor comes to the front as before.
    #[test]
    fn a_front_document_closed_meanwhile_is_not_brought_back() {
        let (mut app, _) = app();
        let [a, _, c] = front_and_modified_background_tab(&mut app);
        close(&mut app, 1).unwrap();
        let d = app.ui.dialog.take().unwrap();
        assert_eq!(d.fields.get(FRONT), Some(&json!(a)));
        close(&mut app, 0).unwrap();
        app.ui.dialog = Some(d);
        app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
        confirm(&mut app).unwrap();
        assert_eq!((docs(&app), front_uid(&app)), (1, Some(c)));
    }

    #[test]
    fn close_all_does_not_remember_a_front_document() {
        let (mut app, _) = app();
        front_and_modified_background_tab(&mut app);
        app.run("file.closeAll", json!({})).unwrap();
        assert!(app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND && !d.fields.contains_key(FRONT)));
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

    /// What the page was handed: each file's name and the document whose own save it was, as the
    /// web build's bridge posts them.
    type Handed = Arc<Mutex<Vec<(String, Option<u64>)>>>;

    /// An app embedded by a page that stores its saves (`saveOnClose`): every save is handed over
    /// by name, as the web build's downloads are, with the document it saved.
    fn hosted() -> (VectorcraftApp, Handed) {
        let handed: Handed = Arc::new(Mutex::new(vec![]));
        let h = handed.clone();
        let download: crate::DownloadFn = Box::new(move |name: &str, _: &[u8]| {
            h.lock().unwrap().push((name.to_string(), saving_document()));
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

    /// The names of the files handed over.
    fn names(handed: &Handed) -> Vec<String> {
        handed.lock().unwrap().iter().map(|(n, _)| n.clone()).collect()
    }

    /// What the host does once it has posted the save a close made: the save's id, with its file
    /// name and document.
    fn posted(app: &mut VectorcraftApp, id: u64) {
        let c = app.ui.host_closing.as_ref().unwrap();
        let save = PostedSave { id, name: c.name.clone(), doc: Some(c.uid) };
        host_posted(app, save);
    }

    /// What the host does once it has posted the last file handed over: save `id`, as the bridge
    /// posts it.
    fn post_last(app: &mut VectorcraftApp, handed: &Handed, id: u64) {
        let (name, doc) = handed.lock().unwrap().last().cloned().unwrap();
        host_posted(app, PostedSave { id, name, doc });
    }

    #[test]
    fn with_save_on_close_a_modified_document_closes_once_the_page_stored_its_save() {
        let (mut app, handed) = hosted();
        named_doc(&mut app, "Logo.vectorcraft");
        handed.lock().unwrap().clear();
        // No question: the document is saved as Save saves it, and stays open.
        assert_eq!(app.run("file.close", json!({})).unwrap(), json!({ "pending": HOST_SAVE, "name": "Logo.vectorcraft" }));
        assert_eq!(names(&handed), ["Logo.vectorcraft"]);
        assert!(app.ui.dialog.is_none());
        assert_eq!(docs(&app), 1);
        assert!(app.ui.status.contains("closes once the page has stored it"), "{}", app.ui.status);
        posted(&mut app, 4);
        // Answers to other saves, or of another file, close nothing; nor does a save of another file.
        host_saved(&mut app, 3, "Logo.vectorcraft", true, None);
        host_saved(&mut app, 4, "Other.vectorcraft", true, None);
        host_posted(&mut app, PostedSave { id: 5, name: "Other.png".into(), doc: None });
        assert_eq!(docs(&app), 1);
        // A later Save of the document is the one waited for.
        app.run("file.save", json!({})).unwrap();
        post_last(&mut app, &handed, 6);
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
        named_doc(&mut app, "Other.vectorcraft");
        app.run("file.close", json!({ "index": 1 })).unwrap();
        handed.lock().unwrap().clear();
        // Saved and handed over, but not stored yet: another modified document waits, and the
        // window's close button asks.
        assert!(app.run("file.close", json!({ "index": 2 })).unwrap_err().contains("closes once the page has stored it"));
        assert!(any_dirty(&app));
        assert!(handed.lock().unwrap().is_empty());
        app.run("file.close", json!({ "index": 2, "discard": true })).unwrap();
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
        assert_eq!((names(&handed), docs(&app)), (vec!["A.vectorcraft".to_string()], 3));
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

    /// A plain Save the page couldn't store leaves the document modified, as a close's does: only
    /// the answer to the newest save of the file counts, and a failed export marks no document.
    #[test]
    fn a_plain_save_the_page_could_not_store_leaves_the_document_modified() {
        let (mut app, handed) = hosted();
        named_doc(&mut app, "Logo.vectorcraft");
        app.run("file.save", json!({})).unwrap();
        post_last(&mut app, &handed, 1);
        assert!(!any_dirty_document(&app));
        // A later save of the file is the one whose answer counts.
        app.run("file.save", json!({})).unwrap();
        post_last(&mut app, &handed, 2);
        host_saved(&mut app, 1, "Logo.vectorcraft", false, Some("stale"));
        assert!(!any_dirty_document(&app), "an older save's answer changes nothing");
        host_posted(&mut app, PostedSave { id: 3, name: "Logo.png".into(), doc: None });
        host_saved(&mut app, 3, "Logo.png", false, None);
        assert!(!any_dirty_document(&app), "a failed export marks no document");
        host_saved(&mut app, 2, "Logo.vectorcraft", false, Some("the server is offline"));
        assert!(app.session.documents()[0].is_dirty());
        assert!(app.ui.status.contains("couldn't store Logo.vectorcraft") && app.ui.status.contains("the server is offline"), "{}", app.ui.status);
        // Stored: nothing changes.
        app.run("file.save", json!({})).unwrap();
        post_last(&mut app, &handed, 4);
        host_saved(&mut app, 4, "Logo.vectorcraft", true, None);
        assert!(!any_dirty_document(&app));
    }

    /// A save the page heard of and answered between two passes of the app (a hidden tab's, which
    /// run no UI pass): the save is heard first, so its answer counts.
    #[test]
    fn a_save_answered_before_the_app_heard_it_was_posted_still_counts() {
        let (mut app, handed) = hosted();
        named_doc(&mut app, "Logo.vectorcraft");
        app.run("file.close", json!({})).unwrap();
        let (name, doc) = handed.lock().unwrap().last().cloned().unwrap();
        let answer = HostAnswer { id: 1, name: name.clone(), ok: true, error: None };
        host_exchange(&mut app, [PostedSave { id: 1, name, doc }], [answer]);
        assert_eq!((docs(&app), app.ui.host_closing.is_none()), (0, true));
        // A plain Save that failed, heard the same way, leaves its document modified.
        named_doc(&mut app, "Logo.vectorcraft");
        app.run("file.save", json!({})).unwrap();
        let (name, doc) = handed.lock().unwrap().last().cloned().unwrap();
        let answer = HostAnswer { id: 2, name: name.clone(), ok: false, error: None };
        host_exchange(&mut app, [PostedSave { id: 2, name, doc }], [answer]);
        assert!(any_dirty_document(&app));
    }

    /// Two documents saved under one file name (from two folders): the answer to one document's
    /// save never closes or marks the other.
    #[test]
    fn a_save_of_another_document_with_the_same_name_is_not_the_one_waited_for() {
        let (mut app, handed) = hosted();
        new_doc(&mut app, false);
        app.run("file.save", json!({ "path": "/a/Logo.vectorcraft" })).unwrap();
        app.run("shape.rectangle", json!({ "x": 0, "y": 0, "width": 10, "height": 10 })).unwrap();
        new_doc(&mut app, false);
        app.run("file.save", json!({ "path": "/b/Logo.vectorcraft" })).unwrap();
        let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
        // The first closes: saved and handed over, waiting for the page.
        app.run("file.close", json!({ "index": 0 })).unwrap();
        post_last(&mut app, &handed, 1);
        // The other is saved under the same name, and stored: the first still waits.
        app.session.set_active(1);
        app.run("file.save", json!({})).unwrap();
        assert_eq!(handed.lock().unwrap().last().cloned().unwrap(), ("Logo.vectorcraft".to_string(), Some(uids[1])));
        post_last(&mut app, &handed, 2);
        host_saved(&mut app, 2, "Logo.vectorcraft", true, None);
        assert_eq!(docs(&app), 2, "the other document's save closes nothing");
        // Its own answer closes it.
        host_saved(&mut app, 1, "Logo.vectorcraft", true, None);
        let left: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
        assert_eq!(left, vec![uids[1]]);
        // A failed save of the one left marks it alone.
        app.run("file.save", json!({})).unwrap();
        post_last(&mut app, &handed, 3);
        host_saved(&mut app, 3, "Logo.vectorcraft", false, None);
        assert!(app.session.documents()[0].is_dirty());
    }

    /// Many exports handed to the page (Export for Screens' files) don't push a document's save out
    /// of what is remembered: a failed save is still heard.
    #[test]
    fn many_exports_leave_a_documents_save_remembered() {
        let (mut app, handed) = hosted();
        named_doc(&mut app, "Logo.vectorcraft");
        app.run("file.save", json!({})).unwrap();
        post_last(&mut app, &handed, 1);
        for id in 2..200 {
            host_posted(&mut app, PostedSave { id, name: format!("Logo-{id}.png"), doc: None });
        }
        host_saved(&mut app, 1, "Logo.vectorcraft", false, Some("the server is offline"));
        assert!(app.session.documents()[0].is_dirty());
    }

    /// A page that never answers doesn't hold the document open for good: closing it again saves
    /// it again and waits for that save; Close All and Quit do the same.
    #[test]
    fn closing_again_while_the_page_has_not_answered_saves_again() {
        let (mut app, handed) = hosted();
        named_doc(&mut app, "Logo.vectorcraft");
        app.run("file.close", json!({})).unwrap();
        posted(&mut app, 1);
        handed.lock().unwrap().clear();
        assert_eq!(app.run("file.close", json!({})).unwrap()["pending"], HOST_SAVE);
        assert_eq!(handed.lock().unwrap().len(), 1, "saved again");
        posted(&mut app, 2);
        host_saved(&mut app, 1, "Logo.vectorcraft", true, None);
        assert_eq!(docs(&app), 1, "the first save's answer no longer counts");
        app.run("app.quit", json!({})).unwrap();
        assert_eq!(handed.lock().unwrap().len(), 2, "Quit saves it again too");
        posted(&mut app, 3);
        host_saved(&mut app, 3, "Logo.vectorcraft", true, None);
        assert_eq!((docs(&app), app.ui.status.as_str()), (0, "quit"));
    }

    /// Closing a modified background tab with `saveOnClose` keeps the document in front, as the
    /// Save Changes question does.
    #[test]
    fn with_save_on_close_closing_a_background_tab_keeps_the_front_document() {
        let (mut app, _) = hosted();
        new_doc(&mut app, false);
        named_doc(&mut app, "B.vectorcraft");
        new_doc(&mut app, false);
        app.session.set_active(0);
        let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
        app.run("file.close", json!({ "index": 1 })).unwrap();
        posted(&mut app, 1);
        host_saved(&mut app, 1, "B.vectorcraft", true, None);
        let left: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
        assert_eq!((left, front_uid(&app)), (vec![uids[0], uids[2]], Some(uids[0])));
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
