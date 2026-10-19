<p align="center">
  <img src="https://raw.githubusercontent.com/readchek/chek.page/main/public/assets/logos/mark.svg" alt="Chek" width="80" height="80" />
</p>

<h1 align="center">Chek</h1>

<p align="center">An e-reader for your books and comics. Your choice of source.</p>

<p align="center">
  <a href="https://chek.page">Website</a>
  ·
  <a href="docs/README.md">Documentation</a>
  ·
  <a href="https://github.com/readchek/chek/issues">Issues</a>
</p>

> In development. A personal project.

## Overview

Chek is a ground-up redesign and direct successor to Chibi Manga. Manga and
manhwa are at its heart, alongside e-books and other comics.

The goal is a clean, reliable reader for local files and libraries served by
Komga, Kavita, or Chek's own server. The planned open-source Chek server will be
a standalone library server with an extension/import mechanism. You can use the
reader with your preferred source; Chek's server is optional.

The landing page is live at [chek.page](https://chek.page). The new clients and
server are still being built; supported formats and launch features are not yet
finalized.

## Repositories

This repository is the Chek server. It is in early development: it can scan a
folder, read EPUB, ComicInfo, PDF, MOBI, AZW3, FB2, plain text and Markdown
metadata, serve the files, comic pages and
covers, and sync each reader's position between devices. See [server design](docs/server.md) for
what is settled and what comes next.

- [chek.page](https://github.com/readchek/chek.page): the landing page.
- [chek-reader](https://github.com/readchek/chek-reader): the reading engine for Apple platforms.

The clients' planned scope is described in [product direction](docs/product.md).

## Running the server

Building needs [libvips](https://www.libvips.org) for cover thumbnails and
[libarchive](https://www.libarchive.org) for RAR and 7-Zip comics:
`apt install libvips-dev libarchive-dev` on Debian or Ubuntu,
`brew install vips libarchive` on macOS.

```sh
cargo run --bin chek
```

On its first start the server has no accounts and logs a one-time setup token.
Use it to create the admin, who gets a session token back:

```sh
curl -X POST localhost:8080/api/v1/setup \
  -H 'content-type: application/json' \
  -d '{"setupToken": "<from the log>", "name": "admin", "password": "<password>"}'
```

Send that token as `Authorization: Bearer <token>` on every other request:

```sh
curl -X POST localhost:8080/api/v1/libraries -H "Authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"name": "Comics", "path": "/path/to/comics"}'
curl localhost:8080/api/v1/libraries/1/publications -H "Authorization: Bearer $TOKEN"
```

Admins add readers with `POST /api/v1/users` and choose which libraries each
one may open. The API is described in [openapi.yaml](openapi.yaml).

Reader apps that speak OPDS 2, such as Thorium or KOReader, can browse and
download from `http://<server>:8080/opds/v2/catalog` with a Chek name and
password.

Settings come from environment variables: `CHEK_BIND` (address, default
`127.0.0.1:8080`), `CHEK_DATA_DIR` (database and covers, default `./data`) and
`CHEK_SCAN_WORKERS` (default one fewer than the cores). The server speaks plain
HTTP, so put it behind a reverse proxy with HTTPS before reaching it from
outside your home network, and list the proxy's address or range in
`CHEK_TRUSTED_PROXIES` (comma-separated, such as `10.0.0.0/8`) so sign-in
limits apply per client rather than to everyone behind the proxy. Behind
Cloudflare, also set `CHEK_CLIENT_IP_HEADER=CF-Connecting-IP`.
`CHEK_SETUP_TOKEN` (32 characters or more) replaces the logged setup token,
for setting up a server without reading its log. Admins can upload files
into a folder library with `PUT /api/v1/libraries/{id}/files/{path}`, which
needs the folder mounted read-write; `CHEK_MAX_UPLOAD_BYTES` caps each file
(default 2 GiB). Admins can also add a Komga server as a library, which
Chek syncs every `CHEK_SOURCE_SYNC_MINUTES` (default 60) and reads pages and
files from; see [Komga libraries](docs/server.md#komga-libraries). Covers and
pages fetched through source extensions are cached on disk up to
`CHEK_EXTENSION_IMAGE_CACHE_MB` (default 2048; 0 turns it off), and
`CHEK_BROWSER_SOLVER_URL` points extensions that need a browser, for sites
behind Cloudflare, at a FlareSolverr-compatible service. Interactive browser
sessions use `CHEK_BROWSER_EXECUTABLE`, an absolute Chromium path; see the
[extension guide](extensions/README.md#interactive-pages). People can
sign in through OpenID Connect providers such as Authentik, Authelia or
Google, set up with `CHEK_PUBLIC_URL`, `CHEK_OIDC_PROVIDERS` and
`CHEK_OIDC_<ID>_*`; see [Sign-in providers](docs/server.md#sign-in-providers).
`RUST_LOG` sets the log level, and `OTEL_EXPORTER_OTLP_ENDPOINT` sends logs and request traces to an
OpenTelemetry collector over OTLP/HTTP, following the standard `OTEL_*`
variables.

To run a demo instance with the reader test corpus loaded, on Dokploy or with
Docker Compose, see [deploying a demo instance](docs/deploy.md).

With Docker, read the setup token from `docker logs chek`:

```sh
docker build -t chek .
docker run -d --name chek -p 8080:8080 -v chek-data:/data -v /path/to/comics:/library:ro chek
```
