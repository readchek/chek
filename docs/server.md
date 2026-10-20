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
| `chek-komga` | Client for Komga's REST API, which Komga source libraries read |
| `chek-library` | Folder scanning, source syncing, change detection and scan jobs, with workers capped below the core count |
| `chek-sources` | Extension host for WebAssembly sources: the contract in `wit/source.wit` and the sandbox that runs it |
| `chek-server` | The `chek` binary: HTTP handlers, config and logging |

Source extensions live in [`extensions/`](../extensions/README.md), a
workspace of its own that builds to WebAssembly: the `chek-extension` SDK,
one crate per source, and `build.sh`, which writes a repository folder.
`cargo run -p chek-sources --example extension-index -- <folder>` writes
the `index.json` for any folder of components.

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
- Books are served as whole files for the app to lay out: EPUB, PDF, MOBI
  (`.mobi`, `.azw`, `.prc`), AZW3, FB2, plain text (`.txt`) and Markdown
  (`.md`, `.markdown`). Kindle books, like comics, take their format from
  their bytes: the MOBI header's version tells KF8 (AZW3) from older MOBI
  whatever the extension says. Text and Markdown files are accepted when
  their start has no NUL bytes and few control characters. FB2 inside a ZIP
  (`.fb2.zip`, `.fbz`) isn't read yet. DRM-protected Kindle books are
  served as they are; Chek doesn't remove DRM.
- Before serving a file or a page, the server compares the file's size and
  modification time with what the scan recorded. If they differ, it starts a
  rescan and answers `409 content_changed`, because stored offsets would no
  longer match the file.
- Covers are JPEG thumbnails that fit within 400 by 600 pixels, made by
  libvips at scan time and stored in the data folder under `covers/`, named
  by the file's content hash. Comic covers come from the first page, EPUB covers
  from the package's cover image and PDF covers from the first page. MOBI
  and AZW3 covers are the image record their EXTH header marks, and FB2
  covers the `<binary>` image its `<coverpage>` points at. Text and Markdown
  books have no cover.
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

## Komga libraries

- An admin can add a Komga server as a source library, with its address and
  one Komga account's API key, or email and password. Everyone granted the
  Chek library reads what that account sees, optionally narrowed to some of
  its Komga libraries. The account needs Komga's page streaming and file
  download roles. The settings are checked before the library is added and
  stored in the database, so use an account made for Chek. They are never
  returned by the API.
- A sync, which is how a source library scans, lists Komga's series and the
  books Komga has finished analyzing, and records each book with Komga's
  metadata: its title, number, summary, release date and authors, and its
  series' publisher, language and reading direction. Comics also get their
  page list, and every book gets a cover made from Komga's thumbnail. A book
  is read again only when it, its file or its series changes in Komga.
  Books gone from Komga are marked unavailable, like missing files.
- Source libraries sync at startup, on request, and every
  `CHEK_SOURCE_SYNC_MINUTES` (60 by default; 0 turns the timer off), since
  Komga can't tell Chek when something changes.
- Pages and files are fetched from Komga on each request and passed
  through, so Komga keeps the only copy. Pages are the original images.
  Komga sends a whole file even when asked for a range, so readers that open
  ZIPs with range requests download the whole file from these libraries.
- Progress, bookmarks and edits are kept in Chek against the publication,
  as for files, and survive syncs.
- The admin who adds a Komga library has their progress in it written back
  to the Komga account, unless they send `writeProgress: false`, so Komga's
  own apps resume where Chek left off. Only one Chek user is written back,
  because every reader of the library shares that one Komga account. EPUB
  positions go to Komga as Readium locators, which Komga checks against the
  book; comics and PDFs go as a page number, and reaching the last page or a
  `totalProgression` of 1 marks the book read. Writes happen after the save
  answers, one at a time so Komga ends at the newest position, and a write
  Komga refuses is logged, not retried.
- Komga's books are matched to publications by Komga's book id, so a book
  moved to another series in Komga keeps its publication.

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

## Sign-in providers

