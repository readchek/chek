//! Format detection and page indexes for the files Chek serves.
//!
//! Everything here is synchronous and does blocking file I/O, so callers on an
//! async runtime run it on a blocking thread.

mod cbz;
mod natural;

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

pub use natural::natural_cmp;

/// A publication format the server recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Epub,
    Pdf,
    Cbz,
}

impl Format {
    /// The name stored in the catalog and returned by the API.
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Epub => "epub",
            Format::Pdf => "pdf",
            Format::Cbz => "cbz",
        }
    }
}

/// One page of a comic archive, as stored in the page index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageEntry {
    /// The archive entry's name.
    pub name: String,
    /// Byte offset of the entry's data in the archive, so a page can be read
    /// without reopening the archive's central directory.
    pub data_offset: u64,
    pub compressed_size: u64,
    pub size: u64,
    pub compression: Compression,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Stored,
    Deflated,
}

impl Compression {
    pub fn as_str(self) -> &'static str {
        match self {
            Compression::Stored => "stored",
            Compression::Deflated => "deflated",
        }
    }
}

/// What a scan learns from one file.
#[derive(Debug, Clone)]
pub struct Inspection {
    pub format: Format,
    /// Comic pages in reading order. Empty for formats without a page index yet.
    pub pages: Vec<PageEntry>,
}

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("could not read file: {0}")]
    Io(#[from] io::Error),
    #[error("invalid archive: {0}")]
    Archive(#[from] zip::result::ZipError),
    #[error("archive has more than {limit} entries")]
    TooManyEntries { limit: usize },
}

/// Archives with more entries than this are rejected rather than indexed.
pub const MAX_ARCHIVE_ENTRIES: usize = 100_000;

/// Picks a format from the file's extension, then checks its leading bytes so a
/// renamed file is not served as something it isn't. Returns `None` for files
/// the server doesn't handle.
pub fn detect(path: &Path) -> Result<Option<Format>, FormatError> {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return Ok(None);
    };
    let claimed = match ext.to_ascii_lowercase().as_str() {
        "epub" => Format::Epub,
        "pdf" => Format::Pdf,
        "cbz" => Format::Cbz,
        _ => return Ok(None),
    };

    let mut magic = [0u8; 5];
    let read = read_prefix(path, &mut magic)?;
    let magic = &magic[..read];
    let matches = match claimed {
        Format::Pdf => magic.starts_with(b"%PDF-"),
        Format::Epub | Format::Cbz => magic.starts_with(b"PK\x03\x04"),
    };
    Ok(matches.then_some(claimed))
}

/// Reads what the catalog needs from a file already identified by [`detect`].
pub fn inspect(path: &Path, format: Format) -> Result<Inspection, FormatError> {
    let pages = match format {
        Format::Cbz => cbz::page_index(path)?,
        Format::Epub | Format::Pdf => Vec::new(),
    };
    Ok(Inspection { format, pages })
}

fn read_prefix(path: &Path, buf: &mut [u8]) -> io::Result<usize> {
    let mut file = File::open(path)?;
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}
