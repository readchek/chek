# Chek extensions

`@chek/extensions` is an Effect-native source SDK. `@chek/core` owns the publication
and asset contracts. Neither package depends on Bun, the API, or a workspace path.
Effect is a peer dependency; use the same Effect version throughout your app.

## Choose the operations your source provides

| Capability          | Return value             | Purpose                                           |
| ------------------- | ------------------------ | ------------------------------------------------- |
| `discover`          | `Stream<DiscoveryEvent>` | Import original files into the durable catalog    |
| `browse` / `search` | `Effect<SeriesPage>`     | Discover works without downloading their chapters |
| `series`            | `Effect<Series>`         | Fetch one work's metadata                         |
| `chapters`          | `Effect<ChapterList>`    | List releases, including language/group variants  |
| `pages`             | `Effect<PageList>`       | Resolve ordered image references for one chapter  |

The manifest must match the implemented handlers exactly. A browsing extension
does not need `discover`. Series and chapter keys are opaque, source-scoped and
stable. Give different releases different chapter keys. Page indices are
zero-based and strictly increasing; pagination retains those absolute indices.
Return `nextCursor: null` when finished. Never turn a title into an identity.

The complete browsing reference is `extensions/json-catalog/src/index.ts` in the
Chek repository. Its schema lives beside it in `catalog.ts`. It depends only on
this SDK and Effect and can be moved to a private repository.

```ts
import { defineExtension, ExtensionError } from "@chek/extensions"
import { Effect, Schema, Stream } from "effect"

export default defineExtension({
  manifest: {
    protocolVersion: 1,
    id: "author.library",
    name: "My library",
    version: "0.1.0",
    description: "Books from my server",
    entrypoint: "dist/index.js",
    capabilities: ["discover"],
    configFields: [
      { key: "origin", label: "Server", required: true, secret: false, grant: "http-origin" },
    ],
  },
  config: Schema.Struct({ origin: Schema.NonEmptyString }),
  validate: () => Effect.void,
  discover: ({ origin }) =>
    Stream.succeed({
      type: "publication",
      publication: {
        sourceKey: "book-123",
        title: "Example book",
        format: "epub",
        mediaType: "application/epub+zip",
        authors: [],
        asset: { kind: "http", url: `${origin}/books/123.epub` },
      },
    }),
})
```

Use `ExtensionContext` for the host's file and HTTP grants. Return `ExtensionError`
for expected failures. Use Effect services, Layers, scoped resources and Streams
inside an extension; provide its own service layers before returning the stream.
Interruption closes scoped resources. Streams apply backpressure. The SDK validates
configuration and every discovery record; the host validates the wire boundary again.

## Host services

Use `yield* SourceHttp`, then `http.request({ url, headers })` for text/JSON or
HTML. It returns status, headers, final URL and text. Decode JSON with an Effect
Schema; map parse failures to an actionable `ExtensionError`. Host HTTP requests
have a 30-second deadline, a 4 MiB response cap, five redirects, and 250 ms spacing
per origin across processes. Grant every required API/image origin explicitly.
Redirects recheck grants and drop explicit headers across origins. Cookie jars
are source- and origin-scoped; page references receive matching cookies on return.
Cookies without an expiry last 24 hours. Upstream trace propagation is disabled.

429, 5xx and network failures produce retryable errors for GET requests. Query
execution retries a transient failure twice, with backoff. POST requests are not
automatically marked retryable. Handlers must remain safe to repeat. Do not put
upstream credentials in titles, metadata, errors or logs; asset headers remain
private to the host. Expected errors should use stable machine-readable codes.

`SourceState` stores JSON under the configured source identity, encrypted at rest.
Each value is limited to 64 KiB; a source has at most 1,024 state entries. Use it
for session tokens and provider state. Validation runs with temporary state;
repeat authentication lazily when a real operation needs it. Concurrent get/set
is not an atomic update: avoid using it as a shared counter or lock.

For incremental imports, read `ExtensionContext.checkpoint`. Emit
`{ type: "checkpoint", cursor }` after a batch of publications. The host persists
the cursor after processing prior events, with the job's lease/attempt fence.
Never save an import cursor directly through `SourceState`: that could advance
before the corresponding publications are durable. Replays must use stable
publication keys. Emitting a null checkpoint resets the resume position.

## Reader API

Send `POST /api/v1/sources/:id/query`, for example:

```json
{ "operation": "chapters", "seriesKey": "work-123", "limit": 30 }
```

The response preserves the operation discriminator and wraps the result in
`data`. Asset references become server URLs that expire in ten minutes; request
the query again to refresh them. Apply the server API token to page requests too.
The server caches query results for 30 seconds and coalesces concurrent identical
requests. Full request/response schemas and endpoint descriptions are available
at `/api/openapi.json`.

Keep `sourceKey` stable. The core assigns catalog IDs and immutable asset revisions.
Optional metadata includes contributors, subjects, series, layout and reading
progression. Discovery imports original files; it does not imply format parsing.

Ship `extension.json` with the same manifest as the executable and export
`./extension.json` from the npm package. The server loads explicitly installed
packages named in `CHEK_EXTENSION_PACKAGES` (comma-separated), or extension folders
under `CHEK_EXTENSIONS_DIR`. No API checkout imports belong in an extension.
The local-file reference lives in `extensions/local-files` in the Chek repository.

`ExtensionCollection` describes an author's public or private npm releases, with
exact versions and SHA-512 integrity. The server can read collection JSON files
from `CHEK_EXTENSION_COLLECTIONS_DIR` and expose them at
`/api/v1/extension-collections`. Collections do not install code or grant trust.

Extensions are explicitly trusted code in separate processes, not OS sandboxes.
The core enforces source grants when importing assets. Never log credentials or
put them in metadata. Extension Effect logs and spans use the server's OTLP
configuration and parent trace; stdout is reserved for the host protocol.

From this repository, `bun run build:packages` emits ESM and declarations.
`bun pm pack --dry-run` within either package inspects its npm contents without
publishing. Both packages can be moved to independent repositories.
