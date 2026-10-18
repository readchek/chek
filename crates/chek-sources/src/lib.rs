//! Extension host for Chek's remote sources.
//!
//! Sources are WebAssembly components that target the `source` world in
//! `wit/source.wit`. The host runs each call in a fresh instance with an empty
//! WASI environment, so an extension has no files, environment or sockets.
//! It can only reach the sites its manifest declares, over HTTPS, through the
//! host's `http` import. Each call is capped in memory, instructions and
//! requests, so a broken or hostile extension fails its call rather than
//! the server.
//!
//! Everything here blocks: call it from async code through `spawn_blocking`.

mod http;
mod installed;
pub mod repository;
mod sites;

use std::sync::Arc;

use reqwest::Method;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

pub use crate::installed::Extensions;

use crate::http::{FetchError, Net};
use crate::sites::Sites;

mod bindings {
    wasmtime::component::bindgen!({ path: "wit", world: "source" });
}

pub use bindings::chek::source::types::{
    Chapter, Contributor, Error as SourceError, Image, Manifest, ReadingDirection, Series,
    SeriesItem, SeriesPage,
};

/// Memory one instance may grow to.
const MAX_MEMORY: usize = 64 << 20;
/// Instructions, roughly, that one call may run. Time spent waiting on the
/// network doesn't count.
const MAX_FUEL: u64 = 5_000_000_000;
/// Requests one call may make.
const MAX_REQUESTS: u32 = 100;
/// The largest response body an extension may read.
const MAX_RESPONSE: u64 = 16 << 20;
/// The largest image the host fetches for an extension.
const MAX_IMAGE: u64 = 32 << 20;
/// Manifest text fields are cut off after this many bytes.
pub(crate) const MAX_FIELD: usize = 200;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a source extension: {0}")]
    Invalid(String),
    #[error("invalid extension manifest: {0}")]
    Manifest(String),
    /// The extension crashed or went over a limit.
    #[error("the extension failed: {0}")]
    Trap(String),
    /// The extension ran and reported an error.
    #[error("{}", describe(.0))]
    Source(SourceError),
    #[error("couldn't fetch the image: {0}")]
    Image(String),
    #[error("the extension doesn't match its index entry: {0}")]
    Mismatch(String),
    #[error("couldn't store the extension: {0}")]
    Storage(String),
}

fn describe(error: &SourceError) -> String {
    match error {
        SourceError::NotFound => "the source has no such item".to_owned(),
        SourceError::Network(reason) => format!("the source couldn't be reached: {reason}"),
        SourceError::Parse(reason) => format!("the source answered unexpectedly: {reason}"),
        SourceError::Other(reason) => format!("the source failed: {reason}"),
    }
}

/// Compiles and runs extensions. Make one and share it.
pub struct Host {
    engine: Engine,
    linker: Linker<State>,
}

impl Host {
    pub fn new() -> Result<Self, Error> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(trap)?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(trap)?;
        bindings::Source::add_to_linker::<State, HasSelf<State>>(&mut linker, |state| state)
            .map_err(trap)?;
        Ok(Host { engine, linker })
    }

    /// Compiles an extension and reads its manifest. Compiling takes a
    /// moment, so keep the returned source rather than loading it per call.
    pub fn load(&self, wasm: &[u8]) -> Result<Source, Error> {
        let component = Component::new(&self.engine, wasm)
            .map_err(|error| Error::Invalid(format!("{error:#}")))?;
        let instance = self
            .linker
            .instantiate_pre(&component)
            .map_err(|error| Error::Invalid(format!("{error:#}")))?;
        let pre = bindings::SourcePre::new(instance)
            .map_err(|error| Error::Invalid(format!("{error:#}")))?;
        // The manifest is read without network, before its sites are known.
        let manifest = run(&pre, None, |bindings, store| bindings.call_manifest(store))?;
        let manifest = check_manifest(manifest)?;
        let sites = Sites::parse(&manifest.sites).map_err(Error::Manifest)?;
        let net = Arc::new(Net::new(sites).map_err(Error::Manifest)?);
        Ok(Source { pre, manifest, net })
    }
}

/// A loaded extension.
pub struct Source {
    pre: bindings::SourcePre<State>,
    manifest: Manifest,
    net: Arc<Net>,
}

/// An image's bytes as the site sent them.
pub struct Fetched {
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

impl Source {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// A page of series matching `query`, counting pages from 1. An empty
    /// query lists what the source shows first.
    pub fn search(&self, query: &str, page: u32) -> Result<SeriesPage, Error> {
        self.call(|bindings, store| bindings.call_search(store, query, page))?
            .map_err(Error::Source)
    }

