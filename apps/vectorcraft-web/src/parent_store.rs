//! Data Recovery's store when the parent page keeps the copies (`?host=parent&recovery=parent`,
//! `docs/web-embed.md`): the entries are held here, in memory, and every change is posted to the
//! parent, which stores it where it likes (its own server, say) and gives the entries back to the
//! next page that frames VectorCraft (`vectorcraft:recoveryEntries`). The store has to answer at
//! once, which a page's storage reached by messages can't, so this copy of it answers; the parent
//! only ever hears what changed. Kept free of browser types so it is tested natively.
//!
//! The parent may give entries several times (each deck a page opens, say): they join those held,
//! and Data Recovery offers the copies among them that a crash left behind. An entry the parent
//! gives that this app also wrote is replaced by the parent's, which is the same unless another
//! app changed it since.

use std::collections::BTreeMap;
use std::sync::Mutex;

use vectorcraft_engine::cmd::recovery::RecoveryStore;

/// A change to the entries, posted to the parent.
#[derive(Debug, PartialEq, Eq)]
pub enum Change<'a> {
    /// `vectorcraft:recoveryWrite { name, bytes }`: create or replace entry `name`.
    Write { name: &'a str, bytes: &'a [u8] },
    /// `vectorcraft:recoveryRemove { name }`: delete entry `name`.
    Remove { name: &'a str },
}

/// Posts a change to the parent → whether it was handed over.
pub type Post = Box<dyn Fn(Change) -> Result<(), String> + Send + Sync>;

/// The clock heartbeats are judged by (Unix seconds).
pub type Clock = Box<dyn Fn() -> Option<i64> + Send + Sync>;

pub struct ParentStore {
    entries: Mutex<BTreeMap<String, Vec<u8>>>,
    post: Post,
    clock: Clock,
}

impl ParentStore {
    pub fn new(post: Post, clock: Clock) -> Self {
        Self { entries: Mutex::default(), post, clock }
    }

    /// Entries the parent gives (`vectorcraft:recoveryEntries`), joining those held.
    pub fn give(&self, entries: impl IntoIterator<Item = (String, Vec<u8>)>) {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).extend(entries);
    }
}