- People can sign in through OpenID Connect providers, such as Authentik,
  Authelia, Keycloak, Pocket ID or Google, as well as with a password. Each
  provider is set up with environment variables:
  - `CHEK_PUBLIC_URL`, the address people reach the server at. Register
    `<CHEK_PUBLIC_URL>/api/v1/auth/callback` as the redirect URI with each
    provider; the server logs it at startup.
  - `CHEK_OIDC_PROVIDERS`, comma-separated ids such as `authentik,google`.
  - Per provider, with the id upper-cased and `-` as `_`:
    `CHEK_OIDC_<ID>_ISSUER` (https, except on loopback for testing),
    `_CLIENT_ID`, `_CLIENT_SECRET` (left out for a public client), `_NAME`
    (button label), `_SCOPES` (default `openid profile email`),
    `_CREATE_USERS` (default `false`), `_ADMIN_GROUPS` (comma-separated) and
    `_GROUPS_CLAIM` (default `groups`).
- An app asks for the provider's page with a PKCE challenge of its own and a
  redirect URI, opens the page in a browser, and gets a one-time code at its
  redirect URI. It trades the code and its verifier for a session token. The
  token never appears in a URL, and a code caught by another app is useless
  without the verifier. Redirect URIs must start with the public URL or a
  prefix in `CHEK_OIDC_REDIRECTS` (`chek://` by default), so a sign-in link
  can't deliver someone's code to a stranger's site.
- Chek trades the provider's code with its client secret and PKCE, and checks
  the ID token's issuer, audience, expiry and nonce. The signature isn't
  checked: the token comes straight from the provider over TLS, which OpenID
  Connect Core 1.0 section 3.1.3.7 allows instead. Claims missing from the ID
  token are read from userinfo, since Authelia puts them only there.
- People are matched by the provider's subject id, never by email or name.
  Someone unknown gets a new account only with `_CREATE_USERS=true`, named
  from `preferred_username`, `name` or `email`, with a number added if taken.
  It's an admin if they're in one of `_ADMIN_GROUPS` and a reader otherwise;
  roles aren't changed on later sign-ins. Readers start with no libraries.
- Otherwise people link a provider from their account. The link is made when
  the app trades the code, not when the provider sends them back, so a
  stranger can't link a victim's provider account to their own Chek account
  by sending them a sign-in link.
- Accounts made by a provider have no password, so OPDS apps, which sign in
  with Basic auth, need one set first. Setting the first password doesn't
  ask for a current one. A provider can't be unlinked while it's the only way
  to sign in.
- Sign-ins in progress live in memory for 10 minutes, and codes for 5, so a
  restart cancels them. Provider discovery is fetched on first use, so a
  provider that's down doesn't stop the server.

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
  source file's name, such as `Microsoft Word - draft.docx`, is ignored.
  MOBI and AZW3 give their EXTH title, authors, publisher, description,
  language and date, reading only the header record. FB2 gives its
  `<title-info>`, including authors, translators, the series from
  `<sequence>`, and the publisher from `<publish-info>`; legacy encodings
  such as Windows-1251 are decoded by the XML declaration. Plain text reads a
  Project Gutenberg header (`Title:`, `Author:`, `Language:`), and Markdown
  reads YAML front matter (`title`, `author`, `series`, `date`, `language`,
  `description`) or else its first `#` heading. A file without metadata
  keeps its file name as its title.
- Lists go series by series, in title order. Within a series, publications
  with a number in their metadata come first in number order, then the rest
  by title. Numbers inside titles and file names compare by value, so
  `Episode 2` comes before `Episode 10` without any metadata.
