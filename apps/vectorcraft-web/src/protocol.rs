//! The rules of the embedding protocol (`host`, `docs/web-embed.md`), kept free of browser types so
//! they are tested natively: which messages the bridge reads, and what a `vectorcraft:open`, a
//! `vectorcraft:configure` and a `vectorcraft:saved` must carry.

/// What a `vectorcraft:open` delivers: a file to open like one picked with File › Open.
#[derive(Debug, PartialEq, Eq)]
pub struct Open {
    /// The file name: the document's name, and the name it is saved under.
    pub name: String,
    pub bytes: Vec<u8>,
}

/// The messages the parent may send.
pub const FROM_PARENT: [&str; 3] = ["vectorcraft:open", "vectorcraft:configure", "vectorcraft:saved"];

/// Whether the bridge reads a message at all: one of [`FROM_PARENT`] from the parent window, on
/// the page's own origin. Anything else is ignored without a reply, so another page learns nothing.
///
/// `own_origin` is the page's origin and `origin` the event's; `from_parent` says whether the
/// event's source is the parent window; `kind` is the message's `type`, when it is a string. An
/// opaque origin (`"null"`: a sandboxed frame, a `file:` page) can't be told apart from another
/// page's, so the bridge hears nothing there.
pub fn accepts(own_origin: &str, origin: &str, from_parent: bool, kind: Option<&str>) -> bool {
    from_parent && !is_opaque(own_origin) && origin == own_origin && kind.is_some_and(|k| FROM_PARENT.contains(&k))
}

/// An origin that names no site (`"null"`).
pub fn is_opaque(origin: &str) -> bool {
    origin.is_empty() || origin == "null"
}

/// Check a `vectorcraft:open` the bridge accepted: it needs a file name and the file's bytes (an
/// empty file is passed on: opening it reports the problem like any unreadable file). The error is
/// the text of the `vectorcraft:failed` reply.
pub fn open(name: Option<String>, bytes: Option<Vec<u8>>) -> Result<Open, String> {
    let Some(name) = name.filter(|n| !n.trim().is_empty()) else {
        return Err("vectorcraft:open needs the file's name (`name`, a non-empty string)".to_string());
    };
    let Some(bytes) = bytes else {
        return Err(format!("vectorcraft:open of {name} needs the file's contents (`bytes`, an ArrayBuffer or Uint8Array)"));
    };
    Ok(Open { name, bytes })
}

/// The most commands (or menus) a page may hide, the longest id or title, and the longest message
/// or link: a page's message is untrusted input.
pub const MAX_HIDDEN: usize = 1_000;
pub const MAX_ID_CHARS: usize = 200;
pub const MAX_MESSAGE_CHARS: usize = 1_000;

/// What a `vectorcraft:configure` sets: the commands, menus and brand mark hidden from the user, and
/// the empty window's message and link. Each message replaces the last whole, so a field left out
/// is cleared.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Configure {
    /// Command ids, or id prefixes ending in `*` (`file.openRecent*`).
    pub hide: Vec<String>,
    /// The menu bar's menus, by their English titles (`File`, `Help`).
    pub hide_menus: Vec<String>,
    /// The application bar's brand mark is hidden.
    pub hide_mark: bool,
    /// Shown in the window while no document is open, in place of the Home screen.
    pub empty_message: Option<String>,
    /// A link after the message; a click on it is posted to the parent as `vectorcraft:emptyLink`.
    pub empty_link: Option<String>,
    /// Closing a modified document saves it and closes it once the parent answers
    /// `vectorcraft:saved`, in place of asking Save Changes.
    pub save_on_close: bool,
}

/// A field of a message: left out (or `null`), of the type it should be, or of another.
#[derive(Debug, Default)]
pub enum Field<T> {
    #[default]
    Absent,
    Given(T),
    Wrong,
}

/// A `vectorcraft:configure`'s fields as the page sent them (`hide`, `hideMenus`, `hideMark`,
/// `emptyMessage`, `emptyLink`, `saveOnClose`). Fields it doesn't name are never read.
#[derive(Debug, Default)]
pub struct ConfigureFields {
    pub hide: Field<Vec<String>>,
    pub hide_menus: Field<Vec<String>>,
    pub hide_mark: Field<bool>,
    pub empty_message: Field<String>,
    pub empty_link: Field<String>,
    pub save_on_close: Field<bool>,
}

