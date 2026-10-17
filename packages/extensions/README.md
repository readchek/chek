# Chek extensions

`@chek/extensions` is an Effect-native source SDK. `@chek/core` owns the publication
and asset contracts. Neither package depends on Bun, the API, or a workspace path.
Effect is a peer dependency; use the same Effect version throughout your app.

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
