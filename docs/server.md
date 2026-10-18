# Server design

Settled with Arya on 2026-10-18. The full reasoning, options and spike results
are in the planning doc; this page keeps the decisions next to the code.

## Decisions

- **First milestone.** One household points the server at folders of books and
  comics, browses them, reads them in the Chek app and resumes on another
  device. The public release adds uploads, Komga and Kavita import, extension
  sources, more formats, metadata editing, sign-in providers, bookmarks and
  annotations, and OPDS 2.
- **Stack.** Rust with axum on tokio, SQLite for storage and libvips for images.
  A latency spike on the real reader corpus measured a comic page from disk at
  6.1 ms p50 and 18 ms p99 under 16 concurrent clients.
- **Data model.** Library (folder or source), series, publication, content (a
  file or a remote item), page, user and progress. Remote sources are modelled
  from day one. Ids are server-made and never derived from paths or titles.
  Missing files are marked unavailable, not deleted.
- **API.** A hand-written [`openapi.yaml`](../openapi.yaml) is the contract. The
  app generates its client from a tagged release. OPDS 2 arrives at public release.
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
| `chek-sources` | Extension host for WebAssembly sources (empty until extensions land) |
| `chek-server` | The `chek` binary: HTTP handlers, config and logging |

## Serving

- Comic pages are read from the offset stored in the page index, so a request
  never reopens the archive's directory. The page's original bytes are sent
  unchanged.
- Before serving a file or a page, the server compares the file's size and
  modification time with what the scan recorded. If they differ, it starts a
  rescan and answers `409 content_changed`, because stored offsets would no
  longer match the file.
- Covers are JPEG thumbnails that fit within 400 by 600 pixels, made by
  libvips at scan time and stored in the data folder under `covers/`, named
  by the file's content hash. CBZ covers come from the first page, EPUB covers
  from the package's cover image and PDF covers from the first page.
- Every folder library is rescanned at startup to catch changes made while
  the server was down. Unchanged files are skipped by size and modification
  time.

## Design rules from the spike

- Page requests read from the page index stored at scan time instead of
  reopening an archive's directory.
- Scans use fewer threads than the machine has cores; a scan on every core
  doubled page p99.
- Every listener sets `TCP_NODELAY`; without it pass-through p99 rose from
  about 50 ms to 84 ms.
