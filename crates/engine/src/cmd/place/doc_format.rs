//! What a build knows of Word 97-2003 documents with or without their reader (vectorcraft-engine's
//! `word-doc` feature): the format `file.place` reports and the compound file's first bytes.

use super::fileio;

/// Word 97-2003 documents as `file.place` reports them (not a format `document.open` reads).
pub(super) const FORMAT: fileio::Format = fileio::Format {
    id: "doc",
    label: "Word 97-2003",
    extensions: &["doc"],
    mime: "application/msword",
    read: cfg!(feature = "word-doc"),
    write: false,
    raster: false,
    options: &[],
};

/// The first bytes of a compound file (\[MS-CFB\] §2.2).
pub(super) const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
