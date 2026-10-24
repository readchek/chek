//! Comix 1.6.54's WebView flow, ported to Chek browser sessions.
mod descrambler;

use chek_extension::prelude::*;
use chek_extension::{browser, browser_session, http::HttpError};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use std::collections::HashSet;

const SITE: &str = "https://comix.to";
const CLIENT: &str = include_str!("client.js");
const CATALOG: &str = include_str!("catalog.js");
const HARVEST_BROWSE: &str = include_str!("../upstream/harvest_browse.js");
const MAX_CHAPTER_PAGES: u32 = 20;

struct Comix;

#[derive(Deserialize)]
struct Listing<T> {
    items: Vec<T>,
    meta: Option<Pagination>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Pagination {
    page: u32,
    last_page: u32,
    #[serde(rename = "last_page")]
    old_last_page: u32,
    total: usize,
}

impl Pagination {
    fn last(&self) -> u32 {
        self.last_page.max(self.old_last_page).max(1)
    }
}

#[derive(Deserialize)]
struct Poster {
    small: Option<String>,
    medium: Option<String>,
    large: Option<String>,
}

impl Poster {
    fn cover(self) -> Option<Image> {
        self.medium
            .or(self.large)
            .or(self.small)
            .filter(|url| !url.is_empty())
            .map(image)
    }
}

#[derive(Deserialize)]
struct Person {
    title: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manga {
    hid: String,
    title: String,
    poster: Option<Poster>,
    synopsis: Option<String>,
    original_language: Option<String>,
    authors: Option<Vec<Person>>,
    author: Option<Vec<Person>>,
    artists: Option<Vec<Person>>,
    artist: Option<Vec<Person>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterData {
    id: u64,
    url: String,
    number: f64,
    name: Option<String>,
    group: Option<Group>,
    #[serde(default)]
    is_official: bool,
}

impl ChapterData {
    fn chapter(self) -> Result<Chapter, Error> {
        let path = self
            .url
            .split_once("/title/")
            .map(|(_, path)| path)
            .ok_or_else(|| Error::Parse("Invalid Comix chapter URL".into()))?;
        let number = self.number.to_string();
        let title = match self.name.filter(|name| !name.trim().is_empty()) {
            Some(name) => format!("Chapter {number}: {name}"),
            None => format!("Chapter {number}"),
        };
        Ok(Chapter {
            id: path.into(),
            title,
            number: Some(number),
            version: self
                .group
                .map(|group| group.name)
                .or_else(|| self.is_official.then(|| "Official".into())),
            published: None,
        })
    }
}

#[derive(Deserialize, Serialize)]
struct ChapterCursor {
    series: String,
    page: u32,
    total: usize,
}

#[derive(Deserialize)]
struct Group {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageList {
    #[serde(default)]
    base_url: String,
    items: Vec<Page>,
}

#[derive(Deserialize)]
struct Page {
    url: String,
    #[serde(default)]
    s: u8,
}

#[derive(Deserialize)]
struct Evaluation<T> {
    value: Option<T>,
    error: Option<String>,
}

struct Session;

impl Session {
    fn harvest<T: DeserializeOwned>(&self, script: &str) -> Result<T, Error> {
        self.read(&format!(
            "(async () => {{ window.__comixHarvestOptions = {{ timeoutMs: 18000 }}; {script}\n const result = await window.__comixHarvestPromise; if (result.error) throw new Error(result.error); return result; }})()"
        ))
    }

    fn open(path: &str) -> Result<Self, Error> {
        let url = format!("{SITE}{path}");
        let session = Self;
        browser_session::open(&url).map_err(Error::Network)?;
        if session.read::<bool>("catalogReady()")? {
            return Ok(session);
        }
        drop(session);
        match browser::load(&url) {
            Ok(response) if (200..300).contains(&response.status) => {}
            Ok(response) => {
                return Err(Error::Network(format!(
                    "Comix verification returned HTTP {}",
                    response.status
                )));
            }
            Err(HttpError::Failed(message)) if message == "no browser solver is configured" => {
                return Err(Error::Network(
                    "Comix verification is required; open verification and retry, or configure a browser solver".into(),
                ));
            }
            Err(error) => {
                return Err(Error::Network(format!(
                    "Comix verification failed: {error:?}"
                )));
            }
        }
        browser_session::open(&url).map_err(Error::Network)?;
        let session = Self;
        if !session.read::<bool>("catalogReady()")? {
            return Err(Error::Network(
                "Comix verification did not load the catalog".into(),
            ));
        }
        Ok(session)
    }

    fn evaluate<T: DeserializeOwned>(&self, expression: &str) -> Result<T, Error> {
        let result = browser_session::evaluate(&format!(
            "(async () => {{ {CLIENT}\n {CATALOG}\n try {{ return {{value: await ({expression})}}; }} catch (error) {{ return {{error: String(error.message || error)}}; }} }})()"
        ))
        .map_err(Error::Network)?;
        let result: Evaluation<T> =
            serde_json::from_str(&result).map_err(|e| Error::Parse(e.to_string()))?;
        match (result.value, result.error) {
            (_, Some(error)) => Err(Error::Network(error)),
            (Some(value), _) => Ok(value),
            _ => Err(Error::Parse("Comix returned no data".into())),
        }
    }

    fn initial<T: DeserializeOwned>(&self, detail: bool) -> Result<T, Error> {
        self.read(&format!("initialData({detail})"))
    }

    fn chapter_page(&self, id: &str, page: u32) -> Result<Listing<ChapterData>, Error> {
        if page > 1 {
            browser_session::open(&format!("{SITE}/title/{}?page={page}", encode(id)))
                .map_err(Error::Network)?;
        }
        let result = self.read(&format!("renderedChapters({}, {page})", json!(id)));
        result.map_err(|error| match error {
            Error::Network(message) => {
                Error::Network(format!("Comix chapter page {page}: {message}"))
            }
            other => other,
        })
    }

    fn read<T: DeserializeOwned>(&self, expression: &str) -> Result<T, Error> {
        for attempt in 0..3 {
            match self.evaluate(expression) {
                Err(Error::Network(message))
                    if attempt < 2 && message == "browser page navigated during evaluation" => {}
                result => return result,
            }
        }
        Err(Error::Network(
            "Comix kept navigating while reading catalog data".into(),
        ))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        browser_session::close();
    }
}

impl Guest for Comix {
    fn manifest() -> Manifest {
        Manifest {
            id: "page.chek.comix".into(),
            name: "Comix".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            sites: [
                "comix.to",
                "comix.ws",
                "static.comix.to",
                "*.kkplayer.wtf",
                "challenges.cloudflare.com",
                "*.softlifejournal.site",
                "*.notesbyamber.site",
                "*.homeandheartshop.site",
                "*.softstylemarket.site",
                "*.digitalsoulstudio.site",
                "*.blogofjason.site",
                "*.warmhomemarket.site",
                "*.alyssablog.site",
                "*.blogofanna.site",
                "*.blogofdaniel.site",
                "*.blogofjustin.site",
                "*.blogofnicole.site",
                "*.brittanyswrites.site",
                "*.dailyinspirationhub.site",
                "*.designwithheart.site",
                "*.ericlog.site",
                "*.handmadewithlove.site",
                "*.hannahjournal.site",
                "*.lifebetweenlines.site",
                "*.lifeinpixels.site",
                "*.madisonscorner.site",
                "*.mattspace.site",
                "*.meganwrites.site",
                "*.mindfuldailylife.site",
                "*.naturallivingstore.site",
                "*.nickslog.site",
                "*.pureandsimpleshop.site",
                "*.puregoodsmarket.site",
                "*.quietmorningthoughts.site",
                "*.rebeccathoughts.site",
                "*.ryancorner.site",
                "*.softfabricstore.site",
                "*.softvisualstudio.site",
                "*.thebrandonblog.site",
                "*.tylerspace.site",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            languages: vec!["en".into()],
            capabilities: vec![
                Capability::Browser,
                Capability::BrowserSession,
                Capability::ImageTransform,
            ],
        }
    }

    fn search(query: String, page: u32) -> Result<SeriesPage, Error> {
        let session = Session::open(&format!(
            "/browse?q={}&page={}&sort=relevance%3Adesc",
            encode(query.trim()),
            page.max(1)
        ))?;
        #[derive(Deserialize)]
        struct Browse {
            items: Vec<BrowseItem>,
        }
        #[derive(Deserialize)]
        struct BrowseItem {
            hid: String,
            title: String,
            cover: Option<String>,
        }
        let list: Browse = session.harvest(HARVEST_BROWSE)?;
        let has_more = session.read::<bool>(&format!("browseHasMore({})", page.max(1)))?;
        Ok(SeriesPage {
            has_more,
            items: list
                .items
                .into_iter()
                .map(|manga| SeriesItem {
                    id: manga.hid,
                    title: manga.title,
                    cover: manga.cover.filter(|url| !url.is_empty()).map(image),
                })
                .collect(),
        })
    }

    fn series(id: String) -> Result<Series, Error> {
        let session = Session::open(&format!("/title/{}", encode(&id)))?;
        let manga: Manga = session.initial(true)?;
        let contributors = manga
            .authors
            .or(manga.author)
            .unwrap_or_default()
            .into_iter()
            .map(|person| Contributor {
                name: person.title,
                role: "author".into(),
            })
            .chain(
                manga
                    .artists
                    .or(manga.artist)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|person| Contributor {
                        name: person.title,
                        role: "artist".into(),
                    }),
            )
            .collect();
        Ok(Series {
            id: manga.hid,
            title: manga.title,
            summary: manga.synopsis,
            cover: manga.poster.and_then(Poster::cover),
            contributors,
            status: None,
            year: None,
            tags: Vec::new(),
            reading_direction: manga.original_language.map(|language| {
                if language == "ja" {
                    ReadingDirection::Rtl
                } else {
                    ReadingDirection::Ltr
                }
            }),
        })
    }

    fn chapters(series_id: String) -> Result<Vec<Chapter>, Error> {
        let session = Session::open(&format!("/title/{}", encode(&series_id)))?;
        let mut chapters = Vec::new();
        let mut seen = HashSet::new();
        let mut previous_total = None;
        for page in 1..=MAX_CHAPTER_PAGES {
            let list = session.chapter_page(&series_id, page)?;
            let meta = list
                .meta
                .ok_or_else(|| Error::Parse("Missing chapter pagination".into()))?;
            if meta.last() > MAX_CHAPTER_PAGES {
                return Err(Error::Other(format!(
                    "Comix lists {} chapter uploads across {} pages; this call supports at most {MAX_CHAPTER_PAGES} pages",
                    meta.total,
                    meta.last()
                )));
            }
            if previous_total.is_some_and(|total| total != meta.total) {
                return Err(Error::Other(
                    "Comix's chapter list changed during this request; retry".into(),
                ));
            }
            previous_total = Some(meta.total);
            for chapter in list.items {
                if !seen.insert(chapter.id) {
                    continue;
                }
                chapters.push(chapter.chapter()?);
            }
            if page >= meta.last() {
                if seen.len() != meta.total {
                    return Err(Error::Other(
                        "Comix returned an incomplete chapter list; retry".into(),
                    ));
                }
                return Ok(chapters);
            }
        }
        Err(Error::Other(
            "Comix's chapter list exceeds this call's page limit".into(),
        ))
    }

    fn pages(chapter_id: String) -> Result<Vec<Image>, Error> {
        let (slug, chapter) = chapter_id
            .split_once('/')
            .ok_or_else(|| Error::Parse("Invalid Comix chapter id".into()))?;
        let _id = chapter
            .split('-')
            .next()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| Error::Parse("Invalid Comix chapter number".into()))?;
        let session = Session::open(&format!("/title/{}/{}", encode(slug), encode(chapter)))?;
        #[derive(Deserialize)]
        struct Reader {
            images: Vec<String>,
        }
        let reader: Reader = session.read("renderedPages()")?;
        if reader.images.is_empty() {
            return Err(Error::Network("Comix did not load chapter images".into()));
        }
        let pages = json!({ "items": reader.images.into_iter().map(|url| json!({"url": url})).collect::<Vec<_>>() });
        let list: PageList = session.read(&format!(
            "normalizePages({pages}, {})",
            json!(Self::manifest().sites),
        ))?;
        Ok(list
            .items
            .into_iter()
            .map(|page| {
                let mut url = if page.url.starts_with("https://") {
                    page.url
                } else {
                    format!(
                        "{}/{}",
                        list.base_url.trim_end_matches('/'),
                        page.url.trim_start_matches('/')
                    )
                };
                if page.s == 1
                    && !url.split('?').nth(1).is_some_and(|query| {
                        query
                            .split('&')
                            .any(|part| part.split('=').next() == Some("v3"))
                    })
                {
                    url.push(if url.contains('?') { '&' } else { '?' });
                    url.push_str("v3");
                }
                Image {
                    url,
                    headers: vec![],
                    transform: Some("comix-v1".into()),
                }
            })
            .collect())
    }

    fn transform_image(
        _: Image,
        response: chek_extension::http::Response,
    ) -> Result<Option<ImageData>, Error> {
        descrambler::transform(response)
    }
}

export_source!(Comix);

impl chek_extension::ChapterPagination for Comix {
    fn fetch(
        series_id: String,
        cursor: Option<String>,
    ) -> Result<chek_extension::ChapterBatch, Error> {
        let cursor: Option<ChapterCursor> = cursor
            .map(|cursor| serde_json::from_str(&cursor))
            .transpose()
            .map_err(|_| Error::Parse("Invalid Comix chapter cursor".into()))?;
        if cursor.as_ref().is_some_and(|cursor| {
            cursor.series != series_id
                || cursor.page < 2
                || cursor.page > 1000
                || cursor.total > 20_000
        }) {
            return Err(Error::Parse("Invalid Comix chapter cursor".into()));
        }
        let start = cursor.as_ref().map_or(1, |cursor| cursor.page);
        let total = cursor.as_ref().map(|cursor| cursor.total);
        let session = Session::open(&format!("/title/{}?page={}", encode(&series_id), start))?;
        let page = start;
        let list: Listing<ChapterData> =
            session.evaluate(&format!("renderedChapters({}, {page})", json!(series_id)))?;
        let meta = list
            .meta
            .ok_or_else(|| Error::Parse("Missing chapter pagination".into()))?;
        if meta.page != page
            || meta.total > 20_000
            || total.is_some_and(|total| total != meta.total)
        {
            return Err(Error::Other(
                "Comix's chapter pagination changed; restart the traversal".into(),
            ));
        }
        let items = list
            .items
            .into_iter()
            .map(ChapterData::chapter)
            .collect::<Result<Vec<_>, _>>()?;
        let next = (page < meta.last())
            .then(|| {
                serde_json::to_string(&ChapterCursor {
                    series: series_id.clone(),
                    page: page + 1,
                    total: meta.total,
                })
            })
            .transpose()
            .map_err(|error| Error::Parse(error.to_string()))?;
        Ok(chek_extension::ChapterBatch {
            items,
            next,
            total: meta.total as u32,
        })
    }
}

chek_extension::export_chapter_pagination!(Comix);
