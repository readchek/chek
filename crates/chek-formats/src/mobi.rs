//! MOBI and AZW3: Kindle books in a Palm database. Record 0 holds the MOBI
//! header and its EXTH block of metadata; images are later records, one of
//! which the EXTH block marks as the cover. Only those records are read, so a
//! large book costs a few small reads.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::metadata::{Metadata, Role, clean, clean_summary, date, push_contributor};
use crate::{FormatError, MAX_ENTRY_BYTES, MAX_FIELD_CHARS};

/// The Palm database header; the record list follows it.
const HEADER_BYTES: usize = 78;
/// Record 0 is a few kilobytes; anything far larger isn't a real header.
const MAX_RECORD0_BYTES: u64 = 1024 * 1024;

/// Whether the file is a Palm database holding a MOBI book.
pub(crate) fn is_mobi(magic: &[u8]) -> bool {
    magic.get(60..68) == Some(b"BOOKMOBI")
}

/// What the MOBI header says, and the record offsets to find images.
struct Book {
    /// 8 for a KF8 (AZW3) book, 6 or lower for an older MOBI.
    version: u32,
    metadata: Metadata,
    cover_record: Option<usize>,
    offsets: Vec<u64>,
    file_len: u64,
}

/// Whether the book is KF8, sold as AZW3, rather than an older MOBI.
pub(crate) fn is_kf8(path: &Path) -> Result<bool, FormatError> {
    Ok(read(path)?.version >= 8)
}

pub(crate) fn metadata(path: &Path) -> Result<Metadata, FormatError> {
    Ok(read(path)?.metadata)
}

/// The cover image's bytes, or `None` when the book doesn't mark one.
pub(crate) fn cover_image(path: &Path) -> Result<Option<Vec<u8>>, FormatError> {
    let book = read(path)?;
    let Some(index) = book.cover_record else {
        return Ok(None);
    };
    let mut file = File::open(path)?;
    Ok(Some(record(
        &mut file,
        &book.offsets,
        index,
        book.file_len,
        MAX_ENTRY_BYTES,
    )?))
}

fn read(path: &Path) -> Result<Book, FormatError> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    let mut header = [0u8; HEADER_BYTES];
    file.read_exact(&mut header)?;
    if !is_mobi(&header) {
        return Err(FormatError::Invalid("not a MOBI book"));
    }
    let count = usize::from(u16::from_be_bytes([header[76], header[77]]));
    if count == 0 {
        return Err(FormatError::Invalid("the book has no records"));
    }
    let mut list = vec![0u8; count * 8];
    file.read_exact(&mut list)?;
    let offsets: Vec<u64> = list
        .chunks_exact(8)
        .map(|r| u64::from(u32::from_be_bytes([r[0], r[1], r[2], r[3]])))
        .collect();
    let record0 = record(&mut file, &offsets, 0, file_len, MAX_RECORD0_BYTES)?;

    let u32_at = |at: usize| -> Option<u32> {
        record0
            .get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    if record0.get(16..20) != Some(b"MOBI") {
        return Err(FormatError::Invalid("the book has no MOBI header"));
    }
    let header_len = u32_at(20).unwrap_or(0) as usize;
    let utf8 = u32_at(28) == Some(65001);
    let decode = |bytes: &[u8]| -> String {
        if utf8 {
            String::from_utf8_lossy(bytes).into_owned()
        } else {
            bytes.iter().map(|&b| cp1252(b)).collect()
        }
    };
    let version = u32_at(36).unwrap_or(0);
    let full_name = match (u32_at(84), u32_at(88)) {
        (Some(at), Some(len)) => record0
            .get(at as usize..(at as usize).saturating_add(len as usize))
            .map(&decode),
        _ => None,
    };
    let first_image = u32_at(108).filter(|&i| i != u32::MAX);
    let has_exth = u32_at(128).is_some_and(|flags| flags & 0x40 != 0);

    let mut metadata = Metadata::default();
    let mut updated_title = None;
    let mut cover_offset = None;
    let exth_at = 16 + header_len;
    if has_exth && record0.get(exth_at..exth_at + 4) == Some(b"EXTH") {
        let count = u32_at(exth_at + 8).unwrap_or(0);
        let mut at = exth_at + 12;
        for _ in 0..count {
            let (Some(kind), Some(len)) = (u32_at(at), u32_at(at + 4)) else {
                break;
            };
            let len = len as usize;
            let Some(data) = record0.get(at + 8..at.saturating_add(len)) else {
                break;
            };
            match kind {
                100 => push_contributor(&mut metadata.contributors, &decode(data), Role::Author),
                101 => metadata.publisher = clean(&decode(data), MAX_FIELD_CHARS),
                103 => metadata.summary = clean_summary(&strip_tags(&decode(data))),
                106 => metadata.published = date(decode(data).trim()),
                201 if data.len() == 4 => {
                    cover_offset = Some(u32::from_be_bytes([data[0], data[1], data[2], data[3]]));
                }
                503 => updated_title = clean(&decode(data), MAX_FIELD_CHARS),
                524 => metadata.language = clean(&decode(data), MAX_FIELD_CHARS),
                _ => {}
            }
            if len < 8 {
                break;
            }
            at += len;
        }
    }
    metadata.title = updated_title.or_else(|| full_name.and_then(|n| clean(&n, MAX_FIELD_CHARS)));
    let cover_record = match (first_image, cover_offset) {
        (Some(first), Some(offset)) => (first as usize)
            .checked_add(offset as usize)
            .filter(|&i| i < offsets.len()),
        _ => None,
    };
    Ok(Book {
        version,
        metadata,
        cover_record,
        offsets,
        file_len,
    })
}

/// One record's bytes, which run to the next record's offset or the end of
/// the file.
fn record(
    file: &mut File,
    offsets: &[u64],
    index: usize,
    file_len: u64,
    limit: u64,
) -> Result<Vec<u8>, FormatError> {
    let start = offsets[index];
    let end = offsets.get(index + 1).copied().unwrap_or(file_len);
    if end < start || end > file_len {
        return Err(FormatError::Invalid("a record lies outside the file"));
    }
    if end - start > limit {
        return Err(FormatError::EntryTooLarge { limit });
    }
    let mut out = vec![0u8; (end - start) as usize];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut out)?;
    Ok(out)
}

/// Descriptions are often HTML; summaries are kept as plain text.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

/// Windows-1252, the encoding of older MOBI books. It matches Latin-1 except
/// for 0x80 to 0x9F.
fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9f => HIGH[usize::from(b - 0x80)],
        b => char::from(b),
    }
}
