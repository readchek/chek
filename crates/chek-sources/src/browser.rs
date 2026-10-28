//! A real browser for extensions whose sites need one, such as sites behind
//! a Cloudflare challenge. The host doesn't run a browser itself: it asks a
//! solver service the admin runs, and keeps what the browser earned, its
//! cookies and user agent, so later requests pass without it.
//!
//! The solver is a trait so services other than FlareSolverr can be added;
//! FlareSolverr's API is also what most alternatives speak.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::Url;
use reqwest::blocking::Client;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use serde_json::json;

use crate::cookies::Jar;
use crate::store::{MAX_TTL, Namespace, Store};

/// How long a solver may take; challenges can take several seconds, and
/// a busy solver queues requests.
const SOLVER_TIMEOUT: Duration = Duration::from_secs(90);
/// How long the solver's browser may spend on one page.
const BROWSER_TIMEOUT_MS: u64 = 60_000;
const USER_AGENT_KEY: &str = "user-agent";

/// What a browser saw after loading a page.
pub(crate) struct Solution {
    pub(crate) status: u16,
    pub(crate) html: String,
    /// Cookies the browser holds afterwards, as `Set-Cookie` values for
    /// the page's URL.
    pub(crate) set_cookies: Vec<String>,
    pub(crate) user_agent: Option<String>,
}

/// Something that loads a page in a real browser.
pub(crate) trait Solver: Send + Sync {
    fn solve(&self, url: &Url) -> Result<Solution, String>;
}

/// A FlareSolverr-compatible service, such as `http://flaresolverr:8191/v1`.
pub(crate) struct FlareSolverr {
    proxy: Option<crate::OutboundProxy>,
    /// Made on the first solve: a blocking client can't be made or dropped
    /// on the async runtime, where the host is set up.
    client: OnceLock<Client>,
    endpoint: Url,
}

impl FlareSolverr {
    pub(crate) fn new(endpoint: &str, proxy: Option<crate::OutboundProxy>) -> Result<Self, String> {
        let endpoint =
            Url::parse(endpoint).map_err(|e| format!("browser solver `{endpoint}`: {e}"))?;
        if !matches!(endpoint.scheme(), "http" | "https") {
            return Err(format!(
                "browser solver `{endpoint}` isn't an http or https address"
            ));
        }
        Ok(Self {
            proxy,
            client: OnceLock::new(),
            endpoint,
        })
    }

    fn client(&self) -> Result<&Client, String> {
        if let Some(client) = self.client.get() {
            return Ok(client);
        }
        let client = Client::builder()
            .no_proxy()
            .timeout(SOLVER_TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(self.client.get_or_init(|| client))
    }
}

#[derive(Deserialize)]
struct Answer {
    status: String,
    #[serde(default)]
    message: String,
    solution: Option<AnswerSolution>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnswerSolution {
    #[serde(default)]
    status: u16,
    #[serde(default)]
    response: String,
    #[serde(default)]
    cookies: Vec<AnswerCookie>,
    #[serde(default)]
    user_agent: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnswerCookie {
    name: String,
    value: String,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    path: Option<String>,
    /// Seconds since the epoch, or -1 for a session cookie.
    #[serde(default)]
    expires: Option<f64>,
    #[serde(default)]
    secure: bool,
    #[serde(default)]
    http_only: bool,
}

impl AnswerCookie {
    fn set_cookie(&self) -> String {
        let mut text = format!("{}={}", self.name, self.value);
        if let Some(domain) = self.domain.as_deref().filter(|d| !d.is_empty()) {
            text.push_str(&format!("; Domain={domain}"));
        }
        text.push_str(&format!("; Path={}", self.path.as_deref().unwrap_or("/")));
        if let Some(expires) = self.expires.filter(|e| *e > 0.0) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0.0, |d| d.as_secs_f64());
            text.push_str(&format!("; Max-Age={}", (expires - now).max(0.0) as u64));
        }
        if self.secure {
            text.push_str("; Secure");
        }
        if self.http_only {
            text.push_str("; HttpOnly");
        }
        text
    }
}

impl Solver for FlareSolverr {
    fn solve(&self, url: &Url) -> Result<Solution, String> {
        let mut request = json!({
            "cmd": "request.get",
            "url": url.as_str(),
            "maxTimeout": BROWSER_TIMEOUT_MS,
        });
        if let Some(proxy) = &self.proxy {
            request["proxy"] = proxy.solver();
        }
        let answer: Answer = self
            .client()?
            .post(self.endpoint.clone())
            .json(&request)
            .send()
            .and_then(|r| r.json())
            .map_err(|e| format!("the browser solver failed: {}", crate::http::describe(&e)))?;
        let solution = match (answer.status.as_str(), answer.solution) {
            ("ok", Some(solution)) => solution,
            _ => {
                return Err(format!(
                    "the browser solver couldn't load the page: {}",
                    answer.message
                ));
            }
        };
        Ok(Solution {
            status: solution.status,
            html: solution.response,
            set_cookies: solution
                .cookies
                .iter()
                .map(AnswerCookie::set_cookie)
                .collect(),
            user_agent: solution.user_agent,
        })
    }
}

/// One extension's use of the solver.
pub(crate) struct Browser {
    solver: Option<Arc<dyn Solver>>,
    jar: Arc<Jar>,
    store: Arc<Store>,
    scope: String,
    /// Clearances are tied to the browser's user agent, so requests that
    /// carry its cookies must send it too.
    user_agent: Mutex<Option<String>>,
    /// One solve at a time, so a burst of challenged requests asks the
    /// solver once.
    solving: Mutex<()>,
    /// Counts solves, so a request that waited on another's solve retries
    /// rather than solving again.
    solves: AtomicU64,
}

impl Browser {
    pub(crate) fn new(
        solver: Option<Arc<dyn Solver>>,
        jar: Arc<Jar>,
        store: Arc<Store>,
        scope: String,
    ) -> Self {
        let user_agent = store
            .get(&scope, Namespace::Browser, USER_AGENT_KEY)
            .and_then(|(_, value)| String::from_utf8(value).ok());
        Self {
            solver,
            jar,
            store,
            scope,
            user_agent: Mutex::new(user_agent),
            solving: Mutex::default(),
            solves: AtomicU64::new(0),
        }
    }

