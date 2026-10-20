# Comix

WASM port of Keiyoushi's Comix 1.6.54 WebView flow. The exact upstream revision
and license are in [upstream/NOTICE](upstream/NOTICE).

Requires Chek's browser, browser-session and image-transform capabilities and a
configured Chromium executable. `CHEK_BROWSER_NATIVE=true` keeps its HTTP transport
behind Chek's restricted CONNECT proxy. The native browser is tried first,
sharing saved cookies, and accepted only when the catalog page loads. If that
fails, a configured `CHEK_BROWSER_SOLVER_URL` obtains fresh clearance through
FlareSolverr once.

The verified local configuration uses a headed Chromium with
`CHEK_BROWSER_ARGS='["--disable-blink-features=AutomationControlled"]'`,
native transport, with FlareSolverr optional. A cold start with an empty cookie
jar and no solver passed search and subsequent page loading locally.
Set `CHEK_BROWSER_EXECUTABLE` to the actual Chromium executable. A display is
required (or a virtual display on Linux).
Headless Chrome remained on the challenge page in local checks, including with
the same launch flag. Challenge acceptance is site-dependent.

Search and page loading sign API requests and decode responses in Rust. The
browser supplies the current build's codec material and request state; signatures are checked
against the site's request preparer before sending. Codec material is discovered
from the loaded module rather than hardcoded or copied from an old capture.
Details read the page's initial data. Chapter listing captures the site's decoded
responses while using native sort and pagination controls, with upstream's
2.5–5 second request pacing. The optional pagination export reads ten normal
20-item pages per call. Its cursor opens the preceding URL page in a fresh
session, then continues through native controls; it checks the reported total
throughout traversal. Image transforms handle the upstream XOR and
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
with native browser transport. A 4,754-entry catalog completed with no duplicate
IDs in 19 minutes 20 seconds using 24 batches. A separate solver-free run returned
all 340 entries for a smaller title. Encoded images remain unverified against a
live response.
Older hosts use the ordinary chapter export, which allows 20 pages per call
and rejects oversized lists before traversal. Hosts with optional chapter
pagination can assemble larger catalogs without extending a WASM call's
deadline. Solver browser failures can still interrupt a call; batch clients
can retry the same cursor instead of starting over.
Upstream preferences, incremental chapter refresh, alternate image-host retries,
and upstream's site-specific CAPTCHA solver are not implemented.
