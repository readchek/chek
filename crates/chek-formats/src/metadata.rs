//! Publication metadata read from the file itself: the EPUB package
//! document, or `ComicInfo.xml` in a comic archive. PDFs are read in
//! [`crate::pdf`].

use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

/// Metadata found in a file. Every field is optional, since files often
/// carry none of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    pub title: Option<String>,
    /// The name of the series the publication belongs to.
    pub series: Option<String>,
    /// The publication's number in its series, as written: `"3"`, `"1.5"`,
    /// `"Vol. 2"`.
    pub number: Option<String>,
    pub contributors: Vec<Contributor>,
    /// May contain HTML, as EPUB descriptions often do.
    pub summary: Option<String>,
    pub publisher: Option<String>,
    /// A BCP 47 language tag.
    pub language: Option<String>,
    /// `YYYY`, `YYYY-MM` or `YYYY-MM-DD`.
    pub published: Option<String>,
    pub reading_direction: Option<ReadingDirection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contributor {
    pub name: String,
    pub role: Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Author,
    Illustrator,
    Penciler,
    Inker,
    Colorist,
    Letterer,
    CoverArtist,
    Editor,
    Translator,
    Contributor,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Author => "author",
            Role::Illustrator => "illustrator",
            Role::Penciler => "penciler",
            Role::Inker => "inker",
            Role::Colorist => "colorist",
            Role::Letterer => "letterer",
            Role::CoverArtist => "coverArtist",
            Role::Editor => "editor",
            Role::Translator => "translator",
            Role::Contributor => "contributor",
        }
    }

    /// Maps a MARC relator code, as EPUB uses for roles.
    fn from_marc(code: &str) -> Self {
        match code.trim().to_ascii_lowercase().as_str() {
            "aut" => Role::Author,
            "ill" | "art" => Role::Illustrator,
            "edt" => Role::Editor,
            "trl" => Role::Translator,
            "cov" => Role::CoverArtist,
            "clr" => Role::Colorist,
            _ => Role::Contributor,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadingDirection {
    Ltr,
    Rtl,
}

impl ReadingDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            ReadingDirection::Ltr => "ltr",
            ReadingDirection::Rtl => "rtl",
        }
    }
}

/// Text fields longer than this are cut, so one odd file can't bloat the catalog.
pub(crate) const MAX_FIELD_CHARS: usize = 1_000;
const MAX_SUMMARY_CHARS: usize = 20_000;
const MAX_CONTRIBUTORS: usize = 50;

