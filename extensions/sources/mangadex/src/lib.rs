//! MangaDex, through its public API: <https://api.mangadex.org/docs/>.
//!
//! Lists English chapters. Listings are cached briefly so browsing doesn't
//! repeat requests; page lists aren't, because the image servers MangaDex
//! hands out expire within minutes.

use std::collections::HashMap;

use chek_extension::prelude::*;
use serde::Deserialize;

const API: &str = "https://api.mangadex.org";
const COVERS: &str = "https://uploads.mangadex.org/covers";
const LANGUAGE: &str = "en";
const PAGE_SIZE: u32 = 20;
/// The feed's largest page.
const FEED_PAGE: u32 = 500;
/// Enough for the longest series without letting one call run unbounded.
const MAX_FEED_PAGES: u32 = 20;
const RATINGS: &str = "contentRating[]=safe&contentRating[]=suggestive";

struct MangaDex;

#[derive(Deserialize)]
struct List<T> {
    data: Vec<T>,
    #[serde(default)]
    total: u32,
}

#[derive(Deserialize)]
struct One<T> {
    data: T,
}

#[derive(Deserialize)]
struct Manga {
    id: String,
    attributes: MangaAttributes,
    #[serde(default)]
    relationships: Vec<Relationship>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MangaAttributes {
    #[serde(default)]
    title: HashMap<String, String>,
    #[serde(default)]
    alt_titles: Vec<HashMap<String, String>>,
    #[serde(default)]
    description: HashMap<String, String>,
    #[serde(default)]
    original_language: String,
    #[serde(default)]
    tags: Vec<Tag>,
}

#[derive(Deserialize)]
struct Tag {
    attributes: TagAttributes,
}

#[derive(Deserialize)]
struct TagAttributes {
    #[serde(default)]
    name: HashMap<String, String>,
}

#[derive(Deserialize)]
struct Relationship {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    attributes: Option<Value>,
}

#[derive(Deserialize)]
struct ChapterData {
    id: String,
    attributes: ChapterAttributes,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterAttributes {
    title: Option<String>,
    volume: Option<String>,
    chapter: Option<String>,
    #[serde(default)]
    pages: u32,
    version: Option<u32>,
    publish_at: Option<String>,
    external_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtHome {
    base_url: String,
    chapter: AtHomeChapter,
}

#[derive(Deserialize)]
struct AtHomeChapter {
    hash: String,
    data: Vec<String>,
}

impl Manga {
    fn title(&self) -> String {
        let a = &self.attributes;
        a.title
            .get(LANGUAGE)
            .or_else(|| a.alt_titles.iter().find_map(|t| t.get(LANGUAGE)))
            .or_else(|| a.title.values().next())
            .cloned()
            .unwrap_or_default()
    }

    fn cover(&self) -> Option<Image> {
        let file = self
            .relationships
            .iter()
            .find(|r| r.kind == "cover_art")?
            .attributes
            .as_ref()?
            .get("fileName")?
            .as_str()?;
        Some(image(format!("{COVERS}/{}/{file}.512.jpg", self.id)))
    }

    fn reading_direction(&self) -> ReadingDirection {
        let long_strip = self.attributes.tags.iter().any(|t| {
            t.attributes
                .name
                .get("en")
                .is_some_and(|n| n == "Long Strip")
        });
        if long_strip {
            ReadingDirection::Ttb
        } else if self.attributes.original_language == "ja" {
            ReadingDirection::Rtl
        } else {
            ReadingDirection::Ltr
        }
    }

    fn contributors(&self) -> Vec<Contributor> {
        self.relationships
            .iter()
            .filter_map(|r| {
                let role = match r.kind.as_str() {
                    "author" => "author",
                    "artist" => "artist",
                    _ => return None,
                };
                let name = r.attributes.as_ref()?.get("name")?.as_str()?;
                Some(Contributor {
                    name: name.to_owned(),
                    role: role.to_owned(),
                })
            })
            .collect()
    }
}

impl Guest for MangaDex {
    fn manifest() -> Manifest {
        Manifest {
            id: "page.chek.mangadex".into(),
            name: "MangaDex".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            sites: vec![
                "api.mangadex.org".into(),
                "uploads.mangadex.org".into(),
                // Chapter pages come from MangaDex@Home servers.
                "*.mangadex.network".into(),
            ],
            languages: vec![LANGUAGE.into()],
            capabilities: vec![],
        }
    }

    fn search(query: String, page: u32) -> Result<SeriesPage, Error> {
        let offset = page.saturating_sub(1) * PAGE_SIZE;
        let order = if query.trim().is_empty() {
            "order[followedCount]=desc".to_owned()
        } else {
            format!("title={}&order[relevance]=desc", encode(query.trim()))
        };
        let list: List<Manga> = Get::new(format!(
            "{API}/manga?{order}&limit={PAGE_SIZE}&offset={offset}&includes[]=cover_art\
             &availableTranslatedLanguage[]={LANGUAGE}&{RATINGS}"
        ))
        .cache(600)
        .json()?;
        Ok(SeriesPage {
            has_more: offset + PAGE_SIZE < list.total,
            items: list
                .data
                .iter()
                .map(|m| SeriesItem {
                    id: m.id.clone(),
                    title: m.title(),
                    cover: m.cover(),
                })
                .collect(),
        })
    }

    fn series(id: String) -> Result<Series, Error> {
        let manga: One<Manga> = Get::new(format!(
            "{API}/manga/{}?includes[]=cover_art&includes[]=author&includes[]=artist",
            encode(&id)
        ))
        .cache(3600)
        .json()?;
        let manga = manga.data;
        Ok(Series {
            title: manga.title(),
            summary: manga.attributes.description.get(LANGUAGE).cloned(),
            cover: manga.cover(),
            contributors: manga.contributors(),
            reading_direction: Some(manga.reading_direction()),
            id,
        })
    }

    fn chapters(series_id: String) -> Result<Vec<Chapter>, Error> {
        let mut chapters = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for page in 0..MAX_FEED_PAGES {
            let offset = page * FEED_PAGE;
            let list: List<ChapterData> = Get::new(format!(
                "{API}/manga/{}/feed?translatedLanguage[]={LANGUAGE}&limit={FEED_PAGE}\
                 &offset={offset}&order[volume]=asc&order[chapter]=asc&includeExternalUrl=0\
                 &{RATINGS}&contentRating[]=erotica",
                encode(&series_id)
            ))
            .cache(600)
            .json()?;
            for chapter in &list.data {
                let a = &chapter.attributes;
                // External chapters are read on the publisher's site.
                if a.pages == 0 || a.external_url.is_some() {
                    continue;
                }
                // Several groups often translate the same chapter; the
                // first listed is kept.
                if let Some(number) = &a.chapter
                    && !seen.insert((a.volume.clone(), number.clone()))
                {
                    continue;
                }
                chapters.push(Chapter {
                    id: chapter.id.clone(),
                    title: chapter_title(a),
                    number: a.chapter.clone(),
                    version: a.version.map(|v| v.to_string()),
                    published: a.publish_at.clone(),
                });
            }
            if offset + FEED_PAGE >= list.total {
                break;
            }
        }
        Ok(chapters)
    }

    fn pages(chapter_id: String) -> Result<Vec<Image>, Error> {
        let home: AtHome =
            Get::new(format!("{API}/at-home/server/{}", encode(&chapter_id))).json()?;
        Ok(home
            .chapter
            .data
            .iter()
            .map(|file| {
                image(format!(
                    "{}/data/{}/{file}",
                    home.base_url, home.chapter.hash
                ))
            })
            .collect())
    }
}

fn chapter_title(a: &ChapterAttributes) -> String {
    let title = a.title.as_deref().map(str::trim).filter(|t| !t.is_empty());
    match (&a.chapter, title) {
        (Some(number), Some(title)) => format!("Chapter {number}: {title}"),
        (Some(number), None) => format!("Chapter {number}"),
        (None, Some(title)) => title.to_owned(),
        (None, None) => "Oneshot".to_owned(),
    }
}

export_source!(MangaDex);
