use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use zip::CompressionMethod;

use crate::{Compression, FormatError, MAX_ARCHIVE_ENTRIES, PageEntry, natural_cmp};

const IMAGE_EXTENSIONS: [&str; 7] = ["jpg", "jpeg", "png", "webp", "gif", "avif", "jxl"];

/// Lists the archive's images in reading order. Entries the server can't read
/// back directly (encrypted, or compressed with anything but deflate) are left
/// out so every indexed page can be served from its offset.
pub(crate) fn page_index(path: &Path) -> Result<Vec<PageEntry>, FormatError> {
    let mut archive = zip::ZipArchive::new(BufReader::new(File::open(path)?))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(FormatError::TooManyEntries {
            limit: MAX_ARCHIVE_ENTRIES,
        });
    }

    let mut pages = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i)?;
        // A name that isn't valid text can't be a page we'd want to show.
        let Ok(name) = entry.name() else {
            continue;
        };
        if entry.is_dir() || entry.encrypted() || !is_page(&name) {
            continue;
        }
        let compression = match entry.compression() {
            CompressionMethod::Stored => Compression::Stored,
            CompressionMethod::Deflated => Compression::Deflated,
            _ => continue,
        };
        let Some(data_offset) = entry.data_start() else {
            continue;
        };
        pages.push(PageEntry {
            name: name.into_owned(),
            data_offset,
            compressed_size: entry.compressed_size(),
            size: entry.size(),
            compression,
        });
    }
    pages.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    Ok(pages)
}

fn is_page(name: &str) -> bool {
    // macOS archivers add resource forks that look like images.
    if name.starts_with("__MACOSX/") || name.rsplit('/').next().is_some_and(|f| f.starts_with('.'))
    {
        return false;
    }
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| IMAGE_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}
