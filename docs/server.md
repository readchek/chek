# Server design

Settled with Arya on 2026-10-18. The full reasoning, options and spike results
are in the planning doc; this page keeps the decisions next to the code.

## Decisions

- **First milestone.** One household points the server at folders of books and
  comics, browses them, reads them in the Chek app and resumes on another
  device. The public release adds uploads, Komga and Kavita import, extension
  sources, more formats, metadata editing, sign-in providers, bookmarks and
  annotations.
- **Stack.** Rust with axum on tokio, SQLite for storage and libvips for images.
  A latency spike on the real reader corpus measured a comic page from disk at
  6.1 ms p50 and 18 ms p99 under 16 concurrent clients.
- **Data model.** Library (folder or source), series, publication, content (a
  file or a remote item), page, user and progress. Remote sources are modelled
  from day one. Ids are server-made and never derived from paths or titles.
  Missing files are marked unavailable, not deleted.
- **API.** A hand-written [`openapi.yaml`](../openapi.yaml) is the contract. The
  app generates its client from a tagged release. An OPDS 2 catalog under
  `/opds/v2` serves other reader apps.
- **Extensions.** WebAssembly components installed per user from extension
  repositories, run in a sandbox with no file or network access except the
  sites they declare. The admin keeps an allow list that ships with the
  official repository and one switch that lets users add any repository.
  Tachiyomi or Mihon compatibility is not part of the core.

## Code layout

| Crate | Responsibility |
| --- | --- |
| `chek-catalog` | Data model, SQLite migrations and queries |
| `chek-formats` | Format detection, metadata, page indexes and covers for each format |
| `chek-library` | Folder scanning, change detection and scan jobs, with workers capped below the core count |
| `chek-sources` | Extension host for WebAssembly sources: the contract in `wit/source.wit` and the sandbox that runs it |
| `chek-server` | The `chek` binary: HTTP handlers, config and logging |

## Serving

- Comic pages are read from the offset stored in the page index, so a request
  never reopens the archive's directory. The page's original bytes are sent
  unchanged.
- CBZ (ZIP) and CBT (tar) comics are indexed this way. Tar stores entries
  uncompressed after their headers, so CBT pages are served like stored ZIP
  entries; ustar, GNU long-name and pax headers are read.
- CBR (RAR 4 and 5) and CB7 (7-Zip) comics are read through libarchive.
  Their entries are compressed, often as one solid stream, so they have no
  offsets to serve from: a page request walks the archive's headers to the
  page and decompresses it. In a solid archive that means decompressing
  every earlier page too, so late pages of large solid archives are slower.
- Comic archives are often saved under the wrong comic extension, such as a
  ZIP named `.cbr`, so a file with any comic extension takes its format from
  its bytes.
- Before serving a file or a page, the server compares the file's size and
  modification time with what the scan recorded. If they differ, it starts a
  rescan and answers `409 content_changed`, because stored offsets would no
  longer match the file.
- Covers are JPEG thumbnails that fit within 400 by 600 pixels, made by
  libvips at scan time and stored in the data folder under `covers/`, named
  by the file's content hash. Comic covers come from the first page, EPUB covers
  from the package's cover image and PDF covers from the first page.
- Every folder library is rescanned at startup to catch changes made while
  the server was down. Unchanged files are skipped by size and modification
  time.

## Uploads

- Admins upload a file into a folder library by path. The body streams to a
  hidden temporary file beside its destination, so a scan never reads a
  partial file. The file must match its extension, and then it is moved into
  place and recorded the way a scan records it.
- Uploads never replace a file; a taken path answers `409 file_exists`.
  Paths must be plain names that stay inside the library, and folders that
  resolve outside it through a link are refused.
- A scan marks every file it didn't list at its start as unavailable, so
  recording an upload waits for a running scan of the same library. The
  request waits up to 20 seconds and then answers 202 while recording
  carries on.
- `CHEK_MAX_UPLOAD_BYTES` caps each file, 2 GiB by default. Uploads need the
  library folder mounted read-write.

## Accounts

- Two roles: admins manage libraries and users and see every library;
  readers see only the libraries an admin granted them. Libraries and
  publications outside a reader's grants answer as not found.
- The first admin is created with a one-time setup token that the server logs
  when it has no accounts, so only someone with access to the log can claim it.
  Deployments that can't read the log set their own in `CHEK_SETUP_TOKEN`,
  which must be at least 32 characters and isn't logged.
- Sign-in returns a random 256-bit session token, sent as a bearer token. The
  database stores only its BLAKE3 hash. Sessions last 90 days after their
  last use.
- Passwords are hashed with Argon2id. Sign-in checks a dummy hash for unknown
  names so timing doesn't reveal which names exist. Changing a password signs
  out the user's other sessions.
- The server keeps at least one admin; demoting or deleting the last one fails.
- People can list their signed-in devices and sign any one out. Sessions have
  a plain id for this, so the token hash never leaves the server.