    pub(crate) fn user_agent(&self) -> Option<String> {
        lock(&self.user_agent).clone()
    }

    /// Whether an actual solver service is configured, rather than only a
    /// local browser executable.
    pub(crate) fn has_solver(&self) -> bool {
        self.solver.is_some()
    }

    pub(crate) fn solves(&self) -> u64 {
        self.solves.load(Ordering::SeqCst)
    }

    /// Loads `url` in the browser and keeps its cookies and user agent.
    /// When another request finished a solve since `seen` was read, returns
    /// without solving, since its clearance is likely good for this one too.
    pub(crate) fn clear(&self, url: &Url, seen: u64) -> Result<Option<Solution>, String> {
        let _solving = lock(&self.solving);
        if self.solves() != seen {
            return Ok(None);
        }
        self.load(url).map(Some)
    }

    /// Loads `url` in the browser, keeping what it earned.
    pub(crate) fn load(&self, url: &Url) -> Result<Solution, String> {
        let solution = self
            .solver
            .as_ref()
            .ok_or("no browser solver is configured")?
            .solve(url)?;
        // Cookies for other sites are refused by the jar, which only takes
        // cookies `url` may set.
        let _ = self
            .jar
            .set(url, solution.set_cookies.iter().map(String::as_str));
        if let Some(agent) = solution.user_agent.as_deref().filter(|a| !a.is_empty()) {
            self.remember_agent(agent);
        }
        self.solves.fetch_add(1, Ordering::SeqCst);
        Ok(solution)
    }

    pub(crate) fn remember_agent(&self, agent: &str) {
        let _ = self.store.set(
            &self.scope,
            Namespace::Browser,
            USER_AGENT_KEY,
            None,
            agent.as_bytes(),
            MAX_TTL,
        );
        *lock(&self.user_agent) = Some(agent.to_owned());
    }
}

/// Whether a response is a Cloudflare challenge page rather than the site.
pub(crate) fn is_challenge(status: u16, headers: &HeaderMap, body: &[u8]) -> bool {
    if !matches!(status, 403 | 429 | 503) {
        return false;
    }
    if headers
        .get("cf-mitigated")
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"challenge"))
    {
        return true;
    }
    let cloudflare = headers
        .get("server")
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"cloudflare"));
    let page = String::from_utf8_lossy(&body[..body.len().min(64 << 10)]);
    cloudflare
        && (page.contains("challenge-platform") || page.contains("<title>Just a moment...</title>"))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Both guarded values are replaced whole, so a panic can't leave either
    // half written.
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}
