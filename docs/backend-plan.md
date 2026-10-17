# Chek backend launch plan

Proposal prepared on 2026-10-17. This is the implementation plan for review, not
an inventory of shipped features. Every stage below contributes to the same
launch; format coverage, durability, and performance are launch requirements.

## Product and launch contract

Build a standalone, self-hosted library server for books, manga, manhwa, comics,
and reading documents. A person should be able to point it at a messy, substantial
existing library, browse it, read it, download it, and resume on another device
without reorganizing everything first. Clients remain usable with local files,
Komga, or Kavita independently of this server.

Launch includes broad ingestion, accurate metadata, search, collections, reading
delivery, offline acquisition, progress/bookmark/annotation sync, multi-user
library permissions, import jobs, operational visibility, and recoverable backups.
The inspection webpage below makes these capabilities usable and inspectable
while native client work proceeds separately.

“No compromises” needs measurable meaning: never sacrifice original files or
acknowledged user state for speed; keep processing off interactive request paths;
and do not advertise support based only on recognizing an extension. Zero latency,
perfect conversion, and uninterrupted service on failed hardware cannot be promised.
The gates below make failures visible and bound the work the server accepts.

## Verified starting point

- This repository has Bun workspaces, an Effect HTTP API, configuration, logging,
  and OTLP telemetry. The current routes are `/` and `/health`; there is no
  implemented library domain, database, ingestion pipeline, or reading API.
- The website is a separate static landing page. Keep the backend inspection UI
  separate from that site's marketing and build/deployment path.
- Existing product decisions establish an independent server, source choice,
  and an owned reader engine. Historical Suwayomi-to-Komga translation is not
  the architecture to copy.
