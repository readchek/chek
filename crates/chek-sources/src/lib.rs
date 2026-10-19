//! Extension host for Chek's remote sources.
//!
//! Sources are WebAssembly components that target the `source` world in
//! `wit/source.wit`. The host runs each call in a fresh instance with an empty
//! WASI environment, so an extension has no files, environment or sockets.
//! It can only reach the sites its manifest declares, over HTTPS, through the
//! host's `http` import. Each call is capped in memory, instructions and
//! requests, so a broken or hostile extension fails its call rather than
//! the server. What an extension keeps between calls, its cache and cookies,
//! lives in the host's state database, kept apart per extension.
//!
//! Everything here blocks: call it from async code through `spawn_blocking`.

mod browser;
mod cookies;
mod http;
mod images;
mod installed;
pub mod repository;
mod sites;
mod store;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use reqwest::Method;
use serde_json::json;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

pub use crate::installed::Extensions;

use crate::browser::{Browser, FlareSolverr, Solver};
use crate::cookies::Jar;
use crate::http::{FetchError, Net};
use crate::images::ImageCache;
use crate::sites::Sites;
use crate::store::Namespace;

mod bindings {
    wasmtime::component::bindgen!({ path: "wit", world: "source" });
}

pub use bindings::chek::source::types::{
    Capability, Chapter, Contributor, Error as SourceError, Image, Manifest, ReadingDirection,
    Series, SeriesItem, SeriesPage,
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
/// The longest key an extension may store a value under.
const MAX_KEY: usize = 1024;
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
    store: Arc<store::Store>,
    images: Arc<ImageCache>,
    solver: Option<Arc<dyn Solver>>,
    /// One jar and browser per extension, so every loaded version shares
    /// them.
    scopes: Mutex<HashMap<String, Scope>>,
}

#[derive(Clone)]
struct Scope {
    jar: Arc<Jar>,
    browser: Option<Arc<Browser>>,
}

/// How the host is set up.
pub struct Options {
    /// The most bytes of fetched images to keep on disk; 0 keeps none.
    pub image_cache_bytes: u64,
    /// A FlareSolverr-compatible endpoint, such as
    /// `http://flaresolverr:8191/v1`, for extensions that need a browser.
    pub browser_solver: Option<String>,
}

impl Host {
    /// Keeps extensions' caches and cookies in `state.db` in `dir`, which
    /// is made if it's missing, and fetched images in `images` there.
    pub fn new(dir: &Path, options: &Options) -> Result<Self, Error> {
        let store = Arc::new(store::Store::open(&dir.join("state.db")).map_err(Error::Storage)?);
        let images = Arc::new(ImageCache::open(
            dir.join("images"),
            options.image_cache_bytes,
        ));
        let solver = match options.browser_solver.as_deref() {
            Some(endpoint) => Some(
                Arc::new(FlareSolverr::new(endpoint).map_err(Error::Storage)?) as Arc<dyn Solver>,
            ),
            None => None,
        };
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(trap)?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(trap)?;
        bindings::Source::add_to_linker::<State, HasSelf<State>>(&mut linker, |state| state)
            .map_err(trap)?;
        Ok(Host {
            engine,
            linker,
            store,
            images,
            solver,
            scopes: Mutex::default(),
        })
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
        // State belongs to the id and sites together, so a component from
        // another repository that reuses an id can't read what the original
        // kept unless it could reach the same sites anyway.
        let mut declared = manifest.sites.clone();
        declared.sort();
        let scope = format!("{} {}", manifest.id, declared.join(" "));
        let Scope { jar, browser } = self
            .scopes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(scope.clone())
            .or_insert_with(|| {
                let jar = Arc::new(Jar::load(&scope, Arc::clone(&self.store)));
                let browser = self.solver.as_ref().map(|solver| {
                    Arc::new(Browser::new(
                        Arc::clone(solver),
                        Arc::clone(&jar),
                        Arc::clone(&self.store),
                        scope.clone(),
                    ))
                });
                Scope { jar, browser }
            })
            .clone();
        let browser = browser.filter(|_| manifest.capabilities.contains(&Capability::Browser));
        let context = Arc::new(Context {
            scope,
            net: Net::new(sites, Arc::clone(&jar), browser).map_err(Error::Manifest)?,
            store: Arc::clone(&self.store),
            images: Arc::clone(&self.images),
            jar,
        });
        Ok(Source {
            pre,
            manifest,
            context,
        })
    }
}

/// A loaded extension.
pub struct Source {
    pre: bindings::SourcePre<State>,
    manifest: Manifest,
    context: Arc<Context>,
}

/// What an extension's calls may use beyond their own instance.
struct Context {
    /// Names the extension's state: its id and sites.
    scope: String,
    net: Net,
    store: Arc<store::Store>,
    images: Arc<ImageCache>,
    jar: Arc<Jar>,
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
        let key = repository::sha256_hex(
            json!([self.context.scope, request_key(&image.url, &image.headers)])
                .to_string()
                .as_bytes(),
        );
        if let Some((content_type, bytes)) = self.context.images.get(&key) {
            return Ok(Fetched {
                content_type,
                bytes,
            });
        }
        let (status, headers, bytes) = self
            .context
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
        let content_type = http::content_type(&headers);
        self.context
            .images
            .put(&key, content_type.as_deref(), &bytes);
        Ok(Fetched {
            content_type,
            bytes,
        })
    }

    fn call<R>(
        &self,
        export: impl FnOnce(&bindings::Source, &mut Store<State>) -> wasmtime::Result<R>,
    ) -> Result<R, Error> {
        run(&self.pre, Some(Arc::clone(&self.context)), export)
    }
}

