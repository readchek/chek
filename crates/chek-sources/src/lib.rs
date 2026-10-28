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
mod calls;
mod cdp;
mod cookies;
mod http;
mod images;
mod installed;
mod proxy;
pub mod repository;
mod resolve;
mod session;
mod sites;
mod store;
mod transform;
mod tunnel;
pub use proxy::OutboundProxy;
mod worker;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::Method;
use serde_json::json;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Cache, CacheConfig, Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

pub use crate::cdp::browser_worker;
pub use crate::installed::Extensions;
pub use crate::transform::image_worker;

use crate::browser::{Browser, FlareSolverr, Solver};
use crate::cookies::Jar;
use crate::http::{FetchError, Net};
use crate::images::ImageCache;
use crate::sites::Sites;
use crate::store::Namespace;

mod bindings {
    wasmtime::component::bindgen!({ path: "wit", world: "source" });
}

mod pagination_bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "paginated-chapters",
        with: { "chek:source/types@0.6.0": crate::bindings::chek::source::types },
    });
}

mod pages_bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "paged-chapters",
        with: { "chek:source/types@0.6.0": crate::bindings::chek::source::types },
    });
}

mod content_bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "chapter-contents",
        with: { "chek:source/types@0.6.0": crate::bindings::chek::source::types },
    });
}

pub use content_bindings::exports::chek::source::chapter_content::{
    Asset, Content, ContentFormat, Reading, Rights,
};
pub use pages_bindings::exports::chek::source::chapter_pages::{ChapterOrder, ChapterPage};
pub use pagination_bindings::exports::chek::source::chapter_pagination::Batch as ChapterBatch;

pub use bindings::chek::source::types::{
    Capability, Chapter, ContentRating, Contributor, Error as SourceError, Image, ImageData,
    Manifest, ReadingDirection, Series, SeriesItem, SeriesPage, Status,
};

/// The contract version this server links, as the package in `source.wit`
/// names it. Components built against another don't link.
pub const CONTRACT_VERSION: &str = "0.6.0";

/// Memory one instance may grow to.
const MAX_MEMORY: usize = 64 << 20;
/// Instructions, roughly, that one call may run. Time spent waiting on the
/// network doesn't count.
const MAX_FUEL: u64 = 5_000_000_000;
/// Pages one read-ahead fetches at once, on top of what the reader asks for.
const READ_AHEAD: usize = 3;
/// Requests one call may make.
const MAX_REQUESTS: u32 = 100;
/// The largest response body an extension may read.
const MAX_RESPONSE: u64 = 16 << 20;
/// The largest image the host fetches for an extension.
const MAX_IMAGE: u64 = 32 << 20;
/// The largest file, such as a book, the host downloads for an extension.
const MAX_ASSET: u64 = 128 << 20;
/// How long downloading one file may take, longer than a request's 30
/// seconds since books are larger than pages.
const ASSET_TIMEOUT: Duration = Duration::from_secs(300);
/// The longest key an extension may store a value under.
const MAX_KEY: usize = 1024;
/// Manifest text fields are cut off after this many bytes.
pub(crate) const MAX_FIELD: usize = 200;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a source extension: {0}")]
    Invalid(String),
    /// A source extension, but built against another version of the
    /// contract, so it doesn't link with this server.
    #[error("the extension was built for another version of Chek's source contract: {0}")]
    Incompatible(String),
    #[error("invalid extension manifest: {0}")]
    Manifest(String),
    /// The extension crashed or went over a limit.
    #[error("the extension failed: {0}")]
    Trap(String),
    /// The extension ran and reported an error.
    #[error("{}", describe(.0))]
    Source(SourceError),
    #[error("the source returned invalid chapter pagination: {0}")]
    Pagination(String),
    #[error("couldn't fetch the image: {0}")]
    Image(String),
    #[error("the source returned invalid chapter content: {0}")]
    Content(String),
    #[error("couldn't fetch the file: {0}")]
    Asset(String),
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
    outbound_proxy: Option<OutboundProxy>,
    engine: Engine,
    linker: Linker<State>,
    store: Arc<store::Store>,
    images: Arc<ImageCache>,
    solver: Option<Arc<dyn Solver>>,
    browser_executable: Option<PathBuf>,
    browser_args: Vec<String>,
    browser_native: bool,
    worker_executable: Option<PathBuf>,
    clock: Arc<std::sync::mpsc::Sender<()>>,
    /// One jar and browser per extension, so every loaded version shares
    /// them.
    scopes: Mutex<HashMap<String, Scope>>,
    /// Compiled components by the digest of their bytes, so every user of
    /// an extension shares one compile.
    components: Mutex<HashMap<String, Component>>,
}

#[derive(Clone)]
struct Scope {
    jar: Arc<Jar>,
    browser: Option<Arc<Browser>>,
}

/// How the host is set up.
pub struct Options {
    /// Authenticated HTTP proxy for extension traffic; never used for server integrations.
    pub outbound_proxy: Option<OutboundProxy>,
    /// The most bytes of fetched images to keep on disk; 0 keeps none.
    pub image_cache_bytes: u64,
    /// A FlareSolverr-compatible endpoint, such as
    /// `http://flaresolverr:8191/v1`, for extensions that need a browser.
    pub browser_solver: Option<String>,
    /// Chromium executable, for interactive browser sessions.
    pub browser_executable: Option<PathBuf>,
    /// Chromium launch arguments, such as `--headless=new`.
    pub browser_args: Vec<String>,
    /// Use Chromium's native transport through the restricted site proxy.
    pub browser_native: bool,
    /// Server executable, invoked in browser or image worker mode.
    pub worker_executable: Option<PathBuf>,
}

