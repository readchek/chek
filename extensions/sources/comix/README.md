# Comix

WASM port of Keiyoushi's Comix 1.6.54 WebView flow. The exact upstream revision
and license are in [upstream/NOTICE](upstream/NOTICE).

Requires Chek's browser, browser-session and image-transform capabilities and a
configured Chromium executable. `CHEK_BROWSER_NATIVE=true` keeps its HTTP transport
behind Chek's restricted CONNECT proxy. When `CHEK_BROWSER_SOLVER_URL` is configured,
the source obtains fresh clearance through FlareSolverr before opening a session.

The verified local configuration uses a headed Chromium with
`CHEK_BROWSER_ARGS='["--disable-blink-features=AutomationControlled"]'`,
native transport and FlareSolverr. Set `CHEK_BROWSER_EXECUTABLE` to the actual
Chromium executable. A display is required (or a virtual display on Linux).
Headless Chrome remained on the challenge page in local checks, including with
the same launch flag. Challenge acceptance is site-dependent.

Search and page loading sign API requests and decode responses in Rust. The browser supplies
the current build's codec material and request state; signatures are checked
against the site's request preparer before sending. Codec material is discovered
from the loaded module rather than hardcoded or copied from an old capture.
Details read the page's initial data. Chapter listing captures the site's decoded
responses while using native sort and pagination controls, with upstream's
2.5–5 second request pacing. Image transforms handle the upstream XOR and
5×5 tile formats.
When a page uses an undeclared image host, the source selects a declared domain
from the site's alternate-host list, or the shared `softlifejournal.site` CDN
for the site's `/hi/` image paths. It preserves the node, path and query.

Current site builds hide API keys behind proxies. The port recovers those keys
from the JavaScript module loaded by the page rather than hardcoding build names.

The API codec uses two SHA-256 chaining states for request signatures and a
1,024-byte XOR table for responses marked `x-enc: 2`. Signatures cover the path
and sorted, unescaped parameters, excluding `_`; paths without parameters have
no trailing `?`. The response's first two decoded bytes select the table offset.
Material length, response size and encoding are checked before parsing.
This API format is separate from the image transforms. Direct HTTP transport
still failed in local verification; the browser remains responsible for
session state and fetching API responses.

This port is experimental. Live search, details, complete chapter lists, page
lists and unencoded images have passed through Chek's installed WASM component
with native browser transport and FlareSolverr. Encoded images remain unverified
against a live response.
Chapter listing currently allows 20 pages per call and rejects oversized lists
as soon as their pagination metadata is available. Larger catalogs are not
supported. Solver browser failures can also interrupt a call.
Upstream preferences, incremental chapter refresh, alternate image-host retries,
and the optional automatic CAPTCHA solver are not implemented.
