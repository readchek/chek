<p align="center">
  <img src="https://raw.githubusercontent.com/readchek/chek.page/main/public/assets/logos/mark.svg" alt="Chek" width="80" height="80" />
</p>

<h1 align="center">Chek</h1>

<p align="center"><strong>Just one more chapter.</strong><br />A reader for your books and comics, wherever you keep them.</p>

<p align="center">
  <a href="https://apps.apple.com/us/app/chek-books-comics/id6761271144"><strong>Get it on the App Store</strong></a>
  ·
  <a href="https://chek.page">Website</a>
  ·
  <a href="https://docs.chek.page">Docs</a>
  ·
  <a href="https://github.com/readchek/chek/issues">Issues</a>
</p>

<p align="center">
  <a href="https://assets.chek.page/chek-promo-16x9.mp4"><img src="https://raw.githubusercontent.com/readchek/chek.page/main/public/assets/video/promo-16x9-poster.webp" alt="Watch the Chek film" height="300" /></a>
  &nbsp;
  <a href="https://assets.chek.page/chek-promo-9x16.mp4"><img src="https://raw.githubusercontent.com/readchek/chek.page/main/public/assets/video/promo-9x16-poster.webp" alt="Watch the Chek film, vertical cut" height="300" /></a>
</p>

<p align="center">
  ▶ Thirty seconds of Chek:
  <a href="https://assets.chek.page/chek-promo-16x9.mp4">widescreen</a>
  ·
  <a href="https://assets.chek.page/chek-promo-9x16.mp4">vertical</a>
</p>

## Chek 2.0 is out

Chek is free on the App Store for iPhone and iPad. It's the successor to
Chibi Manga, rebuilt from the ground up. Manga and manhwa are at its heart,
alongside e-books and every other kind of comic.

- **Your own files.** EPUB, PDF, CBZ, CBT, MOBI, AZW3, FictionBook, Markdown and
  plain text. No account needed, and everything works offline.
- **Your own server.** Read from Chek server, or from Komga and Kavita
  through it. Your place follows you between devices.
- **Quiet by design.** No ads, no tracking, no analytics.

## This repository

This is **Chek server**, an open-source, self-hosted library server written
in Rust. It's optional: the app reads your own files without it.

- Scans folders of books and comics, and reads their metadata: EPUB,
  ComicInfo, PDF, MOBI, AZW3, FB2, plain text and Markdown.
