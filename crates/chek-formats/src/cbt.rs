//! CBT: comics in a tar archive. Tar stores every entry uncompressed after a
//! 512-byte header, so pages are indexed by offset and served like a stored
//! CBZ entry. Headers in ustar, GNU long-name and pax forms are read.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::cbz::is_page;
use crate::epub::MAX_XML_BYTES;
use crate::metadata::{self, Metadata};
use crate::{Compression, FormatError, MAX_ARCHIVE_ENTRIES, PageEntry, natural_cmp};

const BLOCK: u64 = 512;
/// Long names and pax records are small; anything larger isn't a header.
const MAX_HEADER_DATA: u64 = 64 * 1024;

/// Whether the 512-byte header at the start of a file is a ustar header.
pub(crate) fn is_tar(header: &[u8]) -> bool {
    header.len() >= 262 && &header[257..262] == b"ustar"
}

/// Lists the archive's images in reading order and reads its `ComicInfo.xml`.
pub(crate) fn inspect(path: &Path) -> Result<(Vec<PageEntry>, Metadata), FormatError> {
    let mut file = BufReader::new(File::open(path)?);
    let length = file.get_ref().metadata()?.len();
    let mut pages = Vec::new();
    let mut comic_info = None;
    let mut long_name: Option<String> = None;
    let mut entries = 0usize;
    let mut position = 0u64;
    let mut header = [0u8; BLOCK as usize];

    while position + BLOCK <= length {
        file.seek(SeekFrom::Start(position))?;
        file.read_exact(&mut header)?;
        if header.iter().all(|&b| b == 0) {
            break;
        }
        entries += 1;
        if entries > MAX_ARCHIVE_ENTRIES {
            return Err(FormatError::TooManyEntries {
                limit: MAX_ARCHIVE_ENTRIES,
            });
        }
        let size = entry_size(&header).ok_or_else(|| invalid("an entry's size is unreadable"))?;
        let data_offset = position + BLOCK;
        let next = data_offset
            .checked_add(size.div_ceil(BLOCK) * BLOCK)
            .ok_or_else(|| invalid("an entry's size is too large"))?;
        if data_offset + size > length {
            return Err(invalid("an entry runs past the end of the file"));
        }

        match header[156] {
            // GNU long name: the data is the next entry's name.
            b'L' => long_name = Some(read_text(&mut file, data_offset, size)?),
            // pax extended header: its `path` record names the next entry.
            b'x' => {
                let records = read_text(&mut file, data_offset, size)?;
                if let Some(path) = pax_path(&records) {
                    long_name = Some(path);
                }
            }
            // Regular files; anything else (folders, links, devices) is skipped.
            b'0' | b'\0' | b'7' => {
                let name = long_name.take().unwrap_or_else(|| header_name(&header));
                let name = name.trim_start_matches("./").to_owned();
                if name.eq_ignore_ascii_case("ComicInfo.xml") {
                    comic_info = Some((data_offset, size));
                } else if is_page(&name) {
                    pages.push(PageEntry {
                        name,
                        data_offset,
                        compressed_size: size,
                        size,
                        compression: Compression::Stored,
                    });
                }
            }
            _ => long_name = None,
        }
        position = next;
    }

    pages.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    let metadata = comic_info
        .filter(|&(_, size)| size <= MAX_XML_BYTES)
        .and_then(|(offset, size)| read_bytes(&mut file, offset, size).ok())
        .map(|xml| metadata::comic_info(&xml))
        .unwrap_or_default();
    Ok((pages, metadata))
}

/// The name from a ustar header, joined with its prefix field.
fn header_name(header: &[u8]) -> String {
    let name = field(&header[0..100]);
    let prefix = if is_tar(header) {
        field(&header[345..500])
    } else {
        String::new()
    };
    if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    }
}

/// An entry's size: octal text, or base-256 when the high bit is set, as GNU
/// tar writes sizes over 8 GiB.
fn entry_size(header: &[u8]) -> Option<u64> {
    let raw = &header[124..136];
    if raw[0] & 0x80 != 0 {
        return raw[1..]
            .iter()
            .try_fold(0u64, |n, &b| n.checked_mul(256)?.checked_add(u64::from(b)));
    }
    let text = std::str::from_utf8(raw).ok()?;
    let text = text.trim_matches(|c: char| c == '\0' || c == ' ');
    if text.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(text, 8).ok()
}

/// The `path` record from pax extended header data, whose records read
/// `<length> <key>=<value>\n`.
fn pax_path(records: &str) -> Option<String> {
    let mut rest = records;
    while !rest.is_empty() {
        let (length, _) = rest.split_once(' ')?;
        let length: usize = length.parse().ok()?;
        let record = rest.get(..length)?;
        rest = &rest[length..];
        let (_, entry) = record.split_once(' ')?;
        if let Some(path) = entry.strip_prefix("path=") {
            return Some(path.trim_end_matches('\n').to_owned());
        }
    }
    None
}

fn field(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn read_text(file: &mut BufReader<File>, offset: u64, size: u64) -> Result<String, FormatError> {
    if size > MAX_HEADER_DATA {
        return Err(invalid("a long name or pax header is too large"));
    }
    let bytes = read_bytes(file, offset, size)?;
    Ok(field(&bytes))
}

fn read_bytes(file: &mut BufReader<File>, offset: u64, size: u64) -> Result<Vec<u8>, FormatError> {
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn invalid(why: &str) -> FormatError {
    FormatError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("invalid tar archive: {why}"),
    ))
}
