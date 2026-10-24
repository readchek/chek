# Comix

WASM source derived from Keiyoushi's Comix extension. Upstream revisions and
license are in [upstream/NOTICE](upstream/NOTICE).

Requires Chek's browser, browser-session and image-transform capabilities and a
configured Chromium executable. Native browser transport preserves Chromium's
network behavior through Chek's restricted proxy. Challenge acceptance depends
on the site and outbound IP; interactive verification may still be needed.

Search reads rendered catalog cards; details read the page's initial data.
Chapter batches read one native 20-item page per call, validating the requested
page, row count and reported total. Cursors preserve that total across calls.
Reader extraction visits lazy-loaded pages and returns only after every numbered
page has an image URL. These paths do not depend on the site's private API codec.

Image requests stay within the manifest's host allowlist. Image transforms handle
the upstream XOR and 5×5 tile formats. Older hosts without chapter pagination
allow at most 20 chapter pages per call and reject larger catalogs.

This source depends on the site's rendered structure and may need updates when
that structure changes. Upstream preferences and incremental refresh are not
implemented.