/// Runs one export in a fresh instance.
fn run<R>(
    pre: &bindings::SourcePre<State>,
    context: Option<Arc<Context>>,
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
        context,
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
    /// None while the manifest is read.
    context: Option<Arc<Context>>,
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
        use bindings::chek::source::http::{HttpError, Method, Response};
        let Some(context) = &self.context else {
            return Err(HttpError::NotAllowed(
                "the manifest is read without network".to_owned(),
            ));
        };
        let cache = request
            .cache
            .filter(|ttl| *ttl > 0 && matches!(request.method, Method::Get))
            .map(|ttl| (ttl, request_key(&request.url, &request.headers)));
        if let Some((_, key)) = &cache
            && let Some((Some(meta), body)) =
                context.store.get(&context.scope, Namespace::Response, key)
            && let Ok((status, headers)) =
                serde_json::from_str::<(u16, Vec<(String, String)>)>(&meta)
        {
            return Ok(Response {
                status,
                headers,
                body,
                cached: true,
            });
        }
        if self.requests >= MAX_REQUESTS {
            return Err(HttpError::TooManyRequests);
        }
        self.requests += 1;
        let response = context.net.fetch(request, MAX_RESPONSE)?;
        if let Some((ttl, key)) = cache
            && (200..300).contains(&response.status)
            && response.body.len() <= store::MAX_ENTRY
        {
            let meta = json!([response.status, response.headers]).to_string();
            // A response that can't be kept is still returned.
            let _ = context.store.set(
                &context.scope,
                Namespace::Response,
                &key,
                Some(&meta),
                &response.body,
                ttl,
            );
        }
        Ok(response)
    }
}

/// Names a cached response by what was asked for, so requests that differ
/// only in a header, such as a language, are kept apart.
fn request_key(url: &str, headers: &[(String, String)]) -> String {
    let mut headers: Vec<_> = headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.as_str()))
        .collect();
    headers.sort();
    repository::sha256_hex(json!([url, headers]).to_string().as_bytes())
}

impl State {
    fn context(&self) -> Result<&Context, String> {
        self.context
            .as_deref()
            .ok_or_else(|| "not available while the manifest is read".to_owned())
    }
}

impl bindings::chek::source::cache::Host for State {
    fn get(&mut self, key: String) -> Option<Vec<u8>> {
        let context = self.context().ok()?;
        context
            .store
            .get(&context.scope, Namespace::Value, &key)
            .map(|(_, value)| value)
    }

    fn set(&mut self, key: String, value: Vec<u8>, ttl: u32) -> Result<(), String> {
        let context = self.context()?;
        if key.len() > MAX_KEY {
            return Err(format!("keys are limited to {MAX_KEY} bytes"));
        }
        context
            .store
            .set(&context.scope, Namespace::Value, &key, None, &value, ttl)
    }

    fn delete(&mut self, key: String) {
        if let Ok(context) = self.context() {
            context.store.delete(&context.scope, Namespace::Value, &key);
        }
    }
}

impl bindings::chek::source::browser::Host for State {
    fn load(
        &mut self,
        url: String,
    ) -> Result<bindings::chek::source::http::Response, bindings::chek::source::http::HttpError>
    {
        use bindings::chek::source::http::{HttpError, Response};
        if self.requests >= MAX_REQUESTS {
            return Err(HttpError::TooManyRequests);
        }
        self.requests += 1;
        let context = self.context().map_err(HttpError::NotAllowed)?;
        let Some(browser) = context.net.browser() else {
            return Err(HttpError::NotAllowed(
                "the browser needs the `browser` capability and a solver the admin set up"
                    .to_owned(),
            ));
        };
        let url = context
            .net
            .allowed_url(&url)
            .map_err(HttpError::NotAllowed)?;
        let solution = browser.load(&url).map_err(HttpError::Failed)?;
        if solution.html.len() as u64 > MAX_RESPONSE {
            return Err(HttpError::TooLarge);
        }
        Ok(Response {
            status: solution.status,
            headers: vec![("content-type".to_owned(), "text/html".to_owned())],
            body: solution.html.into_bytes(),
            cached: false,
        })
    }
}

impl bindings::chek::source::cookies::Host for State {
    fn get(&mut self, url: String) -> Vec<(String, String)> {
        let Ok(context) = self.context() else {
            return Vec::new();
        };
        match context.net.allowed_url(&url) {
            Ok(url) => context.jar.get(&url),
            Err(_) => Vec::new(),
        }
    }

    fn set(&mut self, url: String, set_cookies: Vec<String>) -> Result<(), String> {
        let context = self.context()?;
        let url = context.net.allowed_url(&url)?;
        context
            .jar
            .set(&url, set_cookies.iter().map(String::as_str))
    }

    fn clear(&mut self) {
        if let Ok(context) = self.context() {
            context.jar.clear();
        }
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
