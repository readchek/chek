//! CBR and CB7: comics in RAR or 7-Zip archives, read through libarchive.
//! Their entries are compressed, often as one solid stream, so a page can't be
//! read from an offset the way CBZ and CBT pages are. Each page is found by
//! walking the archive's headers to its entry instead.

use std::cell::RefCell;
use std::fs::File;
use std::path::Path;
use std::rc::Rc;

use compress_tools::{ArchiveContents, ArchiveIteratorBuilder};

use crate::cbz::is_page;
use crate::epub::MAX_XML_BYTES;
use crate::metadata::{self, Metadata};
use crate::{
    Compression, FormatError, MAX_ARCHIVE_ENTRIES, MAX_ENTRY_BYTES, PageEntry, natural_cmp,
};

pub(crate) fn is_rar(magic: &[u8]) -> bool {
    // RAR 4 continues with 0x00 and RAR 5 with 0x01 0x00.
    magic.starts_with(b"Rar!\x1a\x07")
}

pub(crate) fn is_7z(magic: &[u8]) -> bool {
    magic.starts_with(b"7z\xbc\xaf\x27\x1c")
}

/// Lists the archive's images in reading order and reads its `ComicInfo.xml`.
pub(crate) fn inspect(path: &Path) -> Result<(Vec<PageEntry>, Metadata), FormatError> {
    // libarchive reports each header to the filter, which keeps the names and
    // lets only `ComicInfo.xml` through to be decompressed.
    let listed = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&listed);
    let entries = ArchiveIteratorBuilder::new(File::open(path)?)
        .decoder(lossy)
        .filter(move |name, stat| {
            let size = u64::try_from(stat.st_size).unwrap_or(0);
            let mut seen = seen.borrow_mut();
            if seen.len() <= MAX_ARCHIVE_ENTRIES {
                seen.push((name.to_owned(), size));
            }
            name.eq_ignore_ascii_case("ComicInfo.xml") && size <= MAX_XML_BYTES
        })
        .build()?;

    let mut comic_info: Option<Vec<u8>> = None;
    for content in entries {
        match content {
            ArchiveContents::StartOfEntry(..) if comic_info.is_none() => {
                comic_info = Some(Vec::new());
            }
            ArchiveContents::DataChunk(chunk) => {
                if let Some(xml) = &mut comic_info
                    && xml.len() + chunk.len() <= MAX_XML_BYTES as usize
                {
                    xml.extend_from_slice(&chunk);
                }
            }
            ArchiveContents::Err(e) => return Err(e.into()),
            _ => {}
        }
    }

    let listed = listed.take();
    if listed.len() > MAX_ARCHIVE_ENTRIES {
        return Err(FormatError::TooManyEntries {
            limit: MAX_ARCHIVE_ENTRIES,
        });
    }
    let mut pages: Vec<PageEntry> = listed
        .into_iter()
        .filter(|(name, _)| is_page(name))
        .map(|(name, size)| PageEntry {
            name,
            data_offset: 0,
            compressed_size: size,
            size,
            compression: Compression::Archive,
        })
        .collect();
    pages.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    let metadata = comic_info
        .map(|xml| metadata::comic_info(&xml))
        .unwrap_or_default();
    Ok((pages, metadata))
}

/// Reads one entry by name, walking the archive up to it.
pub(crate) fn read_entry(path: &Path, page: &PageEntry) -> Result<Vec<u8>, FormatError> {
    let wanted = page.name.clone();
    let entries = ArchiveIteratorBuilder::new(File::open(path)?)
        .decoder(lossy)
        .filter(move |name, _| name == wanted)
        .build()?;
    let mut out: Option<Vec<u8>> = None;
    for content in entries {
        match content {
            ArchiveContents::StartOfEntry(..) => {
                out = Some(Vec::with_capacity(page.size.min(MAX_ENTRY_BYTES) as usize));
            }
            ArchiveContents::DataChunk(chunk) => {
                let Some(bytes) = &mut out else { continue };
                // An entry larger than the index says means the file changed.
                if (bytes.len() + chunk.len()) as u64 > page.size {
                    return Err(FormatError::Truncated);
                }
                bytes.extend_from_slice(&chunk);
            }
            ArchiveContents::EndOfEntry => break,
            ArchiveContents::Err(e) => return Err(e.into()),
        }
    }
    match out {
        Some(bytes) if bytes.len() as u64 == page.size => Ok(bytes),
        _ => Err(FormatError::Truncated),
    }
}

/// Entry names that aren't UTF-8 are kept readable rather than failing the
/// archive; reading a page decodes names the same way, so lookups still match.
fn lossy(bytes: &[u8]) -> compress_tools::Result<String> {
    Ok(String::from_utf8_lossy(bytes).into_owned())
}
