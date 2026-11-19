//! The faces of a host's font indexes the shell fetched, settled with the font database
//! (RFC-0034): each one's fetch, done or failed, is handed to [`FontDb::settle`] once. A face the
//! host's newer set no longer lists is stale and dropped quietly: it is no failure, so it reaches
//! neither the status line nor Missing Fonts.

use vectorcraft_text::{FontDb, FontSource, Settled};

/// A face whose fetch ended: its file's bytes, or why it couldn't be fetched.
pub type Fetched = (FontSource, Result<Vec<u8>, String>);

/// What settling a frame's fetches did: whether any face was added (the type is then laid out
/// again), and the faces refused, with why, for the status line and Missing Fonts.
#[derive(Debug, Default, PartialEq)]
pub struct SettledFetches {
    pub added: bool,
    pub refused: Vec<(FontSource, String)>,
}

/// Settle each face the shell fetched, or failed to, with `db` ([`FontDb::settle`]).
pub fn settle_fetches(db: &FontDb, fetched: Vec<Fetched>) -> SettledFetches {
    let mut out = SettledFetches::default();
    for (src, result) in fetched {
        match db.settle(&src, result) {
            Settled::Added(file) => out.added |= !file.added.is_empty(),
            Settled::Refused(why) => out.refused.push((src, why)),
            Settled::Stale => {}
        }
    }
    out
}

/// Whether the shell asks for another frame to take queued faces: only when some are queued and
/// a fetch is free (`in_flight` of at most `parallel`). While every fetch is busy, the one that
/// ends asks for the frame itself, so the page doesn't repaint every frame meanwhile.
pub fn wants_frame(queued: bool, in_flight: usize, parallel: usize) -> bool {
    queued && in_flight < parallel
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_text::test_fonts::{IndexFile, add_host_index, host_index};

    /// Design test 15: a face the host's newer set no longer lists is dropped quietly when its
    /// fetch ends, done or failed: it raises no failure (review round 2, item 2). One still listed
    /// that fails is refused.
    #[test]
    fn a_stale_fetch_raises_no_failure() {
        let db = FontDb::global();
        let index = |bytes: &[u8]| host_index(&[("Web Stale 01", &[IndexFile { path: "w/Regular.ttf", bytes, italic: false, weight: 400 }])]);
        let _index = add_host_index("web-stale", index(b"the first file")).unwrap();
        db.face("Web Stale 01", "Regular");
        let first = db.take_source_requests().into_iter().find(|s| s.family == "Web Stale 01").expect("asked for");
        // The host gives a set listing another file for that style: the first is stale.
        let _index = add_host_index("web-stale", index(b"another file")).unwrap();
        let settled = settle_fetches(db, vec![(first.clone(), Ok(b"the first file".to_vec())), (first, Err("HTTP 404".into()))]);
        assert_eq!(settled, SettledFetches::default(), "neither added nor refused");
        db.face("Web Stale 01", "Regular");
        let second = db.take_source_requests().into_iter().find(|s| s.family == "Web Stale 01").expect("the new file asked for");
        let settled = settle_fetches(db, vec![(second, Err("HTTP 404".into()))]);
        assert_eq!(settled.refused.iter().map(|(_, why)| why.as_str()).collect::<Vec<_>>(), ["HTTP 404"]);
    }

    /// Queued faces ask for a frame only while a fetch is free: with every fetch busy, the page
    /// waits for one to end rather than repainting every frame (review round 3, item 1).
    #[test]
    fn queued_faces_ask_for_a_frame_only_while_a_fetch_is_free() {
        assert!(wants_frame(true, 3, 4));
        assert!(!wants_frame(true, 4, 4), "every fetch busy");
        assert!(!wants_frame(false, 0, 4), "nothing queued");
    }
}
