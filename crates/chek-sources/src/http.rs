//! Requests made for an extension, limited to the sites its manifest declares.

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use reqwest::{Method, Url};

use crate::bindings::chek::source::http::Method as RequestMethod;
use crate::bindings::chek::source::http::{HttpError, Request, Response};
use crate::cookies::Jar;
use crate::sites::Sites;

const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 10;

/// The network one extension may use.
pub(crate) struct Net {
    client: Client,
    sites: Arc<Sites>,
}

/// A failed request, before it's mapped to what the extension or the
/// server's caller sees.
pub(crate) enum FetchError {
    NotAllowed(String),
    TooLarge,
    Timeout,
    Failed(String),
}

impl Net {
    /// Sends and fills `jar` on every request.
    pub(crate) fn new(sites: Sites, jar: Arc<Jar>) -> Result<Self, String> {
        let sites = Arc::new(sites);
        let redirect_sites = Arc::clone(&sites);
        let client = Client::builder()
            .user_agent(concat!("Chek/", env!("CARGO_PKG_VERSION")))
            .cookie_provider(jar)
            .timeout(TIMEOUT)
            .https_only(true)
            .redirect(Policy::custom(move |attempt| {
                if attempt.previous().len() >= MAX_REDIRECTS {
                    attempt.error("too many redirects")
                } else if attempt
                    .url()
                    .host_str()
                    .is_some_and(|host| redirect_sites.allows(host))
                {
                    attempt.follow()
                } else {
                    attempt.error("redirected to a site the extension doesn't declare")
                }
            }))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Net { client, sites })
    }

    /// Parses `url` and checks that it's HTTPS on a declared site.
    pub(crate) fn allowed_url(&self, url: &str) -> Result<Url, String> {
        let url = Url::parse(url).map_err(|error| format!("`{url}`: {error}"))?;
        if url.scheme() != "https" {
            return Err(format!("`{url}` isn't HTTPS"));
        }
        if !url.host_str().is_some_and(|host| self.sites.allows(host)) {
            return Err(format!("`{url}` isn't on a site the extension declares"));
        }
        Ok(url)
    }

    /// Sends a request and reads at most `max_body` bytes of the response.
    pub(crate) fn send(
        &self,
        method: Method,
        url: &str,
        headers: &[(String, String)],
        body: Option<Vec<u8>>,
        max_body: u64,
    ) -> Result<(u16, HeaderMap, Vec<u8>), FetchError> {
        let url = self.allowed_url(url).map_err(FetchError::NotAllowed)?;
        let mut header_map = HeaderMap::new();
        for (name, value) in headers {
            let name = HeaderName::try_from(name.as_str())
                .map_err(|_| FetchError::Failed(format!("invalid header name `{name}`")))?;
            let value = HeaderValue::try_from(value.as_str())
                .map_err(|_| FetchError::Failed(format!("invalid value for header `{name}`")))?;
            header_map.append(name, value);
        }
        let mut request = self.client.request(method, url).headers(header_map);
        if let Some(body) = body {
            request = request.body(body);
        }
        let response = request.send().map_err(map_error)?;
        if response
            .content_length()
            .is_some_and(|length| length > max_body)
        {
            return Err(FetchError::TooLarge);
        }
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let mut bytes = Vec::new();
        response
            .take(max_body + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| FetchError::Failed(error.to_string()))?;
        if bytes.len() as u64 > max_body {
            return Err(FetchError::TooLarge);
        }
        Ok((status, headers, bytes))
    }

    pub(crate) fn fetch(&self, request: Request, max_body: u64) -> Result<Response, HttpError> {
        let method = match request.method {
            RequestMethod::Get => Method::GET,
            RequestMethod::Post => Method::POST,
        };
        let (status, headers, body) = self
            .send(
                method,
                &request.url,
                &request.headers,
                request.body,
                max_body,
            )
            .map_err(|error| match error {
                FetchError::NotAllowed(reason) => HttpError::NotAllowed(reason),
                FetchError::TooLarge => HttpError::TooLarge,
                FetchError::Timeout => HttpError::Timeout,
                FetchError::Failed(reason) => HttpError::Failed(reason),
            })?;
        let headers = headers
            .iter()
            .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_owned())))
            .collect();
        Ok(Response {
            status,
            headers,
            body,
            cached: false,
        })
    }
}

pub(crate) fn content_type(headers: &HeaderMap) -> Option<String> {
    headers.get(CONTENT_TYPE)?.to_str().ok().map(str::to_owned)
}

fn map_error(error: reqwest::Error) -> FetchError {
    if error.is_timeout() {
        FetchError::Timeout
    } else if error.is_redirect() {
        FetchError::NotAllowed(describe(&error))
    } else {
        FetchError::Failed(describe(&error))
    }
}

/// An error and its causes, since reqwest's own message leaves out why a
/// request failed.
pub(crate) fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
