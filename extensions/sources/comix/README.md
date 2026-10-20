# Comix

WASM port of Keiyoushi's Comix 1.6.54 WebView flow. The exact upstream revision
and license are in [upstream/NOTICE](upstream/NOTICE).

Requires Chek's browser-session and image-transform capabilities and a configured
Chromium executable. `CHEK_BROWSER_NATIVE=true` keeps Chromium's HTTP transport
behind Chek's restricted CONNECT proxy.

Search and details read the page's initial data. Chapter listing discovers the
site's own method and calls it after native input; page loading uses the shipped
upstream JavaScript. Image transforms handle the upstream XOR and 5×5 tile formats.

This port is experimental. Live search and details have not yet passed in Chek's
browser worker. Compilation is not evidence that verification succeeds.
Chapter listing currently allows 20 pages per call and rejects incomplete lists.
Upstream preferences, incremental chapter refresh, alternate image-host retries,
and the optional automatic CAPTCHA solver are not implemented.
