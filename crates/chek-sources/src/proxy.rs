//! Admin-configured outbound HTTP proxy, shared by extension transports.

use base64::Engine;
use reqwest::Url;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct OutboundProxy {
    url: String,
    username: Option<String>,
    password: Option<String>,
}

impl OutboundProxy {
    pub fn new(
        url: &str,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<Self, String> {
        let url = Url::parse(url).map_err(|_| "use an http://host:port proxy address")?;
        if url.scheme() != "http"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "use an http://host:port address without credentials, path or query".into(),
            );
        }
        if username.is_some() != password.is_some()
            || username.as_ref().is_some_and(|name| name.contains(':'))
        {
            return Err("set both proxy username and password; username cannot contain ':'".into());
        }
        Ok(Self {
            url: url.to_string(),
            username,
            password,
        })
    }

    pub(crate) fn reqwest(&self) -> Result<reqwest::Proxy, String> {
        let mut proxy = reqwest::Proxy::https(self.url.as_str())
            .map_err(|_| "invalid outbound proxy address")?;
        if let (Some(username), Some(password)) = (&self.username, &self.password) {
            proxy = proxy.basic_auth(username, password);
        }
        Ok(proxy)
    }

    pub(crate) fn address(&self) -> String {
        let url = Url::parse(&self.url).expect("validated proxy URL");
        format!(
            "{}:{}",
            url.host_str().expect("validated proxy host"),
            url.port_or_known_default().unwrap_or(80)
        )
    }

    pub(crate) fn authorization(&self) -> Option<String> {
        self.username
            .as_ref()
            .zip(self.password.as_ref())
            .map(|(username, password)| {
                base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"))
            })
    }

    pub(crate) fn solver(&self) -> serde_json::Value {
        let mut value = serde_json::json!({"url": self.url.as_str()});
        if let (Some(username), Some(password)) = (&self.username, &self.password) {
            value["username"] = username.clone().into();
            value["password"] = password.clone().into();
        }
        value
    }
}
