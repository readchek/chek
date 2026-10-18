//! Slows password guessing. Each client address may get a password wrong a
//! few times in a window; after that, password checks from it are refused
//! until the oldest failure ages out. Counting by address rather than by
//! account means a stranger can't lock someone out of their own account.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::StatusCode;
use axum::http::request::Parts;

use crate::api::{ApiError, AppState};

/// Wrong passwords allowed from one address within [`WINDOW`].
const MAX_FAILURES: usize = 10;
const WINDOW: Duration = Duration::from_secs(15 * 60);
/// Addresses tracked at once; stale ones are dropped when this is reached.
const MAX_ADDRESSES: usize = 10_000;

#[derive(Default)]
pub struct PasswordLimiter {
    failures: Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
}

impl PasswordLimiter {
    /// Fails with `429 too_many_attempts` while the address is over the limit.
    pub fn check(&self, ip: Option<IpAddr>) -> Result<(), ApiError> {
        let Some(ip) = ip else {
            return Ok(());
        };
        let now = Instant::now();
        let mut failures = self.failures.lock().unwrap_or_else(|e| e.into_inner());
        let Some(times) = failures.get_mut(&ip) else {
            return Ok(());
        };
        prune(times, now);
        if times.len() < MAX_FAILURES {
            return Ok(());
        }
        let wait = times
            .front()
            .map_or(WINDOW, |oldest| WINDOW.saturating_sub(now - *oldest));
        Err(ApiError {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "too_many_attempts",
            message: format!(
                "too many wrong passwords from this address; try again in {} minutes",
                wait.as_secs().div_ceil(60).max(1)
            ),
        })
    }

    pub fn record_failure(&self, ip: Option<IpAddr>) {
        let Some(ip) = ip else {
            return;
        };
        let now = Instant::now();
        let mut failures = self.failures.lock().unwrap_or_else(|e| e.into_inner());
        if failures.len() >= MAX_ADDRESSES && !failures.contains_key(&ip) {
            failures.retain(|_, times| {
                prune(times, now);
                !times.is_empty()
            });
            // Still full of active addresses: start over rather than grow.
            if failures.len() >= MAX_ADDRESSES {
                failures.clear();
            }
        }
        let times = failures.entry(ip).or_default();
        prune(times, now);
        times.push_back(now);
    }
}

fn prune(times: &mut VecDeque<Instant>, now: Instant) {
    while times.front().is_some_and(|t| now - *t >= WINDOW) {
        times.pop_front();
    }
}

/// The address a request came from. Behind a reverse proxy listed in
/// `CHEK_TRUSTED_PROXIES`, it's the client the proxy names in
/// `X-Forwarded-For`; otherwise it's the connection's peer, since anyone can
/// send that header. `None` when the server has no connection address.
pub struct ClientIp(pub Option<IpAddr>);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(client_ip(parts, &state.trusted_proxies)))
    }
}

pub fn client_ip(parts: &Parts, trusted_proxies: &[IpAddr]) -> Option<IpAddr> {
    let peer = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()?
        .0
        .ip()
        .to_canonical();
    if !trusted_proxies.contains(&peer) {
        return Some(peer);
    }
    // Each proxy appends the address it received from, so the client is the
    // last entry that isn't one of our own proxies.
    let forwarded = parts
        .headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|entry| entry.trim().parse::<IpAddr>().ok())
        .map(|ip| ip.to_canonical())
        .collect::<Vec<_>>();
    Some(
        forwarded
            .into_iter()
            .rev()
            .find(|ip| !trusted_proxies.contains(ip))
            .unwrap_or(peer),
    )
}
