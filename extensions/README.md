# Source extensions

Sources Chek reads series from, built as WebAssembly components against the
contract in [`crates/chek-sources/wit/source.wit`](../crates/chek-sources/wit/source.wit).
How the server runs them is in [Server design](../docs/server.md#extension-host).

| Path | Contents |
| --- | --- |
| `sdk` | `chek-extension`, the contract's bindings and request helpers |
| `index.schema.json` | The JSON Schema of a repository's `index.json` |

Sources live in their own repositories, outside this one, and Chek installs
them from extension repositories: an `index.json` and the components it
lists, served over HTTPS. A private repository can sit behind a bearer
token, basic auth or headers of its own, which Chek sends only to the
repository's own origin. [Extension repositories](../docs/extension-repositories.md)
is the format and how to host one privately, such as on Cloudflare Workers
or in a private GitHub repository.

## Publish a private repository

`chek ext` takes a source from a new folder to a repository online behind a
token, outside this repository too. It needs Rust with the `wasm32-wasip2`
target, and Node.js for Cloudflare's `wrangler`.

```sh
cargo install --git https://github.com/readchek/chek chek-cli
chek ext new my-source        # a source crate that builds as it stands
cd my-source
chek ext build                # dist/: the component and index.json
chek ext deploy cloudflare    # a Worker that serves dist/ to whoever has the token
```

The first deploy signs in to Cloudflare if needed, sets a random token and
prints it once with the address. Add both in Chek under Sources, Add
Repository. Deploying again publishes an update and keeps the token;
`--rotate-token` replaces it and `--public` serves without one. The Worker's
files are in `.chek/cloudflare`.

## Write a source

Start with `chek ext new my-source`, which makes a crate that builds as it
stands, and implement `Guest`:

- `manifest` names the extension's id in reverse-DNS form, its version, and
  every host it fetches from, including image hosts. Requests anywhere else
  fail. Add `Capability::Browser` for sites behind Cloudflare; the server's
  admin runs the solver that gets past the challenge.
- `search`, `series`, `chapters` and `pages` return the source's own ids,
  and images as URLs with any headers the site needs. Chek fetches and
  caches images itself.
- Each `Chapter` is one release. Fill `language` with the release's BCP 47
  tag (not the series' original language), `release_group` with who
  released it and `official` when that's the publisher, where the site
  says. List every release the site has: Chek groups them by `number` and
  shows one chapter with a choice of releases. Keep `version` for a content
  revision that changes when the pages do; a group's name doesn't belong
  there.
- `search` gets the content ratings the reader chose to see, from `Safe`
  through `Suggestive` and `Erotica` to `Pornographic`, never empty. Ask
  the site for only those where it filters by rating, so pages stay full,
  and set `content_rating` on each `SeriesItem` and `Series` where the site
  says. Chek drops search results rated outside the reader's choice and
  keeps unrated ones, so a source that can't tell leaves it `None`.
- `Get::new(url).cache(seconds).json()` fetches and parses a response, and
  lets Chek answer repeats from its cache. Cache listings; don't cache
  anything with a short-lived token in it.
- `cache` keeps values between calls and `cookies` holds the jar every
  request already sends. Both survive restarts.
- `transform_image` returns `Ok(None)` unless the source transforms images.
  The `image(url)` helper sets `transform: None`, keeping ordinary fetches.

Bump the crate's version for every change: installs update when the index
lists a new version.

## Books and other files

A source whose chapters are books, such as EPUBs or PDFs, also implements
`chek_extension::ChapterContent` and invokes
`chek_extension::export_chapter_content!(YourSource)` beside
`export_source!`:

- `formats` lists what its chapters come in: `ContentFormat::Images`,
  `Epub` or `Pdf`. It's read without network, like `manifest`, and
  `chek ext index` copies it into the index, which must list the same.
- `read(chapter_id)` returns a `Reading`: `Content::File(Asset { .. })` for
  a book, or `Content::Pages(..)` for a source that also has image
  chapters. An `Asset` has the URL and headers Chek downloads it with, its
  format, an `id` that's the same on every call and changes when the file
  does, and its size and a file name when the site gives them.
- Fill `rights` for openly licensed publications: the license (an SPDX id
  or its name), an HTTPS link to it, the credit it asks for, and an HTTPS
  link to the publication's page. Readers see them with the chapter.
- Keep `pages` for image chapters, since hosts without this export call it;
  for a book it can return `Error::Other` saying the chapter is a file.

Chek downloads files from the manifest's sites like images, up to 128 MiB
within five minutes, and refuses one that doesn't start as its format does,
such as a sign-in page served with a 200. Respect each site's terms for
automated downloads, and prefer the catalogs and mirrors it offers for them.

## Interactive pages

Declare `Capability::BrowserSession` to use `browser_session`. The admin must
set `CHEK_BROWSER_EXECUTABLE` to an absolute path to Chromium or Chrome's
headless shell. Chek runs its own Rust CDP worker; no Playwright, Bun, browser
download or separate automation service is needed. The supplied container
does not include Chromium; install it and its system dependencies in a custom
image if needed. Run it as a user that supports Chromium's sandbox.

`open(url)` waits for DOM content, `evaluate(script)` returns JSON text and
awaits promises, and `click(x, y)` and `scroll(x, y, dx, dy)` send browser input
at CSS coordinates in a 1280 by 960 viewport. Use an expression or an async
function invocation for evaluation. Call `close()` when finished. Opening
another page replaces the session; state lasts only for the current export.
Cookies are merged with the extension's jar after each successful operation.

Declare every script, frame and image host the page needs in `manifest.sites`.
The host relays HTTPS requests and redirects through that allow list, sharing
the call's 100-request budget. WebSockets and direct browser networking are
blocked. Downloads and extra pages are refused. GET, POST, HEAD and OPTIONS
are supported; file uploads are not. This path does not use the challenge
solver or reproduce Chromium's TLS fingerprint.

A session lasts at most 120 seconds and allows 128 operations, each up to
30 seconds. Scripts are capped at 64 KiB and JSON results at 1 MiB. Failure,
trap, close or return stops the worker and its browser processes. At most
four browser/image workers run at once; a busy host returns an error.
Chromium still has a substantial memory cost; the WASM memory cap does not
cover it. This worker is process-separated, not an OS security sandbox for
the extension host. Keep Chromium patched and preserve its own sandbox.

The server's `/verify` page is a prototype for manual challenge handoff. Sign in,
enter an installed source's id and an allowed HTTPS URL, complete verification
in its live browser view, then retry search. Its browser retains native TLS
through a public-host-only CONNECT proxy and is reused by that user's extension
calls for up to ten minutes. Closing the browser ends that live session; cookies
remain in the user's source namespace. The page currently supports pointer
clicks and search retries, not keyboard or audio CAPTCHA flows. This does not
guarantee that a site's challenge will accept the browser. FlareSolverr remains
a separate optional path for ordinary HTTP requests.

## Image transforms

Declare `Capability::ImageTransform` and set an image's `transform` string to
source-defined parameters. Chek fetches the image and calls `transform_image`
with the image descriptor and response. Return `Some(ImageData)` with a
raster content type and bytes, or `None` to keep the original. Errors fail
the image request. Clients keep using the same image endpoint.

The hook must depend only on its inputs and component code. HTTP, browser,
cache and cookies are unavailable during it. Include all parameters in
`image.transform`: the final-image cache key includes them, the URL/headers,
extension scope, version and component digest.

`images::rearrange` moves a grid of tiles and returns the image in the format
it came in: JPEG and WebP at quality 90, PNG lossless. Label the result with
the type its bytes start with, not a fixed `image/png`; Chek corrects a
mislabelled one.
`order[destination]` is the source tile index, counting across each row. The
order must be a permutation of the grid, with at most 256 tiles. Right/bottom
remainders stay unchanged. It accepts 8-bit JPEG, PNG or WebP, up to 8 MiB,
8 million pixels and four channels. One native operation is allowed per
hook; it runs in a separate libvips worker with a ten-second deadline. Hook
inputs and outputs are capped at 8 MiB and WASM keeps its 64 MiB memory cap.

## Contract updates

Chapter pagination is an optional export alongside the existing 0.6 source
world. Implement `chek_extension::ChapterPagination` and invoke
`chek_extension::export_chapter_pagination!(YourSource)` as well as
`export_source!`. Return `ChapterBatch { items, next, total }`; `next` is an
opaque cursor for the same series, or `None` when complete. A cursor must
work in a fresh instance and may be retried. Detect catalog changes and fail
rather than mixing incompatible batches. Keep the ordinary `chapters` export
for older hosts.

Each batch has the normal WASM and browser limits. Paginated batches allow
at most 500 chapters and 4 MiB of chapter text, with cursors up to 4 KiB.
The host assembles complete lists for library sync, allowing up to 20,000 chapters, 32 MiB of text, 256 batches and a
30-minute traversal budget checked between calls. It rejects duplicate IDs,
cursor cycles, changing totals and incomplete results before applying a sync.
An ordinary network failure retries the current batch once in a fresh instance;
traps and invalid data are not retried.
Browsing clients never see batches: `/api/v1/me/extensions/{id}/chapters`
answers a page at a time from the complete list, which the server reads the
same way and keeps for ten minutes. Sources without this export are read with
one `chapters` call.

When the site itself lists chapters a page at a time from either end, also
export chapter pages, so a client's page is one call to the site instead of
the whole list: implement `chek_extension::ChapterPages` and invoke
`chek_extension::export_chapter_pages!(YourSource)`. `page(series_id, order,
offset, limit, after)` returns `ChapterPage { items, offset, total }`: up to
`limit` chapters from `offset`, counted from the first chapter
(`OldestFirst`) or the newest (`NewestFirst`), and the series' total as the
site reports it. List a chapter's releases next to each other, as sites
that sort by number do: Chek groups them a page at a time. `after` is a chapter id to start just past; return
`Error::NotFound` when the site can't place it cheaply, and the host finds it
in the complete list instead. Pages allow at most 500 chapters, no more than
`limit`, and 4 MiB of text. Keep `chapters` (and batches, if the site has
them) for library sync, which still reads the complete list.

Chapter content, for sources of books and other files, is another optional
export; see [Books and other files](#books-and-other-files). Existing image
sources need no change: without it, every chapter is read as pages.

The current contract is `chek:source@0.6.0`. Components built against 0.5
must be rebuilt with the current SDK: `search` takes a third argument, the
`ContentRating`s the reader chose to see, and `SeriesItem` and `Series`
gain `content_rating`, set where the site rates its series. Components
built against 0.4 also need `language`, `release_group` and
`official` to every `Chapter`, and move a group name out of `version` if the
source put one there. Components built against 0.3 also need `transform: None` added to directly
constructed images and implement `transform_image`, returning `Ok(None)`
when unused. Bump the source version and regenerate the repository index.
Old components fail loading with a rebuild message; they are not converted
automatically. Android APKs and JVM JARs also require a source port to this
contract rather than repackaging as WASM.