- A file whose metadata names a series (ComicInfo `Series`, an EPUB 3
  series collection, calibre's `calibre:series`, an FB2 `<sequence>` or
  Markdown front matter's `series`) joins every file with that
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
  manifest, search, series, chapters, pages and an image transform hook. Series and
  chapter ids are the source's own, stored as remote ids; pages and covers
  come back as URLs with headers, which the server fetches itself.
- Each call runs in a fresh instance, so nothing in memory carries over
  between calls. WASI is linked with nothing in it: no files, environment,
  sockets or name lookups. The only ways out are the host's imports: `http`,
  `cache`, `cookies`, `browser`, `browser-session` and `images`.
- `cache` keeps values an extension sets, and a GET can ask for its 2xx
  response to be kept for a number of seconds and reused for the same URL
  and headers. Cached responses don't count toward a call's requests. Values
  and responses are capped at 4 MiB each and 30 days, and an extension's
  entries at 128 MiB together, past which the oldest are dropped.
- Every extension has a cookie jar that requests and image fetches send and
  fill, as a browser would, so a login or a challenge clearance carries over
  between calls. `cookies` reads and sets it directly, held to the
  extension's sites. A jar keeps up to 1,000 cookies, session cookies
  included.
- Caches and cookie jars are kept per user, extension id and sites in
  `extensions/state.db` in the data folder, so they survive restarts and
  updates. Existing shared state is not imported into a user's namespace. A
  component that reuses another's id with different sites starts empty, and
  an update that changes the sites does too. None is available while the
  manifest is read.
- For challenge solving, an admin points `CHEK_BROWSER_SOLVER_URL` at a
  FlareSolverr-compatible service, such as `http://flaresolverr:8191/v1`,
  and extensions whose manifest lists the `browser` capability may use it.
  When one of their requests or image fetches meets a challenge (a 403, 429
  or 503 with `cf-mitigated: challenge`, or Cloudflare's challenge page),
  the host has the solver load the URL, keeps the cookies it earned for that
  site and its user agent, and retries once. Later requests send both, so
  they pass without the solver until the clearance expires. A burst of
  challenged requests waits on one solve. Without a solver, or if solving
  fails, the challenge comes back as the site sent it.
- The `browser` interface loads a page in the solver directly and returns
  the rendered HTML, for pages that only work with scripts running. It
  counts as a request. The solver's browser follows the page wherever it
  leads, so the solver is the admin's to trust, like the server itself.
- Interactive pages use a separate `browser-session` capability. With
  `CHEK_BROWSER_EXECUTABLE` set to an absolute Chromium/headless-shell path,
  Chek starts a Rust CDP worker for one export call. It provides async script
  evaluation, click and scroll, with no Playwright or JavaScript runtime on
  the host. Browser HTTP requests are intercepted and relayed through the
  host's HTTPS/site checks, including frames and redirects; a non-forwarding
  proxy blocks direct networking. Cookies merge into the extension's shared
  jar after successful operations. This path does not invoke the solver.
  Sessions allow 128 operations, 30 seconds each and 120 seconds total; script
  input is capped at 64 KiB and JSON results at 1 MiB. Four browser/image
  workers may run together. A supervisor kills the process group and removes
  its temporary directory on close, failure or expiry. Chromium's memory is
  outside the WASM cap; process separation is not an OS security sandbox.
  Navigation during script evaluation returns an error without discarding the
  session. Extensions decide whether their script is safe to retry; the host
  never replays it automatically.
- `CHEK_BROWSER_NATIVE=true` opts browser-session calls into Chromium's native
  HTTP transport through the restricted CONNECT proxy described below. It defaults
  to false. Native sessions retain the 120-second call limit.
- `CHEK_BROWSER_ARGS` is an admin-supplied JSON array of Chromium launch arguments.
  It defaults to `["--headless=new"]`; use `[]` for a visible browser on a host
  with a display. These arguments also apply to verification browsers.
  Extensions cannot supply launch arguments. The host still adds its proxy,
  profile and network restriction arguments, and disables background timer and
  renderer throttling so a covered window can finish within the worker deadline.
- The `/verify` prototype lets a signed-in user open a verification browser
  for an installed source. Frames and clicks require that user's authentication;
  clients cannot send scripts or raw CDP commands. Subsequent extension calls
  reuse the browser until it closes or its ten-minute lifetime expires. Cookies
  persist, but browser processes and DOM state do not survive a server restart.
  Opening this mode is explicit: Chromium makes its own TLS connections through
  a loopback CONNECT proxy limited to declared hosts on port 443. The proxy
  resolves and pins public addresses, rejects private/local targets, limits
  active tunnels to 32 and traffic to 128 MiB per session. The browser still
  blocks direct DNS, QUIC and non-proxied WebRTC. Native requests run without
  CDP interception or paused child frames. This mode differs from the default HTTP
  relay: request counts are not inspected, response sizes are bounded by the
  session traffic budget, and the proxy cannot inspect encrypted traffic.
  The four-worker cap still applies.
  The worker continues processing native browser events between user inputs;
  loading does not depend on preview polling. Live preview is opt-in and requests
  one frame at a time, targeting a 250 ms interval. Each image is decoded before
  it replaces the canvas contents. Browser targets are capped at 24
  concurrently; closed frames and workers leave that count.
  Extension calls and user input are serialized. Verification is confirmed by
  retrying the source operation, never by a click or cookie alone.
- An image may carry source-defined transform parameters. With the
  `image-transform` capability, the host invokes `transform-image` after
  fetching and caches the result by extension scope, version, component
  digest, URL, headers and parameters. The hook has no network or mutable
  state access and must depend only on its inputs and code. Input/output
  bodies are capped at 8 MiB. `images.rearrange` moves tiles using libvips in
  a supervised ten-second worker: at most 256 tiles, 8 million pixels, four
  8-bit channels and one operation per hook. See the
  [extension guide](../extensions/README.md) for setup, limits and the required
  rebuild from contract 0.2 to 0.3.
- The manifest names up to 64 hosts the extension may reach, exactly or as
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
  its SHA-256 digest, sites, languages and capabilities. Indexes are capped at 1 MiB and
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
- A repository can be named by its index, by the folder holding
  `index.json` (`chek.page/extensions`), or as `github:owner/repo[@ref][/path]`,
  which reads the repository's raw files. It's stored as its index's URL.
- Private repositories take credentials: a bearer token, a username and
  password, or any header. They're stored in the database like a Komga
  library's settings and never returned. They're sent only to the index's
  origin, so components hosted elsewhere are fetched without them, and a
  signed request isn't followed to another origin. A private GitHub
  repository with a fine-grained, read-only token is the simplest host.

## Installing extensions

- Each user installs extensions for themselves from the repositories they
  can use. Installing downloads the component the index lists, at most
  32 MiB, checks its SHA-256 digest, and checks that its manifest has the
  index's id, version, sites and capabilities, so a repository can't show
  one set and ship another.
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
- Covers and pages fetched through extensions, for browsing and for
  extension libraries alike, are kept on disk in `extensions/images` and
  served from there the next time anyone asks for the same image from the
  same extension. `CHEK_EXTENSION_IMAGE_CACHE_MB` caps the folder, 2048 by
  default; past it, the least recently read images are deleted, and an
  image larger than a tenth of the cap isn't kept. 0 turns the cache off and
  empties the folder at startup.
- Browsing stores nothing; following a series into an extension library
  does.

## Extension libraries

- An admin makes an extension library, a kind of source library like
  Komga's, from an extension they installed. It runs on that admin's
  install, so readers granted the library read it without installing
  anything. If the admin uninstalls the extension, or its repository stops
  being usable, the library's pages and syncs fail with `source_unavailable`
  until it's installed again.
- Unlike a Komga library, which lists everything its server holds, an
  extension library holds only the series an admin follows into it. Sources
  can hold far more than anyone reads, and every request goes through the
  extension's limits.
- Extensions run in the server, so the server syncs these libraries itself
  rather than through `chek-library`'s scan; they sync on the same schedule
  as Komga libraries.
- Following a series stores it with the source's id as its remote id, and
  each chapter as a publication with format `remote`, backed by remote
  content keyed by the chapter's id. Chapters sort like files: by number,
  then by title. The series' cover becomes every chapter's cover thumbnail.
- A sync follows each series again. Chapters the source no longer lists are
  marked unavailable, not deleted, so progress survives; a chapter whose
  version changed has its page list dropped. Edits survive syncs, as for
  Komga's books.
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
