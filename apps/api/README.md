# Backend code map

Run `bun run dev` (hot reload), `bun run start`, or `bun run typecheck` from the
repository root. The default address is `http://127.0.0.1:8080`; Bun loads
`apps/api/.env`. Copy `.env.example` on a fresh checkout. Configuration and OTLP
setup are described in [`docs/development.md`](../../docs/development.md).

Start with [`src/app.ts`](src/app.ts). It composes the Effect layers; there is no
global service locator. [`src/api/api.ts`](src/api/api.ts) defines the HTTP
contracts and endpoint descriptions. The running server exposes them at
`/api/openapi.json`. `/inspect` is a small Oat UI that uses those same endpoints.

| Area                                                | Responsibility                                                              |
| --------------------------------------------------- | --------------------------------------------------------------------------- |
| `packages/core/src`                                 | Validated publication, series, chapter, page and asset models               |
| `packages/extensions/src`                           | Effect-only authoring contract, host services and process protocol          |
| `extensions/local-files`                            | Original-file discovery reference                                           |
| `extensions/json-catalog`                           | Browse/search/chapters/pages reference, independent of the backend          |
| `src/extensions/registry.ts`                        | Installed manifests and source validation                                   |
| `src/extensions/host.ts`, `process.ts`, `runner.ts` | Scoped subprocess lifetime, transport validation and extension execution    |
| `src/extensions/http.ts`, `state.ts`                | Granted HTTP access, cookies, pacing and encrypted source state             |
| `src/extensions/queries.ts`                         | Capability checks, bounded concurrency, retries and shared query cache      |
| `src/imports/worker.ts`                             | Durable scan jobs, heartbeat, import ordering and checkpoints               |
| `src/catalog/ingestion.ts`, `assets.ts`             | Validate records and atomically store immutable original bytes              |
| `src/catalog/database`                              | SQLite migrations and repositories, called only through the database worker |
| `src/handlers/source-assets.ts`                     | On-demand page fetch, image recognition and private delivery                |

## Two paths through the system

**Import:** create source → queue scan → claim leased job → extension emits
publications → store original bytes → commit publication → acknowledge checkpoint.
Stable source keys preserve catalog IDs. A failed item prevents later checkpoints
from advancing past it. Transient operation failures retry up to three attempts,
with retry times persisted in SQLite. Restarted workers reclaim expired leases.

**Browse/read:** query source → check capability → execute extension → validate
core result → replace private asset references with encrypted, expiring URLs.
The page endpoint checks those URLs and current source grants, fetches on demand,
and stores immutable bytes. The browser never receives upstream headers or paths.
Queries have four shared execution slots, 128 cached results and a 30-second TTL;
identical concurrent queries share execution. Pages use four fetch slots, a
32 MiB per-image cap and a 256-entry, ten-minute cache.

Use `Effect.fn` / `Effect.withSpan` at useful operation boundaries and
`Effect.logInfo`, `Effect.logWarning` or `Effect.logError` for structured events.
The layers in `src/observability` handle console and OTLP export. The HTTP
`x-trace-id` response header links a failure back to its server trace. Source
HTTP spans record origin, method and status without upstream URLs or headers.

## Extension authors

Read [`packages/extensions/README.md`](../../packages/extensions/README.md), then
[`extensions/json-catalog/src/index.ts`](../../extensions/json-catalog/src/index.ts).
A private extension imports only `@chek/extensions` and Effect. It can parse HTML
or call an upstream API, then return the same core models. Komga and Kavita fit
this contract as source extensions; neither has a built-in implementation here.

The runtime trusts explicitly installed code. Subprocesses contain crashes, not
malicious code. Source configuration currently resides in the private catalog
database; session state and cookies are encrypted separately. Back up
`sessions.key` with the data directory to preserve sessions. Do not log secrets.
Source page URLs expire after ten minutes; requery for fresh URLs.

File import is not document parsing. The remote image-page path is usable, but
EPUB/PDF/archive parsing and reader integration remain separate work. Browser
challenge handling and site-specific authentication belong to individual source
implementations and have not been verified against a live manga site.
