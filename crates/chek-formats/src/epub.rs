//! Just enough EPUB to find the cover and metadata: the container points at
//! the package document, whose manifest marks the cover.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event;
use zip::ZipArchive;

use crate::metadata::{self, Metadata, attribute};
use crate::{FormatError, MAX_ENTRY_BYTES};

/// XML documents larger than this are not parsed.
pub(crate) const MAX_XML_BYTES: u64 = 4 * 1024 * 1024;

/// Returns the cover image's bytes, or `None` when the book doesn't mark one.
pub(crate) fn cover_image(path: &Path) -> Result<Option<Vec<u8>>, FormatError> {
    let mut archive = ZipArchive::new(BufReader::new(File::open(path)?))?;
    let Some((package_path, package)) = package(&mut archive)? else {
        return Ok(None);
    };
    let Some(href) = cover_href(&package) else {
        return Ok(None);
    };
    let base = package_path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let Some(entry) = resolve(base, &percent_decode(&href)) else {
        return Ok(None);
    };
    read_entry(&mut archive, &entry, MAX_ENTRY_BYTES)
}

/// Reads the book's metadata from its package document.
pub(crate) fn metadata(path: &Path) -> Result<Metadata, FormatError> {
    let mut archive = ZipArchive::new(BufReader::new(File::open(path)?))?;
    Ok(package(&mut archive)?
        .map(|(_, package)| metadata::epub_package(&package))
        .unwrap_or_default())
}

/// The package document's path and bytes, found through the container file.
fn package<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<Option<(String, Vec<u8>)>, FormatError> {
    let Some(container) = read_entry(archive, "META-INF/container.xml", MAX_XML_BYTES)? else {
        return Ok(None);
    };
    let Some(package_path) = first_attribute(&container, "rootfile", "full-path") else {
        return Ok(None);
    };
    Ok(read_entry(archive, &package_path, MAX_XML_BYTES)?.map(|p| (package_path, p)))
}

pub(crate) fn read_entry<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Result<Option<Vec<u8>>, FormatError> {
    let mut entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if entry.size() > limit {
        return Err(FormatError::EntryTooLarge { limit });
    }
    let mut out = Vec::with_capacity(entry.size() as usize);
    (&mut entry).take(limit + 1).read_to_end(&mut out)?;
    if out.len() as u64 > limit {
        return Err(FormatError::EntryTooLarge { limit });
    }
    Ok(Some(out))
}

struct Item {
    id: String,
    href: String,
    media_type: String,
    properties: String,
}

/// Finds the cover in the package document: the EPUB 3 `cover-image`
/// property first, then the EPUB 2 `<meta name="cover">` pointer, then an
/// image item named like a cover.
fn cover_href(package: &[u8]) -> Option<String> {
    let mut reader = Reader::from_reader(package);
    let mut items = Vec::new();
    let mut cover_id = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) => match e.local_name().as_ref() {
                "item" => items.push(Item {
                    id: attribute(&e, "id").unwrap_or_default(),
                    href: attribute(&e, "href").unwrap_or_default(),
                    media_type: attribute(&e, "media-type").unwrap_or_default(),
                    properties: attribute(&e, "properties").unwrap_or_default(),
                }),
                "meta" if attribute(&e, "name").as_deref() == Some("cover") => {
                    cover_id = attribute(&e, "content");
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    let is_image = |item: &&Item| item.media_type.starts_with("image/");
    items
        .iter()
        .filter(is_image)
        .find(|i| {
            i.properties
                .split_ascii_whitespace()
                .any(|p| p == "cover-image")
        })
        .or_else(|| {
            let id = cover_id.as_deref()?;
            items.iter().filter(is_image).find(|i| i.id == id)
        })
        .or_else(|| {
            items.iter().filter(is_image).find(|i| {
                i.id.to_ascii_lowercase().contains("cover")
                    || i.href.to_ascii_lowercase().contains("cover")
            })
        })
        .map(|i| i.href.clone())
}

fn first_attribute(xml: &[u8], element: &str, name: &str) -> Option<String> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == element => {
                return attribute(&e, name);
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
}

/// Joins an href to the package document's folder, resolving `..` and
/// rejecting paths that climb out of the archive.
fn resolve(base: &str, href: &str) -> Option<String> {
    let href = href.split(['#', '?']).next()?;
    let mut parts: Vec<&str> = if href.starts_with('/') {
        Vec::new()
    } else {
        base.split('/').filter(|p| !p.is_empty()).collect()
    };
    for part in href.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
}