- After 10 wrong passwords from one address within 15 minutes, password
  checks from it answer `429 too_many_attempts` until the oldest failure is
  15 minutes old. Limits are per address, not per account, so a stranger
  can't lock someone out. Session tokens and already accepted Basic
  credentials aren't affected. Behind a reverse proxy, list its address or
  CIDR range in `CHEK_TRUSTED_PROXIES` so the client's address is read from
  `X-Forwarded-For`; otherwise every client shares the proxy's address.
  Behind Cloudflare, set `CHEK_CLIENT_IP_HEADER=CF-Connecting-IP` as well,
  since a tunnel or proxy in between may replace `X-Forwarded-For`. The
  header is read only from trusted proxies, and only helps if every request
  reaches those proxies through Cloudflare; a proxy reachable directly lets
  clients send their own value.
- Requests can also send HTTP Basic credentials, because OPDS reader apps
  can't hold session tokens. A passing check is remembered in memory, keyed
  by the password and the stored hash, so later requests skip Argon2; a
  password change changes the hash and forgets it. OPDS routes answer 401
  with both a Basic challenge and an Authentication for OPDS document, so
  apps prompt for a name and password; KOReader follows the first and
  Thorium needs the second.

## OPDS

- `/opds/v2/catalog` lists "Continue reading", "Recently added" and the
  libraries the user may open. Each library is a navigation feed of its
  series, led by "All publications" and "Recently added" links, and each
  series is a feed of its publications in reading order, 50 per page.
  Publications link back to their series feed. The catalog and each library
  carry a templated `search` link. Files and covers are served
  under `/opds/v2` too, so apps send the credentials they used for the catalog.
- Publication metadata follows the Readium Web Publication model, so roles
  such as penciler or translator keep their own keys.

## Metadata

