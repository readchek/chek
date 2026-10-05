# Chek

Bun monorepo with one workspace: `apps/api`, the Effect backend.

Project domain: https://chek.page
Repository: https://github.com/chibi-app/chek

App design decisions: [starter palette and typography](docs/design.md).

Run from the repository root:

```sh
bun install
bun run dev
bun run start
bun run typecheck
```

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
