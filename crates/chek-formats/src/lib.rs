//! Format detection, metadata and page indexes for the files Chek serves.
//!
//! Everything here is synchronous and does blocking file I/O, so callers on an
//! async runtime run it on a blocking thread.

mod cbt;
mod cbz;
mod cover;
mod epub;
mod libarchive;
mod metadata;
mod natural;
mod pdf;

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use flate2::read::DeflateDecoder;

pub use cover::{cover_from_image, cover_thumbnail};
pub use metadata::{
    Contributor, MAX_FIELD_CHARS, MAX_SUMMARY_CHARS, Metadata, ReadingDirection, Role,
};
pub use natural::{leading_number, natural_cmp, natural_key};

/// A publication format the server recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Epub,
    Pdf,
    Cbz,
    Cbt,
    Cbr,
    Cb7,
}

impl Format {
    /// The name stored in the catalog and returned by the API.
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Epub => "epub",
            Format::Pdf => "pdf",
            Format::Cbz => "cbz",
            Format::Cbt => "cbt",
            Format::Cbr => "cbr",
            Format::Cb7 => "cb7",
        }
    }

    /// The format a file extension claims, ignoring case. The file's bytes
    /// still have to match; see [`detect`].
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "epub" => Some(Format::Epub),
            "pdf" => Some(Format::Pdf),
            "cbz" => Some(Format::Cbz),
            "cbt" => Some(Format::Cbt),
            "cbr" => Some(Format::Cbr),
            "cb7" => Some(Format::Cb7),
            _ => None,
        }
    }

    /// Whether this is a comic archive, read page by page.
    pub fn is_comic(self) -> bool {
        matches!(self, Format::Cbz | Format::Cbt | Format::Cbr | Format::Cb7)
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
    /// Read through libarchive by entry name, for formats whose entries can't
    /// be read from an offset.
    Archive,
}

impl Compression {
    pub fn as_str(self) -> &'static str {
        match self {
            Compression::Stored => "stored",
            Compression::Deflated => "deflated",
            Compression::Archive => "archive",
        }
    }
}

/// What a scan learns from one file.
#[derive(Debug, Clone)]
pub struct Inspection {
    pub format: Format,
    /// Comic pages in reading order. Empty for formats without a page index yet.
    pub pages: Vec<PageEntry>,
    pub metadata: Metadata,
}

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("could not read file: {0}")]
    Io(#[from] io::Error),
    #[error("invalid archive: {0}")]
    Archive(#[from] zip::result::ZipError),
    #[error("invalid archive: {0}")]
    Libarchive(#[from] compress_tools::Error),
    #[error("archive has more than {limit} entries")]
    TooManyEntries { limit: usize },
    #[error("entry is larger than {limit} bytes")]
    EntryTooLarge { limit: u64 },
    #[error(
        "entry is shorter than the page index says; the file may have changed since it was scanned"
    )]
    Truncated,
    #[error("image processing failed: {0}")]
    Image(String),
}

/// Archives with more entries than this are rejected rather than indexed.
pub const MAX_ARCHIVE_ENTRIES: usize = 100_000;

/// Pages and other archive entries larger than this are not read into memory.
pub const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;

/// Picks a format from the file's extension, then checks its leading bytes so a
/// renamed file is not served as something it isn't. Returns `None` for files
/// the server doesn't handle.
pub fn detect(path: &Path) -> Result<Option<Format>, FormatError> {
    let Some(claimed) = path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(Format::from_extension)
    else {
        return Ok(None);
    };

    // A tar header's magic sits 257 bytes in.
    let mut magic = [0u8; 262];
    let read = read_prefix(path, &mut magic)?;
    let magic = &magic[..read];
    Ok(match claimed {
        Format::Pdf => magic.starts_with(b"%PDF-").then_some(claimed),
        Format::Epub => magic.starts_with(b"PK\x03\x04").then_some(claimed),
        // Comic archives are often saved under another comic extension, such
        // as a ZIP named .cbr, so their bytes decide the format.
        Format::Cbz | Format::Cbt | Format::Cbr | Format::Cb7 => {
            if magic.starts_with(b"PK\x03\x04") {
                Some(Format::Cbz)
            } else if libarchive::is_rar(magic) {
                Some(Format::Cbr)
            } else if libarchive::is_7z(magic) {
                Some(Format::Cb7)
            } else if cbt::is_tar(magic) {
                Some(Format::Cbt)
            } else {
                None
            }
        }
    })
}

/// Reads what the catalog needs from a file already identified by [`detect`].
pub fn inspect(path: &Path, format: Format) -> Result<Inspection, FormatError> {
    let (pages, metadata) = match format {
        Format::Cbz => cbz::inspect(path)?,
        Format::Cbt => cbt::inspect(path)?,
        Format::Cbr | Format::Cb7 => libarchive::inspect(path)?,
        // A book whose package can't be read is still served as a file, as
        // before metadata existed, just without metadata.
        Format::Epub => (Vec::new(), epub::metadata(path).unwrap_or_default()),
        Format::Pdf => (Vec::new(), pdf::metadata(path).unwrap_or_default()),
    };
    Ok(Inspection {
        format,
        pages,
        metadata,
    })
}

/// Reads one page's image bytes straight from its offset in the archive,
/// without reopening the archive's directory.
pub fn read_page(path: &Path, page: &PageEntry) -> Result<Vec<u8>, FormatError> {
    if page.size > MAX_ENTRY_BYTES || page.compressed_size > MAX_ENTRY_BYTES {
        return Err(FormatError::EntryTooLarge {
            limit: MAX_ENTRY_BYTES,
        });
    }
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(page.data_offset))?;
    let raw = file.take(page.compressed_size);
    // The index's size bounds the read; one extra byte detects an entry that
    // inflates to more than the index says.
    let mut out = Vec::with_capacity(page.size as usize);
    match page.compression {
        Compression::Stored => raw.take(page.size + 1).read_to_end(&mut out)?,
        Compression::Deflated => DeflateDecoder::new(raw)
            .take(page.size + 1)
            .read_to_end(&mut out)?,
        Compression::Archive => return libarchive::read_entry(path, page),
    };
    if out.len() as u64 != page.size {
        return Err(FormatError::Truncated);
    }
    Ok(out)
}

/// The media type of a page, from its entry name's extension.
pub fn page_media_type(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map_or("", |(_, ext)| ext);
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "avif" => "image/avif",
        "jxl" => "image/jxl",
        _ => "application/octet-stream",
    }
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