    pub fn series(&self, id: &str) -> Result<Series, Error> {
        self.call(|bindings, store| bindings.call_series(store, id))?
            .map_err(Error::Source)
    }

    /// The series' chapters in reading order.
    pub fn chapters(&self, series_id: &str) -> Result<Vec<Chapter>, Error> {
        self.call(|bindings, store| bindings.call_chapters(store, series_id))?
            .map_err(Error::Source)
    }

    /// The chapter's pages in reading order.
    pub fn pages(&self, chapter_id: &str) -> Result<Vec<Image>, Error> {
        self.call(|bindings, store| bindings.call_pages(store, chapter_id))?
            .map_err(Error::Source)
    }

    /// Fetches a cover or page the extension returned, held to the same sites.
    pub fn fetch_image(&self, image: &Image) -> Result<Fetched, Error> {
        let (status, headers, bytes) = self
            .net
            .send(Method::GET, &image.url, &image.headers, None, MAX_IMAGE)
            .map_err(|error| {
                Error::Image(match error {
                    FetchError::NotAllowed(reason) => reason,
                    FetchError::TooLarge => format!("larger than {MAX_IMAGE} bytes"),
                    FetchError::Timeout => "timed out".to_owned(),
                    FetchError::Failed(reason) => reason,
                })
            })?;
        if !(200..300).contains(&status) {
            return Err(Error::Image(format!("the site answered {status}")));
        }
        Ok(Fetched {
            content_type: http::content_type(&headers),
            bytes,
        })
    }

    fn call<R>(
        &self,
        export: impl FnOnce(&bindings::Source, &mut Store<State>) -> wasmtime::Result<R>,
    ) -> Result<R, Error> {
        run(&self.pre, Some(Arc::clone(&self.net)), export)
    }
}

/// Runs one export in a fresh instance.
fn run<R>(
    pre: &bindings::SourcePre<State>,
    net: Option<Arc<Net>>,
    export: impl FnOnce(&bindings::Source, &mut Store<State>) -> wasmtime::Result<R>,
) -> Result<R, Error> {
    let state = State {
        wasi: WasiCtx::builder()
            .allow_tcp(false)
            .allow_udp(false)
            .allow_ip_name_lookup(false)
            .build(),
        table: ResourceTable::new(),
        limits: StoreLimitsBuilder::new().memory_size(MAX_MEMORY).build(),
        net,
        requests: 0,
    };
    let mut store = Store::new(pre.engine(), state);
    store.limiter(|state| &mut state.limits);
    store.set_fuel(MAX_FUEL).map_err(trap)?;
    let bindings = pre.instantiate(&mut store).map_err(trap)?;
    export(&bindings, &mut store).map_err(trap)
}

struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    net: Option<Arc<Net>>,
    requests: u32,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl bindings::chek::source::types::Host for State {}

impl bindings::chek::source::http::Host for State {
    fn fetch(
        &mut self,
        request: bindings::chek::source::http::Request,
    ) -> Result<bindings::chek::source::http::Response, bindings::chek::source::http::HttpError>
    {
        use bindings::chek::source::http::HttpError;
        let Some(net) = &self.net else {
            return Err(HttpError::NotAllowed(
                "the manifest is read without network".to_owned(),
            ));
        };
        if self.requests >= MAX_REQUESTS {
            return Err(HttpError::TooManyRequests);
        }
        self.requests += 1;
        net.fetch(request, MAX_RESPONSE)
    }
}

fn check_manifest(mut manifest: Manifest) -> Result<Manifest, Error> {
    let id = &manifest.id;
    if !valid_id(id) {
        return Err(Error::Manifest(format!(
            "id `{id}` isn't in reverse-DNS form such as `org.example.comics`"
        )));
    }
    if manifest.name.trim().is_empty() {
        return Err(Error::Manifest("the name is empty".to_owned()));
    }
    if manifest.version.trim().is_empty() {
        return Err(Error::Manifest("the version is empty".to_owned()));
    }
    for field in [&mut manifest.name, &mut manifest.version] {
        truncate(field, MAX_FIELD);
    }
    manifest.languages.truncate(64);
    for language in &mut manifest.languages {
        truncate(language, 35);
    }
    Ok(manifest)
}

/// Whether an extension id is in reverse-DNS form, such as
/// `org.example.comics`.
pub(crate) fn valid_id(id: &str) -> bool {
    id.len() <= MAX_FIELD
        && id.contains('.')
        && id.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

pub(crate) fn truncate(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

fn trap(error: wasmtime::Error) -> Error {
    // The root cause names the trap, such as running out of fuel; the
    // outer context is a wasm backtrace, which helps no one reading a log.
    Error::Trap(error.root_cause().to_string())
}