impl RecoveryStore for ParentStore {
    fn list(&self) -> Result<Vec<String>, String> {
        Ok(self.entries.lock().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect())
    }
    fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).get(name).cloned().ok_or_else(|| format!("{name}: no such recovery entry"))
    }
    /// Kept only once the parent has it: a write it never heard of would be missing after a crash.
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        (self.post)(Change::Write { name, bytes }).map_err(|e| format!("couldn't hand {name} to the page: {e}"))?;
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), bytes.to_vec());
        Ok(())
    }
    fn remove(&self, name: &str) -> Result<(), String> {
        let held = self.entries.lock().unwrap_or_else(|e| e.into_inner()).remove(name).is_some();
        // The parent may hold an entry this app never gave nor was given: it hears every remove.
        let posted = (self.post)(Change::Remove { name });
        // Best effort for one this app didn't hold: removing what isn't there is no error.
        if held { posted } else { Ok(()) }
    }
    fn location(&self) -> String {
        "the page".into()
    }
    fn now(&self) -> Option<i64> {
        (self.clock)()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;

    /// What the parent heard, in order: `+name` for a write, `-name` for a remove, and the
    /// entries it keeps.
    #[derive(Default)]
    struct Page {
        heard: Mutex<Vec<String>>,
        kept: Mutex<BTreeMap<String, Vec<u8>>>,
        refuses: Mutex<bool>,
    }

    fn store(page: &Arc<Page>, now: i64) -> Arc<ParentStore> {
        let p = page.clone();
        let post: Post = Box::new(move |change| {
            if *p.refuses.lock().unwrap() {
                return Err("the page is gone".into());
            }
            let mut kept = p.kept.lock().unwrap();
            let mut heard = p.heard.lock().unwrap();
            match change {
                Change::Write { name, bytes } => {
                    heard.push(format!("+{name}"));
                    kept.insert(name.to_string(), bytes.to_vec());
                }
                Change::Remove { name } => {
                    heard.push(format!("-{name}"));
                    kept.remove(name);
                }
            }
            Ok(())
        });
        Arc::new(ParentStore::new(post, Box::new(move || Some(now))))
    }

    fn modified(store: &Arc<ParentStore>) -> Session {
        let mut s = Session::new();
        s.recovery.set_store(store.clone());
        s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
        s
    }

    fn offered(s: &mut Session) -> Vec<String> {
        let l = s.execute("file.recovery.list", &json!({})).unwrap();
        l["copies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["open"] == false && c["running"] == false)
            .map(|c| c["title"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn every_copy_written_reaches_the_parent() {
        let page = Arc::new(Page::default());
        let mut s = modified(&store(&page, 1_000_000));
        s.execute("file.recovery.save", &json!({})).unwrap();
        let heard = page.heard.lock().unwrap().clone();
        assert!(heard.iter().any(|h| h.starts_with('+') && h.ends_with("/heartbeat")), "{heard:?}");
        assert!(heard.iter().any(|h| h.starts_with('+') && h.ends_with(".vectorcraft")), "{heard:?}");
        assert!(heard.iter().any(|h| h.starts_with('+') && h.ends_with(".json")), "{heard:?}");
        assert_eq!(page.kept.lock().unwrap().len(), 3);
    }

    #[test]
    fn a_crash_leaves_copies_the_next_page_offers_from_the_parents_entries() {
        let page = Arc::new(Page::default());
        let mut s = modified(&store(&page, 1_000_000));
        s.execute("file.recovery.save", &json!({})).unwrap();
        drop(s);
        // The next page, once the heartbeat is stale: nothing is offered until the parent gives
        // the entries back.
        let next = store(&page, 1_000_000 + 600);
        let mut t = Session::new();
        t.recovery.set_store(next.clone());
        assert!(offered(&mut t).is_empty());
        next.give(page.kept.lock().unwrap().clone());
        assert_eq!(offered(&mut t).len(), 1);
        let r = t.execute("file.recovery.restore", &json!({})).unwrap();
        assert_eq!(r["restored"].as_array().unwrap().len(), 1);
        // The copy moved into the new app's area, at the parent too.
        assert_eq!(page.kept.lock().unwrap().keys().filter(|n| n.ends_with(".vectorcraft")).count(), 1);
        assert!(offered(&mut t).is_empty());
    }

    #[test]
    fn a_running_apps_copies_are_not_offered() {
        let page = Arc::new(Page::default());
        let mut s = modified(&store(&page, 1_000_000));
        s.execute("file.recovery.save", &json!({})).unwrap();
        // Another page within the heartbeat: the app is running.
        let other = store(&page, 1_000_000 + 60);
        other.give(page.kept.lock().unwrap().clone());
        let mut t = Session::new();
        t.recovery.set_store(other);
        assert!(offered(&mut t).is_empty());
    }

    #[test]
    fn saving_the_document_removes_its_copy_at_the_parent() {
        let page = Arc::new(Page::default());
        let mut s = modified(&store(&page, 1_000_000));
        s.execute("file.recovery.save", &json!({})).unwrap();
        s.execute("file.close", &json!({"discard": true})).unwrap();
        let kept = page.kept.lock().unwrap();
        assert!(!kept.keys().any(|n| n.ends_with(".vectorcraft") || n.ends_with(".json")), "{kept:?}");
    }

    #[test]
    fn a_write_the_parent_did_not_take_fails_and_is_not_kept() {
        let page = Arc::new(Page::default());
        let st = store(&page, 1_000_000);
        *page.refuses.lock().unwrap() = true;
        let err = st.write("1-1/x.json", b"{}").unwrap_err();
        assert!(err.contains("couldn't hand 1-1/x.json to the page"), "{err}");
        assert!(st.list().unwrap().is_empty());
    }

    #[test]
    fn removing_an_entry_never_held_still_tells_the_parent() {
        let page = Arc::new(Page::default());
        let st = store(&page, 1_000_000);
        st.remove("1-1/gone.json").unwrap();
        assert_eq!(*page.heard.lock().unwrap(), vec!["-1-1/gone.json".to_string()]);
    }
}
