//! A small native CDP worker. Chromium owns JavaScript and input; Chek owns
//! every HTTP request. No browser automation runtime is needed.

use std::collections::HashSet;
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket, protocol::WebSocketConfig};

const MAX_MESSAGE: usize = 24 << 20;
const MAX_RESULT: usize = 1 << 20;
const OPERATION: Duration = Duration::from_secs(30);

/// Internal worker entry point. The host bounds its lifetime and kills the
/// entire process group on timeout, including Chromium's child processes.
pub fn browser_worker(executable: &Path) -> Result<(), String> {
    let (sender, input) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        loop {
            let message = read(&mut stdin);
            let failed = message.is_err();
            if sender.send(message).is_err() || failed {
                break;
            }
        }
    });
    let start = input.recv().map_err(|_| "browser input closed")??;
    if start["type"] != "start" {
        return Err("expected browser start".to_owned());
    }
    let result = serve(executable, start, &input);
    if let Err(error) = &result {
        write(&json!({"type": "error", "message": error}))?;
    }
    result
}

fn serve(
    executable: &Path,
    start: Value,
    input: &Receiver<Result<Value, String>>,
) -> Result<(), String> {
    // The supervising host owns this directory and removes it even if this
    // process is killed while Chromium is running.
    let profile = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join("profile");
    // A loopback sink, not a forwarding proxy. Requests not intercepted by
    // CDP (for example a WebSocket) cannot leave through Chromium's network.
    let sink = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let mut proxy = format!(
        "--proxy-server=http://{}",
        sink.local_addr().map_err(|e| e.to_string())?
    );
    std::thread::spawn(move || {
        for connection in sink.incoming() {
            drop(connection);
        }
    });
    let native_sites = start
        .get("sites")
        .map(|value| {
            let entries: Vec<String> =
                serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            crate::sites::Sites::parse(&entries)
        })
        .transpose()?;
    if let Some(sites) = &native_sites {
        let upstream = serde_json::from_value(start["proxy"].clone())
            .map_err(|_| "invalid outbound proxy configuration")?;
        proxy = format!(
            "--proxy-server={}",
            crate::tunnel::start(sites.clone(), upstream)?
        );
    }
    let args: Vec<String> = serde_json::from_value(start["args"].clone())
        .map_err(|e| format!("invalid browser launch arguments: {e}"))?;
    let child = Command::new(executable)
        .args(args)
        .args([
            "--remote-debugging-port=0",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--disable-background-timer-throttling",
            "--disable-backgrounding-occluded-windows",
            "--disable-renderer-backgrounding",
            "--disable-component-update",
            "--disable-extensions",
            "--disable-sync",
            "--disable-quic",
            "--proxy-bypass-list=<-loopback>",
            "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1",
            "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
            "--no-startup-window",
        ])
        .arg(if native_sites.is_some() {
            "--js-flags=--max-old-space-size=256"
        } else {
            "--js-flags=--max-old-space-size=64"
        })
        .arg(proxy)
        .arg(format!("--user-data-dir={}", profile.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("couldn't start Chromium: {e}"))?;
    let mut process = BrowserProcess(child);
    let deadline = Instant::now() + OPERATION;
    let address = loop {
        if let Ok(port) = std::fs::read_to_string(profile.join("DevToolsActivePort")) {
            break port;
        }
        crate::worker::remaining(deadline)?;
        if process.0.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("Chromium exited before opening CDP".to_owned());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut lines = address.lines();
    let port: u16 = lines
        .next()
        .ok_or("missing CDP port")?
        .parse()
        .map_err(|_| "invalid CDP port")?;
    let path = lines
        .next()
        .filter(|p| p.starts_with("/devtools/browser/"))
        .ok_or("invalid CDP path")?;
    let stream = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), OPERATION)
        .map_err(|e| e.to_string())?;
    stream.set_nodelay(true).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(OPERATION))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(OPERATION))
        .map_err(|e| e.to_string())?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let (socket, _) = tungstenite::client::client_with_config(
        format!("ws://127.0.0.1:{port}{path}"),
        stream,
        Some(config),
    )
    .map_err(|e| e.to_string())?;
    let mut cdp = Cdp {
        socket,
        next_id: 0,
        request_id: 0,
        session: String::new(),
        target: String::new(),
        setup: HashSet::new(),
        deadline,
        loaded: false,
        targets: HashSet::new(),
        input,
        native_sites,
    };
    let context = cdp.call("Target.createBrowserContext", json!({}), None)?;
    let context_id = string(&context, "browserContextId")?.to_owned();
    let target = cdp.call(
        "Target.createTarget",
        json!({"url": "about:blank", "browserContextId": context_id, "width":1280, "height":960}),
        None,
    )?;
    cdp.target = string(&target, "targetId")?.to_owned();
    let attached = cdp.call(
        "Target.attachToTarget",
        json!({"targetId": cdp.target, "flatten":true}),
        None,
    )?;
    cdp.session = string(&attached, "sessionId")?.to_owned();
    let session = cdp.session.clone();
    cdp.call("Page.enable", json!({}), Some(&session))?;
    cdp.call("Network.enable", json!({}), Some(&session))?;
    if cdp.native_sites.is_none() {
        cdp.call(
            "Network.setBypassServiceWorker",
            json!({"bypass":true}),
            Some(&session),
        )?;
        cdp.call(
            "Fetch.enable",
            json!({"patterns":[{"urlPattern":"*","requestStage":"Request"}]}),
            Some(&session),
        )?;
        cdp.call(
            "Target.setAutoAttach",
            json!({"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true}),
            Some(&session),
        )?;
    }
    cdp.call("Target.setDiscoverTargets", json!({"discover":true}), None)?;
    cdp.call(
        "Browser.setDownloadBehavior",
        json!({"behavior":"deny","browserContextId":context_id}),
        None,
    )?;
    cdp.call(
        "Emulation.setDeviceMetricsOverride",
        json!({"width":1280,"height":960,"deviceScaleFactor":1,"mobile":false}),
        Some(&session),
    )?;
    let version = cdp.call("Browser.getVersion", json!({}), None)?;
    let saved_agent = start
        .get("userAgent")
        .and_then(Value::as_str)
        .filter(|a| !a.is_empty());
    let agent = saved_agent
        .or_else(|| version["userAgent"].as_str())
        .ok_or("Chromium returned no user agent")?
        .to_owned();
    // Solver clearance cookies must travel with the user agent that earned them,
    // including when Chromium makes the network requests itself.
    if cdp.native_sites.is_none()
        || (saved_agent.is_some() && version["userAgent"].as_str() != Some(agent.as_str()))
    {
        cdp.call(
            "Network.setUserAgentOverride",
            json!({"userAgent":agent}),
            Some(&session),
        )?;
    }
    write(&json!({"type":"ready","user_agent":agent}))?;
    loop {
        let command = loop {
            if cdp.native_sites.is_none() {
                break cdp.input.recv().map_err(|_| "browser input closed")??;
            }
            // Native requests must keep moving while the user is between
            // inputs, even when no client is requesting preview frames.
            match cdp.input.try_recv() {
                Ok(command) => break command?,
                Err(TryRecvError::Disconnected) => return Err("browser input closed".into()),
                Err(TryRecvError::Empty) => {
                    cdp.deadline = Instant::now() + OPERATION;
                    cdp.receive()?;
                }
            }
        };
        cdp.deadline = Instant::now() + OPERATION;
        if let Some(cookies) = command.get("cookies") {
            cdp.call(
                "Storage.clearCookies",
                json!({"browserContextId":context_id}),
                None,
            )?;
            cdp.call(
                "Storage.setCookies",
                json!({"browserContextId":context_id,"cookies":cookies}),
                None,
            )?;
        }
        let result = match command["type"].as_str() {
            Some("open") => {
                let url = string(&command, "url")?;
                if !url.starts_with("https://") {
                    return Err("navigation requires HTTPS".to_owned());
                }
                cdp.loaded = false;
                let navigation = cdp.call("Page.navigate", json!({"url":url}), Some(&session))?;
                if let Some(error) = navigation.get("errorText").and_then(Value::as_str) {
                    return Err(error.to_owned());
                }
                while !cdp.loaded {
                    cdp.receive()?;
                }
                "null".to_owned()
            }
            Some("evaluate") => {
                let script = string(&command, "script")?;
                if script.len() > 64 << 10 {
                    return Err("script exceeds 64 KiB".to_owned());
                }
                let literal = serde_json::to_string(script).map_err(|e| e.to_string())?;
                let expression = format!(
                    "(async()=>{{const v=await (0,eval)({literal});const s=JSON.stringify(v);if(s===undefined)throw Error('script returned no JSON value');if(new TextEncoder().encode(s).length>{MAX_RESULT})throw Error('result exceeds 1 MiB');return s;}})()"
                );
                let result = match cdp.call(
                    "Runtime.evaluate",
                    json!({"expression":expression,"awaitPromise":true,"returnByValue":true}),
                    Some(&session),
                ) {
                    Ok(result) => result,
                    Err(error)
                        if error.contains("\"code\":-32000")
                            && (error.contains("Inspected target navigated or closed")
                                || error.contains("Execution context was destroyed")
                                || error.contains("Cannot find context with specified id")) =>
                    {
                        // A navigation invalidates this evaluation, not the browser.
                        // Let the caller decide whether its script is safe to retry.
                        write(
                            &json!({"type":"error","message":"browser page navigated during evaluation","recoverable":true}),
                        )?;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                if result.get("exceptionDetails").is_some() {
                    return Err(
                        "browser script threw an exception or exceeded its result limit".to_owned(),
                    );
                }
                let text = string(&result["result"], "value")?;
                if text.len() > MAX_RESULT {
                    return Err("result exceeds 1 MiB".to_owned());
                }
                text.to_owned()
            }
            Some("screenshot") => {
                let shot = cdp.call(
                    "Page.captureScreenshot",
                    json!({"format":"jpeg","quality":70,"captureBeyondViewport":false}),
                    Some(&session),
                )?;
                serde_json::to_string(&shot["data"]).map_err(|e| e.to_string())?
            }
            Some("click" | "scroll") => {
                let (x, y) = (
                    command["x"].as_u64().unwrap_or(0),
                    command["y"].as_u64().unwrap_or(0),
                );
                if x >= 1280 || y >= 960 {
                    return Err("input coordinates are outside the viewport".to_owned());
                }
                cdp.call(
                    "Input.dispatchMouseEvent",
                    json!({"type":"mouseMoved","x":x,"y":y}),
                    Some(&session),
                )?;
                if command["type"] == "click" {
                    for kind in ["mousePressed", "mouseReleased"] {
                        cdp.call(
                            "Input.dispatchMouseEvent",
                            json!({"type":kind,"x":x,"y":y,"button":"left","clickCount":1}),
                            Some(&session),
                        )?;
                    }
                } else {
                    cdp.call("Input.dispatchMouseEvent", json!({"type":"mouseWheel","x":x,"y":y,"deltaX":command["deltaX"],"deltaY":command["deltaY"]}), Some(&session))?;
                }
                "null".to_owned()
            }
            _ => return Err("unknown browser operation".to_owned()),
        };
        let cookies = cdp.call(
            "Storage.getCookies",
            json!({"browserContextId":context_id}),
            None,
        )?;
        write(&json!({"type":"result","json":result,"cookies":cookies["cookies"]}))?;
    }
}

struct BrowserProcess(Child);
impl Drop for BrowserProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Cdp<'a> {
    socket: WebSocket<TcpStream>,
    next_id: u64,
    request_id: u32,
    session: String,
    target: String,
    setup: HashSet<u64>,
    deadline: Instant,
    loaded: bool,
    targets: HashSet<String>,
    input: &'a Receiver<Result<Value, String>>,
    native_sites: Option<crate::sites::Sites>,
}

impl Cdp<'_> {
    fn send(&mut self, method: &str, params: Value, session: Option<&str>) -> Result<u64, String> {
        self.next_id += 1;
        let mut command = json!({"id":self.next_id,"method":method,"params":params});
        if let Some(session) = session {
            command["sessionId"] = json!(session);
        }
        self.socket
            .send(Message::Text(command.to_string().into()))
            .map_err(|e| e.to_string())?;
        Ok(self.next_id)
    }

    fn call(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, String> {
        let id = self.send(method, params, session)?;
        loop {
            let message = self.receive()?;
            if message["id"].as_u64() == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(format!("{method}: {error}"));
                }
                return Ok(message["result"].clone());
            }
        }
    }

    fn configure(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<(), String> {
        let id = self.send(method, params, session)?;
        self.setup.insert(id);
        Ok(())
    }

    fn receive(&mut self) -> Result<Value, String> {
        self.socket
            .get_mut()
            .set_read_timeout(Some(
                crate::worker::remaining(self.deadline)?.min(Duration::from_millis(50)),
            ))
            .map_err(|e| e.to_string())?;
        let message = match self.socket.read() {
            Ok(message) => message,
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Ok(Value::Null);
            }
            Err(error) => return Err(error.to_string()),
        };
        let Message::Text(text) = message else {
            return Ok(Value::Null);
        };
        let message: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if let Some(id) = message["id"].as_u64()
            && self.setup.remove(&id)
            && message.get("error").is_some()
        {
            return Err(format!("browser setup failed: {}", message["error"]));
        }
        let params = &message["params"];
        let session = message.get("sessionId").and_then(Value::as_str);
        match message["method"].as_str() {
            Some("Page.domContentEventFired") if session == Some(&self.session) => {
                self.loaded = true
            }
            Some("Page.javascriptDialogOpening") => self.configure(
                "Page.handleJavaScriptDialog",
                json!({"accept":false}),
                session,
            )?,
            Some("Target.targetCreated") => {
                let info = &params["targetInfo"];
                self.targets.insert(string(info, "targetId")?.to_owned());
                if self.targets.len() > 24 {
                    return Err("browser has too many active targets".to_owned());
                }
                if info["type"] == "page" && info["targetId"] != self.target {
                    self.configure(
                        "Target.closeTarget",
                        json!({"targetId":info["targetId"]}),
                        None,
                    )?;
                }
            }
            Some("Target.targetDestroyed") => {
                self.targets.remove(string(params, "targetId")?);
            }
            Some("Target.targetCrashed") if params["targetId"] == self.target => {
                return Err(format!(
                    "browser page crashed (status {}, code {})",
                    params["status"], params["errorCode"]
                ));
            }
            Some("Target.detachedFromTarget") if params["sessionId"] == self.session => {
                return Err("browser page disconnected before the operation completed".into());
            }
            Some("Target.attachedToTarget") if self.native_sites.is_none() => {
                let child = string(params, "sessionId")?;
                self.configure(
                    "Fetch.enable",
                    json!({"patterns":[{"urlPattern":"*","requestStage":"Request"}]}),
                    Some(child),
                )?;
                self.configure(
                    "Target.setAutoAttach",
                    json!({"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true}),
                    Some(child),
                )?;
                self.configure("Runtime.runIfWaitingForDebugger", json!({}), Some(child))?;
            }
            Some("Fetch.requestPaused") if self.native_sites.is_none() => {
                let request = &params["request"];
                self.request_id += 1;
                if self.request_id > crate::MAX_REQUESTS {
                    return Err("browser reached its request limit".to_owned());
                }
                let headers: Vec<_> = request["headers"]
                    .as_object()
                    .ok_or("invalid request headers")?
                    .iter()
                    .filter_map(|(k, v)| Some((k, v.as_str()?)))
                    .collect();
                // CDP omits uploaded file bytes: reject rather than silently
                // changing a multipart/binary body.
                if request["hasPostData"] == true && request.get("postData").is_none() {
                    return Err("browser upload body is unavailable".to_owned());
                }
                let body = request["postData"].as_str().unwrap_or("");
                if body.len() > MAX_RESULT {
                    return Err("browser request exceeds 1 MiB".to_owned());
                }
                write(
                    &json!({"type":"request","id":self.request_id,"url":request["url"],"method":request["method"],"headers":headers,"body":STANDARD.encode(body)}),
                )?;
                let response = self
                    .input
                    .recv_timeout(crate::worker::remaining(self.deadline)?)
                    .map_err(|_| "browser input closed or timed out")??;
                if response["type"] != "response"
                    || response["id"].as_u64() != Some(u64::from(self.request_id))
                {
                    return Err("unexpected network response".to_owned());
                }
                let request_id = &params["requestId"];
                if response.get("error").is_some() {
                    self.configure(
                        "Fetch.failRequest",
                        json!({"requestId":request_id,"errorReason":"BlockedByClient"}),
                        session,
                    )?;
                } else {
                    let headers = response["headers"]
                        .as_array()
                        .ok_or("missing response headers")?
                        .iter()
                        .filter(|h| {
                            !matches!(
                                h[0].as_str(),
                                Some("connection" | "transfer-encoding" | "content-length")
                            )
                        })
                        .map(|h| json!({"name":h[0],"value":h[1]}))
                        .collect::<Vec<_>>();
                    self.configure("Fetch.fulfillRequest", json!({"requestId":request_id,"responseCode":response["status"],"responseHeaders":headers,"body":response["body"]}), session)?;
                }
            }
            _ => {}
        }
        Ok(message)
    }
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("browser response is missing {key}"))
}

fn read(input: &mut impl BufRead) -> Result<Value, String> {
    let mut bytes = Vec::new();
    input
        .take(MAX_MESSAGE as u64 + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_MESSAGE || bytes.last() != Some(&b'\n') {
        return Err("browser input closed or exceeded its limit".to_owned());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn write(value: &Value) -> Result<(), String> {
    let text = value.to_string();
    if text.len() > MAX_MESSAGE {
        return Err("browser output exceeded its limit".to_owned());
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "{text}")
        .and_then(|()| out.flush())
        .map_err(|e| e.to_string())
}