impl Host {
    /// Keeps extensions' caches and cookies in `state.db` in `dir`, which
    /// is made if it's missing, and fetched images in `images` there.
    pub fn new(dir: &Path, options: &Options) -> Result<Self, Error> {
        for path in [&options.browser_executable, &options.worker_executable]
            .into_iter()
            .flatten()
        {
            if !path.is_absolute() || !path.is_file() {
                return Err(Error::Storage(format!(
                    "worker executables must be absolute file paths: {}",
                    path.display()
                )));
            }
        }
        let store = Arc::new(store::Store::open(&dir.join("state.db")).map_err(Error::Storage)?);
        let images = Arc::new(ImageCache::open(
            dir.join("images"),
            options.image_cache_bytes,
        ));
        let solver = match options.browser_solver.as_deref() {
            Some(endpoint) => Some(Arc::new(
                FlareSolverr::new(endpoint, options.outbound_proxy.clone())
                    .map_err(Error::Storage)?,
            ) as Arc<dyn Solver>),
            None => None,
        };
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        // Compiling a source takes most of a second; keeping the machine
        // code on disk lets a restart load it in milliseconds.
        let mut cache = CacheConfig::new();
        cache.with_directory(dir.join("compiled"));
        config.cache(Some(Cache::new(cache).map_err(trap)?));
        let engine = Engine::new(&config).map_err(trap)?;
        let (clock, stopped) = std::sync::mpsc::channel();
        let ticking = engine.clone();
        std::thread::spawn(move || {
            while matches!(
                stopped.recv_timeout(std::time::Duration::from_millis(10)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) {
                ticking.increment_epoch();
            }
        });
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(trap)?;
        bindings::Source::add_to_linker::<State, HasSelf<State>>(&mut linker, |state| state)
            .map_err(trap)?;
        Ok(Host {
            outbound_proxy: options.outbound_proxy.clone(),
            engine,
            linker,
            store,
            images,
            solver,
            browser_executable: options.browser_executable.clone(),
            browser_args: options.browser_args.clone(),
            browser_native: options.browser_native,
            worker_executable: options.worker_executable.clone(),
            clock: Arc::new(clock),
            scopes: Mutex::default(),
            components: Mutex::default(),
        })
    }

    /// Compiles an extension and reads its manifest. Compiling takes a
    /// moment, so keep the returned source rather than loading it per call.
    pub fn load(&self, wasm: &[u8]) -> Result<Source, Error> {
        self.load_for_user(wasm, None)
    }

    /// Drops the compiled component with this digest, once nothing uses it.
    pub(crate) fn forget(&self, digest: &str) {
        self.components
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(digest);
    }

    pub(crate) fn load_for_user(&self, wasm: &[u8], user: Option<i64>) -> Result<Source, Error> {
        let digest = repository::sha256_hex(wasm);
        let compiled = self
            .components
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&digest)
            .cloned();
        let component = match compiled {
            Some(component) => component,
            None => {
                let component = Component::new(&self.engine, wasm)
                    .map_err(|error| Error::Invalid(format!("{error:#}")))?;
                self.components
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(digest.clone(), component.clone());
                component
            }
        };
        // A component that names the contract but not this version was
        // built for another Chek, which is worth saying apart from junk.
        let link_error = |error: wasmtime::Error| {
            let message = format!("{error:#}");
            if message.contains("chek:source/") && !message.contains(CONTRACT_VERSION) {
                Error::Incompatible(message)
            } else {
                Error::Invalid(message)
            }
        };
        let instance = self
            .linker
            .instantiate_pre(&component)
            .map_err(link_error)?;
        let pagination = component
            .get_export_index(None, "chek:source/chapter-pagination@0.6.0")
            .map(|_| pagination_bindings::PaginatedChaptersPre::new(instance.clone()))
            .transpose()
            .map_err(|error| Error::Invalid(format!("invalid chapter pagination: {error:#}")))?;
        let pages = component
            .get_export_index(None, "chek:source/chapter-pages@0.6.0")
            .map(|_| pages_bindings::PagedChaptersPre::new(instance.clone()))
            .transpose()
            .map_err(|error| Error::Invalid(format!("invalid chapter pages: {error:#}")))?;
        let contents = component
            .get_export_index(None, "chek:source/chapter-content@0.6.0")
            .map(|_| content_bindings::ChapterContentsPre::new(instance.clone()))
            .transpose()
            .map_err(|error| Error::Invalid(format!("invalid chapter content: {error:#}")))?;
        let pre = bindings::SourcePre::new(instance).map_err(link_error)?;
        // The manifest is read without network, before its sites are known.
        let manifest = run(&pre, None, false, |bindings, store| {
            bindings.call_manifest(store)
        })?;
        let manifest = check_manifest(manifest)?;
        // So are the formats, which belong with the manifest.
        let formats = match &contents {
            Some(pre) => {
                let mut store = new_store(pre.engine(), None, false)?;
                let bindings = pre.instantiate(&mut store).map_err(trap)?;
                let formats = bindings
                    .chek_source_chapter_content()
                    .call_formats(&mut store)
                    .map_err(trap)?;
                check_formats(formats)?
            }
            None => vec![ContentFormat::Images],
        };
        let sites = Sites::parse(&manifest.sites).map_err(Error::Manifest)?;
        // State belongs to the id and sites together, so a component from
        // another repository that reuses an id can't read what the original
        // kept unless it could reach the same sites anyway.
        let mut declared = manifest.sites.clone();
        declared.sort();
        let shared_scope = format!("{} {}", manifest.id, declared.join(" "));
        let scope = match user {
            Some(user) => format!("user:{user} {shared_scope}"),
            None => shared_scope.clone(),
        };
        let Scope { jar, browser } = self
            .scopes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(scope.clone())
            .or_insert_with(|| {
                let jar = Arc::new(Jar::load(&scope, Arc::clone(&self.store)));
                let browser =
                    (self.solver.is_some() || self.browser_executable.is_some()).then(|| {
                        Arc::new(Browser::new(
                            self.solver.clone(),
                            Arc::clone(&jar),
                            Arc::clone(&self.store),
                            scope.clone(),
                        ))
                    });
                Scope { jar, browser }
            })
            .clone();
        let browser = browser.filter(|_| {
            manifest
                .capabilities
                .iter()
                .any(|c| matches!(c, Capability::Browser | Capability::BrowserSession))
        });
        let verification_pending = Arc::new(AtomicBool::new(false));
        let context = Arc::new(Context {
            browser_args: self.browser_args.clone(),
            native_sites: self.browser_native.then(|| manifest.sites.clone()),
            scope,
            // Another user's component with this id and sites must not
            // answer this one's requests, so shared responses are this
            // build's alone.
            shared_responses: format!("{digest} {shared_scope}"),
            shared_scope,
            net: Net::new(
                self.outbound_proxy.clone(),
                sites,
                Arc::clone(&jar),
                browser,
                manifest.capabilities.contains(&Capability::Browser),
            )
            .map_err(Error::Manifest)?,
            store: Arc::clone(&self.store),
            images: Arc::clone(&self.images),
            jar,
            browser_executable: self
                .browser_executable
                .clone()
                .filter(|_| manifest.capabilities.contains(&Capability::BrowserSession)),
            image_worker: self
                .worker_executable
                .clone()
                .filter(|_| manifest.capabilities.contains(&Capability::ImageTransform)),
            handoff: Mutex::new(None),
            fetching: Mutex::default(),
            warming: AtomicUsize::new(0),
            verification_pending: Arc::clone(&verification_pending),
            calls: calls::Calls::new(verification_pending),
            browsers: calls::Browsers::default(),
            user,
            _clock: Arc::clone(&self.clock),
            worker_executable: self.worker_executable.clone(),
        });
        Ok(Source {
            pre,
            pagination,
            pages,
            contents,
            formats,
            manifest,
            context,
            digest,
        })
    }
}

