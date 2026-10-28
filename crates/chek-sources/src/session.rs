//! Browser sessions with either host-relayed HTTP or restricted native transport.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Context;
use crate::cookies::BrowserCookie;
use crate::worker::{Worker, remaining};

pub(crate) const LIFETIME: Duration = Duration::from_secs(120);
const OPERATION: Duration = Duration::from_secs(30);
const MAX_RESULT: usize = 1 << 20;
const MAX_SCRIPT: usize = 64 << 10;
const MAX_OPERATIONS: u32 = 128;

pub(crate) struct Session {
    worker: Worker,
    operations: u32,
    jar_snapshot: Option<Vec<BrowserCookie>>,
    pub(crate) recoverable: bool,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum Event {
    Ready {
        user_agent: String,
    },
    Request {
        id: u32,
        url: String,
        method: String,
        headers: Vec<(String, String)>,
        body: String,
    },
    Result {
        json: String,
        cookies: Vec<BrowserCookie>,
    },
    Error {
        message: String,
        #[serde(default)]
        recoverable: bool,
    },
}

impl Session {
    pub(crate) fn start(path: &Path, executable: &Path, context: &Context) -> Result<Self, String> {
        Self::launch(
            path,
            executable,
            context,
            context.native_sites.as_deref(),
            LIFETIME,
        )
    }

    pub(crate) fn handoff(
        path: &Path,
        executable: &Path,
        context: &Context,
        sites: &[String],
    ) -> Result<Self, String> {
        Self::launch(
            path,
            executable,
            context,
            Some(sites),
            Duration::from_secs(600),
        )
    }

    /// When the browser closes on its own.
    pub(crate) fn deadline(&self) -> Instant {
        self.worker.deadline
    }

    pub(crate) fn expired(&self) -> bool {
        Instant::now() >= self.worker.deadline
    }

    pub(crate) fn reset_operations(&mut self) {
        self.operations = 0;
    }

    fn launch(
        path: &Path,
        executable: &Path,
        context: &Context,
        sites: Option<&[String]>,
        lifetime: Duration,
    ) -> Result<Self, String> {
        let started = Instant::now();
        let mut command = Command::new(path);
        command.arg("--extension-browser-worker").arg(executable);
        let worker = Worker::start(command, lifetime)?;
        let deadline = Instant::now() + OPERATION;
        // Only a solver-earned clearance is tied to the user agent it was
        // earned with. A local browser brings its own, and forcing an old one
        // would undo the browser's stealth fingerprint.
        let saved_agent = context
            .net
            .browser()
            .filter(|browser| browser.has_solver())
            .and_then(crate::browser::Browser::user_agent);
        let mut start = json!({"type": "start", "userAgent": saved_agent});
        start["args"] = json!(context.browser_args);
        if let Some(sites) = sites {
            start["sites"] = json!(sites);
            start["proxy"] = json!(context.net.outbound_proxy);
        }
        worker.send(&start, deadline)?;
        match serde_json::from_value(worker.receive(deadline)?)
            .map_err(|e| format!("invalid browser greeting: {e}"))?
        {
            Event::Ready { user_agent } if !user_agent.is_empty() && user_agent.len() <= 1024 => {
                if let Some(browser) = context.net.browser() {
                    browser.remember_agent(&user_agent);
                }
            }
            Event::Error { message, .. } => return Err(message),
            _ => return Err("browser worker didn't start".to_owned()),
        }
        tracing::debug!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "source browser started"
        );
        Ok(Self {
            worker,
            operations: 0,
            jar_snapshot: None,
            recoverable: false,
        })
    }

    pub(crate) fn operation(
        &mut self,
        context: &Context,
        mut command: Value,
        requests: &mut u32,
    ) -> Result<String, String> {
        let started = Instant::now();
        self.recoverable = false;
        if self.operations >= MAX_OPERATIONS {
            return Err("browser session reached its operation limit".to_owned());
        }
        if command
            .get("script")
            .and_then(Value::as_str)
            .is_some_and(|s| s.len() > MAX_SCRIPT)
        {
            return Err("browser script is larger than 64 KiB".to_owned());
        }
        self.operations += 1;
        let deadline = (Instant::now() + OPERATION).min(self.worker.deadline);
        let cookies = context.jar.browser_cookies();
        // Leave the live page's cookies alone unless the shared jar changed.
        // Timers may have changed document.cookie since the previous result.
        if self.jar_snapshot.as_ref() != Some(&cookies) {
            command["cookies"] = serde_json::to_value(&cookies).map_err(|e| e.to_string())?;
        }
        self.worker.send(&command, deadline)?;
        loop {
            let event: Event = serde_json::from_value(self.worker.receive(deadline)?)
                .map_err(|e| format!("invalid browser response: {e}"))?;
            match event {
                Event::Request {
                    id,
                    url,
                    method,
                    headers,
                    body,
                } => {
                    if *requests >= crate::MAX_REQUESTS {
                        return Err("browser reached the call's request limit".to_owned());
                    }
                    *requests += 1;
                    if body.len() > 2 << 20
                        || headers.len() > 128
                        || headers
                            .iter()
                            .map(|(n, v)| n.len() + v.len())
                            .sum::<usize>()
                            > 64 << 10
                    {
                        return Err("browser request is too large".to_owned());
                    }
                    let body = STANDARD
                        .decode(body)
                        .map_err(|_| "invalid browser request body")?;
                    let response = match context.net.send_browser(
                        &method,
                        &url,
                        &headers,
                        body,
                        remaining(deadline)?,
                    ) {
                        Ok((status, headers, body)) => {
                            let headers: Vec<_> = headers
                                .iter()
                                .filter_map(|(n, v)| Some((n.as_str(), v.to_str().ok()?)))
                                .collect();
                            json!({"type": "response", "id": id, "status": status, "headers": headers, "body": STANDARD.encode(body)})
                        }
                        Err(error) => json!({"type": "response", "id": id, "error": error}),
                    };
                    self.worker.send(&response, deadline)?;
                }
                Event::Result {
                    json,
                    cookies: updated,
                } => {
                    tracing::debug!(
                        operation = command
                            .get("type")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown"),
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "source browser operation completed"
                    );
                    if json.len() > MAX_RESULT {
                        return Err("browser result is larger than 1 MiB".to_owned());
                    }
                    serde_json::from_str::<Value>(&json)
                        .map_err(|_| "browser result isn't JSON")?;
                    context
                        .jar
                        .merge_browser_cookies(&cookies, &updated, context.net.sites())?;
                    self.jar_snapshot = Some(context.jar.browser_cookies());
                    return Ok(json);
                }
                Event::Error {
                    message,
                    recoverable,
                } => {
                    self.recoverable = recoverable;
                    return Err(message);
                }
                Event::Ready { .. } => return Err("unexpected browser greeting".to_owned()),
            }
        }
    }
}
