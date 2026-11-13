//! Word 97-2003 documents in a build without their reader (vectorcraft-engine's `word-doc`
//! feature, which brings in cfb): a compound file named `.doc`, or one whose directory names a
//! `WordDocument` stream (a template, `.dot`, or a document named otherwise), is recognized and
//! refused with a message, so it is never read as another format. File › Place doesn't offer
//! `.doc` files in such a build (`fileio::PLACE_EXTS`).

pub(super) use super::doc_format::FORMAT;
use super::doc_format::SIGNATURE;
use super::text::Story;
use super::{Result, bad, fileio};

/// The `WordDocument` stream's name as a compound file's directory entry holds it (UTF-16LE,
/// \[MS-CFB\] §2.6.1), which the reader looks for through cfb.
const WORD_DOCUMENT: [u8; 24] = {
    let name = *b"WordDocument";
    let mut utf16 = [0; 24];
    let mut i = 0;
    while i < name.len() {
        utf16[2 * i] = name[i];
        i += 1;
    }
    utf16
};

/// Is the file `name` with `bytes` a Word 97-2003 document: a compound file named `.doc`, or one
/// that names a `WordDocument` stream?
pub(super) fn is_doc(name: &str, bytes: &[u8]) -> bool {
    bytes.starts_with(&SIGNATURE)
        && (FORMAT.extensions.contains(&fileio::extension(name).as_str()) || bytes.windows(WORD_DOCUMENT.len()).any(|w| w == WORD_DOCUMENT))
}

/// This build places no Word 97-2003 document.
pub(super) fn import(_bytes: &[u8], cmd: &str) -> Result<(Story, Vec<String>)> {
    Err(bad(cmd, "this build of VectorCraft doesn't read Word 97-2003 documents; save the document as .docx (Word) or .rtf and place that"))
}
