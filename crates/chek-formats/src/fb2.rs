//! FB2 (FictionBook): one XML document holding the book, its metadata in
//! `<description>` and its images as base64 `<binary>` elements. Many FB2
//! books are in Windows-1251 or another legacy encoding, so the document is
//! decoded to UTF-8 by its declaration before parsing. The document is often
//! zipped, as `.fb2.zip` or `.fbz`, with the book as the archive's one
//! `.fb2` entry.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::metadata::{
    Metadata, Role, attribute, clean, clean_summary, date, element_text, push_contributor,
};
use crate::{FormatError, MAX_ENTRY_BYTES, MAX_FIELD_CHARS};

/// FB2 books carry their images inline, so they can be large; larger ones
/// aren't parsed.
const MAX_DOCUMENT_BYTES: u64 = 128 * 1024 * 1024;

/// Whether the file's start looks like a FictionBook document.
pub(crate) fn is_fb2(prefix: &[u8]) -> bool {
    let text = String::from_utf8_lossy(prefix);
    text.trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with('<')
        && text.contains("<FictionBook")
}

pub(crate) fn metadata(path: &Path) -> Result<Metadata, FormatError> {
    Ok(parse(&document(path)?, false).0)
}

/// The cover image's bytes, or `None` when the book doesn't mark one.
pub(crate) fn cover_image(path: &Path) -> Result<Option<Vec<u8>>, FormatError> {
    Ok(parse(&document(path)?, true).1)
}

/// The name of the `.fb2` entry in a zipped book, or `None` when the archive
/// holds no FictionBook. The first `.fb2` counts; some archives also carry
/// a cover image or notes.
pub(crate) fn zipped_entry(path: &Path) -> Result<Option<String>, FormatError> {
    let archive = zip::ZipArchive::new(BufReader::new(File::open(path)?))?;
    Ok(archive
        .file_names()
        .filter_map(Result::ok)
        .find(|name| name.to_ascii_lowercase().ends_with(".fb2"))
        .map(|name| name.into_owned()))
}

/// The document as UTF-8, from the file or from the `.fb2` entry of a ZIP.
fn document(path: &Path) -> Result<String, FormatError> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 4];
    let read = file.read(&mut magic)?;
    let bytes = if magic[..read].starts_with(b"PK\x03\x04") {
        let mut archive = zip::ZipArchive::new(BufReader::new(File::open(path)?))?;
        let Some(name) = zipped_entry(path)? else {
            return Err(FormatError::Invalid("the ZIP holds no .fb2 entry"));
        };
        crate::epub::read_entry(&mut archive, &name, MAX_DOCUMENT_BYTES)?
            .ok_or(FormatError::Invalid("the ZIP holds no .fb2 entry"))?
    } else {
        if file.metadata()?.len() > MAX_DOCUMENT_BYTES {
            return Err(FormatError::EntryTooLarge {
                limit: MAX_DOCUMENT_BYTES,
            });
        }
        let mut bytes = magic[..read].to_vec();
        file.take(MAX_DOCUMENT_BYTES).read_to_end(&mut bytes)?;
        bytes
    };
    let label = declared_encoding(&bytes);
    let encoding = label
        .and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    // A byte order mark wins over the declaration, as XML requires.
    let (text, _, _) = encoding.decode(&bytes);
    Ok(text.into_owned())
}

/// The `encoding` of the XML declaration, if there is one.
fn declared_encoding(bytes: &[u8]) -> Option<String> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]);
    let declaration = head.split_once("?>")?.0;
    let (_, rest) = declaration.split_once("encoding")?;
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value = rest[1..].split(quote).next()?;
    Some(value.trim().to_owned())
}