/// Parses a `ComicInfo.xml` document, the de facto metadata for comic archives.
pub(crate) fn comic_info(xml: &[u8]) -> Metadata {
    let mut reader = Reader::from_reader(xml);
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut buf = Vec::new();
    let mut depth = 0;
    loop {
        match reader.read_event_into(&mut buf) {
            // Fields are the root element's children; deeper elements such
            // as page lists are skipped.
            Ok(Event::Start(e)) if depth == 1 => {
                let name = e.local_name().as_ref().to_owned();
                buf.clear();
                match element_text(&mut reader, &mut buf) {
                    Some(text) => fields.push((name, text)),
                    None => break,
                }
            }
            Ok(Event::Start(_)) => depth += 1,
            Ok(Event::End(_)) => depth -= 1,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    let field = |name: &str| {
        fields
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| clean(v, MAX_FIELD_CHARS))
    };

    let mut contributors = Vec::new();
    for (element, role) in [
        ("Writer", Role::Author),
        ("Penciller", Role::Penciler),
        ("Inker", Role::Inker),
        ("Colorist", Role::Colorist),
        ("Letterer", Role::Letterer),
        ("CoverArtist", Role::CoverArtist),
        ("Editor", Role::Editor),
        ("Translator", Role::Translator),
    ] {
        // Several people in one role are separated by commas.
        for name in field(element).iter().flat_map(|v| v.split(',')) {
            push_contributor(&mut contributors, name, role);
        }
    }

    let number = field("Number").or_else(|| field("Volume").map(|v| format!("Vol. {v}")));
    let published = field("Year").and_then(|year| {
        let month = field("Month").and_then(|m| m.parse::<u8>().ok());
        let day = field("Day").and_then(|d| d.parse::<u8>().ok());
        date(&match (month, day) {
            (Some(m), Some(d)) => format!("{year}-{m:02}-{d:02}"),
            (Some(m), None) => format!("{year}-{m:02}"),
            _ => year,
        })
    });
    let reading_direction = match field("Manga").as_deref() {
        Some("YesAndRightToLeft") => Some(ReadingDirection::Rtl),
        _ => None,
    };
    Metadata {
        title: field("Title"),
        series: field("Series"),
        number,
        contributors,
        summary: fields
            .iter()
            .find(|(n, _)| n == "Summary")
            .and_then(|(_, v)| clean_summary(v)),
        publisher: field("Publisher"),
        language: field("LanguageISO"),
        published,
        reading_direction,
    }
}

struct Creator {
    id: Option<String>,
    name: String,
    role: Option<String>,
    is_creator: bool,
}

/// Reads the `<metadata>` and `<spine>` of an EPUB package document.
pub(crate) fn epub_package(package: &[u8]) -> Metadata {
    let mut reader = Reader::from_reader(package);
    let mut buf = Vec::new();
    let mut metadata = Metadata::default();
    let mut creators: Vec<Creator> = Vec::new();
    // `<meta refines="#id" property="...">` values, which EPUB 3 uses for
    // roles and series positions.
    let mut refinements: Vec<(String, String, String)> = Vec::new();
    let mut collections: Vec<(Option<String>, String)> = Vec::new();
    let mut calibre_index = None;
    let mut calibre_series = None;
    loop {
        let event = reader.read_event_into(&mut buf);
        let (e, has_body) = match event {
            Ok(Event::Start(e)) => (e.into_owned(), true),
            Ok(Event::Empty(e)) => (e.into_owned(), false),
            Ok(Event::Eof) | Err(_) => break,
            _ => {
                buf.clear();
                continue;
            }
        };
        buf.clear();
        let local = e.local_name().as_ref().to_owned();
        if local == "spine" {
            metadata.reading_direction =
                match attribute(&e, "page-progression-direction").as_deref() {
                    Some("rtl") => Some(ReadingDirection::Rtl),
                    Some("ltr") => Some(ReadingDirection::Ltr),
                    _ => None,
                };
            continue;
        }
        // EPUB 2: `<meta name="calibre:series" content="Name"/>` and
        // `<meta name="calibre:series_index" content="3"/>`.
        if local == "meta" {
            match attribute(&e, "name").as_deref() {
                Some("calibre:series") => calibre_series = attribute(&e, "content"),
                Some("calibre:series_index") => calibre_index = attribute(&e, "content"),
                _ => {}
            }
        }
        // Only Dublin Core elements and EPUB 3 metas carry text we want;
        // reading any other element's text would swallow its children.
        if !has_body || !(is_dublin_core(&e) || local == "meta") {
            continue;
        }
        let Some(text) = element_text(&mut reader, &mut buf) else {
            break;
        };
        buf.clear();
        match (is_dublin_core(&e), local.as_str()) {
            (true, "title") if metadata.title.is_none() => {
                metadata.title = clean(&text, MAX_FIELD_CHARS);
            }
            (true, "creator" | "contributor") => creators.push(Creator {
                id: attribute(&e, "id"),
                name: text,
                role: attribute(&e, "role"),
                is_creator: local == "creator",
            }),
            (true, "description") if metadata.summary.is_none() => {
                metadata.summary = clean_summary(&text);
            }
            (true, "publisher") if metadata.publisher.is_none() => {
                metadata.publisher = clean(&text, MAX_FIELD_CHARS);
            }
            (true, "language") if metadata.language.is_none() => {
                metadata.language = clean(&text, MAX_FIELD_CHARS);
            }
            // EPUB 2 dates can name an event; only a publication date counts.
            (true, "date") if metadata.published.is_none() => {
                if attribute(&e, "event").is_none_or(|ev| ev == "publication") {
                    metadata.published = date(text.trim());
                }
            }
            (false, "meta") => {
                let property = attribute(&e, "property").unwrap_or_default();
                match attribute(&e, "refines") {
                    Some(target) => refinements.push((
                        target.trim_start_matches('#').to_owned(),
                        property,
                        text.trim().to_owned(),
                    )),
                    None if property == "belongs-to-collection" => {
                        collections.push((attribute(&e, "id"), text));
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    let refined = |id: &str, property: &str| {
        refinements
            .iter()
            .find(|(target, p, _)| target == id && p == property)
            .map(|(_, _, value)| value.clone())
    };
    for creator in creators {
        let code = creator
            .role
            .or_else(|| refined(creator.id.as_deref()?, "role"));
        // "Book producer" is the software that made the file, such as calibre.
        if code
            .as_deref()
            .is_some_and(|c| c.trim().eq_ignore_ascii_case("bkp"))
        {
            continue;
        }
        let role = code
            .map(|r| Role::from_marc(&r))
            .unwrap_or(if creator.is_creator {
                Role::Author
            } else {
                Role::Contributor
            });
        push_contributor(&mut metadata.contributors, &creator.name, role);
    }
    // A collection is a series unless it says it's a set of works.
    let series = collections.iter().find(|(id, _)| {
        id.as_deref()
            .is_none_or(|id| refined(id, "collection-type").is_none_or(|t| t == "series"))
    });
    let series_position = series
        .and_then(|(id, _)| id.as_deref())
        .and_then(|id| refined(id, "group-position"));
    metadata.series = series
        .map(|(_, name)| name.clone())
        .or(calibre_series)
        .and_then(|n| clean(&n, MAX_FIELD_CHARS));
    metadata.number = series_position
        .or(calibre_index)
        .and_then(|n| clean(&n, MAX_FIELD_CHARS));
    metadata
}

fn is_dublin_core(e: &BytesStart<'_>) -> bool {
    e.name().prefix().is_some_and(|p| p.as_ref() == "dc")
}

pub(crate) fn push_contributor(contributors: &mut Vec<Contributor>, name: &str, role: Role) {
    if contributors.len() >= MAX_CONTRIBUTORS {
        return;
    }
    if let Some(name) = clean(name, MAX_FIELD_CHARS) {
        contributors.push(Contributor { name, role });
    }
}

/// Trims, collapses runs of whitespace, drops empty values and cuts long ones.
pub(crate) fn clean(value: &str, max_chars: usize) -> Option<String> {
    cut(
        value.split_whitespace().collect::<Vec<_>>().join(" "),
        max_chars,
    )
}

/// Like [`clean`] but keeps line breaks, which plain-text summaries use for
/// paragraphs.
pub(crate) fn clean_summary(value: &str) -> Option<String> {
    let lines: Vec<String> = value
        .trim()
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    cut(lines.join("\n"), MAX_SUMMARY_CHARS)
}

fn cut(value: String, max_chars: usize) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    Some(match value.char_indices().nth(max_chars) {
        Some((cut, _)) => value[..cut].to_owned(),
        None => value,
    })
}

/// Keeps the `YYYY`, `YYYY-MM` or `YYYY-MM-DD` start of a date, so dates with
/// times or time zones still sort and display the same.
fn date(value: &str) -> Option<String> {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let mut parts = value.get(..value.len().min(10))?.split('-');
    let year = parts.next().filter(|y| y.len() == 4 && digits(y))?;
    let mut out = year.to_owned();
    for part in parts.take(2) {
        if part.len() != 2 || !digits(part) {
            break;
        }
        out.push('-');
        out.push_str(part);
    }
    Some(out)
}

/// Reads an element's text up to its end tag, resolving character and
/// predefined entity references. Text inside child elements is included.
/// `None` when the document is malformed or ends before the end tag, so a
/// cut-off file doesn't yield half a value.
pub(crate) fn element_text(reader: &mut Reader<&[u8]>, buf: &mut Vec<u8>) -> Option<String> {
    let mut text = String::new();
    let mut depth = 0;
    loop {
        match reader.read_event_into(buf) {
            Ok(Event::Text(t)) => text.push_str(&t.xml_content(XmlVersion::Implicit1_0)),
            Ok(Event::CData(t)) => text.push_str(&t.xml_content(XmlVersion::Implicit1_0)),
            Ok(Event::GeneralRef(r)) => {
                if let Ok(Some(c)) = r.resolve_char_ref() {
                    text.push(c);
                } else {
                    let entity = match r.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    };
                    text.push_str(entity);
                }
            }
            Ok(Event::Start(_)) => depth += 1,
            Ok(Event::End(_)) if depth == 0 => return Some(text),
            Ok(Event::End(_)) => depth -= 1,
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
}

pub(crate) fn attribute(element: &BytesStart<'_>, name: &str) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == name)
        .and_then(|a| {
            a.normalized_value(XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
}
