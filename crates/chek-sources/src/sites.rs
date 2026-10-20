//! The hosts an extension declares it needs to reach.

use std::net::IpAddr;

/// A manifest's `sites`, checked once at load so every request only compares
/// strings.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sites(Vec<Site>);

#[derive(Clone, Debug)]
enum Site {
    /// Only this host.
    Exact(String),
    /// Any subdomain of this domain, but not the domain itself.
    Subdomains(String),
}

/// Manifests can't name more sites than this.
const MAX_SITES: usize = 64;

impl Sites {
    pub(crate) fn parse(entries: &[String]) -> Result<Self, String> {
        if entries.len() > MAX_SITES {
            return Err(format!(
                "names {} sites; the limit is {MAX_SITES}",
                entries.len()
            ));
        }
        entries
            .iter()
            .map(|entry| parse_site(entry))
            .collect::<Result<_, _>>()
            .map(Sites)
    }

    pub(crate) fn allows(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.0.iter().any(|site| match site {
            Site::Exact(name) => host == *name,
            Site::Subdomains(domain) => host
                .strip_suffix(domain.as_str())
                .is_some_and(|rest| rest.len() > 1 && rest.ends_with('.')),
        })
    }

    /// A permitted host may set a cookie for a parent domain. The jar is
    /// scoped to this extension; this does not permit requests to that parent.
    pub(crate) fn allows_cookie_domain(&self, domain: &str) -> bool {
        let suffix = format!(".{}", domain.to_ascii_lowercase());
        self.allows(domain)
            || self.0.iter().any(|site| {
                let (Site::Exact(host) | Site::Subdomains(host)) = site;
                host.eq_ignore_ascii_case(domain) || host.ends_with(&suffix)
            })
    }
}

fn parse_site(entry: &str) -> Result<Site, String> {
    let entry = entry.to_ascii_lowercase();
    let (wildcard, name) = match entry.strip_prefix("*.") {
        Some(name) => (true, name),
        None => (false, entry.as_str()),
    };
    // Addresses and local names would let an extension reach the server's
    // own network rather than the site it's written for.
    // IPv6 addresses fail the label check below.
    if name.parse::<IpAddr>().is_ok() {
        return Err(format!(
            "site `{entry}` is an IP address; name the host instead"
        ));
    }
    if name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local") {
        return Err(format!("site `{entry}` is a local name"));
    }
    let labels: Vec<&str> = name.split('.').collect();
    let valid = name.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    if !valid {
        return Err(format!(
            "site `{entry}` isn't a host name such as `api.example.org` or `*.example.org`"
        ));
    }
    let name = name.to_owned();
    Ok(if wildcard {
        Site::Subdomains(name)
    } else {
        Site::Exact(name)
    })
}