/// Check a `vectorcraft:configure` the bridge accepted. `hide` is an array of strings, each a
/// command id or a prefix ending in `*`; `hideMenus` an array of menu titles; `hideMark` and
/// `saveOnClose` booleans; `emptyMessage` and `emptyLink` strings. The error is the text of the `vectorcraft:failed`
/// reply, and nothing changes.
pub fn configure(f: ConfigureFields) -> Result<Configure, String> {
    let names = |field: Field<Vec<String>>, key: &str, what: &str| match field {
        Field::Absent => Ok(Vec::new()),
        Field::Given(ids) if ids.len() <= MAX_HIDDEN && ids.iter().all(|id| !id.trim().is_empty() && id.chars().count() <= MAX_ID_CHARS) => Ok(ids),
        _ => Err(format!("vectorcraft:configure's `{key}` is an array of at most {MAX_HIDDEN} {what} (strings of 1 to {MAX_ID_CHARS} characters)")),
    };
    let text = |field: Field<String>, key: &str| match field {
        Field::Absent => Ok(None),
        Field::Given(m) if m.chars().count() <= MAX_MESSAGE_CHARS => Ok(Some(m).filter(|m| !m.trim().is_empty())),
        _ => Err(format!("vectorcraft:configure's `{key}` is a string of at most {MAX_MESSAGE_CHARS} characters")),
    };
    let flag = |field: Field<bool>, key: &str| match field {
        Field::Absent => Ok(false),
        Field::Given(b) => Ok(b),
        Field::Wrong => Err(format!("vectorcraft:configure's `{key}` is a boolean")),
    };
    Ok(Configure {
        hide: names(f.hide, "hide", "command ids (a prefix ends in `*`)")?,
        hide_menus: names(f.hide_menus, "hideMenus", "menu titles")?,
        hide_mark: flag(f.hide_mark, "hideMark")?,
        empty_message: text(f.empty_message, "emptyMessage")?,
        empty_link: text(f.empty_link, "emptyLink")?,
        save_on_close: flag(f.save_on_close, "saveOnClose")?,
    })
}

/// The largest save id: every integer up to it is exact in a JavaScript number.
pub const MAX_SAVE_ID: f64 = 9_007_199_254_740_991.0;

/// What a `vectorcraft:saved` says: whether the parent stored the `vectorcraft:save` it answers.
#[derive(Debug, PartialEq, Eq)]
pub struct Saved {
    /// The answered save's `id`.
    pub id: u64,
    /// The answered save's `name`.
    pub name: String,
    /// Stored (`true`), or not.
    pub ok: bool,
    /// Why not, in the parent's words, for the status line.
    pub error: Option<String>,
}

/// Check a `vectorcraft:saved` the bridge accepted: `id` is the answered save's (a whole number),
/// `name` its file name, `ok` a boolean and `error`, if given, a string of at most
/// [`MAX_MESSAGE_CHARS`]. The error is the text of the `vectorcraft:failed` reply.
pub fn saved(id: Field<f64>, name: Field<String>, ok: Field<bool>, error: Field<String>) -> Result<Saved, String> {
    let id = match id {
        Field::Given(n) if n.is_finite() && n.fract() == 0.0 && (0.0..=MAX_SAVE_ID).contains(&n) => n as u64,
        _ => return Err("vectorcraft:saved needs the answered save's `id` (a whole number)".to_string()),
    };
    let Field::Given(name) = name.into_non_empty() else {
        return Err("vectorcraft:saved needs the answered save's `name` (a non-empty string)".to_string());
    };
    let Field::Given(ok) = ok else {
        return Err(format!("vectorcraft:saved of {name} needs `ok`, a boolean"));
    };
    let error = match error {
        Field::Absent => None,
        Field::Given(e) if e.chars().count() <= MAX_MESSAGE_CHARS => Some(e).filter(|e| !e.trim().is_empty()),
        _ => return Err(format!("vectorcraft:saved's `error` is a string of at most {MAX_MESSAGE_CHARS} characters")),
    };
    Ok(Saved { id, name, ok, error })
}