- Scans read metadata from the file itself: the EPUB package document
  (EPUB 2 and 3, including calibre's series index) and `ComicInfo.xml` in
  comic archives. PDFs give the title, authors and subject from their document
  information, read through libvips' poppler loader; a title that's only a
  source file's name, such as `Microsoft Word - draft.docx`, is ignored. A
  file without metadata keeps its file name as its title.
- Lists go series by series, in title order. Within a series, publications
  with a number in their metadata come first in number order, then the rest
  by title. Numbers inside titles and file names compare by value, so
  `Episode 2` comes before `Episode 10` without any metadata.
- A file whose metadata names a series (ComicInfo `Series`, an EPUB 3
  series collection or calibre's `calibre:series`) joins every file with that
  series name in the library, matched ignoring case, wherever the files sit.
  Other files are grouped by the folder they're in. Retagging a file moves
  it to another series, which changes its series id; reading progress is
  kept per publication, so it isn't lost. Series left empty are removed at
  the end of a scan.
- The API lists a library's series in title order with a publication count
  and the publication whose cover stands for the series, and lists each
  series' publications on their own.

## Search

- Search uses an SQLite FTS5 index over each publication's title, series
  name, contributors and publisher. Scans rewrite a publication's entry
  whenever they record its file, and the migration that adds the index fills
  it from what's already stored.
- Every word of a query must match, ignoring case and accents, and each word
  also matches the start of a longer word so results can follow typing.
  Words are quoted before reaching FTS5, so its query syntax is never
  interpreted. Title matches rank above series, contributor and publisher
  matches, and equal matches stay in reading order.
- Results are paged by position, because relevance has no stable key to page
  by. Only libraries the user can read are searched.
- The tokenizer splits on spaces and punctuation, so a run of Chinese or
  Japanese text without spaces is one word and matches only from its start.
- Metadata that can't be parsed is ignored rather than failing the file,
  since pages and the file are still readable. Text fields are cut at 1,000
  characters and summaries at 20,000.
- Changing what a scan reads forces a rescan of every file through a
  migration that clears the stored file stamps.
- Admins can edit a publication's title, number, description, publisher,
  language, publication date, reading direction and contributors. Edits are
  stored apart from what the file says and written over it each time the
  file is read, so rescans keep them, and the API lists which fields were
  edited. Undoing edits drops them all and reads the file again. Files are
  never rewritten. Series can't be edited yet.

## Reading progress

- Each user has one position per publication, stored as a Readium Locator
  so readers built on Readium can use it as is. The server checks the fields
  it relies on and keeps the rest as sent, up to 16 KiB.
- Every save names the revision it builds on. If another device saved in
  between, the save is refused with `409 progress_conflict` and that
  device's position, and the app asks the reader which one to keep. The
  server never picks the position furthest along, because going back to
  reread is as deliberate as moving forward.
- A position records the device name of the session that saved it, so the
  app can say where the other position came from.
- Continue reading lists publications with a saved position that hasn't
  reached the end (a `totalProgression` below 1, or none), most recently
  saved first. Recently added lists publications by id, newest first, since
  ids grow as scans find new files; a file changed in place keeps its place.
  Both leave out unavailable publications and libraries the user can't read.

## Bookmarks and highlights

- Each user keeps their own bookmarks and highlights per publication, each a
  Readium Locator with an optional color and note. A highlight's locator
  carries the highlighted text in `text.highlight`.
- Changes are last save wins, since two devices editing the same note at
  once is rare and neither position nor text is at stake. Clients fetch the
  whole list for a publication, so deletions on one device show on the next
  fetch elsewhere.
- A publication holds up to 5,000 annotations per user and a note up to
  10,000 characters, so one response always carries the whole list.

## Extension host

- An extension is a WebAssembly component built against the `source` world
  in [`wit/source.wit`](../crates/chek-sources/wit/source.wit). It exports a
  manifest and four calls: search, series, chapters and pages. Series and
  chapter ids are the source's own, stored as remote ids; pages and covers
  come back as URLs with headers, which the server fetches itself.
- Each call runs in a fresh instance, so extensions keep no state between
  calls. WASI is linked with nothing in it: no files, environment, sockets or
  name lookups. The only way out is the host's `http` import.
- The manifest names the hosts the extension may reach, exactly or as
  `*.example.org` for any subdomain. Requests, redirects and image fetches
  go over HTTPS to those hosts only. IP addresses and local names such as
  `localhost` are refused as sites. The manifest itself is read with no
  network.
- A call may use 64 MiB of memory, about 5 billion instructions (half a
  second of a tight loop) and 100 requests. Response bodies are capped at
  16 MiB and images at 32 MiB, and each request times out after 30 seconds.
  Going over a limit fails that call, not the server.
- A site name can still resolve to a private address; the allow list of
  repositories is the guard until the host checks resolved addresses.

## Extension repositories

- A repository is an `index.json` served over HTTPS that lists its
  extensions: id, name, version, the component file relative to the index,
  its SHA-256 digest, sites and languages. Indexes are capped at 1 MiB and
  1,000 extensions.
- Repositories without an owner are the admin's allow list, which everyone
  sees. It starts with the official repository,
  `https://chek.page/extensions/index.json`, which an admin can remove.
- The `allowAnyRepository` setting, off by default, lets other users add
  repositories of their own, which only they see. Turning it off again keeps
  those repositories but marks them unusable, so nothing is installed or run
  from them.
- The server fetches an index when the repository is added, and refuses one
  it can't read. A refresh that fails records why and keeps the last good
  index.

## Installing extensions

- Each user installs extensions for themselves from the repositories they
  can use. Installing downloads the component the index lists, at most
  32 MiB, checks its SHA-256 digest, and checks that its manifest has the
  index's id, version and sites, so a repository can't show one set of sites
  and ship another.
- Components are stored in the data folder as `extensions/<sha256>.wasm`,
  shared by everyone on the same version, and deleted when no install uses
  them. Compiled sources stay in memory once loaded.
- Installing again updates to the version the index lists now; the API
  reports when the index lists a different version. Removing a repository
  removes every install from it.

## Browsing sources

- The API can search a source through an extension the user installed, and
  read a series, its chapters and a chapter's pages. Each call runs the
  extension on a blocking thread.
- Covers and pages come back as links to `/api/v1/me/extensions/{id}/image`,
  which carry the image request the extension made. The server fetches the
  image, held to the extension's sites, and sends its bytes unchanged with
  `nosniff`, so the app never contacts a source's sites itself.
- Browsing stores nothing; following a series into a source library does.

## Source libraries

- An admin makes a source library from an extension they installed. It runs
  on that admin's install, so readers granted the library read it without
  installing anything. If the admin uninstalls the extension, or its
  repository stops being usable, the library's pages and scans fail with
  `source_unavailable` until it's installed again.
- Following a series stores it with the source's id as its remote id, and
  each chapter as a publication with format `remote`, backed by remote
  content keyed by the chapter's id. Chapters sort like files: by number,
  then by title. The series' cover becomes every chapter's cover thumbnail.
- A scan of a source library follows each series again. Chapters the source
  no longer lists are marked unavailable, not deleted, so progress
  survives; a chapter whose version changed has its page list dropped.
- A chapter's page list is asked of the source the first time it's read
  and stored as the image requests the extension returned. Pages are
  fetched through the source on every read, with no cache on the server
  yet. Since sources often hand out page links that expire, a failed fetch
  asks for the page list again and retries once.

## Observability

- Every request is logged with its method, route, path, status, duration,
  client address and user agent, inside a span named after the route (such
  as `GET /opds/v2/libraries/{id}`) so traces group by endpoint.
- With `OTEL_EXPORTER_OTLP_ENDPOINT` set, logs and spans also go to an
  OpenTelemetry collector over OTLP/HTTP, configured by the standard `OTEL_*`
  variables. The exporter has no TLS, since the collector is expected on the
  same private network. Queued data is flushed on shutdown.

## Design rules from the spike

- Page requests read from the page index stored at scan time instead of
  reopening an archive's directory.
- Scans use fewer threads than the machine has cores; a scan on every core
  doubled page p99.
- Every listener sets `TCP_NODELAY`; without it pass-through p99 rose from
  about 50 ms to 84 ms.
