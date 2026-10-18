# Komga

Effect-only Chek extension for Komga. Discovered automatically when this repository's
`extensions` directory is configured. Add a Komga source in `/inspect#sources`.

Set `url` to the server URL, including its reverse-proxy base path if any. Authenticate
with `apiKey`, or `email` and `password`. Optional `libraryId` restricts browsing,
direct lookups, and imports to one library. Grant the server's HTTP origin.
Credentials stay on the server; page and cover URLs use Chek's private asset proxy.

Supports series browsing/search, metadata, book lists, image pages, and original
EPUB/PDF/CBZ/CBR imports. Komga's streaming/download permissions still apply.
Image-compatible EPUBs can be paged; reflowable EPUBs must be imported for a document
reader. Books Komga cannot analyze produce import warnings. Full scans re-enumerate
the library, with unchanged originals deduplicated by Chek.

Browse sorts: `title`, `updated`, `added`. Browse/search filters: `library`,
`collection`, `language`, `genre`, `tag`, `publisher`, `status`. Values use Komga IDs
or metadata values; status uses Komga's enum (`ONGOING`, `ENDED`, `HIATUS`,
`ABANDONED`). Continue with the returned cursor and the same page size.

Uses Komga's [REST API](https://komga.org/docs/openapi/komga-api/), including the
POST list endpoints. Verified against Komga 1.29.1 with the local reader corpus.