/// A loaded extension.
pub struct Source {
    pre: bindings::SourcePre<State>,
    pagination: Option<pagination_bindings::PaginatedChaptersPre<State>>,
    /// Set when the extension answers pages of chapters itself.
    pages: Option<pages_bindings::PagedChaptersPre<State>>,
    /// Set when the extension says how each chapter is read.
    contents: Option<content_bindings::ChapterContentsPre<State>>,
    /// What the extension's chapters come in, sorted; only images without
    /// `contents`.
    formats: Vec<ContentFormat>,
    manifest: Manifest,
    context: Arc<Context>,
    digest: String,
}

/// What an extension's calls may use beyond their own instance.
struct Context {
    native_sites: Option<Vec<String>>,
    browser_args: Vec<String>,
    verification_pending: Arc<AtomicBool>,
    browsers: calls::Browsers,
    handoff: Mutex<Option<session::Session>>,
    /// A gate per image being fetched, so a page asked for while it's
    /// already on its way waits for that fetch instead of starting another.
    fetching: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Counts read-aheads; one ends once a newer one starts.
    warming: AtomicUsize,
    calls: calls::Calls,
    user: Option<i64>,
    /// Names the extension's state: its id and sites.
    scope: String,
    /// The scope without the user, for images and responses fetched
    /// without the user's cookies, which every user may share.
    shared_scope: String,
    shared_responses: String,
    net: Net,
    store: Arc<store::Store>,
    images: Arc<ImageCache>,
    jar: Arc<Jar>,
    browser_executable: Option<PathBuf>,
    worker_executable: Option<PathBuf>,
    image_worker: Option<PathBuf>,
    _clock: Arc<std::sync::mpsc::Sender<()>>,
}

impl Context {
    /// Whether a request to `url` carries the user's cookies, which can
    /// make the site answer differently for them than for anyone else.
    fn personal(&self, url: &str) -> bool {
        reqwest::Url::parse(url).is_ok_and(|url| !self.jar.get(&url).is_empty())
    }

    /// Images fetched without the user's cookies are the same for every
    /// user, so two people reading one chapter fetch it once.
    fn image_scope(&self, url: &str) -> &str {
        if self.personal(url) {
            &self.scope
        } else {
            &self.shared_scope
        }
    }

    fn response_scope(&self, url: &str) -> &str {
        if self.personal(url) {
            &self.scope
        } else {
            &self.shared_responses
        }
    }
}

