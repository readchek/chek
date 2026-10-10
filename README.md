<p align="center">
  <img src="apps/website/public/assets/logos/mark.svg" alt="Chek" width="80" height="80" />
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

> In development. A personal project, starting with iOS.

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

## Development

Install [Bun](https://bun.sh), then run the website locally:

```sh
bun install
bun run dev:website
```

Open [localhost:18357](http://127.0.0.1:18357/).

See the [development guide](docs/development.md) for API setup and code checks,
and the [website guide](docs/website.md) for builds, deployment, and assets.

## Project structure

```text
apps/
  api/        Effect backend
  website/    Landing page
docs/         Product, design, and development guides
```

The clients' planned scope is described in [product direction](docs/product.md).
App and website design decisions live in the [design guide](docs/design.md).