impl Field<String> {
    /// A blank string counts as left out.
    fn into_non_empty(self) -> Self {
        match self {
            Field::Given(s) if s.trim().is_empty() => Field::Absent,
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SITE: &str = "https://cloud.example.com";
    const OPEN: Option<&str> = Some("vectorcraft:open");

    #[test]
    fn accepts_an_open_from_the_parent_on_the_same_origin() {
        assert!(accepts(SITE, SITE, true, OPEN));
    }

    #[test]
    fn ignores_messages_from_another_origin() {
        for origin in [
            "https://evil.example",
            "http://cloud.example.com",
            "https://cloud.example.com:8443",
            "https://cloud.example.com.evil.example",
            "https://sub.cloud.example.com",
            "null",
            "",
        ] {
            assert!(!accepts(SITE, origin, true, OPEN), "{origin}");
        }
    }

    #[test]
    fn ignores_another_window_on_the_same_origin() {
        // Such as another tab of the same site, or a frame the parent page holds.
        assert!(!accepts(SITE, SITE, false, OPEN));
    }

    #[test]
    fn ignores_other_message_types() {
        for kind in [
            None,
            Some(""),
            Some("vectorcraft:save"),
            Some("vectorcraft:ready"),
            Some("VECTORCRAFT:OPEN"),
            Some("photocraft:open"),
            Some("vectorcraft:open "),
        ] {
            assert!(!accepts(SITE, SITE, true, kind), "{kind:?}");
        }
    }

    #[test]
    fn hears_nothing_on_an_opaque_origin() {
        assert!(!accepts("null", "null", true, OPEN));
        assert!(!accepts("", "", true, OPEN));
        assert!(is_opaque("null"));
        assert!(!is_opaque(SITE));
    }

    #[test]
    fn opens_a_named_file() {
        assert_eq!(open(Some("Logo.svg".into()), Some(b"<svg/>".to_vec())), Ok(Open { name: "Logo.svg".into(), bytes: b"<svg/>".to_vec() }));
        // An empty file is the importer's to report.
        assert_eq!(open(Some("Empty.svg".into()), Some(Vec::new())), Ok(Open { name: "Empty.svg".into(), bytes: Vec::new() }));
    }

    #[test]
    fn refuses_an_open_without_a_name() {
        for name in [None, Some(String::new()), Some("   ".to_string()), Some("\t\n".to_string())] {
            let error = open(name.clone(), Some(b"<svg/>".to_vec())).err();
            assert!(error.as_deref().is_some_and(|e| e.contains("`name`")), "{name:?}: {error:?}");
        }
    }

    #[test]
    fn refuses_an_open_without_bytes() {
        let error = open(Some("Logo.svg".into()), None).err();
        assert!(error.as_deref().is_some_and(|e| e.contains("`bytes`") && e.contains("Logo.svg")), "{error:?}");
    }

    #[test]
    fn accepts_a_configure_from_the_parent_on_the_same_origin() {
        assert!(accepts("https://cloud.example", "https://cloud.example", true, Some("vectorcraft:configure")));
        assert!(!accepts("https://cloud.example", "https://evil.example", true, Some("vectorcraft:configure")));
        assert!(!accepts("https://cloud.example", "https://cloud.example", false, Some("vectorcraft:configure")));
    }

    fn fields(hide: Field<Vec<String>>, empty_message: Field<String>) -> ConfigureFields {
        ConfigureFields { hide, empty_message, ..Default::default() }
    }

    #[test]
    fn configures_hidden_commands_and_the_empty_message() {
        let c = configure(fields(
            Field::Given(vec!["file.new".into(), "file.openRecent*".into()]),
            Field::Given("Open a file from the Files app".into()),
        ))
        .unwrap();
        assert_eq!(c.hide, ["file.new", "file.openRecent*"]);
        assert_eq!(c.empty_message.as_deref(), Some("Open a file from the Files app"));
        // Left out, each is cleared; a blank message is none.
        assert_eq!(configure(fields(Field::Absent, Field::Given("  ".into()))).unwrap(), Configure::default());
    }

    #[test]
    fn configures_hidden_menus_the_mark_and_the_empty_link() {
        let c = configure(ConfigureFields {
            hide_menus: Field::Given(vec!["File".into(), "Help".into()]),
            hide_mark: Field::Given(true),
            empty_message: Field::Given("No file open.".into()),
            empty_link: Field::Given("Open one from Files".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(c.hide_menus, ["File", "Help"]);
        assert!(c.hide_mark);
        assert_eq!(c.empty_link.as_deref(), Some("Open one from Files"));
        // A blank link is none, as a blank message is.
        assert_eq!(configure(ConfigureFields { empty_link: Field::Given(" ".into()), ..Default::default() }).unwrap().empty_link, None);
    }

    #[test]
    fn refuses_a_configure_of_the_wrong_shape() {
        assert!(configure(fields(Field::Wrong, Field::Absent)).unwrap_err().contains("`hide`"));
        assert!(configure(fields(Field::Given(vec![String::new()]), Field::Absent)).is_err());
        assert!(configure(fields(Field::Given(vec!["x".repeat(MAX_ID_CHARS + 1)]), Field::Absent)).is_err());
        assert!(configure(fields(Field::Given(vec!["file.new".into(); MAX_HIDDEN + 1]), Field::Absent)).is_err());
        assert!(configure(fields(Field::Absent, Field::Wrong)).unwrap_err().contains("`emptyMessage`"));
        assert!(configure(fields(Field::Absent, Field::Given("x".repeat(MAX_MESSAGE_CHARS + 1)))).is_err());
        assert!(configure(ConfigureFields { hide_menus: Field::Wrong, ..Default::default() }).unwrap_err().contains("`hideMenus`"));
        assert!(configure(ConfigureFields { hide_menus: Field::Given(vec![" ".into()]), ..Default::default() }).is_err());
        assert!(configure(ConfigureFields { hide_mark: Field::Wrong, ..Default::default() }).unwrap_err().contains("`hideMark`"));
        assert!(configure(ConfigureFields { empty_link: Field::Wrong, ..Default::default() }).unwrap_err().contains("`emptyLink`"));
        assert!(configure(ConfigureFields { empty_link: Field::Given("x".repeat(MAX_MESSAGE_CHARS + 1)), ..Default::default() }).is_err());
        assert!(configure(ConfigureFields { save_on_close: Field::Wrong, ..Default::default() }).unwrap_err().contains("`saveOnClose`"));
    }

    #[test]
    fn configures_saving_on_close() {
        assert!(configure(ConfigureFields { save_on_close: Field::Given(true), ..Default::default() }).unwrap().save_on_close);
        // Left out, it is off: a host that never sets it is asked Save Changes as before.
        assert!(!configure(ConfigureFields::default()).unwrap().save_on_close);
    }

    #[test]
    fn accepts_a_saved_answer_from_the_parent_on_the_same_origin() {
        assert!(accepts(SITE, SITE, true, Some("vectorcraft:saved")));
        assert!(!accepts(SITE, "https://evil.example", true, Some("vectorcraft:saved")));
        assert!(!accepts(SITE, SITE, false, Some("vectorcraft:saved")));
    }

    #[test]
    fn reads_a_saved_answer() {
        let name = || Field::Given("Logo.vectorcraft".to_string());
        assert_eq!(
            saved(Field::Given(7.0), name(), Field::Given(true), Field::Absent),
            Ok(Saved { id: 7, name: "Logo.vectorcraft".into(), ok: true, error: None })
        );
        let failed = saved(Field::Given(8.0), name(), Field::Given(false), Field::Given("The server is offline".into())).unwrap();
        assert_eq!((failed.ok, failed.error.as_deref()), (false, Some("The server is offline")));
        // A blank reason is none.
        assert_eq!(saved(Field::Given(8.0), name(), Field::Given(false), Field::Given(" ".into())).unwrap().error, None);
    }

    #[test]
    fn refuses_a_saved_answer_of_the_wrong_shape() {
        let name = || Field::Given("Logo.vectorcraft".to_string());
        for id in [
            Field::Absent,
            Field::Wrong,
            Field::Given(-1.0),
            Field::Given(1.5),
            Field::Given(f64::NAN),
            Field::Given(f64::INFINITY),
            Field::Given(MAX_SAVE_ID * 2.0),
        ] {
            assert!(saved(id, name(), Field::Given(true), Field::Absent).unwrap_err().contains("`id`"));
        }
        for n in [Field::Absent, Field::Wrong, Field::Given("  ".to_string())] {
            assert!(saved(Field::Given(1.0), n, Field::Given(true), Field::Absent).unwrap_err().contains("`name`"));
        }
        for ok in [Field::Absent, Field::Wrong] {
            assert!(saved(Field::Given(1.0), name(), ok, Field::Absent).unwrap_err().contains("`ok`"));
        }
        assert!(saved(Field::Given(1.0), name(), Field::Given(false), Field::Wrong).unwrap_err().contains("`error`"));
        assert!(saved(Field::Given(1.0), name(), Field::Given(false), Field::Given("x".repeat(MAX_MESSAGE_CHARS + 1))).is_err());
    }
}