/// An image's final bytes, after any source transform.
pub struct Fetched {
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

impl Source {
    /// Starts a private browser that remains available for this user's retries.
    pub fn start_handoff(&self, url: &str) -> Result<(), String> {
        let context = &self.context;
        let _call = context.calls.try_lock().map_err(|_| "extension is busy")?;
        if context.user.is_none() {
            return Err("handoff requires a user-owned source".into());
        }
        context.net.allowed_url(url)?;
        let mut slot = context.handoff.try_lock().map_err(|_| "browser is busy")?;
        if slot.as_ref().is_some_and(|session| !session.expired()) {
            return Err("a browser is already open; close it before starting another".into());
        }
        *slot = None;
        context.verification_pending.store(false, Ordering::SeqCst);
        let mut session = session::Session::handoff(
            context
                .worker_executable
                .as_deref()
                .ok_or("no browser worker configured")?,
            context
                .browser_executable
                .as_deref()
                .ok_or("browser-session requires CHEK_BROWSER_EXECUTABLE")?,
            context,
            &self.manifest.sites,
        )?;
        session.operation(context, json!({"type":"open","url":url}), &mut 0)?;
        *slot = Some(session);
        context.verification_pending.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// The user releases the browser for extension retries. Success is only
    /// established by the retried operation, not by this acknowledgement.
    pub fn resume_handoff(&self) -> Result<(), String> {
        let _call = self
            .context
            .calls
            .try_lock()
            .map_err(|_| "extension is busy")?;
        let slot = self
            .context
            .handoff
            .try_lock()
            .map_err(|_| "browser is busy")?;
        self.context
            .verification_pending
            .store(false, Ordering::SeqCst);
        if !slot.as_ref().is_some_and(|session| !session.expired()) {
            return Err("no verification browser is open".into());
        }
        Ok(())
    }

    pub fn handoff_frame(&self) -> Result<String, String> {
        let value = self.handoff_operation(json!({"type":"screenshot"}))?;
        serde_json::from_str(&value).map_err(|e| e.to_string())
    }

    pub fn handoff_click(&self, x: u32, y: u32) -> Result<(), String> {
        if x >= 1280 || y >= 960 {
            return Err("coordinates are outside the browser".into());
        }
        self.handoff_operation(json!({"type":"click","x":x,"y":y}))
            .map(|_| ())
    }

    pub fn close_handoff(&self) -> Result<(), String> {
        let _call = self
            .context
            .calls
            .try_lock()
            .map_err(|_| "extension is busy")?;
        *self
            .context
            .handoff
            .try_lock()
            .map_err(|_| "browser is busy")? = None;
        self.context
            .verification_pending
            .store(false, Ordering::SeqCst);
        Ok(())
    }

    fn handoff_operation(&self, command: serde_json::Value) -> Result<String, String> {
        let _call = self
            .context
            .calls
            .try_lock()
            .map_err(|_| "extension is busy")?;
        let mut slot = self
            .context
            .handoff
            .try_lock()
            .map_err(|_| "browser is busy")?;
        let session = slot.as_mut().ok_or("no verification browser is open")?;
        session.reset_operations();
        let result = session.operation(&self.context, command, &mut 0);
        if let Err(error) = &result {
            tracing::warn!(%error, "verification browser operation failed");
            *slot = None;
            self.context
                .verification_pending
                .store(false, Ordering::SeqCst);
        }
        result
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The formats the extension's chapters come in, never empty.
    pub fn formats(&self) -> &[ContentFormat] {
        &self.formats
    }

    /// A page of series matching `query`, counting pages from 1. An empty
    /// query lists what the source shows first. Items the source rates
    /// outside `content_ratings` are dropped, whether or not it honored them.
    pub fn search(
        &self,
        query: &str,
        page: u32,
        content_ratings: &[ContentRating],
    ) -> Result<SeriesPage, Error> {
        let mut found = self
            .call(|bindings, store| bindings.call_search(store, query, page, content_ratings))?
            .map_err(Error::Source)?;
        found.items.retain(|item| {
            item.content_rating
                .is_none_or(|rating| content_ratings.contains(&rating))
        });
        Ok(found)
    }

    pub fn series(&self, id: &str) -> Result<Series, Error> {
        self.call(|bindings, store| bindings.call_series(store, id))?
            .map_err(Error::Source)
    }

    /// The series' chapters in reading order.
    pub fn chapters(&self, series_id: &str) -> Result<Vec<Chapter>, Error> {
        if self.pagination.is_none() {
            return self
                .call(|bindings, store| bindings.call_chapters(store, series_id))?
                .map_err(Error::Source);
        }
        let deadline = Instant::now() + Duration::from_secs(1800);
        let mut cursor = None;
        let mut cursors = HashSet::new();
        let mut ids = HashSet::new();
        let mut chapters = Vec::new();
        let mut bytes = 0usize;
        let mut total = None;
        for _ in 0..256 {
            if Instant::now() >= deadline {
                return Err(Error::Trap("chapter traversal exceeded 30 minutes".into()));
            }
            let batch = match self.chapter_batch(series_id, cursor.as_deref()) {
                Err(Error::Source(SourceError::Network(error))) => {
                    tracing::debug!(%error, "retrying chapter batch in a fresh instance");
                    std::thread::sleep(Duration::from_secs(1));
                    if Instant::now() >= deadline {
                        return Err(Error::Trap("chapter traversal exceeded 30 minutes".into()));
                    }
                    self.chapter_batch(series_id, cursor.as_deref())?
                }
                result => result?,
            };
            if total.is_some_and(|total| total != batch.total) {
                return Err(Error::Pagination(
                    "chapter total changed during traversal".into(),
                ));
            }
            total = Some(batch.total);
            for chapter in batch.items {
                bytes = bytes.saturating_add(chapter_bytes(&chapter));
                if bytes > 32 << 20 {
                    return Err(Error::Pagination("chapter traversal exceeds 32 MiB".into()));
                }
                if !ids.insert(chapter.id.clone()) {
                    return Err(Error::Pagination(
                        "chapter pagination repeated an id".into(),
                    ));
                }
                chapters.push(chapter);
            }
            if chapters.len() > batch.total as usize {
                return Err(Error::Pagination(
                    "chapter count exceeds its declared total".into(),
                ));
            }
            tracing::debug!(
                chapters = chapters.len(),
                total = batch.total,
                "read chapter batch"
            );
            match batch.next {
                None if chapters.len() == batch.total as usize => return Ok(chapters),
                None => {
                    return Err(Error::Pagination(
                        "chapter pagination ended before its total".into(),
                    ));
                }
                Some(next) if cursors.insert(next.clone()) => cursor = Some(next),
                Some(_) => {
                    return Err(Error::Pagination(
                        "chapter pagination repeated a cursor".into(),
                    ));
                }
            }
        }
        Err(Error::Trap("chapter traversal exceeded 256 batches".into()))
    }

    /// One bounded batch; old components return their complete list on the first call.
    pub fn chapter_batch(
        &self,
        series_id: &str,
        cursor: Option<&str>,
    ) -> Result<ChapterBatch, Error> {
        if cursor.is_some_and(|cursor| cursor.is_empty() || cursor.len() > 4096) {
            return Err(Error::Pagination("invalid chapter cursor".into()));
        }
        let Some(pre) = &self.pagination else {
            if cursor.is_some() {
                return Err(Error::Pagination(
                    "this source has no chapter pagination".into(),
                ));
            }
            let items = self.chapters(series_id)?;
            let total = u32::try_from(items.len())
                .map_err(|_| Error::Pagination("too many chapters".into()))?;
            return Ok(ChapterBatch {
                items,
                next: None,
                total,
            });
        };
        let _call = self.context.calls.lock(false);
        let mut store = new_store(pre.engine(), Some(Arc::clone(&self.context)), false)?;
        let bindings = pre.instantiate(&mut store).map_err(trap)?;
        let batch = bindings
            .chek_source_chapter_pagination()
            .call_fetch(&mut store, series_id, cursor)
            .map_err(trap)?
            .map_err(Error::Source)?;
        if batch.total > 20_000
            || batch.items.len() > 500
            || batch.items.len() > batch.total as usize
            || batch
                .next
                .as_ref()
                .is_some_and(|next| next.is_empty() || next.len() > 4096 || batch.items.is_empty())
        {
            return Err(Error::Pagination("invalid chapter batch bounds".into()));
        }
        let mut ids = HashSet::new();
        if batch
            .items
            .iter()
            .any(|chapter| chapter.id.is_empty() || !ids.insert(&chapter.id))
            || batch.items.iter().map(chapter_bytes).sum::<usize>() > 4 << 20
        {
            return Err(Error::Pagination("invalid chapter batch contents".into()));
        }
        Ok(batch)
    }

    /// One page of a series' chapters straight from the extension, for
    /// sources whose sites list chapters a page at a time from either end.
    /// `None` when the extension doesn't answer pages itself, in which case
    /// the caller pages through [`Source::chapters`].
    pub fn chapter_page(
        &self,
        series_id: &str,
        order: ChapterOrder,
        offset: u32,
        limit: u32,
        after: Option<&str>,
    ) -> Result<Option<ChapterPage>, Error> {
        let Some(pre) = &self.pages else {
            return Ok(None);
        };
        let _call = self.context.calls.lock(false);
        let mut store = new_store(pre.engine(), Some(Arc::clone(&self.context)), false)?;
        let bindings = pre.instantiate(&mut store).map_err(trap)?;
        let page = bindings
            .chek_source_chapter_pages()
            .call_page(&mut store, series_id, order, offset, limit, after)
            .map_err(trap)?
            .map_err(Error::Source)?;
        let mut ids = HashSet::new();
        if page.total > 20_000
            || page.items.len() > limit as usize
            || page.items.len() > 500
            || page
                .items
                .iter()
                .any(|chapter| chapter.id.is_empty() || !ids.insert(&chapter.id))
            || page.items.iter().map(chapter_bytes).sum::<usize>() > 4 << 20
        {
            return Err(Error::Pagination("invalid chapter page".into()));
        }
        Ok(Some(page))
    }

    /// The chapter's pages in reading order.
    pub fn pages(&self, chapter_id: &str) -> Result<Vec<Image>, Error> {
        let waiting = Instant::now();
        let _call = self.context.calls.lock(true);
        tracing::debug!(
            queue_ms = waiting.elapsed().as_millis() as u64,
            "reader source call started"
        );
        run(
            &self.pre,
            Some(Arc::clone(&self.context)),
            false,
            |bindings, store| bindings.call_pages(store, chapter_id),
        )?
        .map_err(Error::Source)
    }

    /// How to read a chapter: its pages, or a file such as a book. Sources
    /// without the chapter content export always answer their pages.
    pub fn content(&self, chapter_id: &str) -> Result<Reading, Error> {
        let Some(pre) = &self.contents else {
            return Ok(Reading {
                content: Content::Pages(self.pages(chapter_id)?),
                rights: None,
            });
        };
        let reading = {
            let _call = self.context.calls.lock(true);
            let mut store = new_store(pre.engine(), Some(Arc::clone(&self.context)), false)?;
            let bindings = pre.instantiate(&mut store).map_err(trap)?;
            bindings
                .chek_source_chapter_content()
                .call_read(&mut store, chapter_id)
                .map_err(trap)?
                .map_err(Error::Source)?
        };
        check_reading(reading, &self.formats)
    }

    /// Downloads a file the extension returned, held to the same sites, and
    /// checks it's the format the extension named.
    pub fn fetch_asset(&self, asset: &Asset) -> Result<Fetched, Error> {
        if asset.format == ContentFormat::Images || !self.formats.contains(&asset.format) {
            return Err(Error::Asset(format!(
                "the extension doesn't declare {}",
                format_name(asset.format)
            )));
        }
        let (status, _, bytes) = self
            .context
            .net
            .send(
                Method::GET,
                &asset.url,
                &asset.headers,
                None,
                MAX_ASSET,
                ASSET_TIMEOUT,
            )
            .map_err(|error| {
                Error::Asset(match error {
                    FetchError::NotAllowed(reason) => reason,
                    FetchError::TooLarge => format!("larger than {MAX_ASSET} bytes"),
                    FetchError::Timeout => "timed out".to_owned(),
                    FetchError::Failed(reason) => reason,
                })
            })?;
        if !(200..300).contains(&status) {
            return Err(Error::Asset(format!("the site answered {status}")));
        }
        // Sites answer errors and sign-in pages with 200 too, so the bytes
        // must start the way the format does before clients are told it's one.
        let magic: &[u8] = match asset.format {
            ContentFormat::Epub => b"PK\x03\x04",
            ContentFormat::Pdf => b"%PDF-",
            ContentFormat::Images => unreachable!("refused above"),
        };
        if !bytes.starts_with(magic) {
            return Err(Error::Asset(format!(
                "the site didn't answer with {}",
                format_name(asset.format)
            )));
        }
        Ok(Fetched {
            content_type: Some(media_type(asset.format).to_owned()),
            bytes,
        })
    }

    /// Fetches a cover or page the extension returned, held to the same sites.
    pub fn fetch_image(&self, image: &Image) -> Result<Fetched, Error> {
        if let Some(parameters) = &image.transform {
            if !self
                .manifest
                .capabilities
                .contains(&Capability::ImageTransform)
            {
                return Err(Error::Image(
                    "the extension doesn't declare image-transform".to_owned(),
                ));
            }
            if parameters.len() > 16 << 10 {
                return Err(Error::Image(
                    "image transform parameters exceed 16 KiB".to_owned(),
                ));
            }
        }
        let key = self.image_key(image);
        if let Some((content_type, bytes)) = self.context.images.get(&key) {
            return Ok(Fetched {
                content_type,
                bytes,
            });
        }
        let gate = Arc::clone(
            self.context
                .fetching
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(key.clone())
                .or_default(),
        );
        let fetched = {
            let _gate = gate.lock().unwrap_or_else(|e| e.into_inner());
            match self.context.images.get(&key) {
                // Another request fetched it while this one waited.
                Some((content_type, bytes)) => Ok(Fetched {
                    content_type,
                    bytes,
                }),
                None => self.fetch_uncached(image, &key),
            }
        };
        self.context
            .fetching
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key);
        fetched
    }

    /// Fetches the pages ahead of a reader into the image cache, a few at a
    /// time, so turning a page finds it already on the server. Starting
    /// another read-ahead, such as for the next chapter, ends this one.
    pub fn read_ahead(self: &Arc<Self>, images: Vec<Image>) {
        if !self.context.images.enabled() || images.is_empty() {
            return;
        }
        let generation = self.context.warming.fetch_add(1, Ordering::SeqCst) + 1;
        let queue = Arc::new(Mutex::new(images.into_iter()));
        for _ in 0..READ_AHEAD {
            let source = Arc::clone(self);
            let queue = Arc::clone(&queue);
            std::thread::spawn(move || {
                while source.context.warming.load(Ordering::SeqCst) == generation {
                    let Some(image) = queue.lock().unwrap_or_else(|e| e.into_inner()).next() else {
                        break;
                    };
                    if source.context.images.contains(&source.image_key(&image)) {
                        continue;
                    }
                    // A page that fails here fails for the reader too, and
                    // they'll see why when they reach it.
                    if source.fetch_image(&image).is_err() {
                        break;
                    }
                }
            });
        }
    }

    /// Names a fetched image in the cache by everything its bytes depend on.
    fn image_key(&self, image: &Image) -> String {
        repository::sha256_hex(
            json!([
                self.context.image_scope(&image.url),
                self.manifest.version,
                self.digest,
                request_key(&image.url, &image.headers),
                image.transform
            ])
            .to_string()
            .as_bytes(),
        )
    }

    fn fetch_uncached(&self, image: &Image, key: &str) -> Result<Fetched, Error> {
        let max_bytes = if image.transform.is_some() {
            transform::MAX_BYTES as u64
        } else {
            MAX_IMAGE
        };
        let (status, headers, bytes) = self
            .context
            .net
            .send(
                Method::GET,
                &image.url,
                &image.headers,
                None,
                max_bytes,
                http::TIMEOUT,
            )
            .map_err(|error| {
                Error::Image(match error {
                    FetchError::NotAllowed(reason) => reason,
                    FetchError::TooLarge => format!("larger than {max_bytes} bytes"),
                    FetchError::Timeout => "timed out".to_owned(),
                    FetchError::Failed(reason) => reason,
                })
            })?;
        if !(200..300).contains(&status) {
            return Err(Error::Image(format!("the site answered {status}")));
        }
        let mut content_type = http::content_type(&headers);
        // Some image hosts answer `application/octet-stream`, which clients
        // then refuse to show; the bytes say what the image is.
        if !content_type
            .as_deref()
            .is_some_and(|t| t.starts_with("image/"))
            && let Some(sniffed) = sniff(&bytes)
        {
            content_type = Some(sniffed.to_owned());
        }
        let mut bytes = bytes;
        if image.transform.is_some() {
            let response = bindings::chek::source::http::Response {
                status,
                headers: headers
                    .iter()
                    .filter_map(|(n, v)| Some((n.to_string(), v.to_str().ok()?.to_owned())))
                    .collect(),
                body: bytes.clone(),
                cached: false,
            };
            if let Some(transformed) = run(
                &self.pre,
                Some(Arc::clone(&self.context)),
                true,
                |bindings, store| bindings.call_transform_image(store, image, &response),
            )?
            .map_err(Error::Source)?
            {
                if transformed.body.len() > transform::MAX_BYTES
                    || !matches!(
                        transformed.content_type.as_str(),
                        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
                    )
                {
                    return Err(Error::Image(
                        "transformed image exceeds 8 MiB or has an unsupported content type"
                            .to_owned(),
                    ));
                }
                // Older transforms label `rearrange` output as PNG, which
                // it no longer always is.
                content_type =
                    Some(sniff(&transformed.body).map_or(transformed.content_type, str::to_owned));
                bytes = transformed.body;
            }
        }
        self.context
            .images
            .put(key, content_type.as_deref(), &bytes);
        Ok(Fetched {
            content_type,
            bytes,
        })
    }

    fn call<R>(
        &self,
        export: impl FnOnce(&bindings::Source, &mut Store<State>) -> wasmtime::Result<R>,
    ) -> Result<R, Error> {
        let waiting = Instant::now();
        let _call = self.context.calls.lock(false);
        tracing::debug!(
            queue_ms = waiting.elapsed().as_millis() as u64,
            "catalog source call started"
        );
        run(&self.pre, Some(Arc::clone(&self.context)), false, export)
    }
}

/// An image's type from its first bytes, for the common page formats.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

fn chapter_bytes(chapter: &Chapter) -> usize {
    chapter.id.len()
        + chapter.title.len()
        + chapter.number.as_ref().map_or(0, String::len)
        + chapter.version.as_ref().map_or(0, String::len)
        + chapter.published.as_ref().map_or(0, String::len)
        + chapter.language.as_ref().map_or(0, String::len)
        + chapter.release_group.as_ref().map_or(0, String::len)
}

/// What a format is called in the API and in indexes.
pub fn format_name(format: ContentFormat) -> &'static str {
    match format {
        ContentFormat::Images => "images",
        ContentFormat::Epub => "epub",
        ContentFormat::Pdf => "pdf",
    }
}

/// The media type a file in `format` is served as.
pub fn media_type(format: ContentFormat) -> &'static str {
    match format {
        ContentFormat::Images => "image/*",
        ContentFormat::Epub => "application/epub+zip",
        ContentFormat::Pdf => "application/pdf",
    }
}

