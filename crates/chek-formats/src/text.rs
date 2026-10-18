//! Plain text and Markdown books. They have no package to read, so metadata
//! comes from conventions: a Project Gutenberg header in plain text, and
//! front matter or the first heading in Markdown.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::metadata::{Metadata, Role, clean, clean_summary, date, push_contributor};
use crate::{FormatError, MAX_FIELD_CHARS};

/// Metadata sits at the start, so only this much is read.
const HEAD_BYTES: u64 = 64 * 1024;

/// Whether the file's start looks like text rather than binary data: no NUL
/// bytes, and few other control characters.
pub(crate) fn is_text(prefix: &[u8]) -> bool {
    if prefix.contains(&0) {
        return false;
    }
    let controls = prefix
        .iter()
        .filter(|&&b| b < 0x20 && !matches!(b, b'\n' | b'\r' | b'\t' | 0x0c))
        .count();
    controls * 100 <= prefix.len()
}

pub(crate) fn plain_metadata(path: &Path) -> Result<Metadata, FormatError> {
    let head = head(path)?;
    let mut metadata = Metadata::default();
    // Project Gutenberg texts open with "Title:", "Author:" and similar
    // lines before the book starts.
    for line in head.lines().take(200) {
        if line.contains("*** START OF") {
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "Title" if metadata.title.is_none() => metadata.title = clean(value, MAX_FIELD_CHARS),
            "Author" => author_list(&mut metadata, value),
            "Translator" => push_contributor(&mut metadata.contributors, value, Role::Translator),
            "Language" if metadata.language.is_none() => metadata.language = language_tag(value),
            _ => {}
        }
    }
    Ok(metadata)
}

pub(crate) fn markdown_metadata(path: &Path) -> Result<Metadata, FormatError> {
    let head = head(path)?;
    let head = head.trim_start_matches('\u{feff}');
    let mut metadata = Metadata::default();
    let mut body = head;
    if let Some(rest) = head
        .strip_prefix("---\n")
        .or_else(|| head.strip_prefix("---\r\n"))
        && let Some(end) = rest.find("\n---")
    {
        front_matter(&mut metadata, &rest[..end]);
        body = &rest[end + 4..];
    }
    if metadata.title.is_none() {
        metadata.title = body
            .lines()
            .find_map(|line| line.strip_prefix("# "))
            .and_then(|title| clean(title.trim_end_matches('#'), MAX_FIELD_CHARS));
    }
    Ok(metadata)
}

/// Gutenberg names the language in English; the catalog keeps BCP 47 tags.
fn language_tag(name: &str) -> Option<String> {
    let name = name.trim();
    let tag = match name.to_ascii_lowercase().as_str() {
        "english" => "en",
        "french" => "fr",
        "german" => "de",
        "spanish" => "es",
        "italian" => "it",
        "portuguese" => "pt",
        "dutch" => "nl",
        "russian" => "ru",
        "finnish" => "fi",
        "swedish" => "sv",
        "danish" => "da",
        "norwegian" => "no",
        "polish" => "pl",
        "latin" => "la",
        "greek" => "el",
        "chinese" => "zh",
        "japanese" => "ja",
        "esperanto" => "eo",
        // Already a tag, such as "en" or "pt-BR".
        _ if name.len() <= 8
            && name
                .split('-')
                .all(|p| p.chars().all(|c| c.is_ascii_alphanumeric())) =>
        {
            return clean(name, MAX_FIELD_CHARS);
        }
        _ => return None,
    };
    Some(tag.to_owned())
}

/// Reads the simple `key: value` and `- item` lines of YAML front matter,
/// which is all books' front matter uses.
fn front_matter(metadata: &mut Metadata, yaml: &str) {
    let mut list_key: Option<String> = None;
    for line in yaml.lines() {
        if let Some(item) = line.trim_start().strip_prefix("- ") {
            if matches!(list_key.as_deref(), Some("author" | "authors")) {
                push_contributor(&mut metadata.contributors, unquote(item), Role::Author);
            }
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = unquote(value.trim());
        list_key = value.is_empty().then(|| key.clone());
        match key.as_str() {
            "title" => metadata.title = clean(value, MAX_FIELD_CHARS),
            "author" | "authors" if !value.is_empty() => author_list(metadata, value),
            "series" => metadata.series = clean(value, MAX_FIELD_CHARS),
            "description" | "summary" => metadata.summary = clean_summary(value),
            "publisher" => metadata.publisher = clean(value, MAX_FIELD_CHARS),
            "lang" | "language" => metadata.language = clean(value, MAX_FIELD_CHARS),
            "date" => metadata.published = date(value),
            _ => {}
        }
    }
}

/// Several authors are separated by commas, "and" or "&", or written as a
/// YAML flow list.
fn author_list(metadata: &mut Metadata, value: &str) {
    let value = value.trim().trim_start_matches('[').trim_end_matches(']');
    for name in value.split([',', '&']).flat_map(|part| part.split(" and ")) {
        push_contributor(
            &mut metadata.contributors,
            unquote(name.trim()),
            Role::Author,
        );
    }
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value)
}

/// The start of the file as text. Bytes that aren't UTF-8 are replaced, so
/// a Latin-1 file still gives readable ASCII fields.
fn head(path: &Path) -> Result<String, FormatError> {
    let mut bytes = Vec::new();
    File::open(path)?.take(HEAD_BYTES).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