- Serves files, comic pages and covers, including RAR and 7-Zip comics.
- Syncs each reader's place, bookmarks and highlights between devices.
- Reads [Komga](docs/server.md#komga-libraries) and
  [Kavita](docs/server.md#kavita-libraries) libraries as its own.
- Runs [source extensions](extensions/README.md) built as WebAssembly components.
- Speaks [OPDS 2](docs/server.md#opds), so apps like Thorium and KOReader can
  browse it too.

The [Chek docs](https://docs.chek.page) cover installing and running the
server, its API, and writing source extensions. [Server design](docs/server.md)
covers what's settled and what comes next, and the API contract lives in
[openapi.yaml](openapi.yaml).

### Other repositories

| Repository | What it is |
| --- | --- |
| [app](https://github.com/readchek/app) | The Chek apps for iPhone, iPad and the web |
| [chek-reader](https://github.com/readchek/chek-reader) | The Swift reading engine behind the app |
| [chek.page](https://github.com/readchek/chek.page) | The website |

## Run the server

The [install guide](https://docs.chek.page/self-hosting/install/) walks
through a full setup. The short version:

### With Docker

CI publishes an image for amd64 and arm64:

```sh
docker run -d --name chek -p 8080:8080 \
  -v chek-data:/data -v /path/to/comics:/library:ro \
  ghcr.io/readchek/chek:latest
```

| Tag | What you get |
| --- | --- |
| `latest` | The newest release |
| `edge` | The newest commit on `main` |
| Any tag with `-browser`, such as `latest-browser` | The same, with Chromium for browser-backed extensions |

For a full setup with browser-backed extensions and an optional outbound
proxy, use the [server Compose deployment](deploy/server/README.md). To run
a demo instance with the reader test corpus, see
[deploying a demo instance](docs/deploy.md).

### From source

Install [libvips](https://www.libvips.org) for cover thumbnails and
[libarchive](https://www.libarchive.org) for RAR and 7-Zip comics, then run
the server:

```sh
apt install libvips-dev libarchive-dev   # Debian or Ubuntu
brew install vips libarchive             # macOS

cargo run --bin chek
```

### First steps

Open `http://<server>:8080/admin` in a browser. The admin panel asks for the
setup token from the log, creates your admin account, and helps you add a
library and the people who read it. It's bundled in the Docker image; from a
source build, set `CHEK_ADMIN_DIR` to a build of the
[web app](https://github.com/readchek/app/tree/main/web)'s
`bun run build:server`. If the server has an https address,
[app.chek.page/admin](https://app.chek.page/admin) manages it too.

To set up without a browser, use the API:

1. **Find the setup token.** On first start the server has no accounts and
   logs a one-time token. With Docker, read it from `docker logs chek`.
2. **Create the admin.** You get a session token back.

   ```sh
   curl -X POST localhost:8080/api/v1/setup \
     -H 'content-type: application/json' \
     -d '{"setupToken": "<from the log>", "name": "admin", "password": "<password>"}'
   ```

3. **Add a library.** Send the token as `Authorization: Bearer <token>` on
   every other request.

   ```sh
   curl -X POST localhost:8080/api/v1/libraries -H "Authorization: Bearer $TOKEN" \
     -H 'content-type: application/json' \
     -d '{"name": "Comics", "path": "/path/to/comics"}'
   curl localhost:8080/api/v1/libraries/1/publications -H "Authorization: Bearer $TOKEN"
   ```

4. **Add readers.** `POST /api/v1/users` creates an account and chooses which
   libraries it may open.

Then connect from the app, or point an OPDS 2 reader at
`http://<server>:8080/opds/v2/catalog` and sign in with a Chek name and password.

## Configuration

Everything is set with environment variables. All of them are optional.

**Basics**

| Variable | Default | What it does |
| --- | --- | --- |
| `CHEK_BIND` | `127.0.0.1:8080` | Address to listen on |
| `CHEK_DATA_DIR` | `./data` | Where the database and covers live |
| `CHEK_SCAN_WORKERS` | cores − 1 | How many files to scan at once |
| `CHEK_SETUP_TOKEN` | logged at start | Your own setup token, 32 characters or more, so you don't need the log |
| `CHEK_ADMIN_DIR` | none; the image sets it | The admin panel's build, served at `/admin`, with `/` sending visitors there |
| `CHEK_MAX_UPLOAD_BYTES` | 2 GiB | Largest file an admin can [upload](docs/server.md#uploads) with `PUT /api/v1/libraries/{id}/files/{path}`; the folder must be mounted read-write |

**Reaching it from outside**

The server speaks plain HTTP. Put it behind a reverse proxy with HTTPS
before opening it beyond your home network.

| Variable | Default | What it does |
| --- | --- | --- |
| `CHEK_TRUSTED_PROXIES` | none | Your proxy's addresses or ranges, comma-separated, such as `10.0.0.0/8`, so sign-in limits apply per client |
| `CHEK_CLIENT_IP_HEADER` | none | Set to `CF-Connecting-IP` behind Cloudflare |
| `CHEK_WEB_ORIGINS` | `https://app.chek.page` | Origins of the [web app](docs/server.md#the-web-app) allowed to call the API |

**Sign-in**

| Variable | What it does |
| --- | --- |
| `CHEK_PUBLIC_URL`, `CHEK_OIDC_PROVIDERS`, `CHEK_OIDC_<ID>_*` | Sign in through OpenID Connect providers such as Authentik, Authelia or Google; see [Sign-in providers](docs/server.md#sign-in-providers) |
| `CHEK_DEMO_USERS` | Shared demo accounts with a public password, which can't be changed and keep no reading places |

**Sources and extensions**

| Variable | Default | What it does |
| --- | --- | --- |
| `CHEK_SOURCE_SYNC_MINUTES` | `60` | How often Komga and Kavita libraries sync |
| `CHEK_EXTENSION_IMAGE_CACHE_MB` | `2048` | Disk cache for covers and pages from extensions; `0` turns it off |
| `CHEK_BROWSER_SOLVER_URL` | none | A FlareSolverr-compatible service, for sites behind Cloudflare |
| `CHEK_BROWSER_EXECUTABLE` | none | Absolute path to Chromium, for [interactive pages](extensions/README.md#interactive-pages) |
| `CHEK_OUTBOUND_PROXY_URL` | none | An HTTP CONNECT proxy for extension traffic; see [Extension host](docs/server.md#extension-host) |

**Logs**

| Variable | What it does |
| --- | --- |
| `RUST_LOG` | Log level |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Sends logs and traces to an OpenTelemetry collector over OTLP/HTTP; the standard `OTEL_*` variables apply |

## Releases

A release is a tag on `main` named by its version, such as `0.3.0`, and CI
publishes its image. Run the Release workflow in the Actions tab with the
version, or tag it yourself:

```sh
git tag 0.3.0 && git push origin 0.3.0
```

To build the image locally, run `docker build -t chek .`.

## License

Chek server is licensed under the [GNU Affero General Public License v3.0](LICENSE).
If you run a changed copy as a service for other people, you share your
changes with them under the same license.

The extension SDK in [`extensions`](extensions) and the contract in
[`crates/chek-sources/wit`](crates/chek-sources/wit) are [MIT](extensions/LICENSE),
so source extensions can use any license.