fn check_formats(mut formats: Vec<ContentFormat>) -> Result<Vec<ContentFormat>, Error> {
    formats.sort_by_key(|format| format_name(*format));
    formats.dedup();
    if formats.is_empty() {
        return Err(Error::Manifest("the extension lists no formats".to_owned()));
    }
    Ok(formats)
}

/// The longest file id, file name or rights text kept from an extension.
const MAX_TEXT: usize = 2048;

fn check_reading(mut reading: Reading, formats: &[ContentFormat]) -> Result<Reading, Error> {
    match &mut reading.content {
        Content::Pages(_) if !formats.contains(&ContentFormat::Images) => {
            return Err(Error::Content(
                "it returned pages but doesn't list images among its formats".to_owned(),
            ));
        }
        Content::Pages(_) => {}
        Content::File(asset) => {
            if asset.format == ContentFormat::Images || !formats.contains(&asset.format) {
                return Err(Error::Content(format!(
                    "it returned a file in {}, which it doesn't list among its formats",
                    format_name(asset.format)
                )));
            }
            if asset.id.is_empty() || asset.id.len() > MAX_FIELD {
                return Err(Error::Content(format!(
                    "a file's id must be between 1 and {MAX_FIELD} bytes"
                )));
            }
            if asset.headers.len() > 64 {
                return Err(Error::Content("a file has more than 64 headers".to_owned()));
            }
            if let Some(name) = &mut asset.file_name {
                truncate(name, MAX_FIELD);
            }
        }
    }
    if let Some(rights) = &mut reading.rights {
        for text in [&mut rights.license, &mut rights.attribution]
            .into_iter()
            .flatten()
        {
            truncate(text, MAX_TEXT);
        }
        // Links go to readers, so only web pages are passed on.
        for link in [&mut rights.license_url, &mut rights.source_url] {
            if link
                .as_deref()
                .is_some_and(|url| url.len() > MAX_TEXT || !url.starts_with("https://"))
            {
                *link = None;
            }
        }
    }
    Ok(reading)
}

