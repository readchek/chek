# Manga API

Run from the repository root:

```bash
bun run dev
bun run start
bun run typecheck
```

The API defaults to `http://127.0.0.1:8080`. Bun loads the app's `.env`; copy
`.env.example` to `.env` on a fresh checkout. See the root README for the OTLP
settings used by the Jarvis test instance.

```text
src/
  index.ts                   Bun runtime entrypoint
  app.ts                     Application layer composition
  server.ts                  HTTP routes and Bun server
  api/api.ts                 API contract and response schemas
  handlers/basic.ts          Root and health handlers
  services/config.ts         Validated Effect configuration service
  observability/logger.ts    Console logger and log level
  observability/telemetry.ts  OTLP logs and traces
```

The contract contains no server setup and can be imported by a typed client.
Handlers translate HTTP input into calls to Effect services and return results.
Add services when there is domain behavior or a resource to manage, such as a
database or an external API. Provide their layers in `app.ts`; keep runtime
startup in `index.ts`.

Logging uses Effect's built-in logger service. Use `Effect.logInfo`,
`Effect.logError`, and `Effect.annotateLogs` in handlers and services; the logging
and telemetry layers handle console output and OTLP export. Use `Effect.fn` or
`Effect.withSpan` for meaningful operations within the automatic HTTP spans.

- `GET /health` returns `{ "status": "ok" }`.
- `GET /` returns `{ "message": "Chibi Manga API" }`.