/// Reads the `<title-info>` and `<publish-info>` metadata, and with `cover`,
/// the cover image the title info points at.
fn parse(xml: &str, cover: bool) -> (Metadata, Option<Vec<u8>>) {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut metadata = Metadata::default();
    let mut path: Vec<String> = Vec::new();
    let mut cover_id = None;
    let mut author = Author::default();
    let mut image = None;
    loop {
        let event = reader.read_event_into(&mut buf);
        match event {
            Ok(Event::Start(e)) => {
                let name = e.local_name().as_ref().to_owned();
                let parent = path.last().map(String::as_str);
                let in_title_info = path.iter().any(|p| p == "title-info");
                let in_publish_info = path.iter().any(|p| p == "publish-info");
                match (name.as_str(), parent) {
                    // The book's text starts here; nothing after it but
                    // images is metadata.
                    ("body", _) if !cover || cover_id.is_none() => break,
                    ("binary", _) => {
                        let id = attribute(&e, "id");
                        buf.clear();
                        let data = element_text(&mut reader, &mut buf);
                        if cover && id.is_some() && id == cover_id {
                            image = data.and_then(|d| decode_image(&d));
                            break;
                        }
                        continue;
                    }
                    ("book-title", _) if in_title_info => {
                        buf.clear();
                        metadata.title = element_text(&mut reader, &mut buf)
                            .and_then(|t| clean(&t, MAX_FIELD_CHARS));
                        continue;
                    }
                    ("annotation", _) if in_title_info => {
                        buf.clear();
                        metadata.summary =
                            element_text(&mut reader, &mut buf).and_then(|t| clean_summary(&t));
                        continue;
                    }
                    ("lang", _) if in_title_info => {
                        buf.clear();
                        metadata.language = element_text(&mut reader, &mut buf)
                            .and_then(|t| clean(&t, MAX_FIELD_CHARS));
                        continue;
                    }
                    ("date", _) if in_title_info => {
                        let value = attribute(&e, "value");
                        buf.clear();
                        let text = element_text(&mut reader, &mut buf);
                        metadata.published = value
                            .as_deref()
                            .and_then(date)
                            .or_else(|| text.as_deref().and_then(|t| date(t.trim())));
                        continue;
                    }
                    ("publisher", _) if in_publish_info => {
                        buf.clear();
                        metadata.publisher = element_text(&mut reader, &mut buf)
                            .and_then(|t| clean(&t, MAX_FIELD_CHARS));
                        continue;
                    }
                    ("year", _) if in_publish_info && metadata.published.is_none() => {
                        buf.clear();
                        metadata.published =
                            element_text(&mut reader, &mut buf).and_then(|t| date(t.trim()));
                        continue;
                    }
                    (
                        "first-name" | "middle-name" | "last-name" | "nickname",
                        Some("author" | "translator"),
                    ) if in_title_info => {
                        buf.clear();
                        let text = element_text(&mut reader, &mut buf).unwrap_or_default();
                        author.set(&name, text);
                        continue;
                    }
                    _ => {}
                }
                path.push(name);
            }
            Ok(Event::Empty(e)) => {
                let name = e.local_name();
                let in_title_info = path.iter().any(|p| p == "title-info");
                if in_title_info && name.as_ref() == "sequence" && metadata.series.is_none() {
                    metadata.series =
                        attribute(&e, "name").and_then(|n| clean(&n, MAX_FIELD_CHARS));
                    metadata.number =
                        attribute(&e, "number").and_then(|n| clean(&n, MAX_FIELD_CHARS));
                } else if in_title_info
                    && name.as_ref() == "image"
                    && path.last().map(String::as_str) == Some("coverpage")
                {
                    cover_id = attribute(&e, "href").map(|h| h.trim_start_matches('#').to_owned());
                }
            }
            Ok(Event::End(_)) => {
                if let Some(name) = path.pop()
                    && path.iter().any(|p| p == "title-info")
                {
                    match name.as_str() {
                        "author" => author.take(&mut metadata, Role::Author),
                        "translator" => author.take(&mut metadata, Role::Translator),
                        _ => {}
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (metadata, image)
}

#[derive(Default)]
struct Author {
    first: String,
    middle: String,
    last: String,
    nickname: String,
}

impl Author {
    fn set(&mut self, field: &str, value: String) {
        match field {
            "first-name" => self.first = value,
            "middle-name" => self.middle = value,
            "last-name" => self.last = value,
            _ => self.nickname = value,
        }
    }

    fn take(&mut self, metadata: &mut Metadata, role: Role) {
        let author = std::mem::take(self);
        let full = [author.first, author.middle, author.last].join(" ");
        let name = if full.trim().is_empty() {
            author.nickname
        } else {
            full
        };
        push_contributor(&mut metadata.contributors, &name, role);
    }
}

fn decode_image(base64: &str) -> Option<Vec<u8>> {
    let compact: String = base64.split_ascii_whitespace().collect();
    if compact.len() as u64 > MAX_ENTRY_BYTES / 3 * 4 {
        return None;
    }
    STANDARD.decode(compact).ok()
}