/// Runs one export in a fresh instance.
fn run<R>(
    pre: &bindings::SourcePre<State>,
    context: Option<Arc<Context>>,
    transforming: bool,
    export: impl FnOnce(&bindings::Source, &mut Store<State>) -> wasmtime::Result<R>,
) -> Result<R, Error> {
    let mut store = new_store(pre.engine(), context, transforming)?;
    let bindings = pre.instantiate(&mut store).map_err(trap)?;
    export(&bindings, &mut store).map_err(trap)
}

fn new_store(
    engine: &Engine,
    context: Option<Arc<Context>>,
    transforming: bool,
) -> Result<Store<State>, Error> {
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
        transforming,
        session: None,
        persistent_session: false,
        holds_browser: false,
        image_operations: 0,
    };
    let mut store = Store::new(engine, state);
    store.limiter(|state| &mut state.limits);
    store.set_fuel(MAX_FUEL).map_err(trap)?;
    store.set_epoch_deadline(if transforming { 1_000 } else { 12_000 });
    Ok(store)
}

struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    /// None while the manifest is read.
    context: Option<Arc<Context>>,
    requests: u32,
    transforming: bool,
    session: Option<session::Session>,
    persistent_session: bool,
    /// Whether this call holds the source's one browser.
    holds_browser: bool,
    image_operations: u32,
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
        if self.transforming {
            return Err(HttpError::NotAllowed(
                "image transforms have no network or state access".to_owned(),
            ));
        }
        let Some(context) = &self.context else {
            return Err(HttpError::NotAllowed(
                "the manifest is read without network".to_owned(),
            ));
        };
        let cache = request
            .cache
            .filter(|ttl| *ttl > 0 && matches!(request.method, Method::Get))
            .map(|ttl| (ttl, request_key(&request.url, &request.headers)));
        let scope = context.response_scope(&request.url);
        if let Some((_, key)) = &cache
            && let Some((Some(meta), body)) = context.store.get(scope, Namespace::Response, key)
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
                scope,
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
        if self.transforming {
            return Err("image transforms have no network or state access".to_owned());
        }
        self.context
            .as_deref()
            .ok_or_else(|| "not available while the manifest is read".to_owned())
    }
}

