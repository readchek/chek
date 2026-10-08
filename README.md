# Chek

Chek is a ground-up redesign and direct successor to Chibi Manga: a versatile
e-reader for your books and comics, wherever you keep them.

The new clients are planned to read local files in a wide range of formats and
connect to providers such as Komga, Kavita, and Chek's own server. The goal is a
clean, reliable reading experience that lets you choose your source. The first
new app is iOS, with more platforms to follow.

Chek's backend is planned as an open-source library server in its own right, with
an extension/import mechanism. Its architecture and supported formats are still
being worked out. These are product goals, not a claim that the current repository
implements them.

Project domain: https://chek.page
Repository: https://github.com/chibi-app/chek

Product scope and predecessor: [product direction](docs/product.md).
App design decisions: [palette, typography, and landing studies](docs/design.md).

## Development

This Bun monorepo contains `apps/api`, the Effect backend, and `apps/website`,
the selected Chek landing page.

Run from the repository root:

```sh
bun install
bun run dev
bun run start
bun run typecheck
bun run dev:website
bun run build:website
bun run deploy:website
```

The website preview runs at http://127.0.0.1:18357/. Its build goes to
`apps/website/dist/`. See [website notes](apps/website/README.md) for assets and
design and deployment details. Production is hosted at https://chek.page.
The landing design and existing Chek logo are selected.

The API reads `apps/api/.env` through Bun and validates configuration with Effect.
Copy `apps/api/.env.example` to `apps/api/.env` on a fresh checkout. Host and port
default independently to `127.0.0.1:8080`; invalid values fail at startup.

The example sends API logs and HTTP request traces to the Jarvis test SigNoz
collector at `http://jarvis:4318` using OTLP/HTTP. Connect to Tailscale to use it.
New runs use the service name `chek-api`; existing deployments retain their configured name.
View the configured service at `http://jarvis:8080`; the same logs are available
in LogChef at `http://jarvis:8125`.

Set `OTEL_LOGS_EXPORTER=none` or `OTEL_TRACES_EXPORTER=none` to disable a signal,
or `OTEL_SDK_DISABLED=true` to disable both. Export is off when the exporter
variables or endpoint are absent. Console logs remain available; `LOG_FORMAT`
accepts `pretty` or `json`, and `LOG_LEVEL` accepts Effect levels such as `Info`,
`Debug`, or `Error`. The API exports logs and traces only. Export is batched and
flushed during graceful shutdown. Use `OTEL_RESOURCE_ATTRIBUTES` for deployment
labels and `OTEL_EXPORTER_OTLP_HEADERS` if a future collector requires auth.
`OTEL_EXPORTER_OTLP_PROTOCOL` supports `http/protobuf` and `http/json`.

Dependencies for all workspaces use the root `bun.lock`. Add API dependencies
from `apps/api` with `bun add <package>`.

Use `bun run lint` and `bun run format:check` to check code, or `bun run lint:fix`
and `bun run format` to apply fixes and formatting. VS Code and Cursor workspace
settings enable format on save and lint fixes on save with the recommended
`oxc.oxc-vscode` extension installed.
