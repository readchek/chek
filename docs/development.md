# Development

Chek uses Bun workspaces and a single root `bun.lock`. Run these commands from
the repository root unless stated otherwise.

## Setup

Install [Bun](https://bun.sh), then install dependencies:

```sh
bun install
```

Start the landing page with `bun run dev:website` and open
[localhost:18357](http://127.0.0.1:18357/). See the [website guide](website.md)
for builds and deployment.

## API

Create the local configuration on a fresh checkout:

```sh
cp apps/api/.env.example apps/api/.env
```

The example uses the maintainer's private telemetry collector. Set
`OTEL_SDK_DISABLED=true` in your `.env` for local development without a collector,
or configure your own OTLP endpoint.

```sh
bun run dev
```

Use `bun run start` to run without hot reload. Bun loads `apps/api/.env` and
Effect validates its values at startup. Host and port default independently to
`127.0.0.1:8080`; invalid values fail at startup.

### Logs and traces

Console logs remain available regardless of telemetry export. `LOG_FORMAT`
accepts `pretty` or `json`; `LOG_LEVEL` accepts Effect levels such as `Info`,
`Debug`, and `Error`.

The API exports logs and HTTP request traces through OTLP/HTTP. Configure
`OTEL_EXPORTER_OTLP_ENDPOINT` and enable each signal with
`OTEL_LOGS_EXPORTER=otlp` and `OTEL_TRACES_EXPORTER=otlp`. Export is off when the
exporter or endpoint is absent. Set an exporter to `none` to disable that signal,
or `OTEL_SDK_DISABLED=true` to disable both. Export is batched and flushed on
graceful shutdown.

`OTEL_EXPORTER_OTLP_PROTOCOL` supports `http/protobuf` and `http/json`. Use
`OTEL_RESOURCE_ATTRIBUTES` for deployment labels and
`OTEL_EXPORTER_OTLP_HEADERS` for collector authentication. Signal-specific
endpoints and batch intervals are shown in
[`apps/api/.env.example`](../apps/api/.env.example).

The maintainer's test setup uses `http://jarvis:4318` over Tailscale, with service
name `chek-api`. Its SigNoz UI is at `http://jarvis:8080` and LogChef at
`http://jarvis:8125`. These addresses require access to that private network.

## Code checks

```sh
bun run typecheck
bun run lint
bun run format:check
```

Use `bun run lint:fix` and `bun run format` to apply fixes. Workspace editor
settings enable formatting and lint fixes on save with the recommended
`oxc.oxc-vscode` extension.

Add API dependencies from `apps/api` with `bun add <package>`. All workspaces
share the root lockfile.