impl bindings::chek::source::browser_session::Host for State {
    fn open(&mut self, url: String) -> Result<(), String> {
        self.release_browser();
        // Held apart from `self`, which records below that it took the browser.
        self.context()?;
        let context = Arc::clone(self.context.as_ref().ok_or("no source context")?);
        let context = context.as_ref();
        context.net.allowed_url(&url)?;
        if context.verification_pending.load(Ordering::SeqCst) {
            return Err("complete the open verification browser and resume before retrying".into());
        }
        // A call waits as long as another's browser may stay open.
        context.browsers.take(session::LIFETIME)?;
        self.holds_browser = true;
        let saved = context
            .handoff
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(mut session) = saved.filter(|session| !session.expired()) {
            session.reset_operations();
            self.session = Some(session);
            self.persistent_session = true;
            return self
                .browser_operation(json!({"type":"open","url":url}))
                .map(|_| ());
        }
        let executable = context
            .browser_executable
            .as_ref()
            .ok_or("browser-session requires its capability and CHEK_BROWSER_EXECUTABLE")?;
        let worker = context
            .worker_executable
            .as_ref()
            .ok_or("no native worker executable is configured")?;
        self.session = Some(session::Session::start(worker, executable, context)?);
        self.browser_operation(json!({"type": "open", "url": url}))
            .map(|_| ())
    }

