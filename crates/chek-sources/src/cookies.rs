//! An extension's cookies. The jar is sent with every request the extension
//! makes and takes every cookie a site sets, as a browser would, so logins
//! and challenge clearances carry over between calls. It's saved after each
//! change and survives restarts.

use std::sync::{Arc, Mutex, MutexGuard};

use cookie_store::{CookieDomain, CookieExpiration, CookieStore};
use reqwest::Url;
use reqwest::header::HeaderValue;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::store::Store;

/// The most cookies a jar keeps. Past this, new cookies are refused, so a
/// site can't grow the saved jar without bound.
const MAX_COOKIES: usize = 1_000;

pub(crate) struct Jar {
    extension_id: String,
    store: Arc<Store>,
    cookies: Mutex<CookieStore>,
}

impl Jar {
    pub(crate) fn browser_cookies(&self) -> Vec<BrowserCookie> {
        Self::snapshot(&self.lock())
    }

    fn snapshot(cookies: &CookieStore) -> Vec<BrowserCookie> {
        cookies
            .iter_unexpired()
            .filter_map(|cookie| {
                let domain = match &cookie.domain {
                    CookieDomain::HostOnly(domain) => domain.clone(),
                    CookieDomain::Suffix(domain) => format!(".{domain}"),
                    _ => return None,
                };
                Some(BrowserCookie {
                    name: cookie.name().to_owned(),
                    value: cookie.value().to_owned(),
                    domain,
                    path: cookie.path.as_ref().to_owned(),
                    expires: match cookie.expires {
                        CookieExpiration::AtUtc(at) => at.unix_timestamp() as f64,
                        CookieExpiration::SessionEnd => -1.0,
                    },
                    secure: cookie.secure().unwrap_or(false),
                    http_only: cookie.http_only().unwrap_or(false),
                    same_site: cookie.same_site().map(|s| s.to_string()),
                })
            })
            .collect()
    }

    pub(crate) fn merge_browser_cookies(
        &self,
        before: &[BrowserCookie],
        after: &[BrowserCookie],
        sites: &crate::sites::Sites,
    ) -> Result<(), String> {
        if after.len() > MAX_COOKIES {
            return Err("browser returned too many cookies".to_owned());
        }
        for cookie in after.iter().filter(|cookie| !before.contains(cookie)) {
            let host = cookie.domain.trim_start_matches('.');
            if !sites.allows_cookie_domain(host) {
                continue;
            }
            let url = Url::parse(&format!("https://{host}/")).map_err(|e| e.to_string())?;
            let header = cookie.header()?;
            self.set(&url, [header.as_str()])?;
        }
        let mut cookies = self.lock();
        let current = Self::snapshot(&cookies);
        for cookie in before {
            if current.contains(cookie)
                && !after.iter().any(|c| {
                    c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path
                })
            {
                cookies.remove(
                    cookie.domain.trim_start_matches('.'),
                    &cookie.path,
                    &cookie.name,
                );
            }
        }
        self.save(&cookies);
        Ok(())
    }

    pub(crate) fn load(extension_id: &str, store: Arc<Store>) -> Self {
        let cookies = store
            .cookie_jar(extension_id)
            .and_then(|json| match cookie_store::serde::json::load(json.as_bytes()) {
                Ok(cookies) => Some(cookies),
                Err(error) => {
                    warn!(extension = extension_id, %error, "couldn't read saved cookies; starting empty");
                    None
                }
            })
            .unwrap_or_default();
        Self {
            extension_id: extension_id.to_owned(),
            store,
            cookies: Mutex::new(cookies),
        }
    }

    fn lock(&self) -> MutexGuard<'_, CookieStore> {
        // Cookie inserts are whole, so a panic can't leave the jar half
        // changed.
        self.cookies.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The cookies a request to `url` would carry, as name and value pairs.
    pub(crate) fn get(&self, url: &Url) -> Vec<(String, String)> {
        self.lock()
            .get_request_values(url)
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect()
    }

    /// Stores cookies as if `url` had answered with these `Set-Cookie`
    /// values.
    pub(crate) fn set<'a>(
        &self,
        url: &Url,
        set_cookies: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), String> {
        let mut cookies = self.lock();
        let mut changed = false;
        let mut refused = None;
        for set_cookie in set_cookies {
            if cookies.iter_unexpired().count() >= MAX_COOKIES {
                refused = Some(format!("the jar holds its limit of {MAX_COOKIES} cookies"));
                break;
            }
            match cookies.parse(set_cookie, url) {
                Ok(_) => changed = true,
                Err(error) => refused = Some(error.to_string()),
            }
        }
        if changed {
            self.save(&cookies);
        }
        refused.map_or(Ok(()), Err)
    }

    /// Removes every cookie.
    pub(crate) fn clear(&self) {
        let mut cookies = self.lock();
        cookies.clear();
        self.save(&cookies);
    }

    fn save(&self, cookies: &CookieStore) {
        // Session cookies are kept too: a restart of the server shouldn't
        // sign an extension out of a site. Expired ones are dropped on load.
        let mut json = Vec::new();
        let saved =
            cookie_store::serde::json::save_incl_expired_and_nonpersistent(cookies, &mut json)
                .map_err(|e| e.to_string())
                .and_then(|()| {
                    self.store
                        .save_cookie_jar(&self.extension_id, &String::from_utf8_lossy(&json))
                });
        if let Err(error) = saved {
            warn!(extension = self.extension_id, %error, "couldn't save cookies");
        }
    }
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserCookie {
    pub(crate) name: String,
    value: String,
    pub(crate) domain: String,
    path: String,
    expires: f64,
    secure: bool,
    http_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    same_site: Option<String>,
}

impl BrowserCookie {
    fn header(&self) -> Result<String, String> {
        for value in [&self.name, &self.value, &self.domain, &self.path] {
            if value.len() > 4096 || value.contains([';', '\r', '\n']) {
                return Err("browser returned an invalid cookie".to_owned());
            }
        }
        let mut header = format!("{}={}; Path={}", self.name, self.value, self.path);
        if self.domain.starts_with('.') {
            header.push_str(&format!("; Domain={}", self.domain));
        }
        if self.expires >= 0.0 {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_secs_f64());
            header.push_str(&format!(
                "; Max-Age={}",
                (self.expires - now).max(0.0) as u64
            ));
        }
        if self.secure {
            header.push_str("; Secure");
        }
        if self.http_only {
            header.push_str("; HttpOnly");
        }
        if let Some(site @ ("Strict" | "Lax" | "None")) = self.same_site.as_deref() {
            header.push_str(&format!("; SameSite={site}"));
        }
        Ok(header)
    }
}

impl reqwest::cookie::CookieStore for Jar {
    fn set_cookies(&self, headers: &mut dyn Iterator<Item = &HeaderValue>, url: &Url) {
        let values: Vec<&str> = headers.filter_map(|h| h.to_str().ok()).collect();
        if !values.is_empty() {
            // A cookie a site sets that the jar refuses is dropped, as a
            // browser would.
            let _ = self.set(url, values);
        }
    }

    fn cookies(&self, url: &Url) -> Option<HeaderValue> {
        let header = self
            .get(url)
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        if header.is_empty() {
            None
        } else {
            HeaderValue::from_str(&header).ok()
        }
    }
}
