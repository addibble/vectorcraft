//! The rules of the embedding protocol (`host`, `docs/web-embed.md`), kept free of browser types so
//! they are tested natively: which messages the bridge reads, and what a `vectorcraft:open` and a
//! `vectorcraft:configure` must carry.

/// What a `vectorcraft:open` delivers: a file to open like one picked with File › Open.
#[derive(Debug, PartialEq, Eq)]
pub struct Open {
    /// The file name: the document's name, and the name it is saved under.
    pub name: String,
    pub bytes: Vec<u8>,
}

/// The messages the parent may send.
pub const FROM_PARENT: [&str; 2] = ["vectorcraft:open", "vectorcraft:configure"];

/// Whether the bridge reads a message at all: a `vectorcraft:open` or `vectorcraft:configure` from
/// the parent window, on the page's own origin. Anything else is ignored without a reply, so another page learns nothing.
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
/// `emptyMessage`, `emptyLink`).
#[derive(Debug, Default)]
pub struct ConfigureFields {
    pub hide: Field<Vec<String>>,
    pub hide_menus: Field<Vec<String>>,
    pub hide_mark: Field<bool>,
    pub empty_message: Field<String>,
    pub empty_link: Field<String>,
}

/// Check a `vectorcraft:configure` the bridge accepted. `hide` is an array of strings, each a
/// command id or a prefix ending in `*`; `hideMenus` an array of menu titles; `hideMark` a boolean;
/// `emptyMessage` and `emptyLink` strings. The error is the text of the `vectorcraft:failed`
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
    let hide_mark = match f.hide_mark {
        Field::Absent => false,
        Field::Given(b) => b,
        Field::Wrong => return Err("vectorcraft:configure's `hideMark` is a boolean".to_string()),
    };
    Ok(Configure {
        hide: names(f.hide, "hide", "command ids (a prefix ends in `*`)")?,
        hide_menus: names(f.hide_menus, "hideMenus", "menu titles")?,
        hide_mark,
        empty_message: text(f.empty_message, "emptyMessage")?,
        empty_link: text(f.empty_link, "emptyLink")?,
    })
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
    }
}