    fn evaluate(&mut self, script: String) -> Result<String, String> {
        self.browser_operation(json!({"type": "evaluate", "script": script}))
    }

    fn click(&mut self, x: u32, y: u32) -> Result<(), String> {
        self.browser_operation(json!({"type": "click", "x": x, "y": y}))
            .map(|_| ())
    }

    fn scroll(&mut self, x: u32, y: u32, delta_x: i32, delta_y: i32) -> Result<(), String> {
        self.browser_operation(
            json!({"type": "scroll", "x": x, "y": y, "deltaX": delta_x, "deltaY": delta_y}),
        )
        .map(|_| ())
    }

    fn close(&mut self) {
        self.release_browser();
    }
}

impl State {
    fn release_browser(&mut self) {
        let session = self.session.take();
        if self.persistent_session
            && let Some(context) = &self.context
        {
            *context.handoff.lock().unwrap_or_else(|e| e.into_inner()) = session;
        }
        self.persistent_session = false;
        if std::mem::take(&mut self.holds_browser)
            && let Some(context) = &self.context
        {
            context.browsers.give_back();
        }
    }

    fn browser_operation(&mut self, command: serde_json::Value) -> Result<String, String> {
        let mut session = self.session.take().ok_or("no browser session is open")?;
        let context = self
            .context
            .as_ref()
            .ok_or("browser isn't available while reading the manifest")?;
        let result = session.operation(context, command, &mut self.requests);
        if result.is_ok() || session.recoverable {
            self.session = Some(session);
        }
        result
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.release_browser();
    }
}

impl bindings::chek::source::images::Host for State {
    fn rearrange(
        &mut self,
        body: Vec<u8>,
        columns: u32,
        rows: u32,
        order: Vec<u32>,
    ) -> Result<Vec<u8>, String> {
        if !self.transforming {
            return Err("image operations are only available in transform-image".to_owned());
        }
        if self.image_operations >= 1 {
            return Err("one native image operation is allowed per transform".to_owned());
        }
        self.image_operations += 1;
        let path = self
            .context
            .as_ref()
            .and_then(|c| c.image_worker.as_ref())
            .ok_or("image-transform requires its capability and an image worker")?;
        transform::rearrange(path, &body, columns, rows, &order)
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
        let Some(browser) = context.net.solver() else {
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