- Reader [PR #8](https://github.com/readchek/chek-reader/pull/8) was inspected at
  head `9fe9b838e56c92f52e534fafcb8daff1b32a0bab`, while open. It adds remote EPUB
  reading through HTTP byte ranges. Source inspection is not a runtime verification.
- Its `ReaderFormat` declares EPUB, PDF, CBZ, CBT, TXT, Markdown, MOBI, AZW3,
  FB2, zipped FB2, CBR, and CB7. The inspected comics and MOBI modules only
  declare formats. Do not treat the README's broad list as working coverage.

## Launch format matrix

For each row, acceptance means identifying the actual bytes, extracting usable
metadata, providing the stated reading representation, downloading the original,
and reporting actionable failures. Native Apple rendering is a separate gate:
backend acceptance does not establish reader-package support.

| Family               | Required launch variants                                                                                                                                                                                                                                   | Backend reading representation                                                                                                         |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| EPUB                 | EPUB 2/3; reflowable and fixed layout; NCX and EPUB navigation; multiple rootfiles with explicit rendition selection; RTL, vertical text, ruby, SVG, MathML, embedded fonts, IDPF/Adobe font obfuscation; ZIP64 and stored/deflated entries; `.kepub.epub` | Original byte ranges plus indexed manifest/resources; preserve styles, semantics, navigation, and layout hints                         |
| ZIP comics           | CBZ and image ZIP; nested folders; Unicode and legacy entry names; mixed images, spreads, extras, ComicInfo.xml                                                                                                                                            | Ordered pages, original archive, per-page resources, prepared previews                                                                 |
| Other comic archives | CBR/RAR4/RAR5, CB7/7z including solid archives, CBT/TAR and common compressed TAR containers; split volumes when the complete set is supplied                                                                                                              | Prepare random-access pages during ingestion; never repeatedly unpack a solid archive during page turns                                |
| Image libraries      | Loose image folders, nested chapter folders, webtoon/long-strip collections; JPEG, PNG, WebP, AVIF, GIF, TIFF, BMP; HEIF/HEIC when packaged decoder qualification passes                                                                                   | Original images plus compatible derivatives; page dimensions, orientation, animation/multipage policy, and strip segmentation metadata |
| PDF                  | Text and scanned documents, linearized/nonlinearized, mixed page sizes and rotation, outlines, labels, embedded fonts, password-protected files with supplied credentials                                                                                  | Original byte ranges, metadata/outline, page previews and text where present; preserve document geometry                               |
| Kindle books         | DRM-free MOBI/PRC, KF8/AZW3, legacy/dual MOBI; inspect `.azw` bytes rather than trusting the suffix                                                                                                                                                        | Preserve original; prepare EPUB when the client cannot open the original; record conversion fidelity and tool version                  |
| FictionBook          | FB2, FB2.ZIP, FBZ; embedded images, notes, series, multiple encodings                                                                                                                                                                                      | Structured metadata and prepared EPUB/resources, with original retained                                                                |
| Text and markup      | TXT with BOM/encoding handling and user override; Markdown with relative assets; HTML/XHTML and packaged HTML with assets                                                                                                                                  | Prepared reflowable publication with stable resource paths and safe previews                                                           |
| Reading documents    | RTF, DOCX, ODT, DjVu, CBZ/CBR-style image PDFs already covered above                                                                                                                                                                                       | Prepared EPUB for text-oriented documents; page-preserving PDF/images for fixed documents; expose conversion warnings                  |

This is the proposed launch commitment, not a claim that a chosen library already
handles every cell. Qualify the exact parser, codec, variant, and deployment image
before implementation locks its dependency set. A failing mandatory cell blocks
launch; it does not silently become a post-launch item.

Include archive encryption with user-supplied passwords, missing-volume diagnostics,
large entry counts, huge images, truncated inputs, inconsistent metadata, and
recoverable malformed content in qualification. Distinguish password encryption,
EPUB font obfuscation, and DRM. Do not claim DRM decryption, proprietary KFX
coverage, or arbitrary interactive EPUB script execution. Preserve media-overlay
and embedded audio/video information and expose capabilities honestly; synchronized
playback needs separate reader qualification. These are explicit support boundaries,
not features hidden behind a generic “supported” label.

## Architecture and deployment

Use one modular Bun/Effect application with separately supervised worker processes.
Package it as one self-hosted distribution, with API and workers independently
restartable. Linux amd64 and arm64 container images are the proposed deployment
targets; macOS is a development host. Do not require a cloud account.

Keep modules concrete: catalog, libraries/sources, ingestion, formats, delivery,
reading state, identity/access, jobs, and operations. Keep the existing workspace
and lockfile. Start these modules inside `apps/api`; a second worker entry point
can share them. Do not create packages or network services merely for symmetry.

Recommended persistence is `bun:sqlite` on local durable storage, with WAL,
foreign keys, short transactions, a bounded write queue, and `synchronous=FULL`.
Keep database work out of the HTTP event loop when it can block; batch ingestion
writes without starving progress updates. Use SQLite full-text search plus
indexed structured filters; include CJK-aware tokenization and substring search
in search qualification rather than assuming English tokenization suffices.

This recommendation assumes one active server host with many readers, not active-active
replicas. SQLite WAL has a single writer and does not support a database on a
network filesystem. Put database and hot assets on local SSD even when original
library mounts live on a NAS. Validate contention under the workload below. If
multi-host availability is a launch requirement, choose PostgreSQL through Bun.sql
before schema implementation, with shared durable content storage and a concrete
failover design. Do not build two database implementations speculatively.

Keep original files outside SQL. Support two explicit library modes:

- Managed import: copy/upload into a server-owned immutable object store, keyed by
  content hash; publish only after a complete, durable write.
- Reference library: index a read-only user folder. Detect changes and missing
  mounts; keep identity/history. Stable remote reading requires a pinned snapshot
  of the selected revision. If one cannot be made, report changed/unavailable
  content rather than serving mixed revisions across range requests.

Store rebuildable indexes, thumbnails, converted publications, and extracted pages
separately from originals and user state. Key derivatives by original hash,
processor version, and options. Retain any representation referenced by an active
reading session or offline acquisition manifest; expire it under a defined policy.

## Identity, metadata, and reader alignment

Separate these concepts so alternate editions and files do not corrupt progress:

| Entity                 | Responsibility                                                                     |
| ---------------------- | ---------------------------------------------------------------------------------- |
| Library / Source       | Ownership, root/configuration, credentials reference, scan policy, permissions     |
| Work                   | Optional grouping of the same intellectual work; no forced deduplication by title  |
| Edition / Publication  | Language, contributors, publisher, release, layout, series memberships             |
| Asset revision         | Exact immutable original bytes, checksum, size, detected format, source provenance |
| Reading representation | Original or derived rendition, resource manifest, processor version, readiness     |
| Resource / Page        | Stable logical href, media type, ordinal, dimensions, spread hints, delivery link  |
| Series / Membership    | Ordering with decimal/special chapter labels, volume/issue, multiple memberships   |
| User state             | Progress events, bookmarks, annotations, shelves, preferences and sync revisions   |

An omnibus, translated edition, alternate scan, and individual chapter must remain
distinguishable. Store external identifiers as namespaced values. Match candidates
with evidence; do not merge automatically by filename, title, or ISBN alone.
Moves and renames preserve identity; replacement bytes create a new asset revision.

Metadata precedence: user overrides, explicit sidecar metadata, embedded metadata,
source metadata, then filename/folder inference. Preserve every source and its
raw values, record provenance, and allow per-field locks. Support ComicInfo.xml,
OPF sidecars, EPUB package metadata, PDF metadata, and connector metadata. Treat
inference as editable. Preserve multilingual names, original titles, dates with
partial precision, reading direction, credits/roles, and unknown values honestly.

Align the API's publication projection with the reader's metadata, `readingOrder`,
`resources`, navigation, warnings, and link properties. Keep database schema and
server-owned IDs independent of Swift implementation types. `Publication` is not
itself an established JSON wire contract; its `Locator` is Codable.

Adopt the existing locator JSON shape now: `href`, `type`, optional `title`,
`locations` (`fragments`, `progression`, `totalProgression`, `position`), and `text`
(`before`, `highlight`, `after`). `position` is 1-based; the reader's page source
uses 0-based indices. Never silently interchange them.

Wrap a locator with publication ID, asset/representation revision, device ID,
mutation ID, and server revision. Preserve unknown extension fields for forward
compatibility. Store logical resource paths separately from authenticated URLs.
The reader's links use percent-decoded paths; define URL encoding once at delivery
and forbid traversal, ambiguous normalization, and double decoding.

Shared schema generation and full manifest unification can be decided separately.
The immediate savings come from stable identifiers, matching locator semantics,
and compatible resource paths. No reader-package refactor is required by this plan.

## Ingestion and format processing

Use a persistent state machine:

`discovered → stabilizing → identified → indexed → preparing → ready`

Expose partial readiness per capability and terminal/recoverable conditions such
as `needs-password`, `missing-volume`, `source-unavailable`, `quarantined`, and
`failed`. A catalog entry may appear before preparation finishes, but cannot claim
that a reading representation is ready when it is still being converted.

1. Discover files through an initial walk, filesystem notifications, periodic
   reconciliation, resumable browser/API upload, or a source connector.
2. Wait for stable writes or a completed upload; enforce quotas and free-space
   reserve. Sniff signatures and container structure, not just file extensions.
3. Snapshot/hash, identify duplicates, and inspect metadata/manifests with bounded
   reads. Check the file did not change during inspection.
4. Build navigation/page order and validate references. Preserve readable content
   when safe repairs are possible; attach warnings describing every repair.
5. Prepare random access, compatibility renditions, covers, small thumbnails, and
   first-reading assets. Prioritize interactive demand over bulk enrichment.
6. Atomically publish the representation and catalog changes. Schedule optional
   deeper indexing without withholding otherwise usable content.

Jobs use durable leases, heartbeats, bounded retry with jitter, deduplication keys,
cancellation, and operator-visible failure reasons. A crashed worker's lease can
be reclaimed. Assume at-least-once execution and make output publication idempotent.
Use a transactional outbox for catalog/sync notifications so commits cannot lose
their events. Reconcile orphaned temporary files and unreferenced completed blobs.

Do not rewrite every parser. Own the domain, format adapter contract, diagnostics,
and delivery pipeline; use qualified libraries/tools for commodity decoding.
Candidates: libarchive for archives, a packaged PDF engine, a packaged image
decoder/transformer, and calibre conversion workers for legacy book/document
formats. These are candidates until exact version, variant fidelity, licensing,
redistribution, architecture support, resource use, and update path are checked.
Calibre documents a conversion CLI; that alone does not prove lossless conversion.

Parsers and converters run in killable processes with CPU, memory, output-size,
wall-time, file-count, and nesting limits. They receive only the input and a scratch
directory, run without network access, and cannot access credentials or other
libraries. Process separation alone is not a sandbox: enforce OS/container
filesystem, network, and resource restrictions in the shipped worker configuration.
Reject traversal, escaping symlinks, XML entity expansion, decompression bombs,
and hostile image dimensions. One bad file must not stop a scan or the API.

## Reading delivery and latency

Provide three complementary paths: original file byte ranges, publication resources,
and ordered page images. Clients select a compatible ready representation using
explicit capabilities. Conversion is an ingestion job, not a hidden GET-side wait.

The PR's remote EPUB code starts with a suffix range, then requests ZIP entry byte
ranges. Serve exact immutable bytes with `206`, correct `Content-Range` and length,
`Accept-Ranges`, strong ETags, conditional requests, `HEAD`, and correct `416`
responses. Implement bounded, open-ended, and suffix ranges. Avoid transfer/content
transformations that change offsets. Verify behavior through the actual reverse
proxy. Apply authorization to every range and resource request.

PR #8's inspected byte source does not pin requests with `If-Range` or an ETag.
Revision-specific immutable URLs are therefore important immediately. Pass a
stable revision-aware identifier through `RemoteOptions.identifier`; do not let a
rotating signed URL become publication identity. Its whole-file fallback uses a
Data-returning loader and checks size after receipt, so server range correctness
also avoids an expensive fallback; it is not a proven client memory bound.

Stream bytes with backpressure and cancellation. No full-archive buffering on a
request. Precompute manifests, page indexes, covers, and expensive renditions;
use bounded memory/disk caches and coalesce duplicate work. Keep original images
available; use device-sized derivatives without making lossy recompression the
only way to read. Segment extremely tall images with stable coordinates.

Search and catalog endpoints use keyset pagination, indexed filters, bounded page
sizes, and summary projections. Fetch full manifests on demand and paginate very
large page/resource lists. Avoid per-item SQL and filesystem work in list requests.
Budget CPU/IO separately for interactive delivery, ingestion, and background jobs.

Proposed launch performance gates, to be measured rather than presented as results:

| Operation                                        | Target                                                                                        |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------- |
| Warm catalog/detail, progress write              | p95 ≤ 50 ms, p99 ≤ 150 ms server duration                                                     |
| Search with filters                              | p95 ≤ 100 ms, p99 ≤ 250 ms server duration                                                    |
| Prepared resource/range response                 | p95 ≤ 30 ms, p99 ≤ 100 ms to first byte at server                                             |
| First readable content, prepared typical title   | p95 ≤ 1 s on LAN; ≤ 2 s at 80 ms RTT / 20 Mbps                                                |
| Normal page advance with bounded client prefetch | p95 ≤ 150 ms on the defined WAN profile                                                       |
| Restart recovery                                 | API ready within 30 s on reference hardware; interrupted work reclaimed without manual repair |

Reference envelope: Linux, 4 modern CPU cores, 8 GiB RAM, local SSD for database
and prepared assets; 100,000 publications / 5 million indexed resources; 20 active
readers and two ingestion workers. Define “typical” as an EPUB up to 50 MiB or a
comic up to 500 MiB with a first rendered resource up to 2 MiB. Report larger files,
slow disks, cold caches, and NAS outages separately, including p99 and peak memory.
These are provisional engineering budgets, not measured capacity claims.

Measure warm and cold operation, mixed reads/writes, cache misses, conversion load,
and actual reader first paint. The cold path must remain bounded and interactive
budgets must survive background ingestion. Admission control should delay imports
before degrading reading. Metadata-only timings cannot establish reading latency.

## Library, integration, and API surface

Proposed `/api/v1` groups:

- Server/capabilities: API/schema versions, installed format capabilities, limits,
  readiness, and supported reading representations.
- Libraries/sources/imports: roots, scan/reconcile, resumable uploads, connector
  configuration, import status, retry/cancel, and diagnostics.
- Publications/series/collections: browse, search, filters, metadata edits, edition
  selection, shelves, covers, and provenance.
- Reading: manifest, original revision, resources, pages, positions where available,
  offline acquisition manifest and integrity hashes.
- User state: progress, bookmarks, annotations, reading status, device registrations,
  batch mutations, and incremental sync.
- Operations: jobs, health/readiness, storage, format support, backups, restore status.

Define schemas with the installed Effect version and derive API documentation from
them. Use typed errors with stable codes, retryability, trace IDs, and safe details.
Use optimistic revisions for metadata edits and idempotency keys for mutations.
Read state is user-private; permission checks cover thumbnails, search, downloads,
events, and resources as well as catalog entries. Support local accounts, admin
and reader roles, library grants, revocable device tokens, secure cookie sessions,
CSRF protection for the web UI, and authentication throttling.

Launch integrations: watched folders, uploads, OPDS import/acquisition and catalog
export, plus Komga/Kavita library import connectors. Verify their current API/auth,
pagination, capability, and state-transfer contracts in the connector stage before
implementation; this plan does not claim parity or bidirectional sync. Keep remote
IDs/provenance and credentials separate. A remote outage must not hide or break
already imported content. Direct client access to those servers stays independent.

Use an out-of-process extension/import protocol with version negotiation, declared
capabilities, explicit host/credential grants, cancellation and rate limits. Ship
first-party connectors through that same boundary. Installing an extension is an
explicit administrator action; arbitrary downloaded code must never run inside
the API process. Private integrations are not required for core functionality.

## Offline state and recovery

Sync uses a monotonically ordered server change cursor, tombstones, mutation IDs,
per-device sequence numbers, and retention rules. Expired cursors trigger a bounded
full resync. Retry safely after disconnects. Keep original client timestamps for
history but do not trust wall clocks to order conflicting writes.

Keep per-device last locations and a canonical resume revision. Serialize updates
with an expected revision; stale offline updates preserve their alternate location
and return a conflict rather than silently overwriting newer progress. Intentional
backward reading remains valid: never resolve progress with `max(percentage)`.
Bookmarks and annotations have independent IDs/revisions and deletion tombstones.
Do not map a locator across different editions or converted renditions by percentage
and pretend it is precise. Keep the previous location and report remapping quality.

Backups must cover catalog, user state, source configuration, secrets recovery
instructions, and managed originals. Derived assets are rebuildable. Use consistent
database snapshots and content manifests; copying an active database file alone
is insufficient. Stage schema upgrades with pre-upgrade backups and a documented
restore path. Verify restored content hashes and state on a separate instance.

Acknowledged writes must survive process crashes on functioning durable storage.
For whole-host loss, proposed defaults are off-host state backups every 15 minutes
and a state-service restore target of 30 minutes when content remains available;
full content recovery time depends on library size and backup bandwidth. Expose
backup freshness and missing originals. Zero-loss host failover requires replicated
durable infrastructure and is a separate deployment requirement to settle early.

Observe request percentiles/errors, event-loop lag, database contention, cache hit
rates, job age/failures, source availability, free disk, sync conflicts, and backup
age. Extend existing OTLP with metrics. Logs must redact credentials, signed URLs,
and reading content. Telemetry failure must not prevent reading or persistence.
Expose liveness separately from readiness and degraded optional dependencies.

## Minimal inspection webpage

Build this alongside the first working backend flow, inside the server distribution.
Use semantic HTML and locally bundled, version-pinned Oat CSS. Use Oat JS only for
its required components and small vanilla modules for fetch, uploads, and SSE.
Alpine is unnecessary unless concrete interaction complexity justifies it.
Use Oat defaults; only change brand-related variables such as `--primary`,
`--primary-foreground`, and `--ring`, with contrast checked in both themes.

The page is a real consumer of the public API, never a parallel mock data model:

- Library table with covers, titles, formats, series, languages, readiness,
  warnings, search/filter controls, and cursor navigation.
- Publication detail using definition lists and tables: metadata/provenance,
  original versus derived files, reading order, nested TOC, page previews,
  dimensions, capabilities, warnings, and download/open links.
- Import form, progress, queued/running/failed jobs, retry/cancel, source errors,
  and per-format failure counts.
- Reading-state view showing saved locators, representation revisions, conflicts,
  and sync status; expandable raw JSON for inspection.
- Operations view for latency summaries, queue age, cache/storage usage, and
  backup freshness. Keep detailed traces in the existing telemetry tools.

Use forms, tables, `<details>`, `<progress>`, `<dialog>`, and Oat components. Start
with a paginated table; do not spend time on a bespoke dashboard or design system.
Show loading, empty, unavailable, partial, and failed states. Verify keyboard use
and actual backend rendering in the Codex browser. Treat publication HTML/SVG as
untrusted: escape metadata; use an isolated, sandboxed preview origin without app
credentials, never inject book markup into the authenticated page.

## Implementation order and launch gate

These are construction stages, not reduced releases. The full matrix and gates
must pass before calling the launch product ready.

| Stage                          | Concrete output                                                                                                                  | Completion evidence                                                                                                           |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| 1. Qualification and contracts | Exact format/variant corpus and processor choices; identity/locator schemas; workload and deployment envelope                    | Every matrix cell has a reading route, known failure behavior, and qualification owner; licenses and packaged codecs checked  |
| 2. Durable end-to-end flow     | Database, immutable assets, users/permissions, jobs, upload/folder import, EPUB and CBZ delivery, Oat library/detail/import page | Real files appear, open remotely, resume, and survive restart; checksums and ranges verified                                  |
| 3. Complete format breadth     | All remaining matrix rows, variants, metadata sources, conversion and random-access preparation                                  | Representative and malformed files inspected per variant; fidelity warnings and failures visible; no advertised stub adapters |
| 4. Complete library workflows  | Search, series/editions, collections, edits, offline acquisition, full state sync, connectors/OPDS/extensions                    | Large library navigation and disconnected/reconnected device flows work with permissions enforced                             |
| 5. Launch qualification        | Performance tuning, resource isolation, backups/restores, upgrade path, packaging, full operations page                          | Mixed-load budgets, crash/restart, disk-full, damaged-input, source-outage, credential-expiry, restore, and upgrade evidence  |

Reliability work starts in stages 1–2 and is exercised throughout; stage 5 collects
the release evidence rather than adding reliability at the end. Work on format
families can proceed independently once contracts and worker boundaries are stable.

Maintain a coverage ledger with exact input variant, checksum, processor build,
metadata result, delivery result, fidelity findings, and client rendering result.
Include user-provided and legally redistributable books, not just generated happy
paths. Run existing checks and manual operational drills. Adding automated tests
or a new test harness requires the user's explicit request under repository rules;
this planning work does not authorize or add them.

First implementation task: qualify the dependencies and shape the schema, then
deliver upload/folder → durable ingest → catalog → range/resource delivery → saved
locator, visible through the Oat page. This proves the foundation while the entire
launch matrix remains mandatory. Do not commit to a calendar until the archive,
legacy conversion, deployment, and reader integration qualification is complete.

## Sources and remaining decisions

Current repository guides supply product decisions. External sources establish
specific interfaces and constraints, not Chek performance or completed support:

- [Reader PR #8](https://github.com/readchek/chek-reader/pull/8), with
  [Locator source](https://github.com/readchek/chek-reader/blob/9fe9b838e56c92f52e534fafcb8daff1b32a0bab/Sources/ChekReaderCore/Locator.swift),
  [remote byte source](https://github.com/readchek/chek-reader/blob/9fe9b838e56c92f52e534fafcb8daff1b32a0bab/Sources/ChekReaderCore/Remote/HTTPByteSource.swift),
  and [format registry](https://github.com/readchek/chek-reader/blob/9fe9b838e56c92f52e534fafcb8daff1b32a0bab/Sources/ChekReaderCore/ReaderFormat.swift).
- [EPUB 3.3](https://www.w3.org/TR/epub-33/) for publication/container semantics.
- [HTTP semantics and ranges](https://www.rfc-editor.org/rfc/rfc9110.html#name-range-requests).
- [SQLite WAL](https://www.sqlite.org/wal.html) for concurrency and deployment constraints.
- [libarchive](https://www.libarchive.org/) and
  [calibre conversion CLI](https://manual.calibre-ebook.com/generated/en/ebook-convert.html)
  as processor candidates requiring qualification.
- [ComicInfo](https://anansi-project.github.io/docs/comicinfo/documentation) and
  [OPDS](https://opds.io/) for metadata/distribution interoperability.
- [Oat](https://oat.ink/) and [theme variables](https://oat.ink/customizing/).

Decisions to settle before dependent implementation: single-host recovery versus
multi-host failover; expected real library/concurrency sizes; approval of the broad
format matrix and explicit boundaries; and which Komga/Kavita/OPDS versions and
imports are launch obligations. Recommended defaults are stated above so ordinary
implementation choices do not require repeated permission.
