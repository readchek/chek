//! Writes Chek source extensions.
//!
//! An extension implements [`Guest`] and exports it with
//! [`export_source!`]. The contract itself is `crates/chek-sources/wit/source.wit`
//! in the Chek repository; this crate generates its bindings and adds
//! helpers for the requests most sources make.
//!
//! ```ignore
//! use chek_extension::prelude::*;
//!
//! struct Example;
//!
//! impl Guest for Example {
//!     fn manifest() -> Manifest { /* id, name, version, sites */ }
//!     fn search(query: String, page: u32, ratings: Vec<ContentRating>) -> Result<SeriesPage, Error> {
//!         let found: Value = Get::new(format!("https://api.example.org/search?q={}", encode(&query)))
//!             .cache(600)
//!             .json()?;
//!         /* ... */
//!     }
//!     /* series, chapters, pages */
//! }
//!
//! export_source!(Example);
//! ```

#[doc(hidden)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "../../crates/chek-sources/wit",
        world: "source",
        pub_export_macro: true,
        export_macro_name: "export_source",
        default_bindings_module: "chek_extension::bindings",
    });
}

pub use bindings::chek::source::types::{
    Capability, Chapter, ContentRating, Contributor, Error, Image, ImageData, Manifest,
    ReadingDirection, Series, SeriesItem, SeriesPage, Status,
};
pub use bindings::chek::source::{browser, browser_session, cache, cookies, http, images};
pub use bindings::{Guest, export_source};
#[doc(hidden)]
pub mod pagination_bindings {
    wit_bindgen::generate!({
        path: "../../crates/chek-sources/wit",
        world: "paginated-chapters",
        with: { "chek:source/types@0.6.0": crate::bindings::chek::source::types },
        pub_export_macro: true,
        export_macro_name: "export_chapter_pagination",
        default_bindings_module: "chek_extension::pagination_bindings",
    });
}
pub use pagination_bindings::export_chapter_pagination;
pub use pagination_bindings::exports::chek::source::chapter_pagination::{
    Batch as ChapterBatch, Guest as ChapterPagination,
};
/// For sites that list chapters a page at a time from either end: implement
/// [`ChapterPages`] and export it with [`export_chapter_pages!`] beside
/// [`export_source!`], and a client's page of chapters becomes one call to
/// the site instead of reading the whole list first.
#[doc(hidden)]
pub mod pages_bindings {
    wit_bindgen::generate!({
        path: "../../crates/chek-sources/wit",
        world: "paged-chapters",
        with: { "chek:source/types@0.6.0": crate::bindings::chek::source::types },
        pub_export_macro: true,
        export_macro_name: "export_chapter_pages",
        default_bindings_module: "chek_extension::pages_bindings",
    });
}
pub use pages_bindings::export_chapter_pages;
pub use pages_bindings::exports::chek::source::chapter_pages::{
    ChapterOrder, ChapterPage, Guest as ChapterPages,
};
pub use serde_json::{self, Value};

use bindings::chek::source::http::{HttpError, Method, Request, Response};

/// What most extensions import.
pub mod prelude {
    pub use crate::{
        Capability, Chapter, ContentRating, Contributor, Error, Get, Guest, Image, ImageData,
        Manifest, ReadingDirection, Series, SeriesItem, SeriesPage, Status, Value, encode,
        export_source, image,
    };
}

/// A request being built. Defaults to a GET with no headers and no cache.
pub struct Get {
    request: Request,
}

impl Get {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            request: Request {
                method: Method::Get,
                url: url.into(),
                headers: Vec::new(),
                body: None,
                cache: None,
            },
        }
    }

    /// Makes it a POST with this body.
    pub fn post(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.request.method = Method::Post;
        self.request.body = Some(body.into());
        self
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.request.headers.push((name.to_owned(), value.into()));
        self
    }

    /// Lets the server answer from its cache for `seconds`. Only a GET's
    /// 2xx responses are kept.
    pub fn cache(mut self, seconds: u32) -> Self {
        self.request.cache = Some(seconds);
        self
    }

    /// Sends the request and returns the response whatever its status.
    pub fn send(self) -> Result<Response, Error> {
        http::fetch(&self.request).map_err(network)
    }

    /// Sends the request and returns the body of a 2xx response. A 404
    /// becomes [`Error::NotFound`].
    pub fn bytes(self) -> Result<Vec<u8>, Error> {
        let url = self.request.url.clone();
        let response = self.send()?;
        match response.status {
            200..=299 => Ok(response.body),
            404 => Err(Error::NotFound),
            status => Err(Error::Network(format!("{url} answered {status}"))),
        }
    }

    pub fn text(self) -> Result<String, Error> {
        String::from_utf8(self.bytes()?).map_err(|e| Error::Parse(e.to_string()))
    }

    /// Sends the request and parses a 2xx response's body as JSON.
    pub fn json<T: serde::de::DeserializeOwned>(self) -> Result<T, Error> {
        serde_json::from_slice(&self.bytes()?).map_err(|e| Error::Parse(e.to_string()))
    }
}

fn network(error: HttpError) -> Error {
    Error::Network(match error {
        HttpError::NotAllowed(reason) => format!("not allowed: {reason}"),
        HttpError::TooLarge => "the response is too large".to_owned(),
        HttpError::TooManyRequests => "this call made too many requests".to_owned(),
        HttpError::Timeout => "timed out".to_owned(),
        HttpError::Failed(reason) => reason,
    })
}

/// An image the server fetches without extra headers.
pub fn image(url: impl Into<String>) -> Image {
    Image {
        url: url.into(),
        headers: Vec::new(),
        transform: None,
    }
}

/// Percent-encodes `text` for a URL's query or path, leaving only
/// unreserved characters as they are.
pub fn encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}
