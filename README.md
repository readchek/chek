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
folder and list what it finds. See [server design](docs/server.md) for what is
settled and what comes next.

- [chek.page](https://github.com/readchek/chek.page): the landing page.
- [chek-reader](https://github.com/readchek/chek-reader): the reading engine for Apple platforms.

The clients' planned scope is described in [product direction](docs/product.md).

## Running the server

The server has no sign-in yet, so it listens on `127.0.0.1:8080` by default.
Don't expose it beyond a network you trust.

```sh
cargo run --bin chek
curl -X POST localhost:8080/api/v1/libraries \
  -H 'content-type: application/json' \
  -d '{"name": "Comics", "path": "/path/to/comics"}'
curl localhost:8080/api/v1/libraries/1/publications
```

Settings come from environment variables: `CHEK_BIND` (address), `CHEK_DATA_DIR`
(database folder, default `./data`) and `CHEK_SCAN_WORKERS` (default one fewer
than the cores). The API is described in [openapi.yaml](openapi.yaml).

With Docker:

```sh
docker build -t chek .
docker run -p 127.0.0.1:8080:8080 -v chek-data:/data -v /path/to/comics:/library:ro chek
```
