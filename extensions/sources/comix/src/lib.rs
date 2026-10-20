//! Comix 1.6.54's WebView flow, ported to Chek browser sessions.
mod descrambler;
mod protocol;

use chek_extension::prelude::*;
use chek_extension::{browser, browser_session, http::HttpError};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};

const SITE: &str = "https://comix.to";
const CLIENT: &str = include_str!("client.js");
const SITE_API: &str = include_str!("site_api.js");
const MAX_CHAPTER_PAGES: u32 = 20;

struct Comix;

#[derive(Deserialize)]
struct Listing<T> {
    items: Vec<T>,
    meta: Option<Pagination>,
    pagination: Option<Pagination>,
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
struct Target {
    x: u32,
    y: u32,
    scroll: i32,
}

#[derive(Deserialize)]
struct Evaluation<T> {
    value: Option<T>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct PreparedRequest {
    kid: String,
    material: Vec<u8>,
    query: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct ApiResponse {
    status: u16,
    encoding: Option<String>,
    body: String,
}

struct Session;

impl Session {
    fn api<T: DeserializeOwned>(&self, path: &str, params: Value) -> Result<T, Error> {
        let prepared: PreparedRequest =
            self.read(&format!("prepareApi({}, {params})", json!(path)))?;
        let query: Vec<_> = prepared
            .query
            .iter()
            .filter(|(key, _)| key.as_str() != "_")
            .collect();
        let canonical = if query.is_empty() {
            path.to_owned()
        } else {
            format!(
                "{path}?{}",
                query
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("&")
            )
        };
        let signature = format!(
            "{}.{}",
            prepared.kid,
            protocol::sign(&prepared.material, &canonical).map_err(Error::Parse)?
        );
        if prepared.query.get("_") != Some(&signature) {
            return Err(Error::Parse(
                "Comix request signing changed; this build is unsupported".into(),
            ));
        }
        let url = format!(
            "/api/v1{path}?{}",
            query
                .iter()
                .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
                .chain(std::iter::once(format!("_={}", encode(&signature))))
                .collect::<Vec<_>>()
                .join("&")
        );
        let response: ApiResponse = self.read(&format!("fetchApi({})", json!(url)))?;
        if !(200..300).contains(&response.status) {
            return Err(Error::Network(format!(
                "Comix API returned HTTP {}",
                response.status
            )));
        }
        let value = match response.encoding.as_deref() {
            Some("2") => {
                protocol::decode(&prepared.material, &response.body).map_err(Error::Parse)?
            }
            None => {
                serde_json::from_str(&response.body).map_err(|e| Error::Parse(e.to_string()))?
            }
            Some(_) => {
                return Err(Error::Parse(
                    "Comix uses an unsupported response encoding".into(),
                ));
            }
        };
        let value = if value.get("status").and_then(Value::as_str) == Some("ok") {
            value
                .get("result")
                .cloned()
                .ok_or_else(|| Error::Parse("Comix API returned no result".into()))?
        } else {
            value
        };
        serde_json::from_value(value).map_err(|e| Error::Parse(e.to_string()))
    }

    fn open(path: &str) -> Result<Self, Error> {
        let url = format!("{SITE}{path}");
        match browser::load(&url) {
            Ok(response) if (200..300).contains(&response.status) => {}
            Ok(response) => {
                return Err(Error::Network(format!(
                    "Comix verification returned HTTP {}",
                    response.status
                )));
            }
            Err(HttpError::Failed(message)) if message == "no browser solver is configured" => {}
            Err(error) => {
                return Err(Error::Network(format!(
                    "Comix verification failed: {error:?}"
                )));
            }
        }
        browser_session::open(&url).map_err(Error::Network)?;
        Ok(Self)
    }

    fn evaluate<T: DeserializeOwned>(&self, expression: &str) -> Result<T, Error> {
        let result = browser_session::evaluate(&format!(
            "(async () => {{ {CLIENT}\n try {{ return {{value: await ({expression})}}; }} catch (error) {{ return {{error: String(error.message || error)}}; }} }})()"
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

    fn tap_chapters(&self, page: u32) -> Result<(), Error> {
        for _ in 0..8 {
            let target: Target = self.evaluate(&format!("chapterTarget({page})"))?;
            if target.scroll == 0 {
                browser_session::click(target.x, target.y).map_err(Error::Network)?;
                if self.evaluate::<bool>("chapterInputReceived()")? {
                    return Ok(());
                }
            } else {
                browser_session::scroll(target.x, target.y, 0, target.scroll)
                    .map_err(Error::Network)?;
            }
        }
        Err(Error::Other(
            "Could not activate Comix's chapter controls".into(),
        ))
    }

    fn initial<T: DeserializeOwned>(&self, detail: bool) -> Result<T, Error> {
        self.read(&format!("initialData({detail})"))
    }

    fn chapter_page(&self, id: &str, page: u32) -> Result<Listing<ChapterData>, Error> {
        let result = self
            .tap_chapters(page)
            .and_then(|()| self.evaluate(&format!("chapterPage({}, {page})", json!(id))));
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
        let order = if query.trim().is_empty() {
            "order[score]"
        } else {
            "order[relevance]"
        };
        let list: Listing<Manga> = session.api("/manga", json!({
            "keyword": query.trim(), "page": page.max(1).to_string(), "limit": "28", order: "desc"
        }))?;
        let has_more = list
            .meta
            .or(list.pagination)
            .is_some_and(|pagination| pagination.page.max(1) < pagination.last());
        Ok(SeriesPage {
            has_more,
            items: list
                .items
                .into_iter()
                .map(|manga| SeriesItem {
                    id: manga.hid,
                    title: manga.title,
                    cover: manga.poster.and_then(Poster::cover),
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
        session.read::<Value>(&format!("chapterApi(({SITE_API}))"))?;
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
                let path = chapter
                    .url
                    .split_once("/title/")
                    .map(|(_, path)| path)
                    .ok_or_else(|| Error::Parse("Invalid Comix chapter URL".into()))?;
                let number = chapter.number.to_string();
                let title = match chapter.name.filter(|name| !name.trim().is_empty()) {
                    Some(name) => format!("Chapter {number}: {name}"),
                    None => format!("Chapter {number}"),
                };
                chapters.push(Chapter {
                    id: path.into(),
                    title,
                    number: Some(number),
                    version: chapter
                        .group
                        .map(|group| group.name)
                        .or_else(|| chapter.is_official.then(|| "Official".into())),
                    published: None,
                });
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
        let id = chapter
            .split('-')
            .next()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| Error::Parse("Invalid Comix chapter number".into()))?;
        let session = Session::open(&format!("/title/{}/{}", encode(slug), encode(chapter)))?;
        let chapter: Value = session.api(&format!("/chapters/{id}"), json!({}))?;
        let pages = std::iter::once(&chapter)
            .chain(
                chapter
                    .as_object()
                    .into_iter()
                    .flat_map(|object| object.values()),
            )
            .find_map(|value| {
                let items = value
                    .as_array()
                    .or_else(|| value.get("items")?.as_array())?;
                (!items.is_empty()
                    && items
                        .iter()
                        .all(|item| item.get("url").is_some_and(Value::is_string)))
                .then(|| {
                    if value.is_array() {
                        json!({ "items": value })
                    } else {
                        value.clone()
                    }
                })
            })
            .ok_or_else(|| Error::Parse("Comix returned no chapter pages".into()))?;
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
